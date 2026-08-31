use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers;
use crate::abstract_layer::common::{SingleSqlStatement, SqlExecutor, SqlStatement};
use crate::db_first::{
    DbFirstColumn, DbFirstForeignKey, DbFirstIndex, DbFirstIndexColumn, DbFirstTable,
};
use crate::hooks::{HookContext, HookOperation};
use crate::migration::{SchemaColumn, schema_column};
use crate::model::{DbBackendTypeMapper, Model, Value, WritableModel};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect,
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
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;
use tiberius::numeric::Numeric;
use tiberius::{Client, Config, Query};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

type ModelUpdateBatch = common_helpers::ModelUpdateBatch;
type CollectMarker<'a, C> = PhantomData<(fn() -> C, &'a ())>;
type MssqlBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
type MssqlClient = Client<tokio_util::compat::Compat<TcpStream>>;

fn build_traced_mssql_query<'a>(
    trace: &'a crate::sql_trace::SqlTraceExecution,
    params: &'a [Value],
) -> crate::Result<Query<'a>> {
    let mut query = Query::new(trace.sql());
    for param in params {
        bind_value(&mut query, param)?;
    }
    Ok(query)
}

async fn traced_mssql_execute(
    client: &mut MssqlClient,
    sql: &str,
    params: &[Value],
) -> crate::Result<u64> {
    let trace = crate::sql_trace::start_sql_trace(sql, params);
    let query = build_traced_mssql_query(&trace, params)?;
    match query.execute(client).await {
        Ok(result) => {
            trace.finish_ok();
            Ok(result.total() as u64)
        }
        Err(error) => Err(trace.finish_external_error("tiberius::Query::execute", error)),
    }
}

async fn traced_mssql_query(
    client: &mut MssqlClient,
    sql: &str,
    params: &[Value],
) -> crate::Result<Vec<tiberius::Row>> {
    let trace = crate::sql_trace::start_sql_trace(sql, params);
    let query = build_traced_mssql_query(&trace, params)?;
    match query.query(client).await {
        Ok(stream) => {
            trace.finish_ok();
            stream.into_first_result().trace().await
        }
        Err(error) => Err(trace.finish_external_error("tiberius::Query::query", error)),
    }
}

fn decode_mssql_model_rows<T: Model>(rows: Vec<tiberius::Row>) -> crate::Result<Vec<T>> {
    let mut results = Vec::new();
    for row in rows {
        let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
            extract_value_from_row(&row, i)
        })?;
        results.push(model);
    }
    Ok(results)
}

/// MSSQL 类型映射器
pub struct MSSQLTypeMapper;

impl DbBackendTypeMapper for MSSQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        if enum_variants.is_some() {
            return common_helpers::sql_type_with_nullability("VARCHAR(255)", is_nullable);
        }

        if is_primary {
            if matches!(rust_type, "Uuid" | "uuid::Uuid") {
                return "UNIQUEIDENTIFIER PRIMARY KEY".to_string();
            }
            let int_type = match rust_type {
                "i8" | "i16" | "u8" => "SMALLINT",
                "i32" | "u16" => "INT",
                "i64" | "u32" | "u64" => "BIGINT",
                _ => "INT",
            };
            if is_auto_increment {
                return format!("{int_type} PRIMARY KEY IDENTITY(1,1)");
            } else {
                return format!("{int_type} PRIMARY KEY");
            }
        }

        let base_type = match rust_type {
            "i8" => "SMALLINT",
            "i16" => "SMALLINT",
            "i32" => "INT",
            "i64" => "BIGINT",
            "u8" => "SMALLINT",
            "u16" => "INT",
            "u32" => "BIGINT",
            "u64" => "BIGINT",
            "f32" => "REAL",
            "f64" => "FLOAT",
            "Decimal" | "rust_decimal::Decimal" | "BigDecimal" | "bigdecimal::BigDecimal" => {
                "DECIMAL(38,18)"
            }
            "Duration" | "std::time::Duration" => "BIGINT",
            "String" => "NVARCHAR(255)",
            "bool" => "BIT",
            "Vec<u8>" | "&[u8]" => "VARBINARY(MAX)",
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => "DATETIME2",
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            "JsonValue" | "serde_json::Value" => "NVARCHAR(MAX)",
            "Uuid" | "uuid::Uuid" => "UNIQUEIDENTIFIER",
            _ => "NVARCHAR(255)",
        };

        common_helpers::sql_type_with_nullability(base_type, is_nullable)
    }
}

pub type Pool = Arc<Mutex<Client<tokio_util::compat::Compat<TcpStream>>>>;

/// MSSQL 数据库连接封装
pub struct Database {
    pool: Pool,
    connection_string: String,
}

fn mssql_escape_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn mssql_fk_action(action: &str) -> Option<&'static str> {
    match action.replace('_', " ").to_ascii_uppercase().as_str() {
        "NO ACTION" => Some("NO ACTION"),
        "RESTRICT" => Some("RESTRICT"),
        "CASCADE" => Some("CASCADE"),
        "SET NULL" => Some("SET NULL"),
        "SET DEFAULT" => Some("SET DEFAULT"),
        _ => None,
    }
}

impl Database {
    pub async fn connect(_db_type: super::DbType, connection_string: &str) -> crate::Result<Self> {
        let client = Self::connect_client(connection_string).await?;
        Ok(Self {
            pool: Arc::new(Mutex::new(client)),
            connection_string: connection_string.to_string(),
        })
    }

    async fn connect_client(connection_string: &str) -> crate::Result<MssqlClient> {
        let config = Config::from_ado_string(connection_string)
            .trace_for("tiberius::Config::from_ado_string")?;
        let tcp = TcpStream::connect(config.get_addr()).trace().await?;
        tcp.set_nodelay(true)
            .trace_for("tokio::net::TcpStream::set_nodelay")?;
        Client::connect(config, tcp.compat_write())
            .trace_for("tiberius::Client::connect")
            .await
    }

    pub(crate) async fn db_first_tables(
        &self,
        schema: Option<&str>,
    ) -> crate::Result<Vec<DbFirstTable>> {
        let schema_name = schema.filter(|value| !value.is_empty()).unwrap_or("dbo");
        let schema_literal = mssql_escape_literal(schema_name);
        let migration_literal = mssql_escape_literal(crate::migration::MIGRATION_TABLE_NAME);
        let query_sql = format!(
            "SELECT TABLE_SCHEMA, TABLE_NAME \
             FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_TYPE = 'BASE TABLE' \
               AND TABLE_SCHEMA = '{}' \
               AND TABLE_NAME != '{}' \
             ORDER BY TABLE_NAME",
            schema_literal, migration_literal
        );
        let mut client = self.pool.lock().await;
        let stream = Query::new(&query_sql).query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        let table_names = rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<&str, _>(0).unwrap_or("").to_string(),
                    row.get::<&str, _>(1).unwrap_or("").to_string(),
                )
            })
            .collect::<Vec<_>>();
        drop(client);

        let mut tables = Vec::with_capacity(table_names.len());
        for (schema_name, table_name) in table_names {
            tables.push(self.db_first_table(&schema_name, &table_name).await?);
        }
        Ok(tables)
    }

    async fn db_first_table(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<DbFirstTable> {
        Ok(DbFirstTable {
            schema: Some(schema_name.to_string()),
            name: table_name.to_string(),
            columns: self.db_first_columns(schema_name, table_name).await?,
            indexes: self.db_first_indexes(schema_name, table_name).await?,
            foreign_keys: self.db_first_foreign_keys(schema_name, table_name).await?,
        })
    }

    async fn db_first_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstColumn>> {
        let schema_literal = mssql_escape_literal(schema_name);
        let table_literal = mssql_escape_literal(table_name);
        let columns_sql = format!(
            "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, \
                    CASE WHEN pk.COLUMN_NAME IS NULL THEN 0 ELSE 1 END AS IS_PRIMARY, \
                    CASE WHEN sc.is_identity = 1 THEN 1 ELSE 0 END AS IS_IDENTITY \
             FROM INFORMATION_SCHEMA.COLUMNS c \
             JOIN sys.schemas s ON s.name = c.TABLE_SCHEMA \
             JOIN sys.tables t ON t.name = c.TABLE_NAME AND t.schema_id = s.schema_id \
             JOIN sys.columns sc ON sc.object_id = t.object_id AND sc.name = c.COLUMN_NAME \
             LEFT JOIN ( \
                 SELECT k.TABLE_SCHEMA, k.TABLE_NAME, k.COLUMN_NAME \
                 FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
                 JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE k \
                   ON tc.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
                  AND tc.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
                  AND tc.TABLE_SCHEMA = k.TABLE_SCHEMA \
                  AND tc.TABLE_NAME = k.TABLE_NAME \
                 WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' \
             ) pk ON pk.TABLE_SCHEMA = c.TABLE_SCHEMA \
                 AND pk.TABLE_NAME = c.TABLE_NAME \
                 AND pk.COLUMN_NAME = c.COLUMN_NAME \
             WHERE c.TABLE_SCHEMA = '{}' AND c.TABLE_NAME = '{}' \
             ORDER BY c.ORDINAL_POSITION",
            schema_literal, table_literal
        );
        let mut client = self.pool.lock().await;
        let stream = Query::new(&columns_sql).query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        let columns = rows
            .into_iter()
            .map(|row| {
                let name = row.get::<&str, _>(0).unwrap_or("").to_string();
                let type_name = row.get::<&str, _>(1).unwrap_or("").to_string();
                let nullable = row.get::<&str, _>(2).unwrap_or("") == "YES";
                let default = row.get::<&str, _>(3).map(str::to_string);
                let primary_key = row.get::<i32, _>(4).unwrap_or(0) != 0;
                let auto_increment = row.get::<i32, _>(5).unwrap_or(0) != 0;
                DbFirstColumn {
                    name,
                    type_name,
                    nullable: nullable && !primary_key,
                    primary_key,
                    auto_increment,
                    enum_variants: Vec::new(),
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
        let schema_literal = mssql_escape_literal(schema_name);
        let table_literal = mssql_escape_literal(table_name);
        let indexes_sql = format!(
            "SELECT i.name, \
                    CASE WHEN i.is_unique = 1 THEN 1 ELSE 0 END AS IS_UNIQUE, \
                    c.name, \
                    CASE WHEN ic.is_descending_key = 1 THEN 1 ELSE 0 END AS IS_DESC \
             FROM sys.indexes i \
             JOIN sys.tables t ON t.object_id = i.object_id \
             JOIN sys.schemas s ON s.schema_id = t.schema_id \
             JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
             JOIN sys.columns c ON c.object_id = t.object_id AND c.column_id = ic.column_id \
             WHERE s.name = '{}' \
               AND t.name = '{}' \
               AND i.name IS NOT NULL \
               AND i.is_primary_key = 0 \
               AND i.is_hypothetical = 0 \
               AND ic.key_ordinal > 0 \
             ORDER BY i.name, ic.key_ordinal",
            schema_literal, table_literal
        );
        let mut client = self.pool.lock().await;
        let stream = Query::new(&indexes_sql).query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        let mut indexes = BTreeMap::<String, DbFirstIndex>::new();
        for row in rows {
            let name = row.get::<&str, _>(0).unwrap_or("").to_string();
            let unique = row.get::<i32, _>(1).unwrap_or(0) != 0;
            let column = row.get::<&str, _>(2).unwrap_or("").to_string();
            let descending = row.get::<i32, _>(3).unwrap_or(0) != 0;
            indexes
                .entry(name.clone())
                .or_insert_with(|| DbFirstIndex {
                    name,
                    columns: Vec::new(),
                    unique,
                })
                .columns
                .push(DbFirstIndexColumn {
                    name: column,
                    descending,
                });
        }
        Ok(indexes.into_values().collect())
    }

    async fn db_first_foreign_keys(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstForeignKey>> {
        let schema_literal = mssql_escape_literal(schema_name);
        let table_literal = mssql_escape_literal(table_name);
        let foreign_keys_sql = format!(
            "SELECT fk.name, parent_col.name, ref_schema.name, ref_table.name, ref_col.name, \
                    fk.delete_referential_action_desc, fk.update_referential_action_desc \
             FROM sys.foreign_keys fk \
             JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
             JOIN sys.tables parent_table ON parent_table.object_id = fk.parent_object_id \
             JOIN sys.schemas parent_schema ON parent_schema.schema_id = parent_table.schema_id \
             JOIN sys.tables ref_table ON ref_table.object_id = fk.referenced_object_id \
             JOIN sys.schemas ref_schema ON ref_schema.schema_id = ref_table.schema_id \
             JOIN sys.columns parent_col \
               ON parent_col.object_id = parent_table.object_id \
              AND parent_col.column_id = fkc.parent_column_id \
             JOIN sys.columns ref_col \
               ON ref_col.object_id = ref_table.object_id \
              AND ref_col.column_id = fkc.referenced_column_id \
             WHERE parent_schema.name = '{}' AND parent_table.name = '{}' \
             ORDER BY fk.name, fkc.constraint_column_id",
            schema_literal, table_literal
        );
        let mut client = self.pool.lock().await;
        let stream = Query::new(&foreign_keys_sql)
            .query(&mut *client)
            .trace()
            .await?;
        let rows = stream.into_first_result().trace().await?;
        let mut foreign_keys = Vec::with_capacity(rows.len());
        for row in rows {
            let name = row.get::<&str, _>(0).unwrap_or("").to_string();
            let column = row.get::<&str, _>(1).unwrap_or("").to_string();
            let ref_schema = row.get::<&str, _>(2).unwrap_or("").to_string();
            let ref_table = row.get::<&str, _>(3).unwrap_or("").to_string();
            let ref_column = row.get::<&str, _>(4).unwrap_or("").to_string();
            let on_delete = row.get::<&str, _>(5).unwrap_or("");
            let on_update = row.get::<&str, _>(6).unwrap_or("");
            foreign_keys.push(DbFirstForeignKey {
                name: Some(name),
                column,
                ref_schema: Some(ref_schema),
                ref_table,
                ref_column,
                on_delete: mssql_fk_action(on_delete).map(str::to_string),
                on_update: mssql_fk_action(on_update).map(str::to_string),
            });
        }
        Ok(foreign_keys)
    }

    pub fn get_pool(&self) -> Pool {
        self.pool.clone()
    }

    pub fn is_valid(&self) -> bool {
        true
    }

    pub async fn exec_sql(&self, sql: &str) -> crate::Result<u64> {
        let mut client = self.pool.lock().await;
        let query = Query::new(sql);
        let result = query.execute(&mut *client).trace().await?;
        Ok(result.total())
    }

    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            pool: self.pool.clone(),
            table_name: None,
            _marker: PhantomData,
        }
    }

    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            pool: self.pool.clone(),
            models,
            conflict: None,
            _marker: PhantomData,
        }
    }

    pub fn insert_partial<T: WritableModel>(&self) -> InsertPartialExecutor<'_, T> {
        InsertPartialExecutor {
            pool: self.pool.clone(),
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

    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        InsertOrUpdateExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        InsertOrIgnoreExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub async fn insert_impl<T: Model>(
        &self,
        models: &[&T],
    ) -> crate::Result<T::AutoIncrementKeyType> {
        if models.is_empty() {
            return Ok(T::AutoIncrementKeyType::default());
        }

        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let columns = T::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_with_columns(
            DbType::MSSQL,
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );

        let mut client = self.pool.lock().await;

        if has_auto_increment {
            // 获取自增主键列名
            let pk_col = T::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            // 使用 OUTPUT 子句获取插入的ID
            let sql_with_output = format!(
                "{} OUTPUT {}",
                sql,
                common_helpers::quote_column_with_prefix(DbType::MSSQL, "inserted", pk_col)
            );
            let mut query = Query::new(&sql_with_output);
            for param in &all_values {
                bind_value(&mut query, param)?;
            }
            let stream = query.query(&mut *client).trace().await?;
            let row = stream.into_row().trace().await?;
            let id: i64 = row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0);
            let result = common_helpers::convert_auto_increment_key::<T::AutoIncrementKeyType>(id)?;
            Ok(result)
        } else {
            let mut query = Query::new(&sql);
            for param in &all_values {
                bind_value(&mut query, param)?;
            }
            query.execute(&mut *client).trace().await?;
            Ok(T::AutoIncrementKeyType::default())
        }
    }

    pub async fn insert_or_update_impl<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<T>(models);
        common_helpers::append_mssql_merge_update_clause::<T>(&mut sql);
        common_helpers::append_mssql_merge_insert_clause::<T>(&mut sql);

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param)?;
        }
        query.execute(&mut *client).trace().await?;
        Ok(())
    }

    pub async fn insert_or_ignore_impl<T: Model>(&self, models: &[&T]) -> crate::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<T>(models);
        common_helpers::append_mssql_merge_insert_clause::<T>(&mut sql);

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param)?;
        }
        let result = query.execute(&mut *client).trace().await?;
        Ok(result.total() as u64)
    }

    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_multi_table<T: Model + 'static, R1: Model, R2: Model>(
        &self,
    ) -> MultiTableSelectExecutor<'_, T, R1, R2> {
        MultiTableSelectExecutor {
            select: Select::<T>::new().from3::<T, R1, R2>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_four_table<T: Model + 'static, R1: Model, R2: Model, R3: Model>(
        &self,
    ) -> FourTableSelectExecutor<'_, T, R1, R2, R3> {
        FourTableSelectExecutor {
            select: Select::<T>::new().from4::<T, R1, R2, R3>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_mapped<T: Model, V>(
        &self,
        mapped: crate::query::builder::MappedSelect<T, V>,
    ) -> MappedSelectExecutor<'_, T, V> {
        MappedSelectExecutor {
            select: mapped,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_grouped<T: Model, V>(
        &self,
        grouped: GroupedSelect<T, V>,
    ) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: grouped,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub async fn transaction(&self) -> crate::Result<Transaction<'_>> {
        self.begin().await
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        let mut client = self.pool.lock().await;
        let table_filter = if let Some((schema_name, table_name)) = T::TABLE_NAME.rsplit_once('.') {
            format!("TABLE_SCHEMA = '{schema_name}' AND TABLE_NAME = '{table_name}'")
        } else {
            format!("TABLE_NAME = '{}'", T::TABLE_NAME)
        };

        // 检查表是否存在
        let check_sql =
            format!("SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE {table_filter}");
        {
            let query = Query::new(&check_sql);
            let stream = query.query(&mut *client).trace().await?;
            let rows = stream.into_first_result().trace().await?;
            if rows.is_empty() {
                return Err(crate::ormer_error!(
                    "Table {} does not exist",
                    T::TABLE_NAME
                ));
            }
            // 尝试读取 COUNT 结果
            if let Ok(Some(count)) = rows[0].try_get::<i32, _>(0) {
                if count == 0 {
                    return Err(crate::ormer_error!(
                        "Table {} does not exist",
                        T::TABLE_NAME
                    ));
                }
            }
        }

        // 查询表的列信息
        let col_sql = format!(
            "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, \
                    CASE WHEN COLUMNPROPERTY(OBJECT_ID(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)), c.COLUMN_NAME, 'IsIdentity') = 1 THEN 1 ELSE 0 END, \
                    CASE WHEN EXISTS ( \
                        SELECT 1 \
                        FROM sys.indexes i \
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                        JOIN sys.columns sc ON sc.object_id = ic.object_id AND sc.column_id = ic.column_id \
                        WHERE i.is_primary_key = 1 \
                          AND i.object_id = OBJECT_ID(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)) \
                          AND sc.name = c.COLUMN_NAME \
                    ) THEN 1 ELSE 0 END \
             FROM INFORMATION_SCHEMA.COLUMNS c WHERE {table_filter} ORDER BY c.ORDINAL_POSITION"
        );
        let query = Query::new(&col_sql);
        let stream = query.query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool, bool)> = Vec::new();
        for row in rows {
            let name: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            let col_type: String = row.get::<&str, _>(1).unwrap_or("").to_string();
            let nullable: String = row.get::<&str, _>(2).unwrap_or("").to_string();
            let auto_increment = row.get::<i32, _>(3).unwrap_or(0) != 0;
            let primary_key = row.get::<i32, _>(4).unwrap_or(0) != 0;
            actual_columns.push((
                name.to_lowercase(),
                col_type.to_lowercase(),
                nullable == "YES",
                primary_key,
                auto_increment,
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
            if actual_name != &expected_col.name.to_lowercase() {
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

            // 提取预期的 SQL 类型
            let effective_rust_type = expected_col.data_type.unwrap_or(expected_col.rust_type);
            let expected_sql_type = MSSQLTypeMapper::sql_type(
                effective_rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                false,
                expected_col.enum_variants,
            );
            // 提取基础类型（第一个单词，去除约束和大小）
            let base_expected = expected_sql_type
                .split([' ', '('])
                .next()
                .unwrap_or("")
                .to_lowercase();

            // 检查列类型
            if &base_expected != actual_type {
                // 处理特殊情况：MSSQL 的 INT/INTEGER
                let compatible_type = (base_expected == "int" && actual_type == "integer")
                    || (base_expected == "integer" && actual_type == "int")
                    || (base_expected == "nvarchar" && actual_type == "varchar")
                    || (base_expected == "nchar" && actual_type == "char");
                if !compatible_type {
                    return Err(crate::ormer_error!(
                        "Schema mismatch: table {}, reason: Column type mismatch at column '{}': expected '{}', but actual is '{}'",
                        T::TABLE_NAME,
                        expected_col.name,
                        base_expected,
                        actual_type
                    ));
                }
            }

            let expected_nullable = expected_col.is_nullable;
            if !expected_col.is_primary && (*actual_nullable != expected_nullable) {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                    T::TABLE_NAME,
                    expected_col.name,
                    if expected_nullable { "" } else { "NOT " },
                    if *actual_nullable { "" } else { "NOT " }
                ));
            }
        }

        drop(client);
        let (schema_name, table_name) = T::TABLE_NAME
            .rsplit_once('.')
            .unwrap_or(("dbo", T::TABLE_NAME));
        let actual_table = self.db_first_table(schema_name, table_name).await?;
        crate::db_first::validate_model_constraints::<T>(
            crate::abstract_layer::DbType::MSSQL,
            &actual_table,
        )?;
        Ok(())
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<Transaction<'_>> {
        let client = Self::connect_client(&self.connection_string).await?;
        let pool = Arc::new(Mutex::new(client));
        {
            let mut client = pool.lock().await;
            traced_mssql_execute(&mut client, "BEGIN TRANSACTION", &[]).await?;
        }
        Ok(Transaction {
            pool,
            state: common_helpers::TransactionState::Active,
            _marker: PhantomData,
        })
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(DbType::MSSQL)?;
        self.exec_raw(&sql, params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let mut client = self.pool.lock().await;
        let rows = traced_mssql_query(&mut client, sql, &params).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(common_helpers::decode_row_values_from_indexed_values(
                row.columns().len(),
                |i| extract_value_from_row(&row, i),
            )?);
        }
        Ok(results.into_iter().collect())
    }

    pub(crate) async fn exec_raw(&self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let mut client = self.pool.lock().await;
        traced_mssql_execute(&mut client, sql, &params).await
    }

    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<(u64, String, u64)>> {
        let mut client = self.pool.lock().await;
        let query =
            Query::new("SELECT version, name, checksum FROM __ormer_migrations ORDER BY version");
        let stream = query.query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        rows.into_iter()
            .filter_map(|row| {
                let version = row.try_get::<i64, _>(0).ok().flatten()?;
                let name = row
                    .try_get::<&str, _>(1)
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string();
                let checksum = row
                    .try_get::<&str, _>(2)
                    .ok()
                    .flatten()?
                    .parse::<u64>()
                    .ok()?;
                Some((version, name, checksum))
            })
            .map(|(version, name, checksum)| {
                Ok((
                    u64::try_from(version)
                        .map_err(|_| crate::ormer_error!("Migration version cannot be negative"))?,
                    name,
                    checksum,
                ))
            })
            .collect()
    }

    pub(crate) async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<SchemaColumn>>> {
        let (schema_name, table_name) = table_name.rsplit_once('.').unwrap_or(("dbo", table_name));
        let mut client = self.pool.lock().await;
        let exists_sql = format!(
            "SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}'",
            schema_name.replace('\'', "''"),
            table_name.replace('\'', "''")
        );
        let stream = Query::new(&exists_sql).query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        let exists = rows
            .first()
            .and_then(|row| row.try_get::<i32, _>(0).ok().flatten())
            .unwrap_or(0)
            > 0;
        if !exists {
            return Ok(None);
        }
        let columns_sql = format!(
            "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, \
                    CASE WHEN EXISTS (
                        SELECT 1 FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
                        JOIN INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                          ON tc.CONSTRAINT_NAME = k.CONSTRAINT_NAME
                         AND tc.TABLE_SCHEMA = k.TABLE_SCHEMA
                         AND tc.TABLE_NAME = k.TABLE_NAME
                        WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
                          AND k.TABLE_SCHEMA = c.TABLE_SCHEMA
                          AND k.TABLE_NAME = c.TABLE_NAME
                          AND k.COLUMN_NAME = c.COLUMN_NAME
                    ) THEN 1 ELSE 0 END AS IS_PRIMARY
             FROM INFORMATION_SCHEMA.COLUMNS c
             WHERE c.TABLE_SCHEMA = '{}' AND c.TABLE_NAME = '{}'
             ORDER BY c.ORDINAL_POSITION",
            schema_name.replace('\'', "''"),
            table_name.replace('\'', "''")
        );
        let stream = Query::new(&columns_sql).query(&mut *client).trace().await?;
        let rows = stream.into_first_result().trace().await?;
        let columns = rows
            .into_iter()
            .map(|row| {
                let name = row.get::<&str, _>(0).unwrap_or("").to_string();
                let type_name = row.get::<&str, _>(1).unwrap_or("").to_string();
                let nullable = row.get::<&str, _>(2).unwrap_or("") == "YES";
                let primary_key = row.get::<i32, _>(3).unwrap_or(0) != 0;
                schema_column(name, type_name, nullable, primary_key)
            })
            .collect();
        Ok(Some(columns))
    }
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: crate::model::WritableModel> {
    pool: Pool,
    table_name: Option<String>,
    _marker: PhantomData<(T, &'a ())>,
}

impl<'a, T: crate::model::WritableModel> CreateTableExecutor<'a, T> {
    pub fn with_table_name(mut self, table_name: &str) -> Self {
        self.table_name = Some(table_name.to_string());
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::MSSQL,
            self.table_name.as_deref(),
        )?;
        Ok(SqlStatement::single(DbType::MSSQL, create_sql, Vec::new()))
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
        let mut client = self.pool.lock().await;
        for statement in sql.statements {
            Query::new(&statement.sql)
                .execute(&mut *client)
                .trace()
                .await?;
        }
        Ok(())
    }
}

/// 删除表执行器
pub struct DropTableExecutor<'a, T: crate::model::WritableModel> {
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

impl<'a, T: crate::model::WritableModel> DropTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        Ok(SqlStatement::single(
            DbType::MSSQL,
            format!(
                "DROP TABLE IF EXISTS {}",
                common_helpers::quote_table_name::<T>(DbType::MSSQL)
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
        let mut client = self.pool.lock().await;
        for statement in sql.statements {
            Query::new(&statement.sql)
                .execute(&mut *client)
                .trace()
                .await?;
        }
        Ok(())
    }
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: PhantomData<&'a ()>,
}

impl_insert_conflict_methods!(InsertExecutor, with_conflict);

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }

        if let Some(conflict) = self
            .conflict
            .as_ref()
            .filter(|conflict| conflict.is_configured())
        {
            let statement =
                common_helpers::build_mssql_insert_conflict_statement::<I::Model>(&refs, conflict)?;
            return Ok(SqlStatement::single(
                DbType::MSSQL,
                statement.sql,
                statement.params,
            ));
        }

        if common_helpers::auto_increment_column::<I::Model>().is_some() {
            let (sql, all_values) =
                common_helpers::build_insert_statement_with_auto_increment_returning::<I::Model>(
                    DbType::MSSQL,
                    &refs,
                )?;

            return Ok(SqlStatement::single(DbType::MSSQL, sql, all_values));
        }

        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::MSSQL,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::batch(
            DbType::MSSQL,
            statements
                .into_iter()
                .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
                .collect(),
        ))
    }

    pub async fn execute(self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(mut self) -> crate::Result<Vec<I::Model>> {
        if self.models.as_refs().is_empty() {
            return Ok(Vec::new());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let sql = self.to_sql()?;
        let mut client = self.pool.lock().await;
        let mut results = Vec::new();
        for statement in &sql.statements {
            let returning_sql =
                common_helpers::mssql_insert_returning_sql::<I::Model>(&statement.sql);
            let rows = traced_mssql_query(&mut client, &returning_sql, &statement.params).await?;
            results.extend(decode_mssql_model_rows::<I::Model>(rows)?);
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

        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let mut client = self.pool.lock().await;

        let result = if has_auto_increment {
            let statement = &sql.statements[0];
            let mut query = Query::new(&statement.sql);
            for param in &statement.params {
                bind_value(&mut query, param)?;
            }
            let stream = query.query(&mut *client).trace().await?;
            let row = stream.into_row().trace().await?;
            let id: i64 = row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0);
            common_helpers::convert_auto_increment_key::<Self::Output>(id)
        } else {
            for statement in &sql.statements {
                let mut query = Query::new(&statement.sql);
                for param in &statement.params {
                    bind_value(&mut query, param)?;
                }
                query.execute(&mut *client).trace().await?;
            }
            Ok(<I::Model as Model>::AutoIncrementKeyType::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

pub struct InsertPartialExecutor<'a, T: Model> {
    pool: Pool,
    assignments: Vec<InsertAssignment>,
    source_table: Option<&'static str>,
    _marker: PhantomData<&'a T>,
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
        common_helpers::validate_insert_model_table::<T>(DbType::MSSQL, self.source_table)?;
        let statement =
            common_helpers::build_partial_insert_statement_with_auto_increment_returning::<T>(
                DbType::MSSQL,
                &self.assignments,
            )?;
        Ok(SqlStatement::single(
            DbType::MSSQL,
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
        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&statement.sql);
        for param in &statement.params {
            bind_value(&mut query, param)?;
        }

        if has_auto_increment {
            let stream = query.query(&mut *client).trace().await?;
            let row = stream.into_row().trace().await?;
            let id: i64 = row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0);
            common_helpers::convert_auto_increment_key::<Self::Output>(id)
        } else {
            query.execute(&mut *client).trace().await?;
            Ok(<T as Model>::AutoIncrementKeyType::default())
        }
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<I::Model>(&refs);
        common_helpers::append_mssql_merge_update_clause::<I::Model>(&mut sql);
        common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);

        Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrUpdateExecutor<'a, I> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(0);
        }
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let statement = &sql.statements[0];
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&statement.sql);
        for param in &statement.params {
            bind_value(&mut query, param)?;
        }
        let result = query.execute(&mut *client).trace().await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(result.total() as u64)
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<I::Model>(&refs);
        common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);

        Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn insert_or_ignore_impl<T: Model>(&self, models: &[&T]) -> crate::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<T>(models);
        common_helpers::append_mssql_merge_insert_clause::<T>(&mut sql);

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param)?;
        }
        let result = query.execute(&mut *client).trace().await?;
        Ok(result.total() as u64)
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrIgnoreExecutor<'a, I> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(0);
        }
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let statement = &sql.statements[0];
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&statement.sql);
        for param in &statement.params {
            bind_value(&mut query, param)?;
        }
        let result = query.execute(&mut *client).trace().await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(result.total() as u64)
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 关联查询执行器
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 多表查询执行器
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 四表查询执行器
pub struct FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 左连接查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 内连接查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 右连接查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 映射查询结果执行器
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: crate::query::builder::MappedSelect<T, V>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 分组查询执行器
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 删除执行器
pub struct DeleteExecutor<'a, T: Model> {
    filters: Vec<crate::query::filter::FilterExpr>,
    versioned: bool,
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

/// 更新执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<UpdateAssignment>,
    filters: Vec<FilterExpr>,
    model_updates: ModelUpdateBatch,
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

/// 事务
pub struct Transaction<'a> {
    pool: Pool,
    state: common_helpers::TransactionState,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Drop for Transaction<'a> {
    fn drop(&mut self) {
        if !self.state.is_active() {
            return;
        }

        self.state = common_helpers::TransactionState::RolledBack;
        let pool = self.pool.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut client = pool.lock().await;
                let query = Query::new("ROLLBACK");
                let _ = query.execute(&mut *client).trace().await;
            });
        }
        // Outside a runtime the private connection is dropped here; closing the
        // connection makes SQL Server roll back its active transaction.
    }
}

impl<'a> Transaction<'a> {
    pub(crate) async fn exec_raw(&mut self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        let mut client = self.pool.lock().await;
        traced_mssql_execute(&mut client, sql, &params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let mut client = self.pool.lock().await;
        let rows = traced_mssql_query(&mut client, sql, &params).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(common_helpers::decode_row_values_from_indexed_values(
                row.columns().len(),
                |i| extract_value_from_row(&row, i),
            )?);
        }
        Ok(results.into_iter().collect())
    }

    pub async fn commit(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        let mut client = self.pool.lock().await;
        let query = Query::new("COMMIT");
        query.execute(&mut *client).trace().await?;
        self.state = common_helpers::TransactionState::Committed;
        Ok(())
    }

    pub async fn rollback(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        {
            let mut client = self.pool.lock().await;
            let query = Query::new("ROLLBACK");
            query.execute(&mut *client).trace().await?;
        }
        self.state = common_helpers::TransactionState::RolledBack;
        Ok(())
    }

    pub async fn close(self) -> crate::Result<()> {
        self.rollback().await
    }

    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            pool: self.pool.clone(),
            models,
            conflict: None,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        TransactionInsertOrUpdateExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        TransactionInsertOrIgnoreExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }
}

/// 事务插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: PhantomData<&'a ()>,
}

impl_insert_conflict_methods!(TransactionInsertExecutor);

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }

        if let Some(conflict) = self
            .conflict
            .as_ref()
            .filter(|conflict| conflict.is_configured())
        {
            let statement =
                common_helpers::build_mssql_insert_conflict_statement::<I::Model>(&refs, conflict)?;
            return Ok(SqlStatement::single(
                DbType::MSSQL,
                statement.sql,
                statement.params,
            ));
        }

        if common_helpers::auto_increment_column::<I::Model>().is_some() {
            let (sql, all_values) =
                common_helpers::build_insert_statement_with_auto_increment_returning::<I::Model>(
                    DbType::MSSQL,
                    &refs,
                )?;

            return Ok(SqlStatement::single(DbType::MSSQL, sql, all_values));
        }

        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::MSSQL,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::batch(
            DbType::MSSQL,
            statements
                .into_iter()
                .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
                .collect(),
        ))
    }

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

        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let mut client = self.pool.lock().await;

        let result = if has_auto_increment {
            let statement = &sql.statements[0];
            let mut query = Query::new(&statement.sql);
            for param in &statement.params {
                bind_value(&mut query, param)?;
            }
            let stream = query.query(&mut *client).trace().await?;
            let row = stream.into_row().trace().await?;
            let id: i64 = row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0);
            common_helpers::convert_auto_increment_key::<<I::Model as Model>::AutoIncrementKeyType>(
                id,
            )
        } else {
            for statement in &sql.statements {
                let mut query = Query::new(&statement.sql);
                for param in &statement.params {
                    bind_value(&mut query, param)?;
                }
                query.execute(&mut *client).trace().await?;
            }
            Ok(<<I::Model as Model>::AutoIncrementKeyType>::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

/// 事务插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<I::Model>(&refs);
        common_helpers::append_mssql_merge_update_clause::<I::Model>(&mut sql);
        common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);

        Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
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
        let statement = &sql.statements[0];
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&statement.sql);
        for param in &statement.params {
            bind_value(&mut query, param)?;
        }
        query.execute(&mut *client).trace().await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 事务插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::MSSQL, Vec::new()));
        }
        let (mut sql, all_values) = common_helpers::build_mssql_merge_source::<I::Model>(&refs);
        common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);

        Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
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
        let statement = &sql.statements[0];
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&statement.sql);
        for param in &statement.params {
            bind_value(&mut query, param)?;
        }
        query.execute(&mut *client).trace().await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 收集 Future
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: CollectMarker<'a, C>,
}

/// 单条记录查询 Future
pub struct FirstFuture<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
}

/// 聚合 Future
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 左连接收集 Future
pub struct LeftJoinCollectFuture<'a, T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 内连接收集 Future
pub struct InnerJoinCollectFuture<'a, T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 右连接收集 Future
pub struct RightJoinCollectFuture<'a, T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 关联收集 Future
pub struct RelatedCollectFuture<'a, T: Model, R: Model> {
    executor: RelatedSelectExecutor<'a, T, R>,
    _marker: PhantomData<&'a ()>,
}

/// 映射收集 Future
pub struct MappedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    executor: MappedSelectExecutor<'a, T, V>,
    _marker: CollectMarker<'a, C>,
}

/// 分组收集 Future
pub struct GroupedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: CollectMarker<'a, C>,
}

/// 流式查询
pub struct SelectStream<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<&'a ()>,
}

impl_backend_executor_methods!(SelectExecutor, pool, Pool, Select);

// SelectExecutor 实现 - 基础方法（不需要 'static）
impl<'a, T: Model> SelectExecutor<'a, T> {
    pub(crate) fn select_model<R: Model>(&self) -> SelectExecutor<'a, R> {
        SelectExecutor {
            select: Select::new().with_context_filters(self.select.context_filters()),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let (sql, params) = self
            .select
            .try_to_sql_with_params(crate::abstract_layer::DbType::MSSQL)?;
        Ok(SqlStatement::single(
            crate::abstract_layer::DbType::MSSQL,
            sql,
            params,
        ))
    }

    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C, T>,
    {
        AggregateFuture {
            aggregate_select: self.select.count(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.sum(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.avg(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.max(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.min(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        MappedSelectExecutor {
            select: self.select.map_to(f),
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

    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(T::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        GroupedSelectExecutor {
            select: self.select.select_column(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

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

    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            executor: self,
            _marker: PhantomData,
        }
    }
}

// SelectExecutor 实现 - 需要 'static 的方法
impl<'a, T: Model + 'static> SelectExecutor<'a, T> {
    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }

    pub fn first(self) -> FirstFuture<'a, T> {
        FirstFuture { executor: self }
    }
}

// GroupedSelectExecutor 实现
impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
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

    pub fn as_model<R: Model>(self) -> crate::query::builder::DerivedSelect<R>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.select.as_model::<R>()
    }
}

impl<'a, T: Model + 'static, V: crate::model::FromRowValues + 'static>
    GroupedSelectExecutor<'a, T, V>
{
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C> {
        GroupedCollectFuture {
            executor: GroupedSelectExecutor {
                select: self.select.clone(),
                pool: self.pool.clone(),
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

// DeleteExecutor 实现
impl<'a, T: Model> DeleteExecutor<'a, T> {
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
            DbType::MSSQL,
            vec![SingleSqlStatement::new(sql, params).with_optimistic_lock(self.versioned, None)],
        ))
    }

    pub fn model(mut self, model: &T) -> Self {
        self.filters
            .extend(common_helpers::model_delete_filters(model));
        self.versioned = T::version_info().is_some();
        self
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let sql = self.to_sql()?;
        let mut client = self.pool.lock().await;
        let mut results = Vec::new();
        for statement in &sql.statements {
            let returning_sql = common_helpers::mssql_delete_returning_sql::<T>(&statement.sql);
            let rows = traced_mssql_query(&mut client, &returning_sql, &statement.params).await?;
            let statement_results = decode_mssql_model_rows::<T>(rows)?;
            if statement.versioned && statement_results.is_empty() {
                return Err(common_helpers::optimistic_lock_conflict::<T>());
            }
            results.extend(statement_results);
        }
        Ok(results)
    }

    fn build_sql_with_params(&self) -> (String, Vec<Value>) {
        common_helpers::build_delete_sql::<T>(DbType::MSSQL, &self.filters)
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
        let mut client = self.pool.lock().await;
        let affected = traced_mssql_execute(&mut client, &statement.sql, &statement.params).await?;
        if statement.versioned && affected == 0 {
            return Err(common_helpers::optimistic_lock_conflict::<T>());
        }
        Ok(affected)
    }
}

// UpdateExecutor 实现
impl<'a, T: Model> UpdateExecutor<'a, T> {
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
            DbType::MSSQL,
            statements
                .into_iter()
                .map(|statement| {
                    SingleSqlStatement::new(statement.sql, statement.params)
                        .with_optimistic_lock(statement.versioned, statement.version_update)
                })
                .collect(),
        ))
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let sql = self.to_sql()?;
        let mut client = self.pool.lock().await;
        let mut results = Vec::new();
        for statement in &sql.statements {
            let returning_sql = common_helpers::mssql_update_returning_sql::<T>(&statement.sql);
            let rows = traced_mssql_query(&mut client, &returning_sql, &statement.params).await?;
            let statement_results = decode_mssql_model_rows::<T>(rows)?;
            if statement.versioned && statement_results.is_empty() {
                return Err(common_helpers::optimistic_lock_conflict::<T>());
            }
            results.extend(statement_results);
        }
        Ok(results)
    }

    fn build_all_sql(&self) -> crate::Result<Vec<common_helpers::ModelSqlStatement>> {
        let mut statements = Vec::new();

        // Base UPDATE from sets/filters (manual .set()/.filter() calls)
        if !self.sets.is_empty() || (self.model_updates.is_empty() && !self.filters.is_empty()) {
            let (sql, params) =
                common_helpers::build_update_sql::<T>(DbType::MSSQL, &self.sets, &self.filters)?;
            statements.push(common_helpers::ModelSqlStatement {
                sql,
                params,
                versioned: false,
                version_update: None,
                param_columns: None,
            });
        }

        if let Some(batch_statements) = common_helpers::build_bulk_model_update_statements::<T>(
            DbType::MSSQL,
            &self.model_updates,
        )? {
            statements.extend(batch_statements);
        } else {
            for plan in &self.model_updates {
                statements.push(common_helpers::build_model_update_sql::<T>(
                    DbType::MSSQL,
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
        let mut client = self.pool.lock().await;
        let mut total: u64 = 0;
        for statement in &sql.statements {
            let affected =
                traced_mssql_execute(&mut client, &statement.sql, &statement.params).await?;
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

impl_backend_join_executor_methods_with_lifetime!(
    LeftJoinedSelectExecutor,
    pool,
    Pool,
    LeftJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    InnerJoinedSelectExecutor,
    pool,
    Pool,
    InnerJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    RightJoinedSelectExecutor,
    pool,
    Pool,
    RightJoinedSelect
);

// Join Executors 实现
impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
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

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
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

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
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

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 生成子查询SQL和参数
    pub fn to_subquery_sql(&self) -> crate::Result<(String, Vec<crate::model::Value>)> {
        self.select.try_to_sql_with_params(DbType::MSSQL)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        MappedCollectFuture {
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

    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }
}

impl_backend_related_executor_methods_with_lifetime!(
    RelatedSelectExecutor,
    pool,
    Pool,
    RelatedSelect
);

impl<'a, T: Model + 'static, R: Model + 'static> RelatedSelectExecutor<'a, T, R> {
    pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<'a, T, R> {
        RelatedCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    pub(crate) fn into_collect_future(self) -> RelatedCollectFuture<'a, T, R> {
        RelatedCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }
}

impl_backend_multi_table_executor_methods_with_lifetime!(
    MultiTableSelectExecutor,
    pool,
    Pool,
    MultiTableSelect
);
impl_backend_four_table_executor_methods_with_lifetime!(
    FourTableSelectExecutor,
    pool,
    Pool,
    FourTableSelect
);

// IntoFuture 实现
impl<'a, T: Model + 'static + std::marker::Send, C: FromIterator<T> + 'static>
    std::future::IntoFuture for CollectFuture<'a, T, C>
{
    type Output = crate::Result<C>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        let SelectExecutor {
            select,
            pool,
            _marker: _,
        } = self.executor;
        Box::pin(async move {
            let (sql, params) =
                select.try_to_sql_with_params(crate::abstract_layer::DbType::MSSQL)?;
            let mut client = pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let mut results = Vec::new();
            for row in rows {
                let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    extract_value_from_row(&row, i)
                })?;
                results.push(model);
            }
            Ok(results.into_iter().collect())
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send,
    R: crate::model::FromValue + 'static + std::marker::Send,
> std::future::IntoFuture for AggregateFuture<'a, T, R>
{
    type Output = crate::Result<R>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .aggregate_select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;
            if rows.is_empty() {
                return Err(crate::ormer_error!("Aggregate query returned no rows"));
            }
            let ormer_value = extract_value_from_row(&rows[0], 0)?;
            R::from_value(&ormer_value)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send + std::marker::Sync> std::future::IntoFuture
    for FirstFuture<'a, T>
{
    type Output = crate::Result<Option<T>>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<T> = self.executor.collect::<Vec<T>>().into_future().await?;
            Ok(results.into_iter().next())
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for LeftJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, Option<J>)>>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .try_to_sql_with_params(crate::abstract_layer::DbType::MSSQL)?;
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let t_model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    extract_value_from_row(&row, i)
                })?;
                let j_model = common_helpers::decode_optional_model_from_indexed_values::<J, _>(
                    t_col_count,
                    |i| extract_value_from_row(&row, i),
                )?;
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for InnerJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, J)>>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .try_to_sql_with_params(crate::abstract_layer::DbType::MSSQL)?;
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let t_model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    extract_value_from_row(&row, i)
                })?;
                let j_model =
                    common_helpers::decode_model_from_indexed_values::<J, _>(t_col_count, |i| {
                        extract_value_from_row(&row, i)
                    })?;
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RightJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(Option<T>, J)>>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let t_model =
                    common_helpers::decode_optional_model_from_indexed_values::<T, _>(0, |i| {
                        extract_value_from_row(&row, i)
                    })?;
                let j_model =
                    common_helpers::decode_model_from_indexed_values::<J, _>(t_col_count, |i| {
                        extract_value_from_row(&row, i)
                    })?;
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    R: Model + 'static + std::marker::Send + std::marker::Sync,
> std::future::IntoFuture for RelatedCollectFuture<'a, T, R>
where
    Self: 'a,
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let mut results = Vec::new();
            for row in rows {
                let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                    extract_value_from_row(&row, i)
                })?;
                results.push(model);
            }
            Ok(results)
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let mut results = Vec::new();
            for row in rows {
                let v = common_helpers::decode_row_values_from_indexed_values(
                    row.columns().len(),
                    |i| extract_value_from_row(&row, i),
                )?;
                results.push(v);
            }
            Ok(results.into_iter().collect())
        })
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
    type IntoFuture = MssqlBoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let rows = traced_mssql_query(&mut client, &sql, &params).await?;

            let mut results = Vec::new();
            for row in rows {
                let v = common_helpers::decode_row_values_from_indexed_values(
                    row.columns().len(),
                    |i| extract_value_from_row(&row, i),
                )?;
                results.push(v);
            }
            Ok(results.into_iter().collect())
        })
    }
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    pub async fn into_iter(self) -> crate::Result<SelectStreamIterator<'a, T>> {
        let (sql, params) = self
            .executor
            .select
            .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
        let mut client = self.executor.pool.lock().await;
        let rows = traced_mssql_query(&mut client, &sql, &params).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
                extract_value_from_row(&row, i)
            })?;
            results.push(model);
        }
        Ok(SelectStreamIterator {
            iter: results.into_iter(),
            _marker: PhantomData,
        })
    }
}

pub struct SelectStreamIterator<'a, T: Model> {
    iter: std::vec::IntoIter<T>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    pub async fn next(&mut self) -> Option<crate::Result<T>> {
        self.iter.next().map(Ok)
    }
}

// 辅助函数：从 tiberius Row 中提取 Value
fn extract_value_from_row(row: &tiberius::Row, idx: usize) -> crate::Result<Value> {
    // 尝试 i32 (INT)
    if let Ok(Some(v)) = row.try_get::<i32, _>(idx) {
        return Ok(Value::Integer(v as i64));
    }
    // 尝试 i64 (BIGINT)
    if let Ok(Some(v)) = row.try_get::<i64, _>(idx) {
        return Ok(Value::Integer(v));
    }
    // 尝试 i16 (SMALLINT)
    if let Ok(Some(v)) = row.try_get::<i16, _>(idx) {
        return Ok(Value::Integer(v as i64));
    }
    // 尝试 Numeric/Decimal
    if let Ok(Some(v)) = row.try_get::<Numeric, _>(idx) {
        return Ok(Value::BigDecimal(v.to_string()));
    }
    // 尝试 UUID (UNIQUEIDENTIFIER)
    if let Ok(Some(v)) = row.try_get::<uuid::Uuid, _>(idx) {
        return Ok(Value::Uuid(v));
    }
    // 尝试 &str (NVARCHAR, VARCHAR, CHAR)
    if let Ok(Some(v)) = row.try_get::<&str, _>(idx) {
        return Ok(Value::Text(v.to_string()));
    }
    // 尝试 f64 (FLOAT)
    if let Ok(Some(v)) = row.try_get::<f64, _>(idx) {
        return Ok(Value::Real(v));
    }
    // 尝试 f32 (REAL)
    if let Ok(Some(v)) = row.try_get::<f32, _>(idx) {
        return Ok(Value::Real(v as f64));
    }
    // 尝试 bool (BIT)
    if let Ok(Some(v)) = row.try_get::<bool, _>(idx) {
        return Ok(if v {
            Value::Integer(1)
        } else {
            Value::Integer(0)
        });
    }
    // 尝试 NaiveDateTime (DATETIME2, DATETIME)
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
        return Ok(Value::DateTime(
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(v, chrono::Utc),
        ));
    }
    // 尝试 NaiveDate (DATE)
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveDate, _>(idx) {
        return Ok(Value::Date(v));
    }
    // 尝试 NaiveTime (TIME)
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveTime, _>(idx) {
        return Ok(Value::Time(v));
    }
    // 尝试 &[u8] (VARBINARY, BINARY)
    if let Ok(Some(v)) = row.try_get::<&[u8], _>(idx) {
        return Ok(Value::Bytes(v.to_vec()));
    }
    // 值为 NULL 或无法识别的类型
    Ok(Value::Null)
}

fn decimal_text_to_mssql_numeric(value: &str) -> Numeric {
    let normalized = value.trim();
    let negative = normalized.starts_with('-');
    let unsigned = normalized.strip_prefix(['-', '+']).unwrap_or(normalized);
    let scale = unsigned
        .split_once('.')
        .map(|(_, fraction)| fraction.len())
        .unwrap_or(0)
        .min(37);
    let mut digits = unsigned.replace('.', "");
    if scale == 0 && digits.is_empty() {
        digits.push('0');
    }
    let mut integer = i128::from_str(&digits).unwrap_or(0);
    if negative {
        integer = -integer;
    }
    Numeric::new_with_scale(integer, scale as u8)
}

// 辅助函数：将 Value 绑定到 Query
fn bind_value<'a>(query: &mut Query<'a>, value: &'a Value) -> crate::Result<()> {
    match value {
        Value::Null => {
            query.bind(Option::<&str>::None);
        }
        Value::Boolean(v) => {
            query.bind(*v);
        }
        Value::Integer(v) => {
            query.bind(*v);
        }
        Value::BigInt(v) => {
            query.bind(*v as i64);
        }
        Value::Real(v) => {
            query.bind(*v);
        }
        Value::Decimal(v) | Value::BigDecimal(v) => {
            query.bind(decimal_text_to_mssql_numeric(v));
        }
        Value::Duration(v) => {
            let micros = v.as_micros().min(i64::MAX as u128) as i64;
            query.bind(micros);
        }
        Value::Text(v) => {
            query.bind(v.as_str());
        }
        Value::TextArray(v) => {
            query.bind(crate::model::stringify_string_vec(v));
        }
        Value::Bytes(v) => {
            query.bind(v.as_slice());
        }
        Value::DateTime(v) => {
            query.bind(v.naive_utc());
        }
        Value::Date(v) => {
            query.bind(*v);
        }
        Value::Time(v) => {
            query.bind(*v);
        }
        Value::Json(v) => {
            query.bind(v.to_string());
        }
        Value::Uuid(v) => {
            query.bind(*v);
        }
        Value::IntegerArray(_) | Value::BigIntArray(_) | Value::NullableBigIntArray(_) => {
            return Err(common_helpers::unsupported_postgresql_array_value(
                DbType::MSSQL,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::bind_value;
    use crate::OrmerError;
    use crate::abstract_layer::DbType;
    use crate::model::Value;
    use tiberius::Query;

    #[test]
    fn mssql_rejects_postgresql_array_values_without_panic() {
        let mut query = Query::new("SELECT @P1");
        let error = bind_value(&mut query, &Value::BigIntArray(vec![1, 2]))
            .expect_err("MSSQL must reject PostgreSQL array values");
        assert!(matches!(
            error,
            OrmerError::UnsupportedFeature {
                backend: DbType::MSSQL,
                feature: "PostgreSQL array values",
            }
        ));
    }
}
