use super::super::DbType;
use super::common_helpers;
use super::{DbExecutor, SqlExecutor, SqlStatement};
use crate::impl_insert_conflict_methods;
use crate::model::{FromRowValues, Model, RelationSelection, WritableModel};
use crate::query::builder::{ContextFilter, NamedFilterQuery, WhereExpr};
use crate::query::insert::InsertConflict;
use crate::raw_sql::{IntoRawSql, RawSql};
#[cfg(any(feature = "sqlite", feature = "mssql", feature = "duckdb"))]
use std::collections::VecDeque;
use std::marker::PhantomData;
#[cfg(any(feature = "sqlite", feature = "mssql", feature = "duckdb"))]
use std::sync::Arc;
#[cfg(any(feature = "sqlite", feature = "mssql", feature = "duckdb"))]
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
#[cfg(any(feature = "sqlite", feature = "mssql", feature = "duckdb"))]
use tokio::sync::Mutex;

#[cfg(feature = "postgresql")]
use bb8_postgres::PostgresConnectionManager;
#[cfg(feature = "postgresql")]
use tokio_postgres::NoTls;

// 导入统一的执行器类型
#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse"
))]
use super::unified::{
    CreateTableExecutor, DropTableExecutor, RelationNestedLoader, ScopedDeleteExecutor,
    ScopedUpdateExecutor, primary_key_filter, relation_owner_key,
};

/// 连接池插入执行器
pub struct PooledInsertExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: PhantomData<I>,
}

impl_insert_conflict_methods!(PooledInsertExecutor);

impl<'a, I: crate::model::Insertable> PooledInsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(
                db_type_for_connection(self.pooled_conn.get_connection()),
                Vec::new(),
            ));
        }

        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(_) => {
                if common_helpers::auto_increment_column::<I::Model>().is_some() {
                    let (sql, all_values) =
                        common_helpers::build_insert_statement_with_conflict_and_auto_increment_returning::<I::Model>(
                            DbType::Sqlite,
                            &refs,
                            self.conflict.as_ref(),
                        )?;
                    return Ok(SqlStatement::single(DbType::Sqlite, sql, all_values));
                }
                let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
                    DbType::Sqlite,
                    &refs,
                    self.conflict.as_ref(),
                )?;
                Ok(SqlStatement::batch(
                    DbType::Sqlite,
                    statements
                        .into_iter()
                        .map(|statement| {
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(_) => {
                if common_helpers::auto_increment_column::<I::Model>().is_some() {
                    let (sql, all_values) =
                        common_helpers::build_insert_statement_with_conflict_and_auto_increment_returning::<I::Model>(
                            DbType::PostgreSQL,
                            &refs,
                            self.conflict.as_ref(),
                        )?;
                    let rust_types = postgresql_backend::pg_insert_param_rust_types::<I::Model>(
                        refs.len(),
                        self.conflict.as_ref(),
                    );
                    return Ok(SqlStatement::batch(
                        DbType::PostgreSQL,
                        vec![
                            super::SingleSqlStatement::new(sql, all_values)
                                .with_param_rust_types(rust_types),
                        ],
                    ));
                }
                let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
                    DbType::PostgreSQL,
                    &refs,
                    self.conflict.as_ref(),
                )?;
                Ok(SqlStatement::batch(
                    DbType::PostgreSQL,
                    statements
                        .into_iter()
                        .map(|statement| {
                            let rust_types = postgresql_backend::pg_insert_param_rust_types::<
                                I::Model,
                            >(
                                statement.row_count, self.conflict.as_ref()
                            );
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                                .with_param_rust_types(rust_types)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(_) => {
                if common_helpers::auto_increment_column::<I::Model>().is_some() {
                    let (sql, all_values) =
                        common_helpers::build_insert_statement_with_conflict_and_auto_increment_returning::<I::Model>(
                            DbType::MySQL,
                            &refs,
                            self.conflict.as_ref(),
                        )?;
                    return Ok(SqlStatement::single(DbType::MySQL, sql, all_values));
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
                        .map(|statement| {
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(_) => {
                if common_helpers::auto_increment_column::<I::Model>().is_some() {
                    let (sql, all_values) =
                        common_helpers::build_insert_statement_with_conflict_and_auto_increment_returning::<I::Model>(
                            DbType::MSSQL,
                            &refs,
                            self.conflict.as_ref(),
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
                        .map(|statement| {
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(_) => {
                if common_helpers::auto_increment_column::<I::Model>().is_some() {
                    let (sql, all_values) =
                        common_helpers::build_insert_statement_with_conflict_and_auto_increment_returning::<I::Model>(
                            DbType::DuckDB,
                            &refs,
                            self.conflict.as_ref(),
                        )?;
                    return Ok(SqlStatement::single(DbType::DuckDB, sql, all_values));
                }
                let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
                    DbType::DuckDB,
                    &refs,
                    self.conflict.as_ref(),
                )?;
                Ok(SqlStatement::batch(
                    DbType::DuckDB,
                    statements
                        .into_iter()
                        .map(|statement| {
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "Model insert on ClickHouse",
            }),
        }
    }

    pub async fn execute(
        self,
    ) -> crate::Result<<I::Model as crate::model::Model>::AutoIncrementKeyType>
    where
        I: Send + Sync,
    {
        <Self as SqlExecutor>::execute(self).await
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for PooledInsertExecutor<'a, I> {
    type Output = <I::Model as crate::model::Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        PooledInsertExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, _sql: SqlStatement) -> crate::Result<Self::Output> {
        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                db.insert(self.models)
                    .with_conflict(self.conflict)
                    .execute()
                    .await
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                db.insert(self.models)
                    .with_conflict(self.conflict)
                    .execute()
                    .await
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                db.insert(self.models)
                    .with_conflict(self.conflict)
                    .execute()
                    .await
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                db.insert(self.models)
                    .with_conflict(self.conflict)
                    .execute()
                    .await
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                db.insert(self.models)
                    .with_conflict(self.conflict)
                    .execute()
                    .await
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "Model insert on ClickHouse",
            }),
        }
    }
}

/// 连接池插入或更新执行器
pub struct PooledInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    _marker: PhantomData<I>,
}

impl<'a, I: crate::model::Insertable> PooledInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(
                db_type_for_connection(self.pooled_conn.get_connection()),
                Vec::new(),
            ));
        }

        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::Sqlite,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::Sqlite),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );

                sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
                let mut first = true;
                for col_name in columns.iter() {
                    if primary_key_columns.contains(col_name) {
                        continue;
                    }
                    if !first {
                        sql.push_str(", ");
                    }
                    sql.push_str(&format!("{col_name} = excluded.{col_name}"));
                    first = false;
                }

                Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::PostgreSQL,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );
                sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
                let mut first = true;
                for col_name in columns.iter() {
                    if primary_key_columns.contains(col_name) {
                        continue;
                    }
                    if !first {
                        sql.push_str(", ");
                    }
                    sql.push_str(&format!("{col_name} = EXCLUDED.{col_name}"));
                    first = false;
                }
                let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
                    .iter()
                    .filter(|col| !col.is_auto_increment)
                    .map(|col| col.data_type.unwrap_or(col.rust_type))
                    .collect();
                Ok(SqlStatement::batch(
                    DbType::PostgreSQL,
                    vec![
                        super::SingleSqlStatement::new(sql, all_values)
                            .with_param_rust_types(rust_types),
                    ],
                ))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(_) => {
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::MySQL,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::MySQL),
                    I::Model::COLUMNS,
                    &refs,
                    common_helpers::BatchInsertValuesMode::All,
                );

                sql.push_str(" ON DUPLICATE KEY UPDATE ");
                let mut first = true;
                for col_name in I::Model::COLUMNS.iter() {
                    if !first {
                        sql.push_str(", ");
                    }
                    sql.push_str(&format!("{col_name} = VALUES({col_name})"));
                    first = false;
                }

                Ok(SqlStatement::single(DbType::MySQL, sql, all_values))
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(_) => {
                let (mut sql, all_values) =
                    common_helpers::build_mssql_merge_source::<I::Model>(&refs);
                common_helpers::append_mssql_merge_update_clause::<I::Model>(&mut sql);
                common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);
                Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::DuckDB,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::DuckDB),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );
                sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
                let mut first = true;
                for col_name in columns.iter() {
                    if primary_key_columns.contains(col_name) {
                        continue;
                    }
                    if !first {
                        sql.push_str(", ");
                    }
                    sql.push_str(&format!("{col_name} = excluded.{col_name}"));
                    first = false;
                }
                Ok(SqlStatement::single(DbType::DuckDB, sql, all_values))
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "conflict writes on ClickHouse",
            }),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

impl<'a, I: crate::model::Insertable> SqlExecutor for PooledInsertOrUpdateExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        PooledInsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, _sql: SqlStatement) -> crate::Result<Self::Output> {
        let refs = self.models.as_refs();
        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.insert_or_update_impl::<I::Model>(&refs).await,
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "conflict writes on ClickHouse",
            }),
        }
    }
}

/// 连接池插入或忽略执行器
pub struct PooledInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    _marker: PhantomData<I>,
}

impl<'a, I: crate::model::Insertable> PooledInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(
                db_type_for_connection(self.pooled_conn.get_connection()),
                Vec::new(),
            ));
        }

        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::Sqlite,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::Sqlite),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );

                sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));
                Ok(SqlStatement::single(DbType::Sqlite, sql, all_values))
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::PostgreSQL,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );
                sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));
                let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
                    .iter()
                    .filter(|col| !col.is_auto_increment)
                    .map(|col| col.data_type.unwrap_or(col.rust_type))
                    .collect();
                Ok(SqlStatement::batch(
                    DbType::PostgreSQL,
                    vec![
                        super::SingleSqlStatement::new(sql, all_values)
                            .with_param_rust_types(rust_types),
                    ],
                ))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(_) => {
                let (sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::MySQL,
                    "INSERT IGNORE INTO",
                    <I::Model as Model>::table_name_for_db(DbType::MySQL),
                    I::Model::COLUMNS,
                    &refs,
                    common_helpers::BatchInsertValuesMode::All,
                );

                Ok(SqlStatement::single(DbType::MySQL, sql, all_values))
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(_) => {
                let (mut sql, all_values) =
                    common_helpers::build_mssql_merge_source::<I::Model>(&refs);
                common_helpers::append_mssql_merge_insert_clause::<I::Model>(&mut sql);
                Ok(SqlStatement::single(DbType::MSSQL, sql, all_values))
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(_) => {
                let columns = I::Model::insert_columns();
                let primary_key_columns = I::Model::primary_key_columns();
                let primary_key = primary_key_columns.join(", ");
                let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
                    DbType::DuckDB,
                    "INSERT INTO",
                    <I::Model as Model>::table_name_for_db(DbType::DuckDB),
                    &columns,
                    &refs,
                    common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                );
                sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));
                Ok(SqlStatement::single(DbType::DuckDB, sql, all_values))
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "conflict writes on ClickHouse",
            }),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

fn db_type_for_connection(connection: &ConnectionWrapper) -> DbType {
    match connection {
        #[cfg(feature = "sqlite")]
        ConnectionWrapper::Sqlite(_) => DbType::Sqlite,
        #[cfg(feature = "postgresql")]
        ConnectionWrapper::PostgreSQL(db) => db.db_type(),
        #[cfg(feature = "mysql")]
        ConnectionWrapper::MySQL(_) => DbType::MySQL,
        #[cfg(feature = "mssql")]
        ConnectionWrapper::MSSQL(_) => DbType::MSSQL,
        #[cfg(feature = "duckdb")]
        ConnectionWrapper::DuckDB(_) => DbType::DuckDB,
        #[cfg(feature = "clickhouse")]
        ConnectionWrapper::ClickHouse(_) => DbType::ClickHouse,
    }
}

impl<'a, I: crate::model::Insertable> SqlExecutor for PooledInsertOrIgnoreExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        PooledInsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, _sql: SqlStatement) -> crate::Result<Self::Output> {
        let refs = self.models.as_refs();
        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.insert_or_ignore_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.insert_or_ignore_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.insert_or_ignore_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db
                .insert_or_ignore_impl::<I::Model>(&refs)
                .await
                .map(|_| ()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => db.insert_or_ignore_batch::<I::Model>(&refs).await,
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "conflict writes on ClickHouse",
            }),
        }
    }
}

// 根据启用的 feature 导入后端实现
#[cfg(feature = "sqlite")]
use super::super::sqlite_backend;

#[cfg(feature = "postgresql")]
use super::super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::super::mysql_backend;

#[cfg(feature = "mssql")]
use super::super::mssql_backend;

#[cfg(feature = "duckdb")]
use super::super::duckdb_backend;

#[cfg(feature = "clickhouse")]
use super::super::clickhouse_backend;

/// 连接包装器 - 包装各后端的 Database 实例
#[allow(clippy::upper_case_acronyms)]
enum ConnectionWrapper {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Database),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::Database),
    #[cfg(feature = "clickhouse")]
    ClickHouse(clickhouse_backend::Database),
}

#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse"
))]
impl ConnectionWrapper {
    /// 检查连接是否有效
    #[cfg(any(
        feature = "sqlite",
        feature = "mssql",
        feature = "duckdb",
        feature = "clickhouse"
    ))]
    async fn is_valid(&self) -> bool {
        match self {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.is_valid().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.is_valid().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.is_valid().await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.is_valid(),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => db.is_valid().await,
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => db.is_valid().await,
        }
    }
}

/// 手工连接池核心结构
///
/// 注意：对于 SQLite 后端，由于其嵌入式特性不支持多线程共享连接，
/// 建议设置 max_size=1。如需并发支持，可考虑启用 MVCC 模式。
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse"
))]
pub struct ManualPool {
    /// 空闲连接队列
    idle_connections: Mutex<VecDeque<ConnectionWrapper>>,
    /// 当前连接总数(包括使用中和空闲的)
    total_connections: AtomicU32,
    /// 连接池配置
    config: PoolConfig,
    /// 数据库类型
    db_type: DbType,
    /// 连接字符串
    connection_string: String,
}

#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse"
))]
impl ManualPool {
    /// 创建新的连接池
    fn new(db_type: DbType, connection_string: String, config: PoolConfig) -> Arc<Self> {
        Arc::new(Self {
            idle_connections: Mutex::new(VecDeque::new()),
            total_connections: AtomicU32::new(0),
            config,
            db_type,
            connection_string,
        })
    }

    /// 创建新的数据库连接
    async fn create_connection(&self) -> crate::Result<ConnectionWrapper> {
        match self.db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => {
                let db = crate::utils::FutureTraceExt::trace(sqlite_backend::Database::connect(
                    self.db_type,
                    &self.connection_string,
                ))
                .await?;
                Ok(ConnectionWrapper::Sqlite(db))
            }
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => Err(crate::ormer_error!(
                "Build the pool through PoolBuilder/ConnectionPool"
            )),
            #[cfg(feature = "questdb")]
            DbType::QuestDB => Err(crate::ormer_error!(
                "Build the pool through PoolBuilder/ConnectionPool"
            )),
            #[cfg(feature = "mysql")]
            DbType::MySQL => Err(crate::ormer_error!(
                "Build the pool through PoolBuilder/ConnectionPool"
            )),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let db = crate::utils::FutureTraceExt::trace(mssql_backend::Database::connect(
                    self.db_type,
                    &self.connection_string,
                ))
                .await?;
                Ok(ConnectionWrapper::MSSQL(db))
            }
            #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => {
                let db = crate::utils::FutureTraceExt::trace(duckdb_backend::Database::connect(
                    self.db_type,
                    &self.connection_string,
                ))
                .await?;
                Ok(ConnectionWrapper::DuckDB(db))
            }
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => {
                let db = clickhouse_backend::Database::connect(&self.connection_string)?;
                Ok(ConnectionWrapper::ClickHouse(db))
            }
        }
    }

    /// 获取连接(异步)
    async fn get(&self) -> crate::Result<ConnectionWrapper> {
        // 尝试从空闲队列获取
        {
            let mut idle = self.idle_connections.lock().await;
            if let Some(conn) = idle.pop_front() {
                // 检查连接是否有效
                if conn.is_valid().await {
                    return Ok(conn);
                }
                // 连接失效,减少计数
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
            }
        }

        // 空闲队列没有可用连接,尝试创建新连接
        let current_total = self.total_connections.load(Ordering::SeqCst);
        if current_total < self.config.max_size {
            // 可以增加连接数
            let conn = crate::utils::FutureTraceExt::trace(self.create_connection()).await?;
            self.total_connections.fetch_add(1, Ordering::SeqCst);
            return Ok(conn);
        }

        // 已达到最大连接数,等待信号量(会有其他连接归还)
        // 注意:这里需要先释放 semaphore permit,然后等待
        // 实际上我们应该等待空闲队列中有连接
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            let mut idle = self.idle_connections.lock().await;
            if let Some(conn) = idle.pop_front() {
                if conn.is_valid().await {
                    return Ok(conn);
                }
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// 归还连接到池
    async fn return_connection(&self, conn: ConnectionWrapper) {
        // 检查连接是否有效
        if conn.is_valid().await {
            let mut idle = self.idle_connections.lock().await;
            idle.push_back(conn);
        } else {
            // 连接失效，减少计数
            self.total_connections.fetch_sub(1, Ordering::SeqCst);
            // 连接失效时不放入空闲队列，会自动被丢弃
        }
    }

    /// 将租约对应连接退役，释放池容量但不进入空闲队列。
    async fn retire_connection(&self) {
        self.total_connections.fetch_sub(1, Ordering::SeqCst);
    }

    /// 维护最小连接数
    async fn maintain_min_connections(&self) {
        let current_total = self.total_connections.load(Ordering::SeqCst);
        let target = self.config.min_size;

        if current_total < target {
            let to_create = target - current_total;
            for _ in 0..to_create {
                if let Ok(conn) = self.create_connection().await {
                    self.total_connections.fetch_add(1, Ordering::SeqCst);
                    let mut idle = self.idle_connections.lock().await;
                    idle.push_back(conn);
                }
            }
        }
    }
}

/// 连接池配置
#[derive(Clone)]
pub struct PoolConfig {
    min_size: u32,
    max_size: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 0,
            max_size: 10,
        }
    }
}

fn validate_pool_config(config: &PoolConfig) -> crate::Result<()> {
    if config.max_size == 0 {
        return Err(crate::OrmerError::invalid_operation(
            "connection pool max_size must be greater than 0",
        ));
    }
    if config.min_size > config.max_size {
        return Err(crate::OrmerError::invalid_operation(format!(
            "connection pool min_size ({}) must not exceed max_size ({})",
            config.min_size, config.max_size
        )));
    }
    Ok(())
}

/// 连接池构建器
pub struct PoolBuilder {
    db_type: DbType,
    connection_string: String,
    config: PoolConfig,
}

impl PoolBuilder {
    pub fn new(db_type: DbType, connection_string: &str) -> Self {
        Self {
            db_type,
            connection_string: connection_string.to_string(),
            config: PoolConfig::default(),
        }
    }

    /// 设置连接池大小范围
    pub fn range(mut self, range: std::ops::Range<u32>) -> Self {
        self.config.min_size = range.start;
        self.config.max_size = range.end;
        self
    }

    /// 构建连接池
    pub async fn build(self) -> crate::Result<ConnectionPool> {
        validate_pool_config(&self.config)?;

        // 注意：SQLite 后端建议设置 max_size=1，因为其嵌入式特性不支持多线程共享连接
        // 如需并发支持，可考虑启用 MVCC 模式（PRAGMA journal_mode = 'mvcc'）

        match self.db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::Sqlite(pool))
            }
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => self.build_postgres_like_pool().await,
            #[cfg(feature = "questdb")]
            DbType::QuestDB => self.build_postgres_like_pool().await,
            #[cfg(feature = "mysql")]
            DbType::MySQL => {
                let opts = crate::utils::ResultTraceExt::trace_for(
                    mysql_async::Opts::from_url(&self.connection_string),
                    "mysql_async::Opts::from_url",
                )?;
                let pool = mysql_async::Pool::new(opts);
                Ok(ConnectionPool::MySQL(pool))
            }
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::MSSQL(pool))
            }
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::DuckDB(pool))
            }
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::ClickHouse(pool))
            }
        }
    }

    #[cfg(feature = "postgresql")]
    async fn build_postgres_like_pool(self) -> crate::Result<ConnectionPool> {
        let manager = crate::utils::ResultTraceExt::trace_for(
            PostgresConnectionManager::new_from_stringlike(&self.connection_string, NoTls),
            "bb8_postgres::PostgresConnectionManager::new_from_stringlike",
        )?;
        let mut builder = bb8::Pool::builder();
        builder = builder.max_size(self.config.max_size);
        if self.config.min_size > 0 {
            builder = builder.min_idle(Some(self.config.min_size));
        }
        let pool = crate::utils::FutureTraceExt::trace(builder.build(manager)).await?;
        Ok(ConnectionPool::PostgreSQL(pool, self.db_type))
    }
}

pub struct ReplicatedPoolBuilder {
    db_type: DbType,
    write_connection: Option<String>,
    read_connections: Vec<String>,
    config: PoolConfig,
}

pub struct ReplicatedConnectionPool {
    db_type: DbType,
    write: ConnectionPool,
    reads: Vec<ConnectionPool>,
    next_read: AtomicUsize,
}

impl ReplicatedPoolBuilder {
    pub(crate) fn new(db_type: DbType) -> Self {
        Self {
            db_type,
            write_connection: None,
            read_connections: Vec::new(),
            config: PoolConfig::default(),
        }
    }

    pub fn write(mut self, connection_string: impl Into<String>) -> Self {
        self.write_connection = Some(connection_string.into());
        self
    }

    pub fn read(mut self, connection_string: impl Into<String>) -> Self {
        self.read_connections.push(connection_string.into());
        self
    }

    pub fn range(mut self, range: std::ops::Range<u32>) -> Self {
        self.config.min_size = range.start;
        self.config.max_size = range.end;
        self
    }

    pub fn max_size(mut self, max_size: u32) -> Self {
        self.config.max_size = max_size;
        self
    }

    pub async fn connect(self) -> crate::Result<ReplicatedConnectionPool> {
        validate_pool_config(&self.config)?;

        let Some(write_connection) = self.write_connection else {
            return Err(crate::ormer_error!(
                "replicated connection pool requires a write connection"
            ));
        };

        let write = PoolBuilder {
            db_type: self.db_type,
            connection_string: write_connection,
            config: self.config.clone(),
        }
        .build()
        .await?;

        let mut reads = Vec::with_capacity(self.read_connections.len());
        for connection_string in self.read_connections {
            reads.push(
                PoolBuilder {
                    db_type: self.db_type,
                    connection_string,
                    config: self.config.clone(),
                }
                .build()
                .await?,
            );
        }

        Ok(ReplicatedConnectionPool {
            db_type: self.db_type,
            write,
            reads,
            next_read: AtomicUsize::new(0),
        })
    }
}

impl ReplicatedConnectionPool {
    pub fn db_type(&self) -> DbType {
        self.db_type
    }

    pub fn write(&self) -> &ConnectionPool {
        &self.write
    }

    pub fn read(&self) -> &ConnectionPool {
        if self.reads.is_empty() {
            return &self.write;
        }
        let index = self.next_read.fetch_add(1, AtomicOrdering::Relaxed) % self.reads.len();
        &self.reads[index]
    }
}

/// 统一的连接池枚举
pub enum ConnectionPool {
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(bb8::Pool<PostgresConnectionManager<NoTls>>, DbType),
    #[cfg(feature = "mysql")]
    MySQL(mysql_async::Pool),
    #[cfg(feature = "mssql")]
    MSSQL(Arc<ManualPool>),
    #[cfg(feature = "duckdb")]
    DuckDB(Arc<ManualPool>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(Arc<ManualPool>),
}

impl ConnectionPool {
    pub fn replicated(db_type: DbType) -> ReplicatedPoolBuilder {
        ReplicatedPoolBuilder::new(db_type)
    }

    pub fn db_type(&self) -> DbType {
        match self {
            #[cfg(feature = "sqlite")]
            ConnectionPool::Sqlite(_) => DbType::Sqlite,
            #[cfg(feature = "postgresql")]
            ConnectionPool::PostgreSQL(_, db_type) => *db_type,
            #[cfg(feature = "mysql")]
            ConnectionPool::MySQL(_) => DbType::MySQL,
            #[cfg(feature = "mssql")]
            ConnectionPool::MSSQL(_) => DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            ConnectionPool::DuckDB(_) => DbType::DuckDB,
            #[cfg(feature = "clickhouse")]
            ConnectionPool::ClickHouse(_) => DbType::ClickHouse,
        }
    }

    /// 从连接池异步获取连接
    ///
    /// 此方法会等待直到有可用连接或创建新连接
    /// 如果池中没有连接且未达到 max_size,会自动创建新连接
    pub async fn get(&self) -> crate::Result<PooledConnection<'_>> {
        match self {
            #[cfg(feature = "sqlite")]
            ConnectionPool::Sqlite(pool) => {
                let conn = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::Sqlite(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "postgresql")]
            ConnectionPool::PostgreSQL(pool, db_type) => {
                let pooled = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                let db = postgresql_backend::Database::from_pooled_connection(*db_type, pooled);
                Ok(PooledConnection {
                    inner: PooledConnectionInner::PostgreSQL,
                    connection: Some(ConnectionWrapper::PostgreSQL(db)),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mysql")]
            ConnectionPool::MySQL(pool) => {
                let db = mysql_backend::Database::from_pool(pool.clone());
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MySQL,
                    connection: Some(ConnectionWrapper::MySQL(db)),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mssql")]
            ConnectionPool::MSSQL(pool) => {
                let conn = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MSSQL(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "duckdb")]
            ConnectionPool::DuckDB(pool) => {
                let conn = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::DuckDB(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "clickhouse")]
            ConnectionPool::ClickHouse(pool) => {
                let conn = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::ClickHouse(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
        }
    }
}

/// 连接池内部类型
#[derive(Clone)]
#[allow(clippy::upper_case_acronyms)]
enum PooledConnectionInner {
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL,
    #[cfg(feature = "mysql")]
    MySQL,
    #[cfg(feature = "mssql")]
    MSSQL(Arc<ManualPool>),
    #[cfg(feature = "duckdb")]
    DuckDB(Arc<ManualPool>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(Arc<ManualPool>),
}

impl PooledConnectionInner {
    async fn return_connection(&self, conn: ConnectionWrapper) {
        match self {
            #[cfg(feature = "sqlite")]
            PooledConnectionInner::Sqlite(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL => {
                // bb8 自动管理连接生命周期，无需手动归还
                let _ = conn;
            }
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL => {
                // mysql_async::Pool 自动管理连接生命周期，无需手动归还
                let _ = conn;
            }
            #[cfg(feature = "mssql")]
            PooledConnectionInner::MSSQL(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "duckdb")]
            PooledConnectionInner::DuckDB(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "clickhouse")]
            PooledConnectionInner::ClickHouse(pool) => pool.return_connection(conn).await,
        }
    }

    async fn close_connection(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            PooledConnectionInner::Sqlite(pool) => pool.retire_connection().await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL => {}
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL => {}
            #[cfg(feature = "mssql")]
            PooledConnectionInner::MSSQL(pool) => pool.retire_connection().await,
            #[cfg(feature = "duckdb")]
            PooledConnectionInner::DuckDB(pool) => pool.retire_connection().await,
            #[cfg(feature = "clickhouse")]
            PooledConnectionInner::ClickHouse(pool) => pool.retire_connection().await,
        }
    }
}

pub struct PooledRawSelectExecutor<'conn, 'pool, T> {
    pooled_conn: &'conn PooledConnection<'pool>,
    sql: RawSql,
    _marker: PhantomData<T>,
}

impl<'conn, 'pool, T> PooledRawSelectExecutor<'conn, 'pool, T> {
    pub async fn collect<C>(self) -> crate::Result<C>
    where
        T: FromRowValues,
        C: FromIterator<T>,
    {
        let raw_sql = self.sql;
        match self.pooled_conn.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                let (sql, params) = raw_sql.render(DbType::Sqlite)?;
                db.select_raw::<T, C>(&sql, params).await
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                let (sql, params) = raw_sql.render(DbType::PostgreSQL)?;
                db.select_raw::<T, C>(&sql, params).await
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                let (sql, params) = raw_sql.render(DbType::MySQL)?;
                db.select_raw::<T, C>(&sql, params).await
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                let (sql, params) = raw_sql.render(DbType::MSSQL)?;
                db.select_raw::<T, C>(&sql, params).await
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                let (sql, params) = raw_sql.render(DbType::DuckDB)?;
                db.select_raw::<T, C>(&sql, params).await
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => {
                let rows = db
                    .select_values(raw_sql, <T as FromRowValues>::row_columns())
                    .await?;
                rows.into_iter()
                    .map(|values| <T as FromRowValues>::from_row_values(&values))
                    .collect()
            }
        }
    }
}

/// 统一的 PooledConnection
/// 包装连接,实现 Database 的所有方法,Drop 时自动归还到池
pub struct PooledConnection<'a> {
    inner: PooledConnectionInner,
    connection: Option<ConnectionWrapper>,
    _marker: PhantomData<&'a ()>,
}

#[derive(Clone)]
pub struct PooledDatabaseScope<'a, 'pool> {
    conn: &'a PooledConnection<'pool>,
    context_filters: Vec<ContextFilter>,
}

impl<'a> Drop for PooledConnection<'a> {
    fn drop(&mut self) {
        if let Some(conn) = self.connection.take() {
            let inner = self.inner.clone();
            // 尝试获取 tokio 运行时句柄
            // 如果成功，使用 spawn 异步归还连接
            // 如果失败（不在 tokio 运行时中），则阻塞执行
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    // 在 tokio 运行时中，异步归还连接
                    handle.spawn(async move {
                        inner.return_connection(conn).await;
                    });
                }
                Err(_) => futures::executor::block_on(inner.return_connection(conn)),
            }
        }
    }
}

impl<'a> PooledConnection<'a> {
    /// 显式归还健康连接；重复归还返回错误。
    pub async fn return_(mut self) -> crate::Result<()> {
        let Some(conn) = self.connection.take() else {
            return Err(crate::ormer_error!("connection already returned"));
        };
        self.inner.return_connection(conn).await;
        Ok(())
    }

    /// 关闭底层连接，不归还连接池。
    pub async fn close(mut self) -> crate::Result<()> {
        let Some(conn) = self.connection.take() else {
            return Err(crate::ormer_error!("connection already returned"));
        };
        drop(conn);
        self.inner.close_connection().await;
        Ok(())
    }

    /// 获取底层连接的引用(内部使用)
    fn get_connection(&self) -> &ConnectionWrapper {
        self.connection.as_ref().expect("Connection already taken")
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => CreateTableExecutor::Sqlite(db.create_table::<T>()),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                CreateTableExecutor::PostgreSQL(db.create_table::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => CreateTableExecutor::MySQL(db.create_table::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => CreateTableExecutor::MSSQL(db.create_table::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => CreateTableExecutor::DuckDB(db.create_table::<T>()),
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => CreateTableExecutor::Unsupported {
                backend: DbType::ClickHouse,
                feature: "CREATE TABLE without explicit ClickHouse engine settings",
                _marker: PhantomData,
            },
        }
    }

    /// 验证表结构
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.validate_table::<T>().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => db.validate_table::<T>().await,
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "validate_table",
            }),
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> PooledInsertExecutor<'_, I> {
        PooledInsertExecutor {
            pooled_conn: self,
            models,
            conflict: None,
            _marker: PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> PooledInsertOrUpdateExecutor<'_, I> {
        PooledInsertOrUpdateExecutor {
            pooled_conn: self,
            models,
            _marker: PhantomData,
        }
    }

    pub fn upsert<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> PooledInsertOrUpdateExecutor<'_, I> {
        self.insert_or_update(models)
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> PooledInsertOrIgnoreExecutor<'_, I> {
        PooledInsertOrIgnoreExecutor {
            pooled_conn: self,
            models,
            _marker: PhantomData,
        }
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> super::unified::SelectExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::SelectExecutor::Sqlite(db.select::<T>())
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::SelectExecutor::PostgreSQL(db.select::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::SelectExecutor::MySQL(db.select::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::SelectExecutor::MSSQL(db.select::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::SelectExecutor::DuckDB(db.select::<T>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => super::unified::SelectExecutor::ClickHouse(
                db,
                crate::query::builder::Select::default(),
            ),
        }
    }

    pub fn scope(&self) -> PooledDatabaseScope<'_, 'a> {
        PooledDatabaseScope {
            conn: self,
            context_filters: Vec::new(),
        }
    }

    /// 创建流式查询执行器
    pub fn stream<T: Model>(&self) -> super::unified::SelectStream<'_, T> {
        PooledConnection::select::<T>(self).stream()
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> super::unified::DeleteExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::DeleteExecutor::PostgreSQL(db.delete::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::DeleteExecutor::MySQL(db.delete::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::DeleteExecutor::MSSQL(db.delete::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::DeleteExecutor::DuckDB(db.delete::<T>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => super::unified::DeleteExecutor::Unsupported {
                backend: DbType::ClickHouse,
                feature: "row delete on ClickHouse",
                _marker: PhantomData,
            },
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> super::unified::UpdateExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::UpdateExecutor::Sqlite(db.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::UpdateExecutor::PostgreSQL(db.update::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::UpdateExecutor::MySQL(db.update::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::UpdateExecutor::MSSQL(db.update::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::UpdateExecutor::DuckDB(db.update::<T>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => super::unified::UpdateExecutor::Unsupported {
                backend: DbType::ClickHouse,
                feature: "row update on ClickHouse; use execute_sql",
                _marker: PhantomData,
            },
        }
    }

    /// 创建 Related 查询执行器
    pub fn related<T: Model + 'static, R: Model>(
        &self,
    ) -> super::unified::RelatedSelectExecutor<'_, T, R> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => super::unified::RelatedSelectExecutor::Sqlite(
                db.related::<T, R>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::RelatedSelectExecutor::PostgreSQL(db.related::<T, R>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                super::unified::RelatedSelectExecutor::MySQL(db.related::<T, R>())
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                super::unified::RelatedSelectExecutor::MSSQL(db.related::<T, R>())
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::RelatedSelectExecutor::DuckDB(db.related::<T, R>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => {
                super::unified::RelatedSelectExecutor::Unsupported {
                    backend: DbType::ClickHouse,
                    feature: "relation select on ClickHouse",
                    _marker: PhantomData,
                }
            }
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<super::unified::Transaction<'_>> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                let txn = crate::utils::FutureTraceExt::trace(db.begin()).await?;
                Ok(super::unified::Transaction::Sqlite(txn))
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                let txn = crate::utils::FutureTraceExt::trace(db.begin()).await?;
                Ok(super::unified::Transaction::PostgreSQL(txn))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                let txn = crate::utils::FutureTraceExt::trace(db.begin()).await?;
                Ok(super::unified::Transaction::MySQL(txn))
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                let txn = crate::utils::FutureTraceExt::trace(db.begin()).await?;
                Ok(super::unified::Transaction::MSSQL(txn))
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                let txn = crate::utils::FutureTraceExt::trace(db.begin()).await?;
                Ok(super::unified::Transaction::DuckDB(txn))
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "transactions on ClickHouse",
            }),
        }
    }

    pub async fn transaction<R, F>(&self, f: F) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(
            &'tx mut super::unified::Transaction<'_>,
        ) -> super::unified::TransactionFuture<'tx, R>,
    {
        self.transaction_opts(super::unified::TransactionOptions::new(), f)
            .await
    }

    pub async fn transaction_opts<R, F>(
        &self,
        options: super::unified::TransactionOptions,
        f: F,
    ) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(
            &'tx mut super::unified::Transaction<'_>,
        ) -> super::unified::TransactionFuture<'tx, R>,
    {
        let mut txn = self.begin().await?;
        if let Err(err) = super::unified::apply_transaction_options(&mut txn, options).await {
            let _ = txn.rollback().await;
            return Err(err);
        }

        match f(&mut txn).await {
            Ok(value) => {
                txn.commit().await?;
                Ok(value)
            }
            Err(err) => {
                let _ = txn.rollback().await;
                Err(err)
            }
        }
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => DropTableExecutor::Sqlite(db.drop_table::<T>()),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                DropTableExecutor::PostgreSQL(db.drop_table::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => DropTableExecutor::MySQL(db.drop_table::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => DropTableExecutor::MSSQL(db.drop_table::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => DropTableExecutor::DuckDB(db.drop_table::<T>()),
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => DropTableExecutor::ClickHouse(db, PhantomData),
        }
    }

    pub fn select_sql<T>(&self, sql: impl IntoRawSql) -> PooledRawSelectExecutor<'_, 'a, T> {
        PooledRawSelectExecutor {
            pooled_conn: self,
            sql: sql.into_raw_sql(),
            _marker: PhantomData,
        }
    }

    /// 执行原生非查询 SQL
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let raw_sql = sql.into_raw_sql();
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                let (sql, params) = raw_sql.render(DbType::Sqlite)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                let (sql, params) = raw_sql.render(DbType::PostgreSQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                let (sql, params) = raw_sql.render(DbType::MySQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                let (sql, params) = raw_sql.render(DbType::MSSQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                let (sql, params) = raw_sql.render(DbType::DuckDB)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => {
                db.execute_sql(raw_sql).await?;
                Ok(0)
            }
        }
    }
}

impl<'a, 'pool> PooledDatabaseScope<'a, 'pool> {
    pub fn select<T: Model>(&self) -> super::unified::SelectExecutor<'_, T> {
        self.conn
            .select::<T>()
            .with_context_filters(self.context_filters.clone())
    }

    pub async fn find_by_id<T: Model + 'static + Send + Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> crate::Result<Option<T>> {
        let where_expr = primary_key_filter::<T>(key)?;
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .collect::<Vec<T>>()
            .await?;
        Ok(results.into_iter().next())
    }

    pub async fn find_related<T: Model + 'static + Send + Sync, S: RelationSelection<T>>(
        &self,
        owner: &T,
        relation: S,
    ) -> crate::Result<Vec<S::Target>>
    where
        for<'b> S: RelationNestedLoader<'b, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
    {
        let path = relation.path_info()?;
        let key = owner.relation_key_value(relation_owner_key(path))?;
        self.select::<T>()
            .select_related_with_selection(vec![key], &relation)
            .await
    }

    pub async fn preload<T: Model + 'static + Send + Sync, S: RelationSelection<T>>(
        &self,
        owners: &mut [T],
        relation: S,
    ) -> crate::Result<()>
    where
        for<'b> S: RelationNestedLoader<'b, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
    {
        self.select::<T>()
            .preload_models_with_selection(owners, relation)
            .await
    }

    pub fn delete<T: WritableModel>(&self) -> ScopedDeleteExecutor<'_, T> {
        ScopedDeleteExecutor {
            inner: self.conn.delete::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }

    pub fn update<T: WritableModel>(&self) -> ScopedUpdateExecutor<'_, T> {
        ScopedUpdateExecutor {
            inner: self.conn.update::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }
}

impl<'a, 'pool, T: Model> NamedFilterQuery<T> for PooledDatabaseScope<'a, 'pool> {
    fn apply_named_filter(mut self, name: &'static str, expr: WhereExpr) -> Self {
        self.context_filters
            .push(ContextFilter::new::<T>(name, expr));
        self
    }
}

impl<'a> DbExecutor for PooledConnection<'a> {
    fn select<T: crate::model::Model>(&self) -> super::unified::SelectExecutor<'_, T> {
        PooledConnection::select::<T>(self)
    }

    fn select_column<T: crate::model::Model, V>(
        &self,
    ) -> super::unified::GroupedSelectExecutor<'_, T, V> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::GroupedSelectExecutor::Sqlite(db.select_column::<T, V>())
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::GroupedSelectExecutor::PostgreSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                super::unified::GroupedSelectExecutor::MySQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                super::unified::GroupedSelectExecutor::MSSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::GroupedSelectExecutor::DuckDB(db.select_column::<T, V>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => {
                super::unified::GroupedSelectExecutor::Unsupported {
                    backend: DbType::ClickHouse,
                    feature: "Model select_column on ClickHouse; use select_sql",
                    _marker: PhantomData,
                }
            }
        }
    }
}
