use super::common::common_helpers;
use super::common::ddl_introspection;
use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{SingleSqlStatement, SqlExecutor, SqlStatement};
use crate::db_first::{
    DbFirstColumn, DbFirstForeignKey, DbFirstIndex, DbFirstIndexColumn, DbFirstTable,
};
use crate::hooks::{HookContext, HookOperation};
use crate::migration::{SchemaColumn, schema_column_with_compression};
use crate::model::{DbBackendTypeMapper, Model, Value, WritableModel};
use crate::query::builder::{
    FourTableSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect, ProjectionSelect,
    RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use crate::query::insert::{
    InsertAssignment, InsertConflict, IntoInsertAssignment, IntoInsertDefaultColumn,
};
use crate::query::update::UpdateAssignment;
use crate::raw_sql::IntoRawSql;
use crate::utils::{FutureTraceExt, ResultTraceExt};
use crate::{
    impl_backend_executor_methods, impl_backend_four_table_executor_methods_with_lifetime,
    impl_backend_join_executor_methods_with_lifetime,
    impl_backend_multi_table_executor_methods_with_lifetime,
    impl_backend_related_executor_methods_with_lifetime, impl_insert_conflict_methods,
};
use chrono::{Datelike, Timelike};
use mysql_async::Pool;
use mysql_async::prelude::*;
use std::collections::BTreeMap;
use std::marker::PhantomData;

type ModelUpdateBatch = common_helpers::ModelUpdateBatch;

/// 执行器连接来源：数据库连接池、已开启事务的连接，或池化租约绑定的独占连接。
///
/// 事务连接与 pinned 连接包装在异步 Mutex 中，使事务内的 select/update/delete 执行器
/// 可以与池内执行器复用同一套构建与链式 API，但 SQL 全部落在同一条连接上。
#[derive(Clone, Copy)]
enum ExecutorConn<'a> {
    Pool(&'a Pool),
    Transaction(&'a tokio::sync::Mutex<Option<mysql_async::Conn>>),
    /// `ConnectionPool::get()` 池化租约持有的独占连接（pinned connection）：
    /// 非事务语句固定走 `conn`，实现跨语句的连接绑定；
    /// `pool` 供流式查询等必须拥有连接所有权的路径另取连接。
    Pinned {
        conn: &'a tokio::sync::Mutex<mysql_async::Conn>,
        pool: &'a Pool,
    },
}

/// 从 [`ExecutorConn`] 租借到的连接：池连接独占所有权，事务/pinned 连接持有互斥锁守卫。
enum ExecutorConnLease<'a> {
    Owned(mysql_async::Conn),
    Guard(tokio::sync::MutexGuard<'a, Option<mysql_async::Conn>>),
    PinnedGuard(tokio::sync::MutexGuard<'a, mysql_async::Conn>),
}

impl<'a> ExecutorConn<'a> {
    /// 按值获取（[`ExecutorConn`] 为 [`Copy`]），使租借的生命周期挂接到
    /// 连接引用本身而非临时的 [`ExecutorConn`] 值上
    async fn lease(self) -> crate::Result<ExecutorConnLease<'a>> {
        match self {
            ExecutorConn::Pool(pool) => {
                Ok(ExecutorConnLease::Owned(pool.get_conn().trace().await?))
            }
            ExecutorConn::Transaction(mutex) => Ok(ExecutorConnLease::Guard(mutex.lock().await)),
            ExecutorConn::Pinned { conn, .. } => {
                Ok(ExecutorConnLease::PinnedGuard(conn.lock().await))
            }
        }
    }
}

impl ExecutorConnLease<'_> {
    fn conn(&mut self) -> crate::Result<&mut mysql_async::Conn> {
        match self {
            ExecutorConnLease::Owned(conn) => Ok(conn),
            ExecutorConnLease::Guard(guard) => guard.as_mut().ok_or_else(|| {
                crate::ormer_error!("Transaction connection is unavailable")
            }),
            ExecutorConnLease::PinnedGuard(guard) => Ok(guard),
        }
    }
}

/// MySQL `dml_returning=false`（见 `Capabilities::of(DbType::MySQL)`）：统一层
/// `*Executor::returning` 已按矩阵先行拦截，此处为直接使用后端 API 的防线，
/// 文案与统一层保持一致。
fn mysql_returning_unsupported() -> crate::OrmerError {
    crate::OrmerError::UnsupportedFeature {
        backend: DbType::MySQL,
        feature: "DML RETURNING",
    }
}

/// MySQL 厂商错误码结构化提取（L10）：`Error::Server(ServerError)` 携带
/// 精确的 code（1062 唯一冲突、1213 死锁等），在 trace 包装层拍平成
/// 字符串前提取，文本启发式仅兜底。
fn mysql_server_error_code(error: &mysql_async::Error) -> Option<String> {
    match error {
        mysql_async::Error::Server(server) => Some(server.code.to_string()),
        _ => None,
    }
}

async fn traced_mysql_query(
    conn: &mut mysql_async::Conn,
    sql: &str,
) -> crate::Result<Vec<mysql_async::Row>> {
    let trace = crate::sql_trace::start_sql_trace(sql, &[]);
    match conn.query(trace.sql()).await {
        Ok(rows) => {
            trace.finish_ok();
            Ok(rows)
        }
        Err(error) => {
            let code = mysql_server_error_code(&error);
            let error = trace.finish_external_error("mysql_async::Conn::query", error);
            Err(error.with_driver_code(code))
        }
    }
}

async fn traced_mysql_query_drop(conn: &mut mysql_async::Conn, sql: &str) -> crate::Result<()> {
    let trace = crate::sql_trace::start_sql_trace(sql, &[]);
    match conn.query_drop(trace.sql()).await {
        Ok(()) => {
            trace.finish_ok();
            Ok(())
        }
        Err(error) => {
            let code = mysql_server_error_code(&error);
            let error = trace.finish_external_error("mysql_async::Conn::query_drop", error);
            Err(error.with_driver_code(code))
        }
    }
}

async fn traced_mysql_exec(
    conn: &mut mysql_async::Conn,
    sql: &str,
    params: Vec<mysql_async::Value>,
    trace_params: &[Value],
) -> crate::Result<Vec<mysql_async::Row>> {
    let trace = crate::sql_trace::start_sql_trace(sql, trace_params);
    match conn
        .exec(trace.sql(), mysql_async::Params::Positional(params))
        .await
    {
        Ok(rows) => {
            trace.finish_ok();
            Ok(rows)
        }
        Err(error) => {
            let code = mysql_server_error_code(&error);
            let error = trace.finish_external_error("mysql_async::Conn::exec", error);
            Err(error.with_driver_code(code))
        }
    }
}

async fn traced_mysql_exec_drop(
    conn: &mut mysql_async::Conn,
    sql: &str,
    params: Vec<mysql_async::Value>,
    trace_params: &[Value],
) -> crate::Result<()> {
    let trace = crate::sql_trace::start_sql_trace(sql, trace_params);
    match conn
        .exec_drop(trace.sql(), mysql_async::Params::Positional(params))
        .await
    {
        Ok(()) => {
            trace.finish_ok();
            Ok(())
        }
        Err(error) => {
            let code = mysql_server_error_code(&error);
            let error = trace.finish_external_error("mysql_async::Conn::exec_drop", error);
            Err(error.with_driver_code(code))
        }
    }
}

fn table_name_for<T: Model>() -> &'static str {
    T::table_name_for_db(DbType::MySQL)
}

/// 追加 MySQL `ON DUPLICATE KEY UPDATE` 子句，与公共层标准 upsert 语义一致：
/// 冲突更新排除主键列，避免冲突落在次级唯一键时把目标行主键覆盖为来源行的占位值。
///
/// 全部列均为主键时退化为 DO NOTHING：MySQL 没有 `ON CONFLICT DO NOTHING`，
/// 等价形式是把语句前缀 `INSERT INTO` 改写为 `INSERT IGNORE INTO`。
pub(crate) fn append_mysql_upsert_clause<T: Model>(sql: &mut String, columns: &[&str]) -> crate::Result<()> {
    let primary_key_columns = T::primary_key_columns();

    sql.push_str(" ON DUPLICATE KEY UPDATE ");
    let mut first = true;
    for column in columns {
        if primary_key_columns.contains(column) {
            continue;
        }
        if !first {
            sql.push_str(", ");
        }
        sql.push_str(&common_helpers::quote_mysql_values_assignment(
            DbType::MySQL,
            column,
        ));
        first = false;
    }

    if first {
        sql.truncate(sql.len() - " ON DUPLICATE KEY UPDATE ".len());
        if !primary_key_columns.is_empty() {
            if let Some(rest) = sql.strip_prefix("INSERT INTO") {
                *sql = format!("INSERT IGNORE INTO{rest}");
            }
        }
    }
    Ok(())
}

/// 生成自增感知的批量 upsert 语句组（公共 helper）：
/// 自增主键已设置的行携带主键冲突更新，未设置的行排除自增列插入。
pub(crate) fn build_mysql_upsert_statements<T: Model>(
    models: &[&T],
) -> crate::Result<Vec<common_helpers::UpsertSqlStatement>> {
    common_helpers::build_auto_increment_aware_upsert_statements::<T>(
        DbType::MySQL,
        "INSERT INTO",
        T::table_name_for_db(DbType::MySQL),
        models,
        |sql, columns| append_mysql_upsert_clause::<T>(sql, columns),
    )
}

fn parse_mysql_enum_variants(type_name: &str) -> Vec<String> {
    let trimmed = type_name.trim();
    if !trimmed.to_ascii_lowercase().starts_with("enum(") || !trimmed.ends_with(')') {
        return Vec::new();
    }
    let inner = &trimmed["enum(".len()..trimmed.len() - 1];
    let mut variants = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\'' {
            continue;
        }
        let mut value = String::new();
        while let Some(ch) = chars.next() {
            if ch == '\'' {
                if chars.peek() == Some(&'\'') {
                    value.push('\'');
                    chars.next();
                } else {
                    break;
                }
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    value.push(next);
                }
            } else {
                value.push(ch);
            }
        }
        variants.push(value);
    }
    variants
}

fn parse_mysql_table_compression(create_options: &str) -> Option<String> {
    let options = create_options.as_bytes();
    let lower = create_options.to_ascii_lowercase();
    let marker = "compression";
    let mut offset = 0;

    while let Some(relative) = lower[offset..].find(marker) {
        let start = offset + relative;
        let end = start + marker.len();
        let has_boundary_before =
            start == 0 || !options[start - 1].is_ascii_alphanumeric() && options[start - 1] != b'_';
        let has_boundary_after =
            end == options.len() || !options[end].is_ascii_alphanumeric() && options[end] != b'_';
        if !has_boundary_before || !has_boundary_after {
            offset = end;
            continue;
        }

        let mut value = create_options[end..].trim_start();
        if let Some(rest) = value.strip_prefix('=') {
            value = rest.trim_start();
        }
        let quote = value.chars().next();
        if matches!(quote, Some('"') | Some('\'')) {
            value = &value[1..];
        }
        let end = value
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '"' || ch == '\'')
            .unwrap_or(value.len());
        let value = value[..end].trim();
        if value.eq_ignore_ascii_case("none") {
            return None;
        }
        if !value.is_empty() {
            return Some(value.to_ascii_uppercase());
        }
        return None;
    }

    None
}

fn mysql_fk_action(action: &str) -> Option<&'static str> {
    match action.replace('_', " ").to_ascii_uppercase().as_str() {
        "NO ACTION" => Some("NO ACTION"),
        "RESTRICT" => Some("RESTRICT"),
        "CASCADE" => Some("CASCADE"),
        "SET NULL" => Some("SET NULL"),
        "SET DEFAULT" => Some("SET DEFAULT"),
        _ => None,
    }
}

fn mysql_value_from_ormer_value(value: &crate::model::Value) -> crate::Result<mysql_async::Value> {
    match value {
        crate::model::Value::Integer(v) => Ok(mysql_async::Value::Int(*v)),
        crate::model::Value::Text(v) => Ok(mysql_async::Value::Bytes(v.as_bytes().to_vec())),
        crate::model::Value::TextArray(v) => Ok(mysql_async::Value::Bytes(
            crate::model::stringify_string_vec(v).into_bytes(),
        )),
        crate::model::Value::Real(v) => Ok(mysql_async::Value::Double(*v)),
        crate::model::Value::Decimal(v) | crate::model::Value::BigDecimal(v) => {
            Ok(mysql_async::Value::Bytes(v.clone().into_bytes()))
        }
        crate::model::Value::Boolean(v) => Ok(mysql_async::Value::Int(if *v { 1 } else { 0 })),
        crate::model::Value::Duration(v) => Ok(mysql_async::Value::Int(
            v.as_micros().min(i64::MAX as u128) as i64,
        )),
        crate::model::Value::Bytes(v) => Ok(mysql_async::Value::Bytes(v.clone())),
        crate::model::Value::DateTime(v) => Ok(mysql_async::Value::Date(
            v.year() as u16,
            v.month() as u8,
            v.day() as u8,
            v.hour() as u8,
            v.minute() as u8,
            v.second() as u8,
            v.timestamp_subsec_micros(),
        )),
        crate::model::Value::Date(v) => Ok(mysql_async::Value::Date(
            v.year() as u16,
            v.month() as u8,
            v.day() as u8,
            0,
            0,
            0,
            0,
        )),
        crate::model::Value::Time(v) => Ok(mysql_async::Value::Time(
            false,
            0,
            v.hour() as u8,
            v.minute() as u8,
            v.second() as u8,
            v.nanosecond() / 1_000,
        )),
        crate::model::Value::Json(v) => Ok(mysql_async::Value::Bytes(v.to_string().into_bytes())),
        crate::model::Value::Uuid(v) => Ok(mysql_async::Value::Bytes(v.to_string().into_bytes())),
        crate::model::Value::BigInt(v) => match i64::try_from(*v) {
            Ok(v) => Ok(mysql_async::Value::Int(v)),
            Err(_) => Err(crate::OrmerError::decode(format!(
                "BigInt value {v} out of range for MySQL BIGINT"
            ))),
        },
        crate::model::Value::IntegerArray(_)
        | crate::model::Value::BigIntArray(_)
        | crate::model::Value::NullableBigIntArray(_) => Err(
            common_helpers::unsupported_postgresql_array_value(DbType::MySQL),
        ),
        crate::model::Value::Null => Ok(mysql_async::Value::NULL),
    }
}

/// MySQL 类型映射器
pub struct MySQLTypeMapper;

impl DbBackendTypeMapper for MySQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        // MySQL 支持 ENUM 类型
        if let Some(variants) = enum_variants {
            let variants_str = variants
                .iter()
                .map(|v| format!("'{}'", v))
                .collect::<Vec<_>>()
                .join(", ");
            return common_helpers::sql_type_with_nullability(
                &format!("ENUM({})", variants_str),
                is_nullable,
            );
        }

        // 基础类型映射（主键与非主键共用同一映射，保证迁移自检一致：
        // i16 主键 → SMALLINT、String 主键 → VARCHAR(255) 等）
        let base_type = match rust_type {
            // 整数类型
            "i8" => "TINYINT",
            "i16" => "SMALLINT",
            "i32" => "INT",
            "i64" => "BIGINT",
            "isize" => "BIGINT",
            // 无符号整数
            "u8" => "TINYINT UNSIGNED",
            "u16" => "SMALLINT UNSIGNED",
            "u32" => "INT UNSIGNED",
            "u64" => "BIGINT UNSIGNED",
            "usize" => "BIGINT UNSIGNED",
            // 浮点类型
            "f32" => "FLOAT",
            "f64" => "DOUBLE",
            "Decimal" | "rust_decimal::Decimal" | "BigDecimal" | "bigdecimal::BigDecimal" => {
                "DECIMAL(65,30)"
            }
            // 时长类型
            "Duration" | "std::time::Duration" => "BIGINT",
            // 字符串类型
            "String" => "VARCHAR(255)",
            // UUID 使用规范连字符字符串存储
            "Uuid" | "uuid::Uuid" => "CHAR(36)",
            // 布尔类型
            "bool" => "TINYINT(1)",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BLOB",
            // 日期时间类型：DATETIME(6) 保留微秒，避免读写往返被四舍五入截断
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => "DATETIME(6)",
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            // JSON 类型
            "JsonValue" | "serde_json::Value" => "JSON",
            // 默认使用 TEXT
            _ => "TEXT",
        };

        // 主键分支只负责拼 PRIMARY KEY / AUTO_INCREMENT 后缀
        if is_primary {
            if is_auto_increment {
                return format!("{base_type} PRIMARY KEY AUTO_INCREMENT");
            }
            return format!("{base_type} PRIMARY KEY");
        }

        common_helpers::sql_type_with_nullability(base_type, is_nullable)
    }
}

/// MySQL 数据库连接封装
pub struct Database {
    pool: Pool,
    /// 池化租约绑定的独占连接（pinned connection）：
    ///
    /// `Some` 时非事务语句全部固定走这条连接（跨语句 pin，`max_size` 对
    /// 池化用法生效）；`Drop` 时连接随 [`mysql_async::Conn`] 的 Drop 送回
    /// 池回收器归池（处于事务中的连接会被先回滚再归池）。
    /// `None` 时（直连 / `from_pool`）每条语句各自从池中取连接。
    pinned: Option<tokio::sync::Mutex<mysql_async::Conn>>,
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: crate::model::WritableModel> {
    pool: ExecutorConn<'a>,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> CreateTableExecutor<'a, T> {
    pub fn with_table_name(mut self, table_name: &str) -> Self {
        self.table_name = Some(table_name.to_string());
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::MySQL,
            self.table_name.as_deref(),
        )?;
        Ok(SqlStatement::single(DbType::MySQL, create_sql, Vec::new()))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: crate::model::WritableModel> SqlExecutor for CreateTableExecutor<'a, T> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        CreateTableExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut lease = self.pool.lease().await?;
        for statement in sql.statements {
            traced_mysql_query_drop(lease.conn()?, &statement.sql).await?;
        }
        Ok(())
    }
}

/// 删除表执行器
pub struct DropTableExecutor<'a, T: crate::model::WritableModel> {
    pool: ExecutorConn<'a>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> DropTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        Ok(SqlStatement::single(
            DbType::MySQL,
            format!(
                "DROP TABLE IF EXISTS {}",
                common_helpers::quote_table_name::<T>(DbType::MySQL)
            ),
            Vec::new(),
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: crate::model::WritableModel> SqlExecutor for DropTableExecutor<'a, T> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DropTableExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut lease = self.pool.lease().await?;
        for statement in sql.statements {
            traced_mysql_query_drop(lease.conn()?, &statement.sql).await?;
        }
        Ok(())
    }
}

/// 清空表执行器：生成 `TRUNCATE TABLE t`。
pub struct TruncateTableExecutor<'a, T: crate::model::WritableModel> {
    pool: ExecutorConn<'a>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> TruncateTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        Ok(SqlStatement::single(
            DbType::MySQL,
            format!(
                "TRUNCATE TABLE {}",
                common_helpers::quote_table_name::<T>(DbType::MySQL)
            ),
            Vec::new(),
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: crate::model::WritableModel> SqlExecutor for TruncateTableExecutor<'a, T> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        TruncateTableExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut lease = self.pool.lease().await?;
        for statement in sql.statements {
            traced_mysql_query_drop(lease.conn()?, &statement.sql).await?;
        }
        Ok(())
    }
}

/// MySQL 插入语句渲染（执行器与连接池共用入口，R7/L12）：按绑定参数上限
/// 分块的 VALUES 语句组，conflict 子句由公共 helper 追加。
pub(crate) fn mysql_insert_to_sql<M: Model>(
    refs: &[&M],
    conflict: Option<&InsertConflict>,
) -> crate::Result<SqlStatement> {
    if refs.is_empty() {
        return Ok(SqlStatement::batch(DbType::MySQL, Vec::new()));
    }

    let statements = common_helpers::build_insert_statements_with_conflict::<M>(
        DbType::MySQL,
        refs,
        conflict,
    )?;

    Ok(SqlStatement::batch(
        DbType::MySQL,
        statements
            .into_iter()
            .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
            .collect(),
    ))
}

/// MySQL upsert（`ON DUPLICATE KEY UPDATE`）语句渲染（执行器与连接池共用）：
/// 自增感知 + 按绑定参数上限分块，全主键模型退化为 INSERT IGNORE。
pub(crate) fn mysql_insert_or_update_to_sql<M: Model>(
    refs: &[&M],
) -> crate::Result<SqlStatement> {
    if refs.is_empty() {
        return Ok(SqlStatement::batch(DbType::MySQL, Vec::new()));
    }

    let statements = build_mysql_upsert_statements::<M>(refs)?;

    Ok(SqlStatement::batch(
        DbType::MySQL,
        statements
            .into_iter()
            .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
            .collect(),
    ))
}

/// MySQL `INSERT IGNORE` 语句渲染（执行器与连接池共用）：写入全部列
/// （含主键），按全列数分块。
pub(crate) fn mysql_insert_or_ignore_to_sql<M: Model>(
    refs: &[&M],
) -> crate::Result<SqlStatement> {
    if refs.is_empty() {
        return Ok(SqlStatement::batch(DbType::MySQL, Vec::new()));
    }

    let columns = M::columns();
    let statements = common_helpers::build_chunked_insert_statements_for_columns::<M>(
        DbType::MySQL,
        columns.len(),
        refs,
        |chunk| {
            let (sql, params) = common_helpers::build_batch_insert_statement::<M>(
                DbType::MySQL,
                "INSERT IGNORE INTO",
                M::table_name_for_db(DbType::MySQL),
                &columns,
                chunk,
                common_helpers::BatchInsertValuesMode::All,
            );
            Ok(common_helpers::InsertSqlStatement {
                sql,
                params,
                row_count: chunk.len(),
            })
        },
    )?;

    Ok(SqlStatement::batch(
        DbType::MySQL,
        statements
            .into_iter()
            .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
            .collect(),
    ))
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    pool: ExecutorConn<'a>,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<I::Model>,
}

impl_insert_conflict_methods!(InsertExecutor, with_conflict);

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        mysql_insert_to_sql::<I::Model>(&self.models.as_refs(), self.conflict.as_ref())
    }

    /// 执行插入并返回自增主键值。
    ///
    /// 返回值约定：仅单行插入时返回的 id 语义可靠（该行的自增 id）；批量插入
    /// 多行（含按参数上限分块）时返回最后一条语句的 `last_insert_id`（末块
    /// 首行），仅供诊断，调用方不应依赖——各后端批量插入返回的 id 选取不一致
    /// （PostgreSQL 取 RETURNING 首行、SQLite 取 `last_insert_rowid`）。
    pub async fn execute(self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(self) -> crate::Result<Vec<I::Model>> {
        Err(mysql_returning_unsupported())
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertExecutor<'a, I> {
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let mut lease = self.pool.lease().await?;
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params)
                .await?;
        }

        // AutoIncrementKeyType 回填约定：批量（含分块）插入时取末块
        // last_insert_id，语义不可靠；单行插入不受影响。
        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let result = if has_auto_increment {
            let last_id = lease.conn()?.last_insert_id().unwrap_or(0);
            common_helpers::convert_auto_increment_key::<Self::Output>(last_id)
        } else {
            Ok(<I::Model as Model>::AutoIncrementKeyType::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

pub struct InsertPartialExecutor<'a, T: Model> {
    db: &'a Database,
    assignments: Vec<InsertAssignment>,
    source_table: Option<&'static str>,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> InsertPartialExecutor<'a, T> {
    fn with_assignments(mut self, assignments: Vec<InsertAssignment>) -> Self {
        self.assignments.extend(assignments);
        self
    }

    fn with_source_table(mut self, source_table: &'static str) -> Self {
        self.source_table = Some(source_table);
        self
    }

    pub fn set<F, A>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> A,
        A: IntoInsertAssignment<T>,
    {
        self.assignments
            .push(f(T::Where::default()).into_insert_assignment());
        self
    }

    pub fn default<F, C>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> C,
        C: IntoInsertDefaultColumn<T>,
    {
        self.assignments.push(InsertAssignment::default(
            f(T::Where::default()).into_insert_default_column(),
        ));
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        common_helpers::validate_insert_model_table::<T>(DbType::MySQL, self.source_table)?;
        let statement =
            common_helpers::build_partial_insert_statement::<T>(DbType::MySQL, &self.assignments)?;
        Ok(SqlStatement::single(
            DbType::MySQL,
            statement.sql,
            statement.params,
        ))
    }

    pub async fn execute(self) -> crate::Result<<T as Model>::AutoIncrementKeyType>
    where
        T: Send + Sync,
    {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: Model + Send + Sync> SqlExecutor for InsertPartialExecutor<'a, T> {
    type Output = <T as Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertPartialExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(<T as Model>::AutoIncrementKeyType::default());
        }

        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;
        let mut lease = self.db.executor_conn().lease().await?;
        traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params).await?;

        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            let last_id = lease.conn()?.last_insert_id().unwrap_or(0);
            common_helpers::convert_auto_increment_key::<Self::Output>(last_id)
        } else {
            Ok(<T as Model>::AutoIncrementKeyType::default())
        }
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pool: ExecutorConn<'a>,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        // 原生 ON DUPLICATE KEY upsert（与事务版语义一致）：
        // 冲突更新排除主键列，全主键模型退化为 INSERT IGNORE
        mysql_insert_or_update_to_sql::<I::Model>(&self.models.as_refs())
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrUpdateExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let mut lease = self.pool.lease().await?;
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params)
                .await?;
        }
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pool: ExecutorConn<'a>,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        // MySQL 的 INSERT IGNORE 写入全部列（含主键），按全列数分块
        mysql_insert_or_ignore_to_sql::<I::Model>(&self.models.as_refs())
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrIgnoreExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let mut lease = self.pool.lease().await?;
        // 分块语句逐条执行（to_sql 可能产出多块）
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params)
                .await?;
        }
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl Database {
    /// 连接到 MySQL 数据库
    pub async fn connect(_db_type: super::DbType, connection_string: &str) -> crate::Result<Self> {
        // 解析连接字符串
        let opts = mysql_async::Opts::from_url(connection_string)
            .trace_for("mysql_async::Opts::from_url")?;

        let pool = Pool::new(opts);

        Ok(Self {
            pool,
            pinned: None,
        })
    }

    pub(crate) async fn db_first_tables(
        &self,
        schema: Option<&str>,
    ) -> crate::Result<Vec<DbFirstTable>> {
        let mut lease = self.executor_conn().lease().await?;
        let rows: Vec<mysql_async::Row> =
            if let Some(schema) = schema.filter(|value| !value.is_empty()) {
                lease
                    .conn()?
                    .exec(
                        "SELECT TABLE_SCHEMA, TABLE_NAME \
                 FROM information_schema.tables \
                 WHERE TABLE_TYPE = 'BASE TABLE' \
                   AND TABLE_SCHEMA = ? \
                   AND TABLE_NAME != ? \
                 ORDER BY TABLE_NAME",
                        (schema, crate::migration::MIGRATION_TABLE_NAME),
                    )
                    .trace()
                    .await?
            } else {
                lease
                    .conn()?
                    .query(format!(
                        "SELECT TABLE_SCHEMA, TABLE_NAME \
                     FROM information_schema.tables \
                     WHERE TABLE_TYPE = 'BASE TABLE' \
                       AND TABLE_SCHEMA = DATABASE() \
                       AND TABLE_NAME != '{}' \
                     ORDER BY TABLE_NAME",
                        crate::migration::MIGRATION_TABLE_NAME.replace('\'', "''")
                    ))
                    .trace()
                    .await?
            };
        // 释放连接租约后再递归拉取列/索引/外键：
        // 池模式下避免嵌套占用多条连接（max_size=1 时死锁），
        // pinned 模式下释放互斥锁避免自锁
        drop(lease);

        let mut tables = Vec::with_capacity(rows.len());
        for row in rows {
            let schema_name: String = row.get(0).unwrap_or_default();
            let table_name: String = row.get(1).unwrap_or_default();
            let columns = self.db_first_columns(&schema_name, &table_name).await?;
            let indexes = self.db_first_indexes(&schema_name, &table_name).await?;
            let foreign_keys = self
                .db_first_foreign_keys(&schema_name, &table_name)
                .await?;
            tables.push(DbFirstTable {
                schema: Some(schema_name),
                name: table_name,
                columns,
                indexes,
                foreign_keys,
            });
        }
        Ok(tables)
    }

    async fn db_first_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstColumn>> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let rows: Vec<mysql_async::Row> = conn
            .exec(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, EXTRA, COLUMN_DEFAULT \
                 FROM information_schema.columns \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (schema_name, table_name),
            )
            .trace()
            .await?;
        let columns = rows
            .into_iter()
            .map(|row| {
                let name: String = row.get(0).unwrap_or_default();
                let type_name: String = row.get(1).unwrap_or_default();
                let nullable: String = row.get(2).unwrap_or_default();
                let key: String = row.get(3).unwrap_or_default();
                let extra: String = row.get(4).unwrap_or_default();
                let default: Option<String> = row.get(5).unwrap_or(None);
                DbFirstColumn {
                    name,
                    type_name: type_name.clone(),
                    nullable: nullable == "YES",
                    primary_key: key == "PRI",
                    auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
                    enum_variants: parse_mysql_enum_variants(&type_name),
                    default,
                }
            })
            .collect();
        Ok(columns)
    }

    async fn db_first_indexes(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstIndex>> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let rows: Vec<mysql_async::Row> = conn
            .exec(
                "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME, COLLATION \
                 FROM information_schema.statistics \
                 WHERE TABLE_SCHEMA = ? \
                   AND TABLE_NAME = ? \
                   AND INDEX_NAME != 'PRIMARY' \
                 ORDER BY INDEX_NAME, SEQ_IN_INDEX",
                (schema_name, table_name),
            )
            .trace()
            .await?;
        let mut indexes = BTreeMap::<String, DbFirstIndex>::new();
        for row in rows {
            let name: String = row.get(0).unwrap_or_default();
            let non_unique: u64 = row.get(1).unwrap_or(1);
            let column: String = row.get(2).unwrap_or_default();
            let collation: String = row.get(3).unwrap_or_default();
            indexes
                .entry(name.clone())
                .or_insert_with(|| DbFirstIndex {
                    name,
                    columns: Vec::new(),
                    unique: non_unique == 0,
                })
                .columns
                .push(DbFirstIndexColumn {
                    name: column,
                    descending: collation == "D",
                });
        }
        Ok(indexes.into_values().collect())
    }

    async fn db_first_foreign_keys(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstForeignKey>> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let rows: Vec<mysql_async::Row> = conn
            .exec(
                "SELECT kcu.CONSTRAINT_NAME, kcu.COLUMN_NAME, \
                        kcu.REFERENCED_TABLE_SCHEMA, kcu.REFERENCED_TABLE_NAME, \
                        kcu.REFERENCED_COLUMN_NAME, rc.DELETE_RULE, rc.UPDATE_RULE \
                 FROM information_schema.key_column_usage kcu \
                 JOIN information_schema.referential_constraints rc \
                   ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
                  AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
                  AND rc.TABLE_NAME = kcu.TABLE_NAME \
                 WHERE kcu.TABLE_SCHEMA = ? \
                   AND kcu.TABLE_NAME = ? \
                   AND kcu.REFERENCED_TABLE_NAME IS NOT NULL \
                 ORDER BY kcu.CONSTRAINT_NAME, kcu.ORDINAL_POSITION",
                (schema_name, table_name),
            )
            .trace()
            .await?;
        let mut foreign_keys = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.get(0).unwrap_or_default();
            let column: String = row.get(1).unwrap_or_default();
            let ref_schema: String = row.get(2).unwrap_or_default();
            let ref_table: String = row.get(3).unwrap_or_default();
            let ref_column: String = row.get(4).unwrap_or_default();
            let on_delete: String = row.get(5).unwrap_or_default();
            let on_update: String = row.get(6).unwrap_or_default();
            foreign_keys.push(DbFirstForeignKey {
                name: Some(name),
                column,
                ref_schema: Some(ref_schema),
                ref_table,
                ref_column,
                on_delete: mysql_fk_action(&on_delete).map(str::to_string),
                on_update: mysql_fk_action(&on_update).map(str::to_string),
            });
        }
        Ok(foreign_keys)
    }

    /// 从已有的 mysql_async Pool 创建 Database
    ///
    /// mysql_async::Pool 本身就是连接池，内部使用 Arc 管理，clone 是轻量操作。
    pub fn from_pool(pool: Pool) -> Self {
        Self {
            pool,
            pinned: None,
        }
    }

    /// 从池中取出的独占连接创建 Database（pinned connection 模式）
    ///
    /// `conn` 应来自 `pool.get_conn()`；此后非事务语句固定走这条连接，
    /// `Database` Drop 时连接随 [`mysql_async::Conn`] 的 Drop 自动归还池。
    pub fn from_conn(pool: Pool, conn: mysql_async::Conn) -> Self {
        Self {
            pool,
            pinned: Some(tokio::sync::Mutex::new(conn)),
        }
    }

    /// 当前连接来源：pinned 独占连接（若有），否则连接池
    fn executor_conn(&self) -> ExecutorConn<'_> {
        match &self.pinned {
            Some(conn) => ExecutorConn::Pinned {
                conn,
                pool: &self.pool,
            },
            None => ExecutorConn::Pool(&self.pool),
        }
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            pool: self.executor_conn(),
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        let mut lease = self.executor_conn().lease().await?;

        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>(lease.conn()?).trace().await?;

        if !table_exists {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Table does not exist",
                T::TABLE_NAME
            ));
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>(lease.conn()?).await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(
        &self,
        conn: &mut mysql_async::Conn,
    ) -> crate::Result<bool> {
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?";

        let result: Option<u64> = conn
            .exec_first(sql, (table_name_for::<T>(),))
            .trace()
            .await?;

        Ok(result.unwrap_or(0) > 0)
    }

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(
        &self,
        conn: &mut mysql_async::Conn,
    ) -> crate::Result<()> {
        let table_options: Option<String> = conn
            .exec_first(
                "SELECT CREATE_OPTIONS FROM INFORMATION_SCHEMA.TABLES \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
                (table_name_for::<T>(),),
            )
            .trace()
            .await?;
        let actual_compression = table_options
            .as_deref()
            .and_then(parse_mysql_table_compression);

        // 查询表的列信息
        let sql = r#"
            SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, EXTRA
            FROM INFORMATION_SCHEMA.COLUMNS
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
            ORDER BY ORDINAL_POSITION
        "#;

        let rows: Vec<mysql_async::Row> = conn.exec(sql, (table_name_for::<T>(),)).trace().await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool, bool)> = Vec::new();
        for row in rows {
            let name: String = row.get(0).unwrap_or_default();
            let col_type: String = row.get(1).unwrap_or_default();
            let is_nullable: String = row.get(2).unwrap_or_default();
            let key: String = row.get(3).unwrap_or_default();
            let extra: String = row.get(4).unwrap_or_default();
            actual_columns.push((
                name,
                col_type,
                is_nullable == "YES",
                key.eq_ignore_ascii_case("PRI"),
                extra
                    .split(',')
                    .any(|value| value.trim().eq_ignore_ascii_case("auto_increment")),
            ));
        }

        // 比较列数量
        if actual_columns.len() != T::COLUMNS.len() {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Column count mismatch: expected {}, but actual is {}",
                T::TABLE_NAME,
                T::COLUMNS.len(),
                actual_columns.len()
            ));
        }

        // 比较每一列的定义
        for (i, expected_col) in T::COLUMN_SCHEMA.iter().enumerate() {
            if i >= actual_columns.len() {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Missing column: {}",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            let (actual_name, actual_type, actual_nullable, actual_primary, actual_auto_increment) =
                &actual_columns[i];

            // 检查列名
            if actual_name != expected_col.name {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column name mismatch at position {}: expected '{}', but actual is '{}'",
                    T::TABLE_NAME,
                    i,
                    expected_col.name,
                    actual_name
                ));
            }

            if expected_col.is_primary != *actual_primary {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Primary key mismatch for '{}': expected {}primary key, but actual is {}primary key",
                    T::TABLE_NAME,
                    expected_col.name,
                    if expected_col.is_primary { "" } else { "not " },
                    if *actual_primary { "" } else { "not " }
                ));
            }

            if expected_col.is_auto_increment != *actual_auto_increment {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Auto-increment mismatch for '{}': expected {}, but actual is {}",
                    T::TABLE_NAME,
                    expected_col.name,
                    expected_col.is_auto_increment,
                    actual_auto_increment
                ));
            }

            // 检查列类型（只比较基础类型，不包含约束）
            let effective_rust_type = expected_col.data_type.unwrap_or(expected_col.rust_type);
            let expected_type = crate::abstract_layer::DbType::MySQL.sql_type(
                effective_rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
            );

            // 对于类型比较，统一按非主键映射提取基础类型（主键与非主键共用同一
            // 映射，保证与建表 DDL、迁移自检一致），再去掉约束后缀
            let full_type = crate::abstract_layer::DbType::MySQL.sql_type(
                effective_rust_type,
                false,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
            );
            let type_to_compare = full_type.replace(" NOT NULL", "");

            if !self.types_compatible(actual_type, &type_to_compare) {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column type mismatch for '{}': expected '{expected_type}', but actual is '{actual_type}'",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            // 检查 NOT NULL 约束（主键列除外，因为主键自动 NOT NULL）
            if !expected_col.is_primary {
                let expected_nullable = expected_col.is_nullable;
                if *actual_nullable != expected_nullable {
                    return Err(crate::ormer_error!(
                        "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                        T::TABLE_NAME,
                        expected_col.name,
                        if expected_nullable { "" } else { "NOT " },
                        if *actual_nullable { "" } else { "NOT " }
                    ));
                }
            }
        }

        let expected_compression = crate::model::table_compression_algorithm::<T>()?;
        let expected_compression_name = expected_compression.map(|value| value.as_upper_str());
        if expected_compression_name != actual_compression.as_deref() {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Compression mismatch: expected {}, but actual is {}",
                T::TABLE_NAME,
                expected_compression_name.unwrap_or("NONE"),
                actual_compression.as_deref().unwrap_or("NONE")
            ));
        }

        let table_name = table_name_for::<T>();
        let actual_table = self
            .db_first_tables(None)
            .await?
            .into_iter()
            .find(|table| table.name == table_name)
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Table metadata is unavailable",
                    T::TABLE_NAME
                )
            })?;
        crate::db_first::validate_model_constraints::<T>(
            crate::abstract_layer::DbType::MySQL,
            &actual_table,
        )?;
        Ok(())
    }

    /// 检查 SQL 类型是否兼容
    fn types_compatible(&self, actual: &str, expected: &str) -> bool {
        // 标准化类型名称
        fn normalize(s: &str) -> String {
            let upper = s.to_uppercase();
            // 提取基础类型名（去掉括号内的参数）
            let base_type = if let Some(pos) = upper.find('(') {
                &upper[..pos]
            } else {
                &upper[..]
            };

            match base_type {
                // 整数类型
                "TINYINT" => "TINYINT".to_string(),
                "SMALLINT" => "SMALLINT".to_string(),
                "MEDIUMINT" => "MEDIUMINT".to_string(),
                "INT" | "INTEGER" => "INT".to_string(),
                "BIGINT" => "BIGINT".to_string(),
                // 无符号整数
                t if t.ends_with(" UNSIGNED") => {
                    let unsigned_type = t.replace(" ", "");
                    match unsigned_type.as_str() {
                        "TINYINTUNSIGNED" => "TINYINT UNSIGNED".to_string(),
                        "SMALLINTUNSIGNED" => "SMALLINT UNSIGNED".to_string(),
                        "MEDIUMINTUNSIGNED" => "MEDIUMINT UNSIGNED".to_string(),
                        "INTUNSIGNED" | "INTEGERUNSIGNED" => "INT UNSIGNED".to_string(),
                        "BIGINTUNSIGNED" => "BIGINT UNSIGNED".to_string(),
                        _ => t.to_string(),
                    }
                }
                // 浮点类型
                "FLOAT" => "FLOAT".to_string(),
                "DOUBLE" | "DOUBLEPRECISION" => "DOUBLE".to_string(),
                // 保留 CHAR/VARCHAR 长度，UUID 的 CHAR(36) 不能与其他文本类型等价
                "VARCHAR" | "CHAR" => upper,
                "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" => "TEXT".to_string(),
                // 布尔类型（MySQL 使用 TINYINT(1) 存储布尔值）
                "BOOL" | "BOOLEAN" => "TINYINT".to_string(),
                // 字节类型
                "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "VARBINARY" | "BINARY" => {
                    "BLOB".to_string()
                }
                // 其他
                _ => base_type.to_string(),
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            pool: self.executor_conn(),
            models,
            conflict: None,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn insert_partial<T: WritableModel>(&self) -> InsertPartialExecutor<'_, T> {
        InsertPartialExecutor {
            db: self,
            assignments: Vec::new(),
            source_table: None,
            _marker: PhantomData,
        }
    }

    pub fn insert_model<T>(
        &self,
        model: impl crate::model::InsertModel<T>,
    ) -> InsertPartialExecutor<'_, T>
    where
        T: WritableModel,
    {
        self.insert_partial::<T>()
            .with_source_table(model.insert_table_name())
            .with_assignments(model.insert_assignments())
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        InsertOrUpdateExecutor {
            pool: self.executor_conn(),
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        InsertOrIgnoreExecutor {
            pool: self.executor_conn(),
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let mut lease = self.executor_conn().lease().await?;

        // 构建自增感知的批量 upsert 语句组：冲突更新排除主键列，
        // 全主键模型退化为 INSERT IGNORE
        let statements = build_mysql_upsert_statements::<T>(models)?;
        for statement in &statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params)
                .await?;
        }

        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略；按全列数分块）
    pub async fn insert_or_ignore_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let mut lease = self.executor_conn().lease().await?;

        // 构建批量插入或忽略的 SQL: INSERT IGNORE INTO table (cols) VALUES (...), (...)
        // MySQL 的 INSERT IGNORE 写入全部列（含主键），按全列数分块
        let columns = T::columns();
        let statements = common_helpers::build_chunked_insert_statements_for_columns::<T>(
            DbType::MySQL,
            columns.len(),
            models,
            |chunk| {
                let (sql, params) = common_helpers::build_batch_insert_statement::<T>(
                    DbType::MySQL,
                    "INSERT IGNORE INTO",
                    T::table_name_for_db(DbType::MySQL),
                    &columns,
                    chunk,
                    common_helpers::BatchInsertValuesMode::All,
                );
                Ok(common_helpers::InsertSqlStatement {
                    sql,
                    params,
                    row_count: chunk.len(),
                })
            },
        )?;
        for statement in statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, params, &statement.params)
                .await?;
        }

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: self.executor_conn(),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        ProjectionSelectExecutor {
            select: ProjectionSelect::<T, V>::new(),
            pool: self.executor_conn(),
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            pool: self.executor_conn(),
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            pool: self.executor_conn(),
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<R>(),
            pool: self.executor_conn(),
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<Transaction<'_>> {
        self.begin_with_opts(Default::default()).await
    }

    /// 开始事务并应用事务选项。
    ///
    /// MySQL 的 `SET TRANSACTION`（不带 SESSION/GLOBAL）只对"下一个事务"生效，
    /// 因此选项必须在 `START TRANSACTION` 之前下发到同一连接上。
    ///
    /// 事务始终使用从池中另取的专用连接（与 PostgreSQL 池化模式一致），
    /// 不复用 pinned 连接，事务期间其他语句仍固定走绑定的连接。
    pub async fn begin_with_opts(
        &self,
        options: crate::abstract_layer::TransactionOptions,
    ) -> crate::Result<Transaction<'_>> {
        let mut conn = self.pool.get_conn().trace().await?;

        if let Some(isolation) = options.isolation {
            let sql = format!(
                "SET TRANSACTION ISOLATION LEVEL {}",
                super::common::isolation_level_sql(isolation)
            );
            traced_mysql_query_drop(&mut conn, &sql).await?;
        }
        if options.read_only {
            traced_mysql_query_drop(&mut conn, "SET TRANSACTION READ ONLY").await?;
        }

        traced_mysql_query_drop(&mut conn, "START TRANSACTION").await?;

        Ok(Transaction {
            conn: tokio::sync::Mutex::new(Some(conn)),
            state: common_helpers::TransactionState::Active,
            _marker: std::marker::PhantomData,
        })
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            pool: self.executor_conn(),
            _marker: std::marker::PhantomData,
        }
    }

    /// 清空表数据 - 返回执行器（`TRUNCATE TABLE`）
    pub fn truncate_table<T: WritableModel>(&self) -> TruncateTableExecutor<'_, T> {
        TruncateTableExecutor {
            pool: self.executor_conn(),
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(DbType::MySQL)?;
        self.exec_raw(&sql, params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let mysql_params = values_to_params(&params)?;
        let rows: Vec<mysql_async::Row> = if mysql_params.is_empty() {
            traced_mysql_query(conn, sql).await?
        } else {
            traced_mysql_exec(conn, sql, mysql_params, &params).await?
        };

        let mut results = Vec::new();
        for row in rows {
            results.push(common_helpers::decode_row_values_from_indexed_values(
                row.columns_ref().len(),
                |i| convert_mysql_value(&row, i),
            )?);
        }
        Ok(results.into_iter().collect())
    }

    pub(crate) async fn exec_raw(&self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let mysql_params = values_to_params(&params)?;
        if mysql_params.is_empty() {
            traced_mysql_query_drop(conn, sql).await?;
        } else {
            traced_mysql_exec_drop(conn, sql, mysql_params, &params).await?;
        }
        Ok(conn.affected_rows())
    }

    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<(u64, String, u64)>> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let rows: Vec<mysql_async::Row> = conn
            .query("SELECT version, name, checksum FROM __ormer_migrations ORDER BY version")
            .trace()
            .await?;
        rows.into_iter()
            .map(|row| {
                let version = row
                    .get::<u64, _>(0)
                    .ok_or_else(|| crate::ormer_error!("Migration version is NULL"))?;
                let name = row.get::<String, _>(1).unwrap_or_default();
                let checksum = row
                    .get::<String, _>(2)
                    .ok_or_else(|| crate::ormer_error!("Migration checksum is NULL"))?
                    .parse::<u64>()
                    .map_err(|_| crate::ormer_error!("Migration checksum is invalid"))?;
                Ok((version, name, checksum))
            })
            .collect()
    }

    pub(crate) async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<SchemaColumn>>> {
        let mut lease = self.executor_conn().lease().await?;
        let conn = lease.conn()?;
        let exists: Option<u64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?",
                (table_name,),
            )
            .trace()
            .await?;
        if exists.unwrap_or(0) == 0 {
            return Ok(None);
        }
        let rows: Vec<mysql_async::Row> = conn
            .exec(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY \
                 FROM information_schema.columns \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (table_name,),
            )
            .trace()
            .await?;
        let create_options: Option<String> = conn
            .exec_first(
                "SELECT CREATE_OPTIONS FROM INFORMATION_SCHEMA.TABLES \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
                (table_name,),
            )
            .trace()
            .await?;
        let table_compression = create_options
            .as_deref()
            .and_then(parse_mysql_table_compression);
        let columns = rows
            .into_iter()
            .map(|row| {
                let name: String = row.get(0).unwrap_or_default();
                let type_name: String = row.get(1).unwrap_or_default();
                let nullable: String = row.get(2).unwrap_or_default();
                let key: String = row.get(3).unwrap_or_default();
                schema_column_with_compression(
                    name,
                    type_name,
                    nullable == "YES",
                    key == "PRI",
                    table_compression.clone(),
                )
            })
            .collect();
        Ok(Some(columns))
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        let Ok(mut lease) = self.executor_conn().lease().await else {
            return false;
        };
        let Ok(conn) = lease.conn() else {
            return false;
        };
        traced_mysql_query_drop(conn, "SELECT 1").await.is_ok()
    }
}

/// MySQL 事务对象
pub struct Transaction<'a> {
    conn: tokio::sync::Mutex<Option<mysql_async::Conn>>,
    state: common_helpers::TransactionState,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Drop for Transaction<'a> {
    fn drop(&mut self) {
        if !self.state.is_active() {
            return;
        }

        self.state = common_helpers::TransactionState::RolledBack;
        if let Some(mut conn) = self.conn.get_mut().take() {
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.spawn(async move {
                        if let Err(err) = traced_mysql_query_drop(&mut conn, "ROLLBACK").await {
                            eprintln!("[ormer] failed to roll back abandoned MySQL transaction: {err}");
                        }
                        // conn 随任务 Drop 归还池：若仍在事务中，mysql_async
                        // 回收器会先回滚清理再归池（dirty connection 路径）
                    });
                }
                Err(_) => {
                    // 不在 tokio 运行时上下文：无法下发 ROLLBACK，直接丢弃连接，
                    // 绝不在 Drop 中 panic。连接若归还池，mysql_async 回收器同样会
                    // 清理残留事务；若运行时已关闭，连接随驱动任务取消而断开，
                    // 服务端会随 socket 关闭回滚该事务。
                    eprintln!(
                        "[ormer] MySQL transaction dropped outside a tokio runtime; \
                         rollback is left to the connection pool recycler"
                    );
                }
            }
        }
    }
}

/// 事务中的插入执行器 - 现在持有连接的引用而不是所有权  
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    conn: &'a mut Option<mysql_async::Conn>,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl_insert_conflict_methods!(TransactionInsertExecutor);

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MySQL, Vec::new()));
        }
        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::MySQL,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::batch(
            DbType::MySQL,
            statements
                .into_iter()
                .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
                .collect(),
        ))
    }

    /// 执行插入并返回自增主键值。
    ///
    /// 返回值约定：仅单行插入时返回的 id 语义可靠（该行的自增 id）；批量插入
    /// 多行（含按参数上限分块）时返回最后一条语句的 `last_insert_id`（末块
    /// 首行），仅供诊断，调用方不应依赖——各后端批量插入返回的 id 选取不一致
    /// （PostgreSQL 取 RETURNING 首行、SQLite 取 `last_insert_rowid`）。
    pub async fn execute(self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor
    for TransactionInsertExecutor<'a, I>
{
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        TransactionInsertExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| {
                crate::ormer_error!("Transaction already committed or rolled back")
            })?;
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(conn, &statement.sql, params, &statement.params).await?;
        }

        // AutoIncrementKeyType 回填约定：批量（含分块）插入时取末块
        // last_insert_id，语义不可靠；单行插入不受影响。
        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let result = if has_auto_increment {
            let last_id = conn.last_insert_id().unwrap_or(0);
            common_helpers::convert_auto_increment_key::<
                <I::Model as Model>::AutoIncrementKeyType,
            >(last_id)
        } else {
            Ok(<<I::Model as Model>::AutoIncrementKeyType>::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

/// 事务中的插入或更新执行器 - 现在持有连接的引用而不是所有权  
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    conn: &'a mut Option<mysql_async::Conn>,
    models: I,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MySQL, Vec::new()));
        }
        // 原生 ON DUPLICATE KEY upsert（与非事务版语义一致）：
        // 冲突更新排除主键列，全主键模型退化为 INSERT IGNORE
        let statements = build_mysql_upsert_statements::<I::Model>(&refs)?;

        Ok(SqlStatement::batch(
            DbType::MySQL,
            statements
                .into_iter()
                .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
                .collect(),
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor
    for TransactionInsertOrUpdateExecutor<'a, I>
{
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        TransactionInsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;

        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| {
                crate::ormer_error!("Transaction already committed or rolled back")
            })?;
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(conn, &statement.sql, params, &statement.params).await?;
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl<'a> Transaction<'a> {
    /// Insert one row and read it back in the same transaction.
    ///
    /// This is a two-statement MySQL compatibility helper, not SQL RETURNING.
    pub async fn insert_returning<I>(&mut self, mut models: I) -> crate::Result<Vec<I::Model>>
    where
        I: crate::model::Insertable + Send + Sync,
        I::Model: crate::model::FromRowValues,
    {
        if models.as_refs().len() != 1 {
            return Err(crate::ormer_error!(
                "MySQL insert_returning supports exactly one model"
            ));
        }

        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        models.run_before_insert(hook_ctx.clone()).await?;
        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::MySQL,
            &models.as_refs(),
            None,
        )?;

        let conn = self
            .conn
            .get_mut()
            .as_mut()
            .ok_or_else(|| crate::ormer_error!("Transaction connection is unavailable"))?;
        let mut last_id = 0;
        for statement in &statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(conn, &statement.sql, params, &statement.params).await?;
            last_id = conn.last_insert_id().unwrap_or(last_id);
        }

        let auto_increment_pk = I::Model::COLUMN_SCHEMA
            .iter()
            .find(|column| column.is_primary && column.is_auto_increment)
            .map(|column| column.name);
        if let (Some(column), false) = (auto_increment_pk, last_id == 0) {
            for model in models.as_refs_mut() {
                model.assign_column_value(column, Value::Integer(last_id as i64))?;
            }
        }

        let model_refs = models.as_refs();
        let model = model_refs
            .first()
            .ok_or_else(|| crate::ormer_error!("MySQL insert_returning model is unavailable"))?;
        let filters = common_helpers::model_primary_key_filters(*model);
        let Some(filter) = common_helpers::and_filter_exprs(filters) else {
            return Err(crate::ormer_error!(
                "MySQL insert_returning requires a primary key"
            ));
        };

        let mut sql = format!(
            "SELECT {} FROM {}",
            common_helpers::quote_column_list(DbType::MySQL, &I::Model::columns()),
            common_helpers::quote_table_name::<I::Model>(DbType::MySQL)
        );
        let mut params = Vec::new();
        let mut param_idx = 1;
        common_helpers::format_filter_with_params(
            &filter,
            &mut sql,
            &mut param_idx,
            &mut params,
            DbType::MySQL,
        )?;
        let result = self
            .select_raw::<I::Model, Vec<I::Model>>(&sql, params)
            .await?;
        models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }

    /// Update one model by primary key and read the row back in the transaction.
    pub async fn update_model_returning<T>(&mut self, model: &T) -> crate::Result<Option<T>>
    where
        T: WritableModel + crate::model::FromRowValues + Sync,
    {
        let Some(plan) = common_helpers::model_update_plan(model, None) else {
            return self.select_by_primary_key(model).await;
        };
        let statement = common_helpers::build_model_update_sql::<T>(DbType::MySQL, &plan)?;
        self.exec_raw(&statement.sql, statement.params).await?;
        self.select_by_primary_key(model).await
    }

    /// Delete one model by primary key and return the row read before deletion.
    pub async fn delete_model_returning<T>(&mut self, model: &T) -> crate::Result<Option<T>>
    where
        T: WritableModel + crate::model::FromRowValues + Sync,
    {
        let existing = self.select_by_primary_key(model).await?;
        if existing.is_none() {
            return Ok(None);
        }
        let filters = common_helpers::model_delete_filters(model);
        let (sql, params) = common_helpers::build_delete_sql::<T>(DbType::MySQL, &filters)?;
        self.exec_raw(&sql, params).await?;
        Ok(existing)
    }

    async fn select_by_primary_key<T>(&mut self, model: &T) -> crate::Result<Option<T>>
    where
        T: Model + crate::model::FromRowValues + Sync,
    {
        let Some(filter) =
            common_helpers::and_filter_exprs(common_helpers::model_primary_key_filters(model))
        else {
            return Err(crate::ormer_error!(
                "MySQL returning helpers require a primary key"
            ));
        };

        let mut sql = format!(
            "SELECT {} FROM {}",
            common_helpers::quote_column_list(DbType::MySQL, &T::columns()),
            common_helpers::quote_table_name::<T>(DbType::MySQL)
        );
        let mut params = Vec::new();
        let mut param_idx = 1;
        common_helpers::format_filter_with_params(
            &filter,
            &mut sql,
            &mut param_idx,
            &mut params,
            DbType::MySQL,
        )?;
        Ok(self
            .select_raw::<T, Vec<T>>(&sql, params)
            .await?
            .into_iter()
            .next())
    }

    pub(crate) async fn exec_raw(&mut self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let conn = self
            .conn
            .get_mut()
            .as_mut()
            .ok_or_else(|| crate::ormer_error!("Transaction connection is unavailable"))?;
        let mysql_params = values_to_params(&params)?;
        if mysql_params.is_empty() {
            traced_mysql_query_drop(conn, sql).await?;
        } else {
            traced_mysql_exec_drop(conn, sql, mysql_params, &params).await?;
        }
        Ok(conn.affected_rows())
    }

    /// 接收器为 `&self`（R3：raw select 三路径合一，统一层 ConnRef 持共享
    /// 引用）：事务由单一所有者持有，`lock().await` 无争用，与原
    /// `&mut self + get_mut()` 路径行为等价。
    pub(crate) async fn select_raw<V, C>(
        &self,
        sql: &str,
        params: Vec<Value>,
    ) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let mut conn_guard = self.conn.lock().await;
        let conn = conn_guard
            .as_mut()
            .ok_or_else(|| crate::ormer_error!("Transaction connection is unavailable"))?;
        let mysql_params = values_to_params(&params)?;
        let rows: Vec<mysql_async::Row> = if mysql_params.is_empty() {
            traced_mysql_query(conn, sql).await?
        } else {
            traced_mysql_exec(conn, sql, mysql_params, &params).await?
        };
        drop(conn_guard);

        let mut results = Vec::new();
        for row in rows {
            results.push(common_helpers::decode_row_values_from_indexed_values(
                row.columns_ref().len(),
                |i| convert_mysql_value(&row, i),
            )?);
        }
        Ok(results.into_iter().collect())
    }

    /// 提交事务
    pub async fn commit(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        if let Some(mut conn) = self.conn.get_mut().take() {
            traced_mysql_query_drop(&mut conn, "COMMIT").await?;
        }
        self.state = common_helpers::TransactionState::Committed;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        if let Some(mut conn) = self.conn.get_mut().take() {
            traced_mysql_query_drop(&mut conn, "ROLLBACK").await?;
        }
        self.state = common_helpers::TransactionState::RolledBack;
        Ok(())
    }

    /// 关闭并回滚事务
    pub async fn close(self) -> crate::Result<()> {
        self.rollback().await
    }

    /// 创建 Select 查询执行器（查询在事务连接上执行）
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: ExecutorConn::Transaction(&self.conn),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器（查询在事务连接上执行）
    pub fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        ProjectionSelectExecutor {
            select: ProjectionSelect::<T, V>::new(),
            pool: ExecutorConn::Transaction(&self.conn),
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器（删除在事务连接上执行）
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            pool: ExecutorConn::Transaction(&self.conn),
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器（更新在事务连接上执行）
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            pool: ExecutorConn::Transaction(&self.conn),
            _marker: PhantomData,
        }
    }

    /// 插入记录 - 返回执行器  
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            conn: self.conn.get_mut(),
            models,
            conflict: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器  
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        TransactionInsertOrUpdateExecutor {
            conn: self.conn.get_mut(),
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        TransactionInsertOrIgnoreExecutor {
            conn: self.conn.get_mut(),
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&mut self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        // 构建自增感知的批量 upsert 语句组：冲突更新排除主键列，
        // 全主键模型退化为 INSERT IGNORE
        let statements = build_mysql_upsert_statements::<T>(models)?;

        let conn = self
            .conn
            .get_mut()
            .as_mut()
            .ok_or_else(|| {
                crate::ormer_error!("Transaction already committed or rolled back")
            })?;
        for statement in &statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(conn, &statement.sql, params, &statement.params).await?;
        }

        Ok(())
    }
}

/// 事务中的插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    conn: &'a mut Option<mysql_async::Conn>,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        // MySQL 的 INSERT IGNORE 写入全部列（含主键），按全列数分块
        // （与非事务执行器共用同一渲染入口）
        mysql_insert_or_ignore_to_sql::<I::Model>(&self.models.as_refs())
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor
    for TransactionInsertOrIgnoreExecutor<'a, I>
{
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        TransactionInsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;

        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| {
                crate::ormer_error!("Transaction already committed or rolled back")
            })?;
        // 分块语句逐条执行（to_sql 可能产出多块）
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(conn, &statement.sql, params, &statement.params).await?;
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, J)>,
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<T>,
}

/// Projection 查询执行器（字段投影与分组聚合合一）
pub struct ProjectionSelectExecutor<'a, T: Model, V> {
    select: ProjectionSelect<T, V>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, V)>,
}

#[deprecated(
    since = "0.2.12",
    note = "MappedSelectExecutor 已合并为 ProjectionSelectExecutor，请改用 ProjectionSelectExecutor"
)]
pub type MappedSelectExecutor<'a, T, V> = ProjectionSelectExecutor<'a, T, V>;

#[deprecated(
    since = "0.2.12",
    note = "GroupedSelectExecutor 已合并为 ProjectionSelectExecutor，请改用 ProjectionSelectExecutor"
)]
pub type GroupedSelectExecutor<'a, T, V> = ProjectionSelectExecutor<'a, T, V>;

impl<'a, T: Model, V> ProjectionSelectExecutor<'a, T, V> {
    /// 生成子查询SQL和参数
    pub fn to_subquery_sql(&self) -> crate::Result<(String, Vec<crate::model::Value>)> {
        self.select.try_to_sql_with_params(DbType::MySQL)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> ProjectionCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        ProjectionCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }

    pub fn as_model<R: Model>(self) -> crate::query::builder::DerivedSelect<R>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.select.as_model::<R>()
    }

    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        Self {
            select: self.select.group_by(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 HAVING 条件
    pub fn having<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        Self {
            select: self.select.having(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F, W>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

/// Projection 收集 Future（字段投影与分组聚合合一）
pub struct ProjectionCollectFuture<'a, T: Model, V, C> {
    executor: ProjectionSelectExecutor<'a, T, V>,
    _marker: PhantomData<(T, V, C)>,
}

#[deprecated(
    since = "0.2.12",
    note = "MappedCollectFuture 已合并为 ProjectionCollectFuture，请改用 ProjectionCollectFuture"
)]
pub type MappedCollectFuture<'a, T, V, C> = ProjectionCollectFuture<'a, T, V, C>;

#[deprecated(
    since = "0.2.12",
    note = "GroupedCollectFuture 已合并为 ProjectionCollectFuture，请改用 ProjectionCollectFuture"
)]
pub type GroupedCollectFuture<'a, T, V, C> = ProjectionCollectFuture<'a, T, V, C>;

impl<
    'a,
    T: Model + 'static + Send,
    V: crate::model::FromRowValues + 'static + Send,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for ProjectionCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            // 分组聚合路径与原 GroupedCollectFuture 一致不做前置校验；
            // 无分组路径与原 MappedCollectFuture 一致走 try_ 校验。
            let (sql, params) = if self.executor.select.is_grouped() {
                self.executor.select.to_sql_with_params(DbType::MySQL)
            } else {
                self.executor.select.try_to_sql_with_params(DbType::MySQL)?
            };
            let mut lease = self.executor.pool.lease().await?;

            // 将ormer::Value转换为mysql_async::Params
            let mysql_params = values_to_params(&params)?;

            let rows: Vec<mysql_async::Row> = if mysql_params.is_empty() {
                traced_mysql_query(lease.conn()?, &sql).await?
            } else {
                traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?
            };

            let mut results = Vec::new();
            for row in rows {
                let v = common_helpers::decode_row_values_from_indexed_values(
                    row.columns_ref().len(),
                    |i| convert_mysql_value(&row, i),
                )?;
                results.push(v);
            }

            Ok(results.into_iter().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{mysql_value_from_ormer_value, parse_mysql_table_compression};
    use crate::abstract_layer::DbType;
    use crate::{OrmerError, Value};

    #[test]
    fn parses_mysql_table_compression_options() {
        assert_eq!(
            parse_mysql_table_compression("COMPRESSION=\"lz4\""),
            Some("LZ4".to_string())
        );
        assert_eq!(
            parse_mysql_table_compression("ROW_FORMAT=COMPRESSED COMPRESSION='zlib'"),
            Some("ZLIB".to_string())
        );
        assert_eq!(parse_mysql_table_compression("COMPRESSION=none"), None);
        assert_eq!(parse_mysql_table_compression("ROW_FORMAT=DYNAMIC"), None);
    }

    #[test]
    fn mysql_rejects_postgresql_array_values_without_panic() {
        let error = mysql_value_from_ormer_value(&Value::IntegerArray(vec![1, 2]))
            .expect_err("MySQL must reject PostgreSQL array values");
        assert!(matches!(
            error,
            OrmerError::UnsupportedFeature {
                backend: DbType::MySQL,
                feature: "PostgreSQL array values",
            }
        ));
    }
}

impl_backend_executor_methods!(SelectExecutor, pool, &'a Pool, Select);

impl<'a, T: Model> SelectExecutor<'a, T> {
    pub(crate) fn select_model<R: Model>(&self) -> SelectExecutor<'a, R> {
        SelectExecutor {
            select: Select::new().with_context_filters(self.select.context_filters()),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join::<J>(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join::<J>(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join::<J>(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn left_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join_derived::<J>(derived, f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn inner_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join_derived::<J>(derived, f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn right_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join_derived::<J>(derived, f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 映射查询结果到自定义类型
    pub fn map_to<F, M>(self, f: F) -> ProjectionSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        let mapped_select = self.select.map_to(f);
        ProjectionSelectExecutor {
            select: mapped_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 忽略指定字段，查询时用默认常量替代真实列值
    pub fn ignore<F, M>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        Self {
            select: self.select.ignore(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> ProjectionSelectExecutor<'a, T, V>
    where
        F: FnOnce(T::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        let grouped_select = self.select.select_column(f);
        ProjectionSelectExecutor {
            select: grouped_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }

    /// 执行查询并返回第一条记录
    pub fn first(self) -> FirstFuture<'a, T> {
        FirstFuture { executor: self }
    }

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
    {
        let aggregate_select = self.select.count(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.sum(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.avg(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.max(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.min(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询
    /// select::<User>().from::<User, Role>()
    pub fn from<R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    {
        RelatedSelectExecutor {
            select: self.select.from::<R>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<R1, R2>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    {
        FourTableSelectExecutor {
            select: self.select.from4::<R1, R2, R3>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<C>,
}

/// First future for单条记录查询
pub struct FirstFuture<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model + 'static + Send, R: crate::model::FromValue + 'static + Send>
    std::future::IntoFuture for AggregateFuture<'a, T, R>
{
    type Output = crate::Result<R>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.aggregate_select.try_to_sql_with_params(DbType::MySQL)?;

            // 将ormer::Value转换为mysql_async Value
            let mut lease = self.pool.lease().await?;

            // 构建参数
            let mysql_params: Vec<mysql_async::Value> = params
                .into_iter()
                .map(|v| mysql_value_from_ormer_value(&v))
                .collect::<crate::Result<_>>()?;

            let mut exec_result = lease.conn()?.exec_iter(&sql, mysql_params).trace().await?;

            if let Some(row) = exec_result.next().trace().await? {
                // 获取第一个列的值
                let value: Option<mysql_async::Value> = row.get(0);
                let value = value.unwrap_or(mysql_async::Value::NULL);

                // 将mysql_async::Value转换为ormer::Value
                let ormer_value = match value {
                    mysql_async::Value::Int(i) => crate::model::Value::Integer(i),
                    mysql_async::Value::UInt(u) => crate::model::Value::Integer(u as i64),
                    mysql_async::Value::Float(f) => crate::model::Value::Real(f as f64),
                    mysql_async::Value::Double(d) => crate::model::Value::Real(d),
                    mysql_async::Value::Bytes(b) => {
                        // 尝试将字节解析为数值（MySQL聚合函数可能返回字符串形式的数值）
                        if let Ok(s) = String::from_utf8(b.clone()) {
                            // 尝试解析为整数
                            if let Ok(i) = s.parse::<i64>() {
                                crate::model::Value::Integer(i)
                            } else if let Ok(f) = s.parse::<f64>() {
                                // 尝试解析为浮点数
                                crate::model::Value::Real(f)
                            } else {
                                // 作为文本处理
                                crate::model::Value::Text(s)
                            }
                        } else {
                            crate::model::Value::Null
                        }
                    }
                    mysql_async::Value::Date(_, _, _, _, _, _, _)
                    | mysql_async::Value::Time(_, _, _, _, _, _) => crate::model::Value::Null,
                    mysql_async::Value::NULL => crate::model::Value::Null,
                };

                // 使用 FromValue 转换为目标类型
                R::from_value(&ormer_value)
            } else {
                // 如果没有结果，返回 NULL 的转换
                R::from_value(&crate::model::Value::Null)
            }
        })
    }
}

impl<'a, T: Model + 'static + Send, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<'a, T, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model + 'static + Send + std::marker::Sync> std::future::IntoFuture
    for FirstFuture<'a, T>
{
    type Output = crate::Result<Option<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<T> = self.executor.collect_inner().await?;
            Ok(results.into_iter().next())
        })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;

        let mut lease = self.pool.lease().await?;

        let mysql_params = values_to_params(&params)?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();

        for row in rows {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                convert_mysql_model_value::<T>(&row, i)
            })?;
            results.push(model);
        }

        Ok(results.into_iter().collect())
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;
        Ok(SqlStatement::single(DbType::MySQL, sql, params))
    }
}

/// Delete 执行器
pub struct DeleteExecutor<'a, T: Model> {
    filters: Vec<FilterExpr>,
    versioned: bool,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> DeleteExecutor<'a, T> {
    /// 添加 WHERE 条件
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        let where_obj = T::Where::default();
        let expr = crate::query::filter::FilterExpr::from(f(where_obj).into());
        self.filters.push(expr);
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let (sql, params) = self.build_sql_with_params();
        Ok(SqlStatement::batch(
            DbType::MySQL,
            vec![SingleSqlStatement::new(sql, params).with_optimistic_lock(self.versioned, None)],
        ))
    }

    pub fn model(mut self, model: &T) -> Self {
        self.filters
            .extend(common_helpers::model_delete_filters(model));
        self.versioned = T::version_info().is_some();
        self
    }

    /// 执行删除操作并返回影响的行数
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(self) -> crate::Result<Vec<T>> {
        Err(mysql_returning_unsupported())
    }

    fn build_sql_with_params(&self) -> (String, Vec<Value>) {
        common_helpers::build_delete_sql::<T>(DbType::MySQL, &self.filters)
            .unwrap_or_else(|err| panic!("Failed to build delete SQL: {}", err))
    }
}

impl<'a, T: Model> SqlExecutor for DeleteExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(0);
        }

        let statement = &sql.statements[0];
        let mysql_params = values_to_params(&statement.params)?;
        let mut lease = self.pool.lease().await?;
        traced_mysql_exec_drop(lease.conn()?, &statement.sql, mysql_params, &statement.params).await?;
        let affected = lease.conn()?.affected_rows();
        if statement.versioned && affected == 0 {
            return Err(common_helpers::optimistic_lock_conflict::<T>());
        }
        Ok(affected)
    }
}

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for DeleteExecutor<'a, T> {
    type Output = crate::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<UpdateAssignment>,
    filters: Vec<FilterExpr>,
    model_updates: ModelUpdateBatch,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> UpdateExecutor<'a, T> {
    /// 添加 WHERE 条件
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        let where_obj = T::Where::default();
        let expr = crate::query::filter::FilterExpr::from(f(where_obj).into());
        self.filters.push(expr);
        self
    }

    /// 设置要更新的字段
    pub fn set<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut T::Update),
    {
        let mut update = T::Update::default();
        f(&mut update);
        self.sets
            .extend(<T::Update as crate::query::update::UpdateFields>::assignments(&update));
        self
    }

    /// 从模型实例设置所有非主键字段，并自动添加主键作为 WHERE 条件
    ///
    /// ```ignore
    /// let user = User { id: 1, name: "Bob".into(), age: 25, email: Some("bob@test.com".into()) };
    /// db.update::<User>().set_model(&user).execute().await?;
    /// ```
    pub fn set_model(mut self, model: &T) -> Self {
        if let Some(plan) = common_helpers::model_update_plan(model, None) {
            self.model_updates.push(plan);
        }
        self
    }

    pub fn set_model_fields(mut self, model: &T, fields: &[String]) -> Self {
        if let Some(plan) = common_helpers::model_update_plan(model, Some(fields)) {
            self.model_updates.push(plan);
        }
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let statements = self.build_all_sql()?;
        Ok(SqlStatement::batch(
            DbType::MySQL,
            statements
                .into_iter()
                .map(|statement| {
                    SingleSqlStatement::new(statement.sql, statement.params)
                        .with_optimistic_lock(statement.versioned, statement.version_update)
                })
                .collect(),
        ))
    }

    /// 执行更新操作
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(self) -> crate::Result<Vec<T>> {
        Err(mysql_returning_unsupported())
    }

    fn build_all_sql(&self) -> crate::Result<Vec<common_helpers::ModelSqlStatement>> {
        let mut statements = Vec::new();

        // Base UPDATE from sets/filters
        if !self.sets.is_empty() || (self.model_updates.is_empty() && !self.filters.is_empty()) {
            let (sql, params) =
                common_helpers::build_update_sql::<T>(DbType::MySQL, &self.sets, &self.filters)?;
            statements.push(common_helpers::ModelSqlStatement {
                sql,
                params,
                versioned: false,
                version_update: None,
                param_columns: None,
            });
        }

        if let Some(batch_statements) = common_helpers::build_bulk_model_update_statements::<T>(
            DbType::MySQL,
            &self.model_updates,
        )? {
            statements.extend(batch_statements);
        } else {
            for plan in &self.model_updates {
                statements.push(common_helpers::build_model_update_sql::<T>(
                    DbType::MySQL,
                    plan,
                )?);
            }
        }

        Ok(statements)
    }
}

impl<'a, T: Model> SqlExecutor for UpdateExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        UpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut lease = self.pool.lease().await?;
        let mut total: u64 = 0;
        for statement in &sql.statements {
            let mysql_params = values_to_params(&statement.params)?;
            traced_mysql_exec_drop(lease.conn()?, &statement.sql, mysql_params, &statement.params)
                .await?;
            let affected = lease.conn()?.affected_rows();
            if statement.versioned && affected == 0 {
                return Err(common_helpers::optimistic_lock_conflict::<T>());
            }
            if affected > 0 {
                if let Some(update) = &statement.version_update {
                    update.apply();
                }
            }
            total += affected;
        }
        Ok(total)
    }
}

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for UpdateExecutor<'a, T> {
    type Output = crate::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 mysql_async 参数
fn values_to_params(values: &[crate::model::Value]) -> crate::Result<Vec<mysql_async::Value>> {
    let mut params: Vec<mysql_async::Value> = Vec::new();

    for value in values {
        params.push(mysql_value_from_ormer_value(value)?);
    }

    Ok(params)
}

/// Related 查询执行器（支持2表关联查询）
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, R)>,
}

/// SelectStream - 流式查询执行器 (MySQL)
pub struct SelectStream<'a, T: Model> {
    select: Select<T>,
    pool: ExecutorConn<'a>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            select: self.select,
            pool: self.pool,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器 (真正的流式查询)
    ///
    /// 使用 mysql_async 的 Query::stream() 实现真正的流式查询，
    /// 逐行读取数据而不是一次性加载所有结果到内存中。
    pub async fn into_iter(self) -> crate::Result<SelectStreamIterator<'a, T>> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;

        // 将参数转换为 mysql_async::Value
        let mysql_params = values_to_params(&params)?;

        // 流拥有连接所有权，无法借用事务连接，事务内流式查询不支持；
        // pinned 模式下绑定连接被 Mutex 共享，同样无法移交所有权，
        // 从来源池另取一条连接给流专用（流结束时随 Conn Drop 归池）
        let pool = match self.pool {
            ExecutorConn::Pool(pool) | ExecutorConn::Pinned { pool, .. } => pool,
            ExecutorConn::Transaction(_) => {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: DbType::MySQL,
                    feature: "stream queries inside a transaction",
                });
            }
        };
        let conn = pool.get_conn().trace().await?;

        // 使用 Query::stream() 实现真正的流式查询
        // 该方法返回 'static 的流，流拥有连接的所有权
        use mysql_async::prelude::Query;
        let stream = sql
            .with(mysql_params)
            .stream::<mysql_async::Row, _>(conn)
            .trace()
            .await?;

        Ok(SelectStreamIterator {
            stream: Some(stream),
            _marker: std::marker::PhantomData,
        })
    }
}

/// 将 MySQL Row 解析为 Model
fn parse_mysql_row<T: Model>(row: &mysql_async::Row) -> crate::Result<T> {
    common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
        convert_mysql_model_value::<T>(row, i)
    })
}

/// SelectStreamIterator - 真正的流式查询迭代器 (MySQL)
///
/// 使用 mysql_async 的 Query::stream() 实现真正的流式查询，
/// 逐行读取数据而不是一次性加载所有结果到内存中。
///
/// 该流拥有连接的所有权，当流被消费完毕或丢弃时，
/// 连接会自动释放回连接池。
pub struct SelectStreamIterator<'a, T: Model> {
    stream: Option<
        mysql_async::ResultSetStream<
            'static,
            'static,
            'static,
            mysql_async::Row,
            mysql_async::BinaryProtocol,
        >,
    >,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据 (真正的流式查询)
    ///
    /// 逐行从数据库中读取数据，内存占用为 O(1)。
    pub async fn next(&mut self) -> Option<crate::Result<T>> {
        use futures::StreamExt;

        let stream = self.stream.as_mut()?;

        match stream.next().await {
            Some(Ok(row)) => {
                // 解析行数据
                match parse_mysql_row::<T>(&row) {
                    Ok(model) => Some(Ok(model)),
                    Err(e) => Some(Err(e)),
                }
            }
            Some(Err(e)) => Some(Err(crate::ormer_error!(
                "mysql_async::ResultSetStream::next failed: {e}"
            ))),
            None => None,
        }
    }
}

impl_backend_related_executor_methods_with_lifetime!(
    RelatedSelectExecutor,
    pool,
    &'a Pool,
    RelatedSelect
);

impl<'a, T: Model, R: Model> RelatedSelectExecutor<'a, T, R> {
    pub async fn collect<C: FromIterator<T>>(self) -> crate::Result<C> {
        let results = self.collect_inner().trace().await?;
        Ok(results.into_iter().collect())
    }

    pub(crate) fn into_collect_future(self) -> RelatedCollectFuture<'a, T, R> {
        RelatedCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;

        let mysql_params = values_to_params(&params)?;

        let mut lease = self.pool.lease().await?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                convert_mysql_model_value::<T>(&row, i)
            })?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct RelatedCollectFuture<'a, T: Model, R: Model> {
    executor: RelatedSelectExecutor<'a, T, R>,
}

impl<'a, T: Model + 'static + Send, R: Model + 'static + Send> std::future::IntoFuture
    for RelatedCollectFuture<'a, T, R>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// Multi-Table 查询执行器（支持3表关联查询）
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, R1, R2)>,
}

impl_backend_multi_table_executor_methods_with_lifetime!(
    MultiTableSelectExecutor,
    pool,
    &'a Pool,
    MultiTableSelect
);

impl<'a, T: Model, R1: Model, R2: Model> MultiTableSelectExecutor<'a, T, R1, R2> {
    /// 多表查询的主表行收集入口（L18）：统一层 collect() 经此取回行数据，
    /// 实现复用既有 collect_inner。
    pub(crate) async fn collect_rows(self) -> crate::Result<Vec<T>> {
        self.collect_inner().await
    }

    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;

        let mysql_params = values_to_params(&params)?;

        let mut lease = self.pool.lease().await?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                convert_mysql_model_value::<T>(&row, i)
            })?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct MultiTableCollectFuture<'a, T: Model, R1: Model, R2: Model> {
    executor: MultiTableSelectExecutor<'a, T, R1, R2>,
}
/// 关联/多表查询的同谓词行数统计：`SELECT COUNT(*) FROM (<原子查询>)`。
macro_rules! impl_related_count_mysql {
    ($executor:ident, [$($g:tt)*], [$($ty:tt)*]) => {
        impl<$($g)*> $executor<$($ty)*> {
            /// 统计同谓词总行数（列表分页 total_count 用）。
            pub async fn count(self) -> crate::Result<i64> {
                let (sql, params) = self.select.to_count_sql_with_params(DbType::MySQL);
                let mysql_params = values_to_params(&params)?;
                let mut lease = self.pool.lease().await?;
                let rows: Vec<mysql_async::Row> =
                    traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;
                let total = rows
                    .first()
                    .and_then(|row| row.get::<i64, usize>(0))
                    .unwrap_or(0);
                Ok(total)
            }
        }
    };
}
impl_related_count_mysql!(RelatedSelectExecutor, ['a, T: Model, R: Model], ['a, T, R]);
impl_related_count_mysql!(
    MultiTableSelectExecutor,
    ['a, T: Model, R1: Model, R2: Model],
    ['a, T, R1, R2]
);
impl_related_count_mysql!(
    FourTableSelectExecutor,
    ['a, T: Model, R1: Model, R2: Model, R3: Model],
    ['a, T, R1, R2, R3]
);


impl<'a, T: Model + 'static + Send, R1: Model + 'static + Send, R2: Model + 'static + Send>
    std::future::IntoFuture for MultiTableCollectFuture<'a, T, R1, R2>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// FourTable 查询执行器（支持4表关联查询）
pub struct FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    pool: ExecutorConn<'a>,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

impl_backend_four_table_executor_methods_with_lifetime!(
    FourTableSelectExecutor,
    pool,
    &'a Pool,
    FourTableSelect
);

impl<'a, T: Model, R1: Model, R2: Model, R3: Model> FourTableSelectExecutor<'a, T, R1, R2, R3> {
    /// 多表查询的主表行收集入口（L18）：统一层 collect() 经此取回行数据，
    /// 实现复用既有 collect_inner。
    pub(crate) async fn collect_rows(self) -> crate::Result<Vec<T>> {
        self.collect_inner().await
    }

    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::MySQL)?;
        let mut lease = self.pool.lease().await?;

        let mysql_params = values_to_params(&params)?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                convert_mysql_model_value::<T>(&row, i)
            })?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct FourTableCollectFuture<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    executor: FourTableSelectExecutor<'a, T, R1, R2, R3>,
}

impl<
    'a,
    T: Model + 'static + Send,
    R1: Model + 'static + Send,
    R2: Model + 'static + Send,
    R3: Model + 'static + Send,
> std::future::IntoFuture for FourTableCollectFuture<'a, T, R1, R2, R3>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl_backend_join_executor_methods_with_lifetime!(
    LeftJoinedSelectExecutor,
    pool,
    &'a Pool,
    LeftJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    InnerJoinedSelectExecutor,
    pool,
    &'a Pool,
    InnerJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    RightJoinedSelectExecutor,
    pool,
    &'a Pool,
    RightJoinedSelect
);

/// LeftJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J> {
        LeftJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

/// LEFT JOIN Collect future
pub struct LeftJoinCollectFuture<'a, T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for LeftJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, Option<J>)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::MySQL);

        let mysql_params = values_to_params(&params)?;

        let mut lease = self.pool.lease().await?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let t_model =
                common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    convert_mysql_model_value::<T>(&row, i)
                })?;

            // 尝试读取 J 的列
            let j_model = common_helpers::decode_optional_model_from_indexed_values::<J, _>(
                t_col_count,
                |i| convert_mysql_model_value_at::<J>(&row, i, i - t_col_count),
            )?;

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// InnerJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J> {
        InnerJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<'a, T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for InnerJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::MySQL);

        let mysql_params = values_to_params(&params)?;

        let mut lease = self.pool.lease().await?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let t_model =
                common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    convert_mysql_model_value::<T>(&row, i)
                })?;
            let j_model =
                common_helpers::decode_model_from_indexed_values::<J, _>(t_col_count, |i| {
                    convert_mysql_model_value_at::<J>(&row, i, i - t_col_count)
                })?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// RightJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J> {
        RightJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<'a, T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for RightJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(Option<T>, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::MySQL);

        let mysql_params = values_to_params(&params)?;

        let mut lease = self.pool.lease().await?;

        let rows: Vec<mysql_async::Row> =
            traced_mysql_exec(lease.conn()?, &sql, mysql_params, &params).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let t_model =
                common_helpers::decode_optional_model_from_indexed_values::<T, _>(0, |i| {
                    convert_mysql_model_value::<T>(&row, i)
                })?;
            let j_model =
                common_helpers::decode_model_from_indexed_values::<J, _>(t_col_count, |i| {
                    convert_mysql_model_value_at::<J>(&row, i, i - t_col_count)
                })?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// 模型路径的列值解码入口（带 rust_type）。
fn convert_mysql_model_value<T: Model>(
    row: &mysql_async::Row,
    column_index: usize,
) -> crate::Result<crate::model::Value> {
    convert_mysql_model_value_at::<T>(row, column_index, column_index)
}

/// 从 mysql_async 原生值提取整数值（二进制协议下整数列为 Int/UInt）。
fn mysql_value_int(value: &mysql_async::Value) -> Option<i64> {
    match value {
        mysql_async::Value::Int(i) => Some(*i),
        mysql_async::Value::UInt(u) => Some(*u as i64),
        _ => None,
    }
}

/// DECIMAL 列元数据判定：SUM/AVG 等聚合在 MySQL 返回 DECIMAL 列，
/// 二进制协议下以文本 Bytes 返回。
fn mysql_is_decimal_column(row: &mysql_async::Row, index: usize) -> bool {
    use mysql_async::consts::ColumnType;
    row.columns().get(index).is_some_and(|col| {
        matches!(
            col.column_type(),
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL
        )
    })
}

/// DECIMAL 列文本（如 SUM 聚合的 "110"）无损落到整数模型字段；
/// 带非零小数位（如 AVG 的 "36.667"）返回 None，由 strict helper 报错。
fn mysql_decimal_int_text(value: &mysql_async::Value) -> Option<i64> {
    let mysql_async::Value::Bytes(bytes) = value else {
        return None;
    };
    let text = std::str::from_utf8(bytes).ok()?;
    if let Ok(int) = text.parse::<i64>() {
        return Some(int);
    }
    let real = text.parse::<f64>().ok()?;
    (real.fract() == 0.0).then_some(real as i64)
}

/// DECIMAL 列文本落到浮点模型字段（如 AVG 聚合的 "36.667"）。
fn mysql_decimal_real_text(value: &mysql_async::Value) -> Option<f64> {
    let mysql_async::Value::Bytes(bytes) = value else {
        return None;
    };
    std::str::from_utf8(bytes).ok()?.parse::<f64>().ok()
}

fn mysql_value_real(value: &mysql_async::Value) -> Option<f64> {
    match value {
        mysql_async::Value::Float(f) => Some(*f as f64),
        mysql_async::Value::Double(d) => Some(*d),
        _ => None,
    }
}

fn mysql_value_bool(value: &mysql_async::Value) -> Option<i8> {
    mysql_value_int(value).map(|i| i as i8)
}

fn mysql_value_bytes(value: &mysql_async::Value) -> Option<Vec<u8>> {
    match value {
        mysql_async::Value::Bytes(b) => Some(b.clone()),
        _ => None,
    }
}

fn mysql_value_datetime(value: &mysql_async::Value) -> Option<chrono::DateTime<chrono::Utc>> {
    match value {
        mysql_async::Value::Date(year, month, day, hour, minute, second, micros) => {
            chrono::NaiveDate::from_ymd_opt(*year as i32, *month as u32, *day as u32)?
                .and_hms_micro_opt(*hour as u32, *minute as u32, *second as u32, *micros)
                .map(|naive| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                })
        }
        _ => None,
    }
}

/// 提取文本值：文本列为 UTF-8 Bytes；二进制协议下 DATE/TIME 以 Value::Date/Time
/// 返回，按目标类型格式化为公共 strict helper 期望的日期/时间文本形式。
fn mysql_value_string(value: &mysql_async::Value, rust_type: &str) -> Option<String> {
    match value {
        mysql_async::Value::Bytes(bytes) => String::from_utf8(bytes.clone()).ok(),
        mysql_async::Value::Date(year, month, day, _, _, _, _)
            if matches!(rust_type, "NaiveDate" | "chrono::NaiveDate") =>
        {
            Some(format!("{year:04}-{month:02}-{day:02}"))
        }
        mysql_async::Value::Time(false, 0, hours, minutes, seconds, micros)
            if matches!(rust_type, "NaiveTime" | "chrono::NaiveTime") =>
        {
            Some(format!("{hours:02}:{minutes:02}:{seconds:02}.{micros:06}"))
        }
        _ => None,
    }
}

/// 字符串数组列（写入侧 stringify_string_vec 存文本）：显式按文本解码，
/// 不落数值嗅探——单元素 "123" 必须仍解码为文本。
fn is_mysql_vec_string_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>"
    )
}

/// 模型路径（带 rust_type）的列值解码：按目标列类型走公共 strict helper，
/// 不对文本做数值嗅探——VARCHAR 中的 "123"/"3.14" 必须仍解码为文本。
/// 数值嗅探只保留在无 rust_type 的聚合/原生查询路径（convert_mysql_value）。
fn convert_mysql_model_value_at<T: Model>(
    row: &mysql_async::Row,
    row_index: usize,
    model_column_index: usize,
) -> crate::Result<crate::model::Value> {
    let columns = T::column_schema();
    let column = columns
        .get(model_column_index)
        .ok_or_else(|| crate::ormer_error!("Column index out of bounds: {}", model_column_index))?;

    let Some(value) = row
        .get::<Option<mysql_async::Value>, _>(row_index)
        .unwrap_or(None)
        .filter(|value| !matches!(value, mysql_async::Value::NULL))
    else {
        return Ok(crate::model::Value::Null);
    };

    let rust_type = column.data_type.unwrap_or(column.rust_type);

    // ENUM 列以变体文本返回，直接按文本解码
    if column.enum_variants.is_some() {
        return match &value {
            mysql_async::Value::Bytes(bytes) => String::from_utf8(bytes.clone())
                .map(crate::model::Value::Text)
                .map_err(|err| {
                    crate::ormer_error!(
                        "Failed to decode MySQL enum column '{}' as UTF-8: {}",
                        column.name,
                        err
                    )
                }),
            _ => convert_mysql_value(row, row_index),
        };
    }

    match rust_type {
        // MySQL 以 BIGINT 微秒存储 Duration（写入侧 mysql_value_from_ormer_value）
        "Duration" | "std::time::Duration" => {
            return mysql_value_int(&value)
                .map(|micros| {
                    crate::model::Value::Duration(std::time::Duration::from_micros(
                        micros.max(0) as u64,
                    ))
                })
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse column '{}' (expected Duration microseconds)",
                        column.name
                    )
                });
        }
        // strict helper 未覆盖的整数宽度类型按整数解码
        "isize" | "usize" => {
            return mysql_value_int(&value)
                .map(crate::model::Value::Integer)
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse column '{}' (expected integer type)",
                        column.name
                    )
                });
        }
        // JSON 列以 JSON 文本返回，解析为 Json 值
        "JsonValue" | "serde_json::Value" => {
            return mysql_value_bytes(&value)
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .map(crate::model::Value::Json)
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse column '{}' (expected JSON text)",
                        column.name
                    )
                });
        }
        // 字符串数组列以 stringify_string_vec 文本存储，按文本解码
        // （FromValue for Vec<String> 接受 Text），不落数值嗅探
        _ if is_mysql_vec_string_type(rust_type) => {
            return mysql_value_bytes(&value)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(crate::model::Value::Text)
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse column '{}' (expected string array text)",
                        column.name
                    )
                });
        }
        // 未知自定义类型：保持历史无类型解码
        _ if !ddl_introspection::is_strict_rust_type(rust_type) => {
            return convert_mysql_value(row, row_index);
        }
        _ => {}
    }

    // DECIMAL 聚合列（SUM/AVG）按列元数据放行到整数/浮点模型字段；
    // VARCHAR 等文本列仍不做数值嗅探。
    let is_decimal_col = mysql_is_decimal_column(row, row_index);
    common_helpers::parse_column_value_strict(
        rust_type,
        column.is_nullable,
        column.name,
        || {
            mysql_value_int(&value).or_else(|| {
                is_decimal_col
                    .then(|| mysql_decimal_int_text(&value))
                    .flatten()
            })
        },
        || mysql_value_string(&value, rust_type),
        || {
            mysql_value_real(&value).or_else(|| {
                is_decimal_col
                    .then(|| mysql_decimal_real_text(&value))
                    .flatten()
            })
        },
        || mysql_value_bool(&value),
        || mysql_value_bytes(&value),
        || mysql_value_datetime(&value),
    )
}

/// 将 MySQL 行中的值转换为 ormer Value
fn convert_mysql_value(row: &mysql_async::Row, index: usize) -> crate::Result<crate::model::Value> {
    use mysql_async::Value;
    use mysql_async::consts::ColumnType;

    // 获取原始值
    let value = row.get::<Option<Value>, _>(index).unwrap_or(None);

    // 检查列类型是否为二进制类型（BLOB / TINYBLOB / MEDIUMBLOB / LONGBLOB / BINARY / VARBINARY）
    let is_binary_col = row.columns().get(index).is_some_and(|col| {
        matches!(
            col.column_type(),
            ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_STRING
                | ColumnType::MYSQL_TYPE_VAR_STRING
        ) && col.character_set() == 63 // 63 = binary charset
    });
    let is_decimal_col = mysql_is_decimal_column(row, index);

    match value {
        Some(Value::NULL) | None => Ok(crate::model::Value::Null),
        Some(Value::Int(i)) => Ok(crate::model::Value::Integer(i)),
        Some(Value::UInt(u)) => Ok(crate::model::Value::Integer(u as i64)),
        Some(Value::Float(f)) => Ok(crate::model::Value::Real(f as f64)),
        Some(Value::Double(d)) => Ok(crate::model::Value::Real(d)),
        Some(Value::Date(year, month, day, hour, minute, second, micros)) => {
            let date = chrono::NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
                .ok_or_else(|| {
                    crate::ormer_error!("Invalid MySQL DATE value at index {}", index)
                })?;

            if hour == 0 && minute == 0 && second == 0 && micros == 0 {
                Ok(crate::model::Value::Date(date))
            } else {
                let datetime = date
                    .and_hms_micro_opt(hour as u32, minute as u32, second as u32, micros)
                    .ok_or_else(|| {
                        crate::ormer_error!("Invalid MySQL DATETIME value at index {}", index)
                    })?;
                Ok(crate::model::Value::DateTime(
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                        datetime,
                        chrono::Utc,
                    ),
                ))
            }
        }
        Some(Value::Time(negative, days, hours, minutes, seconds, micros)) => {
            if !negative && days == 0 {
                let time = chrono::NaiveTime::from_hms_micro_opt(
                    hours as u32,
                    minutes as u32,
                    seconds as u32,
                    micros,
                )
                .ok_or_else(|| {
                    crate::ormer_error!("Invalid MySQL TIME value at index {}", index)
                })?;
                Ok(crate::model::Value::Time(time))
            } else {
                let total_hours = days.saturating_mul(24).saturating_add(hours as u32);
                let sign = if negative { "-" } else { "" };
                let value = if micros == 0 {
                    format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}")
                } else {
                    format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}.{micros:06}")
                };
                Ok(crate::model::Value::Text(value))
            }
        }
        Some(Value::Bytes(b)) if is_binary_col => {
            // 二进制列（BLOB/BINARY）直接作为 Bytes 返回
            Ok(crate::model::Value::Bytes(b))
        }
        Some(Value::Bytes(b)) if is_decimal_col => match String::from_utf8(b.clone()) {
            Ok(s) => Ok(crate::model::Value::BigDecimal(s)),
            Err(_) => Ok(crate::model::Value::Bytes(b)),
        },
        Some(Value::Bytes(b)) => {
            // 文本列：尝试将字节解析为数值（MySQL聚合函数可能返回字符串形式的数值）
            if let Ok(s) = String::from_utf8(b.clone()) {
                if let Ok(i) = s.parse::<i64>() {
                    Ok(crate::model::Value::Integer(i))
                } else if let Ok(f) = s.parse::<f64>() {
                    Ok(crate::model::Value::Real(f))
                } else {
                    Ok(crate::model::Value::Text(s))
                }
            } else {
                // 非 UTF-8 字节，作为二进制数据处理
                Ok(crate::model::Value::Bytes(b))
            }
        }
    }
}

