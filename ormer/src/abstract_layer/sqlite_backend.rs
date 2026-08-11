use super::common::common_helpers;
use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{SingleSqlStatement, SqlExecutor, SqlStatement};
use crate::hooks::{HookContext, HookOperation};
use crate::migration::{SchemaColumn, schema_column};
use crate::model::{DbBackendTypeMapper, Model, Row, Value, WritableModel};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MappedSelect,
    MultiTableSelect, RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use crate::query::insert::{
    InsertAssignment, InsertConflict, IntoInsertAssignment, IntoInsertDefaultColumn,
};
use crate::query::update::UpdateAssignment;
use crate::raw_sql::IntoRawSql;
use crate::utils::{FutureTraceExt, ResultTraceExt};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

type ModelUpdateBatch = Vec<(Vec<(String, Value)>, Vec<FilterExpr>)>;

/// 判断错误是否为约束冲突错误（如主键/唯一键重复）
/// turso 不支持 INSERT OR IGNORE / ON CONFLICT 语法，因此需要在执行阶段通过捕获此类错误来实现忽略行为。
fn is_constraint_error(e: &crate::OrmerError) -> bool {
    let msg = e.to_string();
    msg.contains("UNIQUE constraint failed") || msg.contains("constraint")
}

fn table_name_for<T: Model>() -> &'static str {
    T::table_name_for_db(DbType::Sqlite)
}

// 导入宏
use crate::impl_backend_executor_methods;
use crate::impl_backend_join_executor_methods;
use crate::impl_backend_related_executor_methods;
use crate::impl_insert_conflict_methods;

/// Sqlite 类型映射器
pub struct SqliteTypeMapper;

impl DbBackendTypeMapper for SqliteTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        // SQLite 不支持原生 ENUM,降级为 TEXT
        if enum_variants.is_some() {
            return common_helpers::sql_type_with_nullability("TEXT", is_nullable || is_primary);
        }

        // 首先处理主键类型
        if is_primary {
            if is_auto_increment {
                return "INTEGER PRIMARY KEY AUTOINCREMENT".to_string();
            } else {
                return "INTEGER PRIMARY KEY".to_string();
            }
        }

        // 基础类型映射（SQLite 类型系统更简单）
        let base_type = match rust_type {
            // 整数类型
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => "INTEGER",
            // 浮点类型
            "f32" | "f64" => "REAL",
            // 时长类型
            "Duration" | "std::time::Duration" => "INTEGER",
            // 字符串类型
            "String" => "TEXT",
            // 布尔类型（SQLite 没有原生 bool，用 INTEGER 存储）
            "bool" => "INTEGER",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BLOB",
            // 日期时间类型（SQLite 存储为 TEXT 或 INTEGER）
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => "TEXT",
            "NaiveDate" | "chrono::NaiveDate" => "TEXT",
            "NaiveTime" | "chrono::NaiveTime" => "TEXT",
            // JSON 类型（SQLite 存储为 TEXT）
            "JsonValue" | "serde_json::Value" => "TEXT",
            // 默认使用 TEXT
            _ => "TEXT",
        };

        common_helpers::sql_type_with_nullability(base_type, is_nullable)
    }
}

/// Sqlite 数据库连接封装
pub struct Database {
    conn: Arc<turso::Connection>,
}

// SAFETY: turso::Connection uses internal synchronization mechanisms
// that make it safe to share between threads. The turso library
// doesn't explicitly implement Send, but the local connection mode
// is safe to share because all operations are serialized through
// async/await.
unsafe impl Send for Database {}
unsafe impl Sync for Database {}

// Wrapper type to make turso::Connection explicitly Send
#[allow(dead_code)]
struct SendableConnection(turso::Connection);

unsafe impl Send for SendableConnection {}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: crate::model::WritableModel> {
    db: &'a Database,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> CreateTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::Sqlite,
            self.table_name.as_deref(),
        )?;
        Ok(SqlStatement::single(DbType::Sqlite, create_sql, Vec::new()))
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
        for statement in sql.statements {
            self.db.conn.execute(&statement.sql, ()).trace().await?;
        }
        Ok(())
    }
}

/// 删除表执行器（基于Model）
pub struct DropTableExecutor<'a, T: crate::model::WritableModel> {
    db: &'a Database,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> DropTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        Ok(SqlStatement::single(
            DbType::Sqlite,
            format!(
                "DROP TABLE IF EXISTS {}",
                common_helpers::quote_table_name::<T>(DbType::Sqlite)
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
        for statement in sql.statements {
            self.db.conn.execute(&statement.sql, ()).trace().await?;
        }
        Ok(())
    }
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<I::Model>,
}

impl_insert_conflict_methods!(InsertExecutor, with_conflict);

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        let (sql, all_values) = common_helpers::build_insert_statement_with_conflict::<I::Model>(
            DbType::Sqlite,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        <Self as SqlExecutor>::execute(self).await
    }

    /// 执行插入并返回插入的行数据（SQLite RETURNING 支持）
    pub async fn returning(mut self) -> crate::Result<Vec<I::Model>> {
        if self.models.as_refs().is_empty() {
            return Ok(Vec::new());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let refs = self.models.as_refs();
        let (sql, all_values) = common_helpers::build_insert_statement_with_conflict::<I::Model>(
            DbType::Sqlite,
            &refs,
            self.conflict.as_ref(),
        )?;
        let all_params = values_into_params(all_values)?;

        let sql_with_returning = format!("{} RETURNING *", sql);
        let mut rows = self
            .db
            .conn
            .query(&sql_with_returning, all_params)
            .trace()
            .await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let model = common_helpers::decode_model_from_indexed_values::<I::Model, _>(0, |i| {
                let value = row.get_value(i)?;
                convert_turso_value(&value)
            })?;
            results.push(model);
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(results)
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

        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;

        let rows_affected = self.db.conn.execute(&statement.sql, params).trace().await?;
        self.models.run_after_insert(hook_ctx).await?;

        // 获取自增ID（如果有自增主键）
        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            if rows_affected == 0 {
                return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
            }
            let last_id = self.db.conn.last_insert_rowid();
            let result = common_helpers::convert_auto_increment_key::<Self::Output>(last_id)?;
            return Ok(result);
        }

        Ok(<I::Model as Model>::AutoIncrementKeyType::default())
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
        common_helpers::validate_insert_model_table::<T>(DbType::Sqlite, self.source_table)?;
        let statement =
            common_helpers::build_partial_insert_statement::<T>(DbType::Sqlite, &self.assignments)?;
        Ok(SqlStatement::batch(
            DbType::Sqlite,
            vec![SingleSqlStatement::new(statement.sql, statement.params)],
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
        let rows_affected = self.db.conn.execute(&statement.sql, params).trace().await?;

        if rows_affected == 0 {
            return Ok(<T as Model>::AutoIncrementKeyType::default());
        }

        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            let last_id = self.db.conn.last_insert_rowid();
            return common_helpers::convert_auto_increment_key::<Self::Output>(last_id);
        }

        Ok(<T as Model>::AutoIncrementKeyType::default())
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        // turso 不支持 INSERT OR REPLACE / ON CONFLICT，因此生成普通 INSERT INTO SQL，
        // 在执行阶段通过 DELETE + INSERT 实现 upsert 语义。
        let (sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::Sqlite,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::Sqlite),
            I::Model::COLUMNS,
            &refs,
            common_helpers::BatchInsertValuesMode::All,
        );

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        if self.models.as_refs().is_empty() {
            return Ok(());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let refs = self.models.as_refs();
        let columns = I::Model::COLUMNS;
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<I::Model>(DbType::Sqlite);
        let pk_columns = I::Model::primary_key_columns();

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, columns);
        let insert_placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({insert_placeholders})");

        let where_clauses: Vec<String> = pk_columns
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                common_helpers::quote_assignment(
                    DbType::Sqlite,
                    c,
                    &common_helpers::placeholder(DbType::Sqlite, idx + 1),
                )
            })
            .collect();
        let delete_sql = format!(
            "DELETE FROM {table_name} WHERE {}",
            where_clauses.join(" AND ")
        );

        for model in refs.iter() {
            // 先删除已有记录
            let pk_values = model.primary_key_values();
            let delete_params = values_into_params(pk_values)?;
            self.db
                .conn
                .execute(&delete_sql, delete_params)
                .trace()
                .await?;

            // 然后插入新记录
            let all_values = model.field_values();
            let insert_params = values_into_params(all_values)?;
            self.db
                .conn
                .execute(&insert_sql, insert_params)
                .trace()
                .await?;
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrUpdateExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;
        self.db.conn.execute(&statement.sql, params).trace().await?;
        Ok(())
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        let columns = I::Model::insert_columns();
        // turso 不支持 INSERT OR IGNORE / ON CONFLICT，因此生成普通 INSERT INTO SQL，
        // 在执行阶段捕获约束冲突错误并忽略。
        let (sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::Sqlite,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::Sqlite),
            &columns,
            &refs,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        if self.models.as_refs().is_empty() {
            return Ok(());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let refs = self.models.as_refs();
        let columns = I::Model::insert_columns();
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<I::Model>(DbType::Sqlite);
        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let placeholders_str = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let sql = format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders_str})");

        for model in refs.iter() {
            let values = model.insert_values();
            let params = values_into_params(values)?;
            match self.db.conn.execute(&sql, params).trace().await {
                Ok(_) => {}
                Err(e) if is_constraint_error(&e) => {
                    // 忽略约束冲突（重复主键/唯一键）
                }
                Err(e) => return Err(e),
            }
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrIgnoreExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(());
        }
        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;
        match self.db.conn.execute(&statement.sql, params).trace().await {
            Ok(_) => {}
            Err(e) if is_constraint_error(&e) => {
                // 忽略约束冲突（重复主键/唯一键）
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }
}

impl Database {
    /// 连接到 Sqlite 数据库 (本地模式)
    pub async fn connect(_db_type: super::DbType, path: &str) -> crate::Result<Self> {
        let db = turso::Builder::new_local(path).build().trace().await?;

        let conn = Arc::new(db.connect().trace_for("turso::Database::connect")?);

        Ok(Self { conn })
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            db: self,
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().trace().await?;

        if !table_exists {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {} does not exist",
                T::TABLE_NAME
            ));
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>().await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> crate::Result<bool> {
        let sql = "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?";

        let mut rows = self
            .conn
            .query(sql, [table_name_for::<T>()])
            .trace()
            .await?;

        if let Some(row) = rows.next().trace().await? {
            let count = row.get_value(0).trace_for("turso::Row::get_value")?;

            match count {
                turso::Value::Integer(c) => Ok(c > 0),
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(&self) -> crate::Result<()> {
        // 查询表的列信息
        let sql = format!("PRAGMA table_info({})", table_name_for::<T>());

        let mut rows = self.conn.query(&sql, ()).trace().await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool)> = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let name = row.get_value(1).trace_for("turso::Row::get_value")?;
            let col_type = row.get_value(2).trace_for("turso::Row::get_value")?;
            let notnull = row.get_value(3).trace_for("turso::Row::get_value")?;
            let pk = row.get_value(5).trace_for("turso::Row::get_value")?;

            if let (
                turso::Value::Text(name),
                turso::Value::Text(col_type),
                turso::Value::Integer(notnull),
                turso::Value::Integer(pk),
            ) = (name, col_type, notnull, pk)
            {
                actual_columns.push((name, col_type, notnull != 0, pk != 0));
            }
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

            let (actual_name, actual_type, actual_notnull, actual_pk) = &actual_columns[i];

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

            // 检查主键约束
            if expected_col.is_primary != *actual_pk {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Primary key mismatch for '{}': expected {}primary key, but actual is {}primary key",
                    T::TABLE_NAME,
                    expected_col.name,
                    if expected_col.is_primary { "" } else { "not " },
                    if *actual_pk { "" } else { "not " }
                ));
            }

            // 检查列类型（只比较基础类型，不包含 NOT NULL 约束）
            let expected_type = crate::abstract_layer::DbType::Sqlite.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
            );

            // 对于类型比较，我们需要提取基础类型（不包含约束）
            let type_to_compare = if expected_col.is_primary {
                // 主键的基础类型，不包含任何约束
                match expected_col.rust_type {
                    "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                        "INTEGER".to_string()
                    }
                    "f32" | "f64" => "REAL".to_string(),
                    "String" => "TEXT".to_string(),
                    "bool" => "INTEGER".to_string(),
                    "Vec<u8>" | "&[u8]" => "BLOB".to_string(),
                    _ => "TEXT".to_string(),
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::Sqlite.sql_type(
                    expected_col.rust_type,
                    false,
                    expected_col.is_auto_increment,
                    expected_col.is_nullable,
                    expected_col.enum_variants,
                );
                // 去掉 " NOT NULL" 后缀
                full_type.replace(" NOT NULL", "")
            };

            if !self.types_compatible(actual_type, &type_to_compare) {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column type mismatch for '{}': expected '{expected_type}', but actual is '{actual_type}'",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            // 检查 NOT NULL 约束（主键列自动 NOT NULL，所以不需要额外检查）
            if !expected_col.is_primary {
                let expected_notnull = !expected_col.is_nullable;
                if *actual_notnull != expected_notnull {
                    return Err(crate::ormer_error!(
                        "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                        T::TABLE_NAME,
                        expected_col.name,
                        if expected_notnull { "NOT " } else { "" },
                        if *actual_notnull { "NOT " } else { "" }
                    ));
                }
            }
        }

        Ok(())
    }

    /// 检查 SQL 类型是否兼容
    fn types_compatible(&self, actual: &str, expected: &str) -> bool {
        // 标准化类型名称（SQLite 类型别名）
        fn normalize(s: &str) -> String {
            match s.to_uppercase().as_str() {
                "INT" | "INTEGER" | "MEDIUMINT" | "BIGINT" | "INT64" => "INTEGER".to_string(),
                "VARCHAR" | "CHARACTER" | "NCHAR" | "NVARCHAR" | "TEXT" | "CLOB" => {
                    "TEXT".to_string()
                }
                "BLOB" => "BLOB".to_string(),
                "REAL" | "FLOAT" | "DOUBLE" | "DECIMAL" | "NUMERIC" => "REAL".to_string(),
                _ => s.to_string(),
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            db: self,
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
            _marker: std::marker::PhantomData,
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
            db: self,
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
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入记录，返回自增主键值（如果有自增主键）或 ()
    /// 对于批量插入，返回的是第一条插入记录的自增ID（即最小值）
    pub(crate) async fn insert_impl<T: Model>(
        &self,
        models: &[&T],
    ) -> crate::Result<T::AutoIncrementKeyType> {
        if models.is_empty() {
            return Ok(T::AutoIncrementKeyType::default());
        }

        let (sql, all_values) = common_helpers::build_insert_statement::<T>(DbType::Sqlite, models);
        let all_params = values_into_params(all_values)?;

        self.conn.execute(&sql, all_params).trace().await?;

        // 获取自增ID（如果有自增主键）
        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            let last_id = self.conn.last_insert_rowid();
            // 将 i64 转换为对应的主键类型
            let result =
                common_helpers::convert_auto_increment_key::<T::AutoIncrementKeyType>(last_id)?;
            Ok(result)
        } else {
            Ok(T::AutoIncrementKeyType::default())
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    /// turso 不支持 ON CONFLICT，因此通过 DELETE + INSERT 实现 upsert 语义。
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let pk_columns = T::primary_key_columns();
        let table_name = common_helpers::quote_table_name::<T>(DbType::Sqlite);

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let insert_placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({insert_placeholders})");

        let where_clauses: Vec<String> = pk_columns
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                common_helpers::quote_assignment(
                    DbType::Sqlite,
                    c,
                    &common_helpers::placeholder(DbType::Sqlite, idx + 1),
                )
            })
            .collect();
        let delete_sql = format!(
            "DELETE FROM {table_name} WHERE {}",
            where_clauses.join(" AND ")
        );

        for model in models.iter() {
            let pk_values = model.primary_key_values();
            let delete_params = values_into_params(pk_values)?;
            self.conn
                .execute(&delete_sql, delete_params)
                .trace()
                .await?;

            let all_values = model.insert_values();
            let insert_params = values_into_params(all_values)?;
            self.conn
                .execute(&insert_sql, insert_params)
                .trace()
                .await?;
        }

        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略）
    /// turso 不支持 ON CONFLICT，因此通过捕获约束错误实现忽略语义。
    pub async fn insert_or_ignore_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<T>(DbType::Sqlite);

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders})");

        for model in models.iter() {
            let values = model.insert_values();
            let params = values_into_params(values)?;
            match self.conn.execute(&insert_sql, params).trace().await {
                Ok(_) => {}
                Err(e) if is_constraint_error(&e) => {
                    // 忽略约束冲突（重复主键/唯一键）
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<T> {
        DeleteExecutor {
            filters: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<Transaction> {
        self.conn.execute("BEGIN", ()).trace().await?;
        Ok(Transaction {
            conn: self.conn.clone(),
            committed: false,
            rolled_back: false,
        })
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            db: self,
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(DbType::Sqlite)?;
        self.exec_raw(&sql, params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let turso_params = values_into_params(params)?;
        let mut rows = if turso_params.is_empty() {
            self.conn.query(sql, ()).trace().await?
        } else {
            self.conn.query(sql, turso_params).trace().await?
        };

        let mut results = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let mut values = Vec::new();
            for i in 0..row.column_count() {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                values.push(convert_turso_value(&value)?);
            }
            results.push(V::from_row_values(&values)?);
        }

        Ok(results.into_iter().collect())
    }

    pub(crate) async fn exec_raw(&self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let turso_params = values_into_params(params)?;
        if turso_params.is_empty() {
            Ok(self.conn.execute(sql, ()).trace().await?)
        } else {
            Ok(self.conn.execute(sql, turso_params).trace().await?)
        }
    }

    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<(u64, String, u64)>> {
        let mut rows = self
            .conn
            .query(
                "SELECT version, name, checksum FROM __ormer_migrations ORDER BY version",
                (),
            )
            .trace()
            .await?;
        let mut versions = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let version = match row.get_value(0).trace_for("turso::Row::get_value")? {
                turso::Value::Integer(version) if version >= 0 => version as u64,
                _ => continue,
            };
            let name = match row.get_value(1).trace_for("turso::Row::get_value")? {
                turso::Value::Text(name) => name,
                _ => String::new(),
            };
            let checksum = match row.get_value(2).trace_for("turso::Row::get_value")? {
                turso::Value::Integer(checksum) if checksum >= 0 => checksum as u64,
                turso::Value::Text(checksum) => checksum.parse::<u64>().unwrap_or(0),
                _ => 0,
            };
            versions.push((version, name, checksum));
        }
        Ok(versions)
    }

    pub(crate) async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<SchemaColumn>>> {
        let mut exists = self
            .conn
            .query(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
                [table_name],
            )
            .trace()
            .await?;
        let exists = match exists.next().trace().await? {
            Some(row) => matches!(
                row.get_value(0).trace_for("turso::Row::get_value")?,
                turso::Value::Integer(count) if count > 0
            ),
            None => false,
        };
        if !exists {
            return Ok(None);
        }

        let escaped = table_name.replace('\'', "''");
        let mut rows = self
            .conn
            .query(&format!("PRAGMA table_info('{escaped}')"), ())
            .trace()
            .await?;
        let mut columns = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let name = match row.get_value(1).trace_for("turso::Row::get_value")? {
                turso::Value::Text(value) => value,
                _ => continue,
            };
            let type_name = match row.get_value(2).trace_for("turso::Row::get_value")? {
                turso::Value::Text(value) => value,
                _ => String::new(),
            };
            let nullable = !matches!(
                row.get_value(3).trace_for("turso::Row::get_value")?,
                turso::Value::Integer(value) if value != 0
            );
            let primary_key = matches!(
                row.get_value(5).trace_for("turso::Row::get_value")?,
                turso::Value::Integer(value) if value != 0
            );
            columns.push(schema_column(name, type_name, nullable, primary_key));
        }
        Ok(Some(columns))
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        self.conn.execute("SELECT 1", ()).trace().await.is_ok()
    }
}

/// Sqlite 事务对象
pub struct Transaction {
    conn: Arc<turso::Connection>,
    committed: bool,
    rolled_back: bool,
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.committed || self.rolled_back {
            return;
        }

        let conn = Arc::clone(&self.conn);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = conn.execute("ROLLBACK", ()).trace().await;
            });
        }
    }
}

/// 事务中的插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<I::Model>,
}

impl_insert_conflict_methods!(TransactionInsertExecutor);

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        let (sql, all_values) = common_helpers::build_insert_statement_with_conflict::<I::Model>(
            DbType::Sqlite,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(mut self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        let sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;

        let statement = &sql.statements[0];
        let all_params = values_to_params(&statement.params)?;

        let rows_affected = self
            .txn
            .conn
            .execute(&statement.sql, all_params)
            .trace()
            .await?;
        self.models.run_after_insert(hook_ctx).await?;

        // 获取自增ID（如果有自增主键）
        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            if rows_affected == 0 {
                return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
            }
            let last_id = self.txn.conn.last_insert_rowid();
            let result = common_helpers::convert_auto_increment_key::<
                <I::Model as Model>::AutoIncrementKeyType,
            >(last_id)?;
            Ok(result)
        } else {
            Ok(<I::Model as Model>::AutoIncrementKeyType::default())
        }
    }
}

/// 事务中的插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        let (sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::Sqlite,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::Sqlite),
            I::Model::COLUMNS,
            &refs,
            common_helpers::BatchInsertValuesMode::All,
        );

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        if self.models.as_refs().is_empty() {
            return Ok(());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;

        let refs = self.models.as_refs();
        let columns = I::Model::COLUMNS;
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<I::Model>(DbType::Sqlite);
        let pk_columns = I::Model::primary_key_columns();

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, columns);
        let insert_placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({insert_placeholders})");

        let where_clauses: Vec<String> = pk_columns
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                common_helpers::quote_assignment(
                    DbType::Sqlite,
                    c,
                    &common_helpers::placeholder(DbType::Sqlite, idx + 1),
                )
            })
            .collect();
        let delete_sql = format!(
            "DELETE FROM {table_name} WHERE {}",
            where_clauses.join(" AND ")
        );

        for model in refs.iter() {
            let pk_values = model.primary_key_values();
            let delete_params = values_into_params(pk_values)?;
            self.txn
                .conn
                .execute(&delete_sql, delete_params)
                .trace()
                .await?;

            let all_values = model.field_values();
            let insert_params = values_into_params(all_values)?;
            self.txn
                .conn
                .execute(&insert_sql, insert_params)
                .trace()
                .await?;
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 事务中的插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::Sqlite, Vec::new()));
        }

        let columns = I::Model::insert_columns();
        let (sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::Sqlite,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::Sqlite),
            &columns,
            &refs,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );

        Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        if self.models.as_refs().is_empty() {
            return Ok(());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;

        let refs = self.models.as_refs();
        let columns = I::Model::insert_columns();
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<I::Model>(DbType::Sqlite);

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders})");

        for model in refs.iter() {
            let values = model.insert_values();
            let params = values_into_params(values)?;
            match self.txn.conn.execute(&insert_sql, params).trace().await {
                Ok(_) => {}
                Err(e) if is_constraint_error(&e) => {
                    // 忽略约束冲突（重复主键/唯一键）
                }
                Err(e) => return Err(e),
            }
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl Transaction {
    pub(crate) async fn exec_raw(&mut self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let turso_params = values_into_params(params)?;
        if turso_params.is_empty() {
            Ok(self.conn.execute(sql, ()).trace().await?)
        } else {
            Ok(self.conn.execute(sql, turso_params).trace().await?)
        }
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let turso_params = values_into_params(params)?;
        let mut rows = if turso_params.is_empty() {
            self.conn.query(sql, ()).trace().await?
        } else {
            self.conn.query(sql, turso_params).trace().await?
        };

        let mut results = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let mut values = Vec::new();
            for i in 0..row.column_count() {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                values.push(convert_turso_value(&value)?);
            }
            results.push(V::from_row_values(&values)?);
        }

        Ok(results.into_iter().collect())
    }

    /// 提交事务
    pub async fn commit(mut self) -> crate::Result<()> {
        if self.committed || self.rolled_back {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back"
            ));
        }
        self.conn.execute("COMMIT", ()).trace().await?;
        self.committed = true;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> crate::Result<()> {
        if self.committed || self.rolled_back {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back"
            ));
        }
        self.conn.execute("ROLLBACK", ()).trace().await?;
        self.rolled_back = true;
        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<T> {
        DeleteExecutor {
            filters: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            txn: self,
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
            txn: self,
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
            txn: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入记录（内部使用），返回自增主键值（如果有自增主键）或 ()
    #[allow(dead_code)]
    async fn insert_impl<T: Model>(
        &mut self,
        models: &[&T],
    ) -> crate::Result<T::AutoIncrementKeyType> {
        if models.is_empty() {
            return Ok(T::AutoIncrementKeyType::default());
        }

        let (sql, all_values) = common_helpers::build_insert_statement::<T>(DbType::Sqlite, models);
        let all_params = values_into_params(all_values)?;

        self.conn.execute(&sql, all_params).trace().await?;

        // 获取自增ID（如果有自增主键）
        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        if has_auto_increment {
            let last_id = self.conn.last_insert_rowid();
            let result =
                common_helpers::convert_auto_increment_key::<T::AutoIncrementKeyType>(last_id)?;
            Ok(result)
        } else {
            Ok(T::AutoIncrementKeyType::default())
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）（内部使用）
    #[allow(dead_code)]
    async fn insert_or_update_impl<T: Model>(&mut self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let pk_columns = T::primary_key_columns();
        let table_name = common_helpers::quote_table_name::<T>(DbType::Sqlite);

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let insert_placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({insert_placeholders})");

        let where_clauses: Vec<String> = pk_columns
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                common_helpers::quote_assignment(
                    DbType::Sqlite,
                    c,
                    &common_helpers::placeholder(DbType::Sqlite, idx + 1),
                )
            })
            .collect();
        let delete_sql = format!(
            "DELETE FROM {table_name} WHERE {}",
            where_clauses.join(" AND ")
        );

        for model in models.iter() {
            let pk_values = model.primary_key_values();
            let delete_params = values_into_params(pk_values)?;
            self.conn
                .execute(&delete_sql, delete_params)
                .trace()
                .await?;

            let all_values = model.insert_values();
            let insert_params = values_into_params(all_values)?;
            self.conn
                .execute(&insert_sql, insert_params)
                .trace()
                .await?;
        }

        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略）（内部使用）
    #[allow(dead_code)]
    async fn insert_or_ignore_impl<T: Model>(&mut self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let table_name = common_helpers::quote_table_name::<T>(DbType::Sqlite);

        let columns_str = common_helpers::quote_column_list(DbType::Sqlite, &columns);
        let placeholders = common_helpers::placeholder_list(DbType::Sqlite, 1, col_count);
        let insert_sql =
            format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders})");

        for model in models.iter() {
            let values = model.insert_values();
            let params = values_into_params(values)?;
            match self.conn.execute(&insert_sql, params).trace().await {
                Ok(_) => {}
                Err(e) if is_constraint_error(&e) => {
                    // 忽略约束冲突（重复主键/唯一键）
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    conn: Arc<turso::Connection>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> Clone for SelectExecutor<'a, T> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for LeftJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for InnerJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for RightJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// Related 查询执行器（支持多表关联查询）
pub struct RelatedSelectExecutor<T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTable 查询执行器（支持3个表关联查询）
#[allow(dead_code)]
pub struct MultiTableSelectExecutor<T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTable 查询执行器（支持4个表关联查询）
#[allow(dead_code)]
pub struct FourTableSelectExecutor<T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

/// Mapped 查询执行器（字段投影查询）
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: MappedSelect<T, V>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<&'a (T, V)>,
}

/// Grouped 查询执行器（分组聚合查询）
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<&'a (T, V)>,
}

impl<'a, T: Model, V> Clone for MappedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, V> Clone for GroupedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    pub(crate) fn select_model<R: Model>(&self) -> SelectExecutor<'a, R> {
        SelectExecutor {
            select: Select::new(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join::<J>(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join::<J>(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join::<J>(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持:
    /// - 单字段:map_to(|r| r.uid) -> MappedSelectExecutor<T, i32>
    /// - 元组:map_to(|r| (r.uid, r.id)) -> MappedSelectExecutor<T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        let mapped_select = self.select.map_to(f);
        MappedSelectExecutor {
            select: mapped_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 忽略指定字段，查询时用默认常量替代真实列值
    pub fn ignore<F, M>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        Self {
            select: self.select.ignore(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 选择列(支持聚合函数)- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        let grouped_select = self.select.select_column(f);
        GroupedSelectExecutor {
            select: grouped_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    /// 执行查询并返回第一条记录
    pub fn first(self) -> FirstFuture<'a, T> {
        FirstFuture { executor: self }
    }

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
    {
        let aggregate_select = self.select.count(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.sum(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.avg(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.max(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.min(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询(支持4个表)
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            select: self.select,
            conn: super::common::StreamConnection::Sqlite(self.conn),
            _marker: std::marker::PhantomData,
        }
    }
}

// 使用宏生成通用的 filter/order_by/range 方法
impl_backend_executor_methods!(SelectExecutor, conn, Arc<turso::Connection>, Select);

// LEFT JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    LeftJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    LeftJoinedSelect
);

impl<T: Model, J: Model> LeftJoinedSelectExecutor<T, J> {
    /// 获取 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        self.select.to_sql_with_params(DbType::Sqlite).0
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<T, J> {
        LeftJoinCollectFuture {
            executor: self.clone(),
        }
    }

    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().trace().await? {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            // 尝试读取 J 的列（从 t_col_count 开始）
            let mut j_data = HashMap::new();
            let mut j_is_null = true;
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                if let Ok(value) = row.get_value(idx) {
                    let ormer_value = convert_turso_value(&value)?;
                    // 检查是否为 NULL，只有非 NULL 值才设置 j_is_null = false
                    if !matches!(ormer_value, Value::Null) {
                        j_is_null = false;
                    }
                    j_data.insert(col_name.to_string(), ormer_value);
                }
            }

            let j_model = if j_is_null {
                None
            } else {
                Some(J::from_row(&Row::new(j_data))?)
            };

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

// INNER JOIN Executor
// INNER JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    InnerJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    InnerJoinedSelect
);

impl<T: Model, J: Model> InnerJoinedSelectExecutor<T, J> {
    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        InnerJoinCollectFuture {
            executor: self.clone(),
        }
    }

    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().trace().await? {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let value = row.get_value(idx).trace_for("turso::Row::get_value")?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

// RIGHT JOIN Executor
// RIGHT JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    RightJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    RightJoinedSelect
);

impl<T: Model, J: Model> RightJoinedSelectExecutor<T, J> {
    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(&self) -> RightJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        RightJoinCollectFuture {
            executor: self.clone(),
        }
    }

    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().trace().await? {
            let mut t_data = HashMap::new();
            let mut t_is_null = true;
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                if let Ok(value) = row.get_value(i) {
                    t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
                    t_is_null = false;
                }
            }
            let t_model = if t_is_null {
                None
            } else {
                Some(T::from_row(&Row::new(t_data))?)
            };

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let value = row.get_value(idx).trace_for("turso::Row::get_value")?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: std::marker::PhantomData<C>,
}

// SAFETY: CollectFuture contains SelectExecutor which references Database (Send + Sync),
// and the async operations are all await-based which ensures thread safety
unsafe impl<'a, T: Model + Send, C: FromIterator<T> + Send> Send for CollectFuture<'a, T, C> {}

/// First future for单条记录查询
pub struct FirstFuture<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
}

// SAFETY: FirstFuture contains SelectExecutor which references Database (Send + Sync)
unsafe impl<'a, T: Model + Send> Send for FirstFuture<'a, T> {}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

impl<
    T: Model + 'static + std::marker::Send,
    R: crate::model::FromValue + 'static + std::marker::Send,
> std::future::IntoFuture for AggregateFuture<T, R>
{
    type Output = crate::Result<R>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.aggregate_select.to_sql_with_params(DbType::Sqlite);

            let turso_params = values_into_params(params)?;

            let mut rows = if turso_params.is_empty() {
                self.conn.query(&sql, ()).trace().await?
            } else {
                self.conn.query(&sql, turso_params).trace().await?
            };

            if let Some(row) = rows.next().trace().await? {
                let value = row.get_value(0).trace_for("turso::Row::get_value")?;

                // 将turso::Value转换为ormer::Value
                let ormer_value = match value {
                    turso::Value::Integer(i) => crate::model::Value::Integer(i),
                    turso::Value::Real(r) => crate::model::Value::Real(r),
                    turso::Value::Text(t) => crate::model::Value::Text(t),
                    turso::Value::Blob(b) => {
                        crate::model::Value::Text(String::from_utf8_lossy(&b).to_string())
                    }
                    turso::Value::Null => crate::model::Value::Null,
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

/// LEFT JOIN Collect future
pub struct LeftJoinCollectFuture<T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<T, J>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for LeftJoinCollectFuture<T, J> {}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<T, J>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for InnerJoinCollectFuture<T, J> {}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<T, J>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for RightJoinCollectFuture<T, J> {}

/// Grouped Collect future（分组聚合查询）
pub struct GroupedCollectFuture<'a, T: Model, V, C> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(T, V, C)>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<'a, T: Model + Send, V: Send, C: Send> Send for GroupedCollectFuture<'a, T, V, C> {}

impl<'a, T: Model + 'static + std::marker::Send + std::marker::Sync, C: FromIterator<T> + 'static>
    std::future::IntoFuture for CollectFuture<'a, T, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model + 'static + std::marker::Send + std::marker::Sync> std::future::IntoFuture
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

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for LeftJoinCollectFuture<T, J>
{
    type Output = crate::Result<Vec<(T, Option<J>)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for InnerJoinCollectFuture<T, J>
{
    type Output = crate::Result<Vec<(T, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RightJoinCollectFuture<T, J>
{
    type Output = crate::Result<Vec<(Option<T>, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

// RelatedSelectExecutor
// 使用宏生成通用的 filter/range 方法
impl_backend_related_executor_methods!(
    RelatedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    RelatedSelect
);

impl<T: Model, R: Model> RelatedSelectExecutor<T, R> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<T, R> {
        RelatedCollectFuture { executor: self }
    }

    pub(crate) fn into_collect_future(self) -> RelatedCollectFuture<T, R> {
        RelatedCollectFuture { executor: self }
    }

    async fn collect_inner<C: FromIterator<T>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().trace().await? {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                convert_turso_value(&value)
            })?;
            results.push(model);
        }

        Ok(results.into_iter().collect())
    }
}

/// Related Collect future
pub struct RelatedCollectFuture<T: Model, R: Model> {
    executor: RelatedSelectExecutor<T, R>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, R: Model + Send> Send for RelatedCollectFuture<T, R> {}

impl<T: Model + 'static + std::marker::Send, R: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RelatedCollectFuture<T, R>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().trace().await? {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                convert_turso_value(&value)
            })?;
            results.push(model);
        }

        Ok(results.into_iter().collect())
    }
}

/// Delete 执行器
pub struct DeleteExecutor<T: Model> {
    filters: Vec<FilterExpr>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

impl<T: Model> DeleteExecutor<T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let (sql, params) = self.build_ormer_sql();
        Ok(SqlStatement::single(DbType::Sqlite, sql, params))
    }

    /// 执行删除操作并返回影响的行数
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    /// 执行删除并返回被删除的行数据（SQLite RETURNING 支持）
    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let sql = self.to_sql()?;
        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;

        let sql_with_returning = format!("{} RETURNING *", statement.sql);
        let mut rows = self.conn.query(&sql_with_returning, params).trace().await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().trace().await? {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                let value = row.get_value(i)?;
                convert_turso_value(&value)
            })?;
            results.push(model);
        }

        Ok(results)
    }

    /// 执行删除操作并返回影响的行数（execute 的别名）
    pub async fn exec(self) -> crate::Result<u64> {
        self.execute().await
    }

    fn build_ormer_sql(&self) -> (String, Vec<Value>) {
        let mut sql = format!(
            "DELETE FROM {}",
            common_helpers::quote_table_name::<T>(DbType::Sqlite)
        );
        let mut ormer_params = Vec::new();

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let _ = common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut ormer_params,
                    DbType::Sqlite,
                );
            }
        }

        (sql, ormer_params)
    }

    #[allow(dead_code)]
    fn build_sql(&self) -> (String, Vec<turso::Value>) {
        let (sql, ormer_params) = self.build_ormer_sql();
        let turso_params = values_into_params(ormer_params).unwrap_or_default();
        (sql, turso_params)
    }
}

impl<T: Model> SqlExecutor for DeleteExecutor<T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(0);
        }
        let statement = &sql.statements[0];
        let params = values_to_params(&statement.params)?;
        let result = self.conn.execute(&statement.sql, params).trace().await?;
        Ok(result)
    }
}

impl<T: Model + 'static + std::marker::Send> std::future::IntoFuture for DeleteExecutor<T> {
    type Output = crate::Result<u64>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<T: Model> {
    sets: Vec<UpdateAssignment>,
    filters: Vec<FilterExpr>,
    model_updates: ModelUpdateBatch,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

impl<T: Model> UpdateExecutor<T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
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
        let mut model_sets = Vec::new();
        for (col_name, value) in model.non_pk_field_values() {
            model_sets.push((col_name.to_string(), value));
        }
        let pk_columns = T::primary_key_columns();
        let pk_values = model.primary_key_values();
        let mut model_filters = Vec::new();
        for (col, val) in pk_columns.iter().zip(pk_values) {
            let filter_val = common_helpers::value_to_filter_value(&val);
            model_filters.push(crate::query::filter::FilterExpr::Comparison {
                column: col.to_string(),
                operator: "=".to_string(),
                value: filter_val,
            });
        }
        self.model_updates.push((model_sets, model_filters));
        self
    }

    pub fn set_model_fields(mut self, model: &T, fields: &[String]) -> Self {
        let model_sets = model
            .non_pk_field_values_for_columns(fields)
            .into_iter()
            .map(|(col_name, value)| (col_name.to_string(), value))
            .collect::<Vec<_>>();
        let pk_columns = T::primary_key_columns();
        let pk_values = model.primary_key_values();
        let model_filters = pk_columns
            .iter()
            .zip(pk_values)
            .map(|(col, val)| crate::query::filter::FilterExpr::Comparison {
                column: col.to_string(),
                operator: "=".to_string(),
                value: common_helpers::value_to_filter_value(&val),
            })
            .collect();

        if !model_sets.is_empty() {
            self.model_updates.push((model_sets, model_filters));
        }
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let statements = self.build_all_ormer_sql()?;
        Ok(SqlStatement::batch(
            DbType::Sqlite,
            statements
                .into_iter()
                .map(|(sql, params)| SingleSqlStatement::new(sql, params))
                .collect(),
        ))
    }

    /// 执行更新操作
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    /// 执行更新并返回被更新的行数据（SQLite RETURNING 支持）
    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let statements = self.to_sql()?;
        let mut results = Vec::new();
        for statement in &statements.statements {
            let params = values_to_params(&statement.params)?;
            let sql_with_returning = format!("{} RETURNING *", statement.sql);
            let mut rows = self.conn.query(&sql_with_returning, params).trace().await?;
            while let Some(row) = rows.next().trace().await? {
                let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    let value = row.get_value(i)?;
                    convert_turso_value(&value)
                })?;
                results.push(model);
            }
        }
        Ok(results)
    }

    /// 执行更新操作（execute 的别名）
    pub async fn exec(self) -> crate::Result<u64> {
        self.execute().await
    }

    fn build_all_ormer_sql(&self) -> crate::Result<Vec<(String, Vec<Value>)>> {
        let mut statements = Vec::new();

        if !self.sets.is_empty() || (self.model_updates.is_empty() && !self.filters.is_empty()) {
            let mut sql = format!(
                "UPDATE {} SET ",
                common_helpers::quote_table_name::<T>(DbType::Sqlite)
            );
            let mut ormer_params = Vec::new();
            let mut first = true;
            for assignment in &self.sets {
                if !first {
                    sql.push_str(", ");
                }
                sql.push_str(&common_helpers::format_update_assignment(
                    DbType::Sqlite,
                    assignment,
                    &mut ormer_params,
                ));
                first = false;
            }
            if !self.filters.is_empty() {
                sql.push_str(" WHERE ");
                let mut param_idx = ormer_params.len() + 1;
                for (i, filter) in self.filters.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(" AND ");
                    }
                    let _ = common_helpers::format_filter_with_params(
                        filter,
                        &mut sql,
                        &mut param_idx,
                        &mut ormer_params,
                        DbType::Sqlite,
                    );
                }
            }
            statements.push((sql, ormer_params));
        }

        for (model_sets, model_filters) in &self.model_updates {
            let mut sql = format!(
                "UPDATE {} SET ",
                common_helpers::quote_table_name::<T>(DbType::Sqlite)
            );
            let mut ormer_params = Vec::new();
            let mut first = true;
            for (col_name, value) in model_sets {
                if !first {
                    sql.push_str(", ");
                }
                sql.push_str(&common_helpers::quote_assignment(
                    DbType::Sqlite,
                    col_name,
                    "?",
                ));
                ormer_params.push(value.clone());
                first = false;
            }
            if !model_filters.is_empty() {
                sql.push_str(" WHERE ");
                let mut param_idx = ormer_params.len() + 1;
                for (i, filter) in model_filters.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(" AND ");
                    }
                    let _ = common_helpers::format_filter_with_params(
                        filter,
                        &mut sql,
                        &mut param_idx,
                        &mut ormer_params,
                        DbType::Sqlite,
                    );
                }
            }
            statements.push((sql, ormer_params));
        }

        Ok(statements)
    }

    #[allow(dead_code)]
    fn build_all_sql(&self) -> crate::Result<Vec<(String, Vec<turso::Value>)>> {
        self.build_all_ormer_sql()?
            .into_iter()
            .map(|(sql, ormer_params)| Ok((sql, values_into_params(ormer_params)?)))
            .collect()
    }
}

impl<T: Model> SqlExecutor for UpdateExecutor<T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        UpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut total = 0;
        for statement in &sql.statements {
            let params = values_to_params(&statement.params)?;
            total += self.conn.execute(&statement.sql, params).trace().await?;
        }
        Ok(total)
    }
}

impl<T: Model + 'static + std::marker::Send> std::future::IntoFuture for UpdateExecutor<T> {
    type Output = crate::Result<u64>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 turso 参数
fn value_to_turso_value(value: Value) -> turso::Value {
    match value {
        Value::Integer(v) => turso::Value::Integer(v),
        Value::Text(v) => turso::Value::Text(v),
        Value::TextArray(v) => turso::Value::Text(crate::model::stringify_string_vec(&v)),
        Value::Real(v) => turso::Value::Real(v),
        Value::Boolean(v) => turso::Value::Integer(if v { 1 } else { 0 }),
        Value::Bytes(v) => turso::Value::Blob(v),
        Value::Duration(v) => turso::Value::Integer(v.as_micros().min(i64::MAX as u128) as i64),
        Value::DateTime(v) => turso::Value::Text(v.to_rfc3339()),
        Value::Date(date) => turso::Value::Text(date.to_string()),
        Value::Time(time) => turso::Value::Text(time.to_string()),
        Value::Json(v) => turso::Value::Text(v.to_string()),
        Value::Uuid(v) => turso::Value::Text(v.to_string()),
        Value::BigInt(v) => turso::Value::Integer(v as i64),
        Value::IntegerArray(_) | Value::BigIntArray(_) | Value::NullableBigIntArray(_) => {
            panic!("SQLite backend does not support PostgreSQL array values")
        }
        Value::Null => turso::Value::Null,
    }
}

fn values_to_params(values: &[Value]) -> crate::Result<Vec<turso::Value>> {
    Ok(values.iter().cloned().map(value_to_turso_value).collect())
}

fn values_into_params(values: Vec<Value>) -> crate::Result<Vec<turso::Value>> {
    Ok(values.into_iter().map(value_to_turso_value).collect())
}

/// 将 turso Value 转换为 ormer Value
fn convert_turso_value(value: &turso::Value) -> crate::Result<Value> {
    match value {
        turso::Value::Integer(v) => Ok(Value::Integer(*v)),
        turso::Value::Text(v) => {
            // 尝试解析为 DateTime
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                return Ok(Value::DateTime(dt.with_timezone(&chrono::Utc)));
            }
            Ok(Value::Text(v.clone()))
        }
        turso::Value::Real(v) => Ok(Value::Real(*v)),
        turso::Value::Null => Ok(Value::Null),
        turso::Value::Blob(v) => Ok(Value::Bytes(v.clone())),
    }
}

/// Mapped Select Collect future
pub struct MappedCollectFuture<'a, T: Model + 'static, V: 'static, C: FromIterator<V> + 'static> {
    executor: MappedSelectExecutor<'a, T, V>,
    _marker: PhantomData<C>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<'a, T: Model + Send, V: Send, C: FromIterator<V> + Send> Send
    for MappedCollectFuture<'a, T, V, C>
{
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// ModelCollectWithFuture - 用于collect_with的Future,支持类型转换
pub struct ModelCollectWithFuture<'a, T: Model, V, C, M, F> {
    executor: MappedSelectExecutor<'a, T, V>,
    transform: F,
    _marker: PhantomData<(C, M)>,
}

// SAFETY: Contains executor which references Database (Send + Sync), and transform function is Send
unsafe impl<'a, T: Model + Send, V: Send, C: Send, M: Send, F: Send> Send
    for ModelCollectWithFuture<'a, T, V, C, M, F>
{
}

impl<'a, T, V, C, M, F> std::future::IntoFuture for ModelCollectWithFuture<'a, T, V, C, M, F>
where
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<M> + 'static,
    M: 'static + std::marker::Send,
    F: Fn(V) -> M + Clone + Send + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<V> = self.executor.collect_inner().trace().await?;
            Ok(results.into_iter().map(|v| (self.transform)(v)).collect())
        })
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 获取子查询的 SQL 和参数
    pub fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        self.select.to_sql_with_params(DbType::Sqlite)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(self) -> MappedCollectFuture<'a, T, V, C> {
        MappedCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果，同时应用转换函数
    /// 用于将查询结果转换为其他类型（如Model）
    /// 示例：collect_with(|v| Uids { id: v })
    pub fn collect_with<C, F, M>(self, f: F) -> ModelCollectWithFuture<'a, T, V, C, M, F>
    where
        C: FromIterator<M> + 'static,
        F: Fn(V) -> M + Clone + 'static,
        M: 'static,
    {
        ModelCollectWithFuture {
            executor: self.clone(),
            transform: f,
            _marker: PhantomData,
        }
    }

    async fn collect_inner<C: FromIterator<V>>(self) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
    {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().trace().await? {
            // 获取行中的所有值
            let column_count = self.select.column_names().len();
            let typed_value =
                common_helpers::decode_row_values_from_indexed_values(column_count, |i| {
                    let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                    convert_turso_value(&value)
                })?;
            results.push(typed_value);
        }

        Ok(results.into_iter().collect())
    }
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        GroupedCollectFuture {
            executor: self.clone(),
            _marker: PhantomData,
        }
    }

    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        Self {
            select: self.select.group_by(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 HAVING 条件
    pub fn having<F>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::WhereExpr,
    {
        Self {
            select: self.select.having(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for GroupedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<V> = self.executor.collect_inner().trace().await?;
            Ok(results.into_iter().collect())
        })
    }
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    async fn collect_inner<C: FromIterator<V>>(self) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
    {
        let (sql, params) = self.select.build_sql(DbType::Sqlite);

        let turso_params = values_into_params(params)?;

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).trace().await?
        } else {
            self.conn.query(&sql, turso_params).trace().await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().trace().await? {
            // 获取行中的所有值（从 column_count 获取列数）
            let column_count = self.select.column_count();
            let typed_value =
                common_helpers::decode_row_values_from_indexed_values(column_count, |i| {
                    let value = row.get_value(i).trace_for("turso::Row::get_value")?;
                    convert_turso_value(&value)
                })?;
            results.push(typed_value);
        }

        Ok(results.into_iter().collect())
    }
}

/// SelectStream - 流式查询执行器 (SQLite/Turso)
///
/// 该执行器用于创建流式查询，允许逐行读取数据而不是一次性加载所有结果到内存中。
/// 适用于处理大量数据的场景，内存占用为 O(1)。
///
/// # 示例
///
/// ```text
/// let mut stream = db.select::<User>().stream().into_iter().trace().await?;
/// while let Some(result) = stream.next().await {
///     let user = result?;
///     println!("User: {:?}", user);
/// }
/// ```
///
/// # 连接管理
///
/// 该执行器持有 `Arc<turso::Connection>` 的克隆，确保在流式查询期间连接保持活跃。
/// 当 `SelectStreamIterator` 被 drop 时，连接会自动释放（通过 Arc 的引用计数）。
pub struct SelectStream<'a, T: Model> {
    select: Select<T>,
    conn: super::common::StreamConnection<'a>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器
    pub async fn into_iter(self) -> crate::Result<SelectStreamIterator<'a, T>> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        // 从 StreamConnection 获取连接
        let conn = self.conn.expect_sqlite().clone();

        let turso_params = values_into_params(params)?;

        let rows = if turso_params.is_empty() {
            conn.query(&sql, ()).trace().await?
        } else {
            conn.query(&sql, turso_params).trace().await?
        };

        Ok(SelectStreamIterator {
            conn: super::common::StreamConnection::Sqlite(conn),
            rows,
            polluted: false,
            _marker: std::marker::PhantomData,
        })
    }
}

/// SelectStreamIterator - 流式查询迭代器 (SQLite/Turso)
///
/// 该迭代器用于逐行读取流式查询的结果。
/// 每次调用 `next()` 方法会从数据库中获取下一行数据。
///
/// # 错误处理
///
/// 如果在解析行数据时发生错误，迭代器会被标记为"污染"状态，
/// 后续调用 `next()` 将直接返回 `None`，避免连续错误。
///
/// # 资源释放
///
/// 当迭代器被 drop 时（无论是正常完成、提前终止还是发生错误），
/// 底层的 turso::Rows 会自动关闭游标，连接会通过 Arc 的引用计数自动释放。
pub struct SelectStreamIterator<'a, T: Model> {
    #[allow(dead_code)]
    conn: super::common::StreamConnection<'a>,
    rows: turso::Rows,
    polluted: bool, // 标记是否发生解析错误，污染后不再尝试读取
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> Drop for SelectStreamIterator<'a, T> {
    fn drop(&mut self) {
        // turso::Rows 会在 Drop 时自动关闭游标并释放相关资源
        // StreamConnection 中的 Arc<turso::Connection> 会在最后一个引用释放时自动清理
        // 不需要显式操作，Rust 的 RAII 机制会确保资源正确释放
    }
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<crate::Result<T>> {
        // 如果已经污染，直接返回 None
        if self.polluted {
            return None;
        }

        match self.rows.next().trace_for("turso::Rows::next").await {
            Ok(Some(row)) => {
                // 解析行数据为 Model
                let mut data = HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    match row.get_value(i) {
                        Ok(value) => match convert_turso_value(&value) {
                            Ok(ormer_value) => {
                                data.insert(col_name.to_string(), ormer_value);
                            }
                            Err(e) => {
                                self.polluted = true;
                                return Some(Err(e));
                            }
                        },
                        Err(e) => {
                            self.polluted = true;
                            return Some(Err(crate::ormer_error!(
                                "turso::Row::get_value failed: {e}"
                            )));
                        }
                    }
                }
                let ormer_row = Row::new(data);
                Some(T::from_row(&ormer_row))
            }
            Ok(None) => None,
            Err(e) => {
                self.polluted = true;
                Some(Err(e))
            }
        }
    }
}
