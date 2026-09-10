use super::super::DbType;
// 池化 insert 系执行器的语句构建仅在有本地写后端的组合中使用
#[cfg_attr(
    not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql",
        feature = "duckdb"
    )),
    allow(unused_imports)
)]
use super::common_helpers;
use super::{DbExecutor, SqlExecutor, SqlStatement};
use crate::impl_insert_conflict_methods;
use crate::model::{Model, RelationSelection, WritableModel};
use crate::query::builder::{ContextFilter, NamedFilterQuery, WhereExpr};
use crate::query::insert::InsertConflict;
use crate::raw_sql::IntoRawSql;
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
use std::collections::VecDeque;
use std::marker::PhantomData;
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
use std::sync::Arc;
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};

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
    feature = "clickhouse",
    feature = "influxdb"
))]
use super::unified::{
    ConnRefMut, CreateTableExecutor, DropTableExecutor, InsertPartialExecutor,
    RelationNestedLoader, SaveExecutor, ScopedDeleteExecutor, ScopedUpdateExecutor,
    TruncateTableExecutor, apply_transaction_options_or_rollback, exec_raw_sql_on,
    find_by_id_with_executor, find_related_with_executor, preload_with_executor, run_txn_closure,
};
use crate::model::Tracked;

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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::InfluxDB,
                feature: "insert to_sql (InfluxDB writes use Line Protocol over HTTP)",
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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => {
                let mut models = self.models;
                if models.as_refs().is_empty() {
                    return Ok(<I::Model as crate::model::Model>::AutoIncrementKeyType::default());
                }
                super::super::influxdb_backend::validate_influx_model::<I::Model>(
                    DbType::InfluxDB,
                )?;
                let ctx = crate::HookContext::new(crate::HookOperation::Insert);
                // 钩子可修改模型字段，Line Protocol 必须在 before_insert 之后渲染
                models.run_before_insert(ctx).await?;
                {
                    let refs = models.as_refs();
                    let lines = super::super::influxdb_backend::render_line_protocol::<I::Model>(
                        &refs,
                    )?;
                    db.write_lines_with_policy(
                        &lines,
                        super::super::influxdb_backend::model_retention_policy_name::<I::Model>()
                            .as_deref(),
                    )
                    .await?;
                }
                models.run_after_insert(ctx).await?;
                Ok(<I::Model as crate::model::Model>::AutoIncrementKeyType::default())
            }
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
        // 能力矩阵优先：insert_conflict=false 的后端（ClickHouse/InfluxDB/QuestDB）
        // 统一拒绝；QuestDB 复用 PostgreSQL 连接，按运行时 db_type 判定。
        let db_type = db_type_for_connection(self.pooled_conn.get_connection());
        crate::Capabilities::ensure(
            db_type,
            |caps| caps.insert_conflict,
            "insert conflict handling",
        )?;
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(db_type, Vec::new()));
        }

        match self.pooled_conn.get_connection() {
            // 各后端 to_sql 与池层共用同一渲染入口（自增主键拆分、按绑定参数
            // 上限分块、标识符引用均由 common_helpers 统一生成），保证池层
            // to_sql 输出与后端 execute 路径逐字一致。
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(_) => {
                super::super::sqlite_backend::sqlite_insert_or_update_to_sql::<I::Model>(&refs)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(_) => {
                // 与 postgresql_backend::InsertOrUpdateExecutor::to_sql 调用同一组
                // common_helpers（后端未暴露共享入口，池层按相同算法组装）。
                let statements =
                    common_helpers::build_auto_increment_aware_upsert_statements::<I::Model>(
                        DbType::PostgreSQL,
                        "INSERT INTO",
                        <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
                        &refs,
                        |sql, columns| {
                            common_helpers::append_standard_upsert_clause::<I::Model>(
                                DbType::PostgreSQL,
                                sql,
                                columns,
                            )
                        },
                    )?;
                Ok(SqlStatement::batch(
                    DbType::PostgreSQL,
                    statements
                        .into_iter()
                        .map(|statement| {
                            let rust_types = pg_upsert_param_rust_types_for::<I::Model>(
                                statement.row_count,
                                &statement.columns,
                            );
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                                .with_param_rust_types(rust_types)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(_) => {
                // 与 mysql_backend::InsertOrUpdateExecutor 同款语义：
                // 冲突更新排除主键列，自增主键未设置的行纯插入，全主键模型退化为 INSERT IGNORE
                super::super::mysql_backend::mysql_insert_or_update_to_sql::<I::Model>(&refs)
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(_) => {
                // 与 mssql 后端 to_sql 同源：按参数上限（2100）分块的 MERGE 语句组
                let statements =
                    common_helpers::build_mssql_merge_statements::<I::Model>(&refs, true)?;
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
                super::super::duckdb_backend::duckdb_insert_or_update_to_sql::<I::Model>(&refs)
            }
            // 矩阵兜底：正常不可达（insert_conflict=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "insert conflict handling",
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

    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unused_variables)
    )]
    async fn execute_with_sql(self, _sql: SqlStatement) -> crate::Result<Self::Output> {
        let db_type = db_type_for_connection(self.pooled_conn.get_connection());
        crate::Capabilities::ensure(
            db_type,
            |caps| caps.insert_conflict,
            "insert conflict handling",
        )?;
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
            // 矩阵兜底：正常不可达（insert_conflict=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "insert conflict handling",
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
        // 能力矩阵优先：insert_ignore=false 的后端（ClickHouse/InfluxDB/QuestDB）
        // 统一拒绝；QuestDB 复用 PostgreSQL 连接，按运行时 db_type 判定。
        let db_type = db_type_for_connection(self.pooled_conn.get_connection());
        crate::Capabilities::ensure(db_type, |caps| caps.insert_ignore, "insert ignore")?;
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(db_type, Vec::new()));
        }

        match self.pooled_conn.get_connection() {
            // 各后端 to_sql 与池层共用同一渲染入口（按绑定参数上限分块、标识符
            // 引用均由 common_helpers 统一生成），保证池层 to_sql 输出与后端
            // execute 路径逐字一致。
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(_) => {
                super::super::sqlite_backend::sqlite_insert_or_ignore_to_sql::<I::Model>(&refs)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(_) => {
                // 与 postgresql_backend::build_pg_insert_or_ignore_statements 调用同一组
                // common_helpers（后端该函数为私有，池层按相同算法组装）。
                let columns = I::Model::insert_columns();
                let primary_key = common_helpers::quote_column_list(
                    DbType::PostgreSQL,
                    &I::Model::primary_key_columns(),
                );
                let statements = common_helpers::build_chunked_insert_statements::<I::Model>(
                    DbType::PostgreSQL,
                    &refs,
                    |chunk| {
                        let (mut sql, params) =
                            common_helpers::build_batch_insert_statement::<I::Model>(
                                DbType::PostgreSQL,
                                "INSERT INTO",
                                <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
                                &columns,
                                chunk,
                                common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
                            );
                        sql.push_str(" ON CONFLICT (");
                        sql.push_str(&primary_key);
                        sql.push_str(") DO NOTHING");
                        Ok(common_helpers::InsertSqlStatement {
                            sql,
                            params,
                            row_count: chunk.len(),
                        })
                    },
                )?;
                Ok(SqlStatement::batch(
                    DbType::PostgreSQL,
                    statements
                        .into_iter()
                        .map(|statement| {
                            let rust_types = pg_upsert_param_rust_types_for::<I::Model>(
                                statement.row_count,
                                &columns,
                            );
                            super::SingleSqlStatement::new(statement.sql, statement.params)
                                .with_param_rust_types(rust_types)
                        })
                        .collect(),
                ))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(_) => {
                // MySQL 的 INSERT IGNORE 写入全部列（含主键），按全列数分块
                super::super::mysql_backend::mysql_insert_or_ignore_to_sql::<I::Model>(&refs)
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(_) => {
                // 与 mssql 后端 to_sql 同源：按参数上限（2100）分块的 MERGE 语句组
                let statements =
                    common_helpers::build_mssql_merge_statements::<I::Model>(&refs, false)?;
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
                super::super::duckdb_backend::duckdb_insert_or_ignore_to_sql::<I::Model>(&refs)
            }
            // 矩阵兜底：正常不可达（insert_ignore=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "insert ignore",
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
        #[cfg(feature = "influxdb")]
        ConnectionWrapper::InfluxDB(_) => DbType::InfluxDB,
    }
}

/// 按实际写入列推导 PostgreSQL upsert/ignore 语句的参数类型（行数 × 每行列类型）。
///
/// 与 `postgresql_backend::pg_upsert_param_rust_types` 保持同算法逐字对齐
/// （该函数为后端私有，池层无法直接调用），保证池层 to_sql 产出的参数类型
/// 与后端执行路径一致。
#[cfg(feature = "postgresql")]
fn pg_upsert_param_rust_types_for<T: Model>(
    row_count: usize,
    columns: &[&str],
) -> Vec<&'static str> {
    let per_row: Vec<&'static str> = columns
        .iter()
        .filter_map(|column| {
            T::COLUMN_SCHEMA
                .iter()
                .find(|schema| schema.name == *column)
                .map(|schema| schema.data_type.unwrap_or(schema.rust_type))
        })
        .collect();
    let mut rust_types = Vec::with_capacity(row_count * per_row.len());
    for _ in 0..row_count {
        rust_types.extend(per_row.iter().copied());
    }
    rust_types
}

impl<'a, I: crate::model::Insertable> SqlExecutor for PooledInsertOrIgnoreExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        PooledInsertOrIgnoreExecutor::to_sql(self)
    }

    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unused_variables)
    )]
    async fn execute_with_sql(self, _sql: SqlStatement) -> crate::Result<Self::Output> {
        let db_type = db_type_for_connection(self.pooled_conn.get_connection());
        crate::Capabilities::ensure(db_type, |caps| caps.insert_ignore, "insert ignore")?;
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
            // 矩阵兜底：正常不可达（insert_ignore=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "insert ignore",
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

#[cfg(feature = "influxdb")]
use super::super::influxdb_backend;

/// 连接包装器 - 包装各后端的 Database 实例
/// （R3：raw select 三路径合一，池连接经 [`super::unified::ConnRef::Pooled`]
/// 参与统一分派，故提升为 crate 可见）
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum ConnectionWrapper {
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
    #[cfg(feature = "influxdb")]
    InfluxDB(influxdb_backend::Database),
}

#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
impl ConnectionWrapper {
    /// 检查连接是否有效
    #[cfg(any(
        feature = "sqlite",
        feature = "mssql",
        feature = "duckdb",
        feature = "clickhouse",
        feature = "influxdb"
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
            ConnectionWrapper::MSSQL(db) => db.is_valid().await,
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => db.is_valid().await,
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => db.is_valid().await,
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => db.is_valid().await,
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
    feature = "clickhouse",
    feature = "influxdb"
))]
pub struct ManualPool {
    /// 空闲连接队列（附带建连/入队时间，供 idle_timeout / max_lifetime 判定）
    idle_connections: Mutex<VecDeque<IdleConnection>>,
    /// 当前连接总数(包括使用中和空闲的)
    total_connections: AtomicU32,
    /// 容量信号量：可用名额 = 空闲连接数 + 剩余可建连接数，
    /// 用于等待/唤醒（替换旧的 10ms 轮询）并控制并发建连不超过 max_size
    permits: Semaphore,
    /// 连接池配置
    config: PoolConfig,
    /// 数据库类型
    db_type: DbType,
    /// 连接字符串
    connection_string: String,
}

/// 空闲队列条目：连接 + 生命周期时间戳。
#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
struct IdleConnection {
    conn: ConnectionWrapper,
    /// 建连时间，用于 max_lifetime 判定
    created_at: Instant,
    /// 最近一次入队时间，用于 idle_timeout 判定
    idle_since: Instant,
}

#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
impl IdleConnection {
    fn fresh(conn: ConnectionWrapper) -> Self {
        let now = Instant::now();
        Self {
            conn,
            created_at: now,
            idle_since: now,
        }
    }

    /// 是否已超过 idle_timeout / max_lifetime；任一超限即应退役重建。
    fn is_expired(&self, config: &PoolConfig) -> bool {
        let now = Instant::now();
        config
            .idle_timeout
            .is_some_and(|timeout| now.duration_since(self.idle_since) >= timeout)
            || config
                .max_lifetime
                .is_some_and(|lifetime| now.duration_since(self.created_at) >= lifetime)
    }
}

#[cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
impl ManualPool {
    /// 创建新的连接池
    fn new(db_type: DbType, connection_string: String, config: PoolConfig) -> Arc<Self> {
        let permits = Semaphore::new(config.max_size as usize);
        Arc::new(Self {
            idle_connections: Mutex::new(VecDeque::new()),
            total_connections: AtomicU32::new(0),
            permits,
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
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => {
                let db = influxdb_backend::Database::connect(&self.connection_string)?;
                Ok(ConnectionWrapper::InfluxDB(db))
            }
        }
    }

    /// 获取连接(异步)
    ///
    /// 通过信号量等待空闲连接或可用容量（不轮询），超过 `acquire_timeout` 返回明确错误；
    /// 等待者获得资格后若空闲队列无连接且 total < max_size，会主动新建连接（重建路径），
    /// 数据库重启导致连接全部失效退役后不会再永久死等。
    /// 返回连接与其建连时间（供租借方归还时回传，维持 max_lifetime 累计）。
    async fn get(&self) -> crate::Result<(ConnectionWrapper, Instant)> {
        // 顺带补齐 min_size（低成本路径，容量紧张时跳过）
        self.maintain_min_connections(false).await;

        // ticket 用于在取出失效连接后保留已获得的等待资格，避免重复排队
        let mut ticket: Option<SemaphorePermit<'_>> = None;
        loop {
            let permit = match ticket.take() {
                Some(permit) => permit,
                None => self.acquire_permit().await?,
            };

            // 锁内只做队列摘取与超期判定，健康检查（可能是一次网络请求）移到锁外执行
            let candidate = self.idle_connections.lock().await.pop_front();
            let Some(entry) = candidate else {
                // 无空闲连接：原子预占名额后新建连接，避免并发下超过 max_size
                if self.reserve_slot() {
                    match self.create_connection().await {
                        Ok(conn) => {
                            // 新连接占用了本次资格对应的容量
                            permit.forget();
                            return Ok((conn, Instant::now()));
                        }
                        Err(err) => {
                            // 建连失败，回退已预占的名额（permit 随作用域结束自动归还）
                            self.release_slot();
                            return Err(err);
                        }
                    }
                }
                // 瞬态无容量可用（如 min_size 补建进行中），释放资格重新等待
                drop(permit);
                continue;
            };

            // 空闲超时 / 达到最大寿命的连接直接退役并携带资格重试（重建路径）
            if entry.is_expired(&self.config) {
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
                ticket = Some(permit);
                continue;
            }

            let conn = entry.conn;
            let created_at = entry.created_at;
            if conn.is_valid().await {
                // 该空闲连接被取走，对应的信号量名额随之消耗
                permit.forget();
                return Ok((conn, created_at));
            }
            // 连接失效，退役并携带资格重试：后续轮次可取其他空闲连接或重建
            self.total_connections.fetch_sub(1, Ordering::SeqCst);
            ticket = Some(permit);
        }
    }

    /// 获取一个等待资格；超过 `acquire_timeout` 时返回明确的超时错误
    async fn acquire_permit(&self) -> crate::Result<SemaphorePermit<'_>> {
        let Some(timeout) = self.config.acquire_timeout else {
            return self
                .permits
                .acquire()
                .await
                .map_err(|_| self.semaphore_closed_error());
        };
        match tokio::time::timeout(timeout, self.permits.acquire()).await {
            Ok(result) => result.map_err(|_| self.semaphore_closed_error()),
            Err(_) => Err(crate::OrmerError::Pool {
                backend: self.db_type,
                message: format!(
                    "timed out after {} ms waiting for an available connection (pool max_size = {}, \
                     adjust via PoolBuilder::acquire_timeout)",
                    timeout.as_millis(),
                    self.config.max_size,
                ),
            }),
        }
    }

    fn semaphore_closed_error(&self) -> crate::OrmerError {
        crate::OrmerError::Pool {
            backend: self.db_type,
            message: "connection pool semaphore closed".to_string(),
        }
    }

    /// 原子预占一个连接名额，已达 max_size 时返回 false
    fn reserve_slot(&self) -> bool {
        self.total_connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |total| {
                (total < self.config.max_size).then_some(total + 1)
            })
            .is_ok()
    }

    /// 回退一个已预占的连接名额
    fn release_slot(&self) {
        self.total_connections.fetch_sub(1, Ordering::SeqCst);
    }

    /// 归还连接到池。
    ///
    /// `created_at` 由租借方（`PooledConnection`）在取用连接时随行带回，
    /// 保证 max_lifetime 跨借用周期累计；`None`（未知）以当前时刻近似。
    async fn return_connection(&self, conn: ConnectionWrapper, created_at: Option<Instant>) {
        // 检查连接是否有效（锁外执行，避免持锁期间发起网络请求）
        if !conn.is_valid().await {
            // 连接失效，退役并释放容量（唤醒等待者重建）
            self.total_connections.fetch_sub(1, Ordering::SeqCst);
            self.permits.add_permits(1);
            return;
        }

        let now = Instant::now();
        let entry = IdleConnection {
            conn,
            created_at: created_at.unwrap_or(now),
            idle_since: now,
        };
        {
            let mut idle = self.idle_connections.lock().await;
            idle.push_back(entry);
        }
        // 唤醒一个等待者取用该空闲连接
        self.permits.add_permits(1);
    }

    /// 将租约对应连接退役，释放池容量并唤醒等待者重建，但不进入空闲队列。
    async fn retire_connection(&self) {
        self.total_connections.fetch_sub(1, Ordering::SeqCst);
        self.permits.add_permits(1);
    }

    /// 补齐 min_size 空闲连接。
    ///
    /// `wait_for_capacity` 为 true 时（建池阶段）等待容量可用；
    /// 为 false 时（get() 顺带补建）容量紧张则直接跳过，不阻塞调用方。
    async fn maintain_min_connections(&self, wait_for_capacity: bool) {
        while self.total_connections.load(Ordering::SeqCst) < self.config.min_size {
            let permit = if wait_for_capacity {
                match self.permits.acquire().await {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            } else {
                match self.permits.try_acquire() {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            };
            if !self.reserve_slot() {
                // 已达 max_size，无法继续补建（permit 随作用域结束自动归还）
                return;
            }
            match self.create_connection().await {
                Ok(conn) => {
                    {
                        let mut idle = self.idle_connections.lock().await;
                        idle.push_back(IdleConnection::fresh(conn));
                    }
                    // 新空闲连接已入队，归还等待资格供等待者取用
                    drop(permit);
                }
                Err(_) => {
                    self.release_slot();
                    return;
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
    /// 获取连接的最长等待时间；`None` 表示无限等待
    acquire_timeout: Option<Duration>,
    /// 空闲连接在池内滞留超过该时长后退役；`None` 表示不回收空闲连接
    idle_timeout: Option<Duration>,
    /// 连接自建立起的最长寿命，到期退役重建；`None` 表示不限寿命
    max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 0,
            max_size: 10,
            acquire_timeout: Some(Duration::from_secs(30)),
            idle_timeout: None,
            max_lifetime: None,
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
    if config.acquire_timeout == Some(Duration::ZERO) {
        return Err(crate::OrmerError::invalid_operation(
            "connection pool acquire_timeout must be greater than zero when set",
        ));
    }
    if config.idle_timeout == Some(Duration::ZERO) {
        return Err(crate::OrmerError::invalid_operation(
            "connection pool idle_timeout must be greater than zero when set",
        ));
    }
    if config.max_lifetime == Some(Duration::ZERO) {
        return Err(crate::OrmerError::invalid_operation(
            "connection pool max_lifetime must be greater than zero when set",
        ));
    }
    Ok(())
}

/// 连接池构建器
pub struct PoolBuilder {
    db_type: DbType,
    connection_string: String,
    config: PoolConfig,
    /// 用户是否显式设置过 acquire_timeout（L29：显式设置时映射到
    /// bb8 的 connection_timeout；默认值不映射，保持零行为变化）。
    acquire_timeout_set: bool,
}

impl PoolBuilder {
    pub fn new(db_type: DbType, connection_string: &str) -> Self {
        Self {
            db_type,
            connection_string: connection_string.to_string(),
            config: PoolConfig::default(),
            acquire_timeout_set: false,
        }
    }

    /// 设置连接池大小范围
    pub fn range(mut self, range: std::ops::Range<u32>) -> Self {
        self.config.min_size = range.start;
        self.config.max_size = range.end;
        self
    }

    /// 设置获取连接的最长等待时间，超时返回 `OrmerError::Pool` 错误；
    /// 传入 `None` 表示无限等待。
    ///
    /// - 内置手工连接池的后端（sqlite/mssql/duckdb/clickhouse/influxdb）
    ///   原生生效；
    /// - PostgreSQL/TimescaleDB/QuestDB（bb8）：显式设置时映射到 bb8 的
    ///   connection_timeout（默认 30s 不映射，保持 bb8 自身默认行为）；
    /// - MySQL（mysql_async）：当前版本（0.37）的 `PoolOpts` 无获取超时项，
    ///   显式设置非默认值时在 build 阶段返回明确的不支持错误，
    ///   不再静默丢弃。
    pub fn acquire_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.config.acquire_timeout = timeout.into();
        self.acquire_timeout_set = true;
        self
    }

    /// 设置空闲连接回收时间：连接在池内空闲超过该时长后，下次被取用时
    /// 会被退役（由后续请求按需重建），而不是继续复用；传入 `None` 表示
    /// 不回收空闲连接（默认）。
    ///
    /// 仅对使用内置手工连接池的后端生效
    /// （sqlite/mssql/duckdb/clickhouse/influxdb）。
    pub fn idle_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.config.idle_timeout = timeout.into();
        self
    }

    /// 设置连接最大寿命：连接自建立起超过该时长后在取用时退役重建，
    /// 避免长期运行的单条连接累积状态；传入 `None` 表示不限寿命（默认）。
    ///
    /// 仅对使用内置手工连接池的后端生效
    /// （sqlite/mssql/duckdb/clickhouse/influxdb）。
    pub fn max_lifetime(mut self, lifetime: impl Into<Option<Duration>>) -> Self {
        self.config.max_lifetime = lifetime.into();
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
                    pool.maintain_min_connections(true).await;
                }
                Ok(ConnectionPool::Sqlite(pool))
            }
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => self.build_postgres_like_pool().await,
            #[cfg(feature = "questdb")]
            DbType::QuestDB => self.build_postgres_like_pool().await,
            #[cfg(feature = "mysql")]
            DbType::MySQL => {
                // mysql_async 0.37 的 PoolOpts 没有获取超时项（L29）：
                // 显式设置的非默认 acquire_timeout 不再静默丢弃，
                // 在 build 阶段返回明确错误（默认值不映射，零行为变化）。
                if self.acquire_timeout_set
                    && self.config.acquire_timeout
                        != Some(Duration::from_secs(30))
                {
                    return Err(crate::OrmerError::Pool {
                        backend: DbType::MySQL,
                        message: format!(
                            "mysql_async pool does not support acquire_timeout \
                             (got {:?}); remove PoolBuilder::acquire_timeout or use \
                             the manual-pool backends (sqlite/mssql/duckdb)",
                            self.config.acquire_timeout
                        ),
                    });
                }
                let opts = crate::utils::ResultTraceExt::trace_for(
                    mysql_async::Opts::from_url(&self.connection_string),
                    "mysql_async::Opts::from_url",
                )?;
                // 应用池容量约束，连接数受 max_size/min_size 限制
                let constraints = mysql_async::PoolConstraints::new(
                    self.config.min_size as usize,
                    self.config.max_size as usize,
                )
                .ok_or_else(|| {
                    crate::OrmerError::invalid_operation(
                        "connection pool constraints are invalid (min > max or max == 0)",
                    )
                })?;
                let builder =
                    mysql_async::OptsBuilder::from_opts(opts).pool_opts(
                        mysql_async::PoolOpts::default().with_constraints(constraints),
                    );
                let pool = mysql_async::Pool::new(builder);
                Ok(ConnectionPool::MySQL(pool))
            }
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections(true).await;
                }
                Ok(ConnectionPool::MSSQL(pool))
            }
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections(true).await;
                }
                Ok(ConnectionPool::DuckDB(pool))
            }
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections(true).await;
                }
                Ok(ConnectionPool::ClickHouse(pool))
            }
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections(true).await;
                }
                Ok(ConnectionPool::InfluxDB(pool))
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
        // L29：显式设置的 acquire_timeout 映射到 bb8 的 connection_timeout
        // （获取/检出的最长等待）；默认 30s 不映射，保持 bb8 自身默认。
        if self.acquire_timeout_set {
            if let Some(timeout) = self.config.acquire_timeout {
                builder = builder.connection_timeout(timeout);
            }
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
    /// 用户是否显式设置过 acquire_timeout（与 PoolBuilder 同语义，L29）。
    acquire_timeout_set: bool,
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
            acquire_timeout_set: false,
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

    /// 设置获取连接的最长等待时间，超时返回 `OrmerError::Pool` 错误；
    /// 传入 `None` 表示无限等待。
    ///
    /// 生效范围与 [`PoolBuilder::acquire_timeout`] 一致：手工池后端原生
    /// 生效；PostgreSQL（bb8）显式设置时映射 connection_timeout；
    /// MySQL（mysql_async 0.37）无对应项，显式非默认值 build 时报错。
    pub fn acquire_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.config.acquire_timeout = timeout.into();
        self.acquire_timeout_set = true;
        self
    }

    /// 设置空闲连接回收时间：连接在池内空闲超过该时长后，下次被取用时
    /// 会被退役（由后续请求按需重建），而不是继续复用；传入 `None` 表示
    /// 不回收空闲连接（默认）。
    ///
    /// 仅对使用内置手工连接池的后端生效
    /// （sqlite/mssql/duckdb/clickhouse/influxdb）。
    pub fn idle_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.config.idle_timeout = timeout.into();
        self
    }

    /// 设置连接最大寿命：连接自建立起超过该时长后在取用时退役重建，
    /// 避免长期运行的单条连接累积状态；传入 `None` 表示不限寿命（默认）。
    ///
    /// 仅对使用内置手工连接池的后端生效
    /// （sqlite/mssql/duckdb/clickhouse/influxdb）。
    pub fn max_lifetime(mut self, lifetime: impl Into<Option<Duration>>) -> Self {
        self.config.max_lifetime = lifetime.into();
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
            acquire_timeout_set: self.acquire_timeout_set,
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
                    acquire_timeout_set: self.acquire_timeout_set,
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
    #[cfg(feature = "influxdb")]
    InfluxDB(Arc<ManualPool>),
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
            #[cfg(feature = "influxdb")]
            ConnectionPool::InfluxDB(_) => DbType::InfluxDB,
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
                let (conn, created_at) = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::Sqlite(pool.clone()),
                    connection: Some(conn),
                    created_at: Some(created_at),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "postgresql")]
            ConnectionPool::PostgreSQL(pool, db_type) => {
                let db = postgresql_backend::Database::from_pool(*db_type, pool.clone()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::PostgreSQL,
                    connection: Some(ConnectionWrapper::PostgreSQL(db)),
                    created_at: None,
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mysql")]
            ConnectionPool::MySQL(pool) => {
                // 真正从池中取出一条独占连接绑定到 Database（pinned connection）：
                // 非事务多语句固定走同一条连接，max_size 对池化用法生效；
                // 超时受 mysql_async 池自身 acquire 语义约束。
                let conn = crate::utils::FutureTraceExt::trace(pool.get_conn()).await?;
                let db = mysql_backend::Database::from_conn(pool.clone(), conn);
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MySQL,
                    connection: Some(ConnectionWrapper::MySQL(db)),
                    created_at: None,
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mssql")]
            ConnectionPool::MSSQL(pool) => {
                let (conn, created_at) = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MSSQL(pool.clone()),
                    connection: Some(conn),
                    created_at: Some(created_at),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "duckdb")]
            ConnectionPool::DuckDB(pool) => {
                let (conn, created_at) = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::DuckDB(pool.clone()),
                    connection: Some(conn),
                    created_at: Some(created_at),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "clickhouse")]
            ConnectionPool::ClickHouse(pool) => {
                let (conn, created_at) = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::ClickHouse(pool.clone()),
                    connection: Some(conn),
                    created_at: Some(created_at),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "influxdb")]
            ConnectionPool::InfluxDB(pool) => {
                let (conn, created_at) = crate::utils::FutureTraceExt::trace(pool.get()).await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::InfluxDB(pool.clone()),
                    connection: Some(conn),
                    created_at: Some(created_at),
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
    #[cfg(feature = "influxdb")]
    InfluxDB(Arc<ManualPool>),
}

impl PooledConnectionInner {
    /// 归还连接是否需要异步执行。
    ///
    /// PostgreSQL（bb8 `PooledConnection`）与 MySQL（pinned `mysql_async::Conn`）
    /// 的归还完全由 `ConnectionWrapper` 自身的同步 Drop 完成，连接自动回池，
    /// 不经过 spawn，运行时关闭时也不会丢失归还。
    fn return_needs_async(&self) -> bool {
        #[cfg(feature = "postgresql")]
        if matches!(self, PooledConnectionInner::PostgreSQL) {
            return false;
        }
        #[cfg(feature = "mysql")]
        if matches!(self, PooledConnectionInner::MySQL) {
            return false;
        }
        true
    }

    /// 仅 bb8/MySQL 池自管理的后端编译组合中 `created_at` 无消费分支
    #[allow(unused_variables)]
    async fn return_connection(&self, conn: ConnectionWrapper, created_at: Option<Instant>) {
        match self {
            #[cfg(feature = "sqlite")]
            PooledConnectionInner::Sqlite(pool) => pool.return_connection(conn, created_at).await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL => {
                // 连接由 ConnectionWrapper::PostgreSQL 内的 bb8 PooledConnection 直接持有，
                // 在此 drop 即自动归还池并唤醒等待者
                let _ = conn;
            }
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL => {
                // pinned Conn 随 ConnectionWrapper drop 送回 mysql_async 池回收器：
                // 处于事务中的连接会被先回滚清理再归池，之后才对后续租借可见
                let _ = conn;
            }
            #[cfg(feature = "mssql")]
            PooledConnectionInner::MSSQL(pool) => pool.return_connection(conn, created_at).await,
            #[cfg(feature = "duckdb")]
            PooledConnectionInner::DuckDB(pool) => pool.return_connection(conn, created_at).await,
            #[cfg(feature = "clickhouse")]
            PooledConnectionInner::ClickHouse(pool) => {
                pool.return_connection(conn, created_at).await
            }
            #[cfg(feature = "influxdb")]
            PooledConnectionInner::InfluxDB(pool) => pool.return_connection(conn, created_at).await,
        }
    }

    async fn close_connection(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            PooledConnectionInner::Sqlite(pool) => pool.retire_connection().await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL => {
                // 连接已在 close() 中随 ConnectionWrapper drop 归还 bb8 池，
                // bb8 不支持池外销毁连接，与 MySQL 池行为一致
            }
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL => {}
            #[cfg(feature = "mssql")]
            PooledConnectionInner::MSSQL(pool) => pool.retire_connection().await,
            #[cfg(feature = "duckdb")]
            PooledConnectionInner::DuckDB(pool) => pool.retire_connection().await,
            #[cfg(feature = "clickhouse")]
            PooledConnectionInner::ClickHouse(pool) => pool.retire_connection().await,
            #[cfg(feature = "influxdb")]
            PooledConnectionInner::InfluxDB(pool) => pool.retire_connection().await,
        }
    }
}

// PooledRawSelectExecutor 已合并为 unified::RawSelectExecutor（R3：raw select
// 三路径合一），旧名在 unified.rs 保留为 deprecated 类型别名。

/// 统一的 PooledConnection
/// 包装连接,实现 Database 的所有方法,Drop 时自动归还到池
pub struct PooledConnection<'a> {
    inner: PooledConnectionInner,
    connection: Option<ConnectionWrapper>,
    /// 建连时间：取用连接时从 ManualPool 带出，归还时带回池，
    /// 使 max_lifetime 跨借用周期累计；PG/MySQL 池自管理，无此信息
    created_at: Option<Instant>,
    _marker: PhantomData<&'a ()>,
}

#[derive(Clone)]
pub struct PooledDatabaseScope<'a, 'pool> {
    conn: &'a PooledConnection<'pool>,
    context_filters: Vec<ContextFilter>,
}

impl<'a> Drop for PooledConnection<'a> {
    fn drop(&mut self) {
        let Some(conn) = self.connection.take() else {
            return;
        };
        let inner = self.inner.clone();

        // PG（bb8）与 MySQL（pinned Conn）的归还由 ConnectionWrapper 的同步 Drop
        // 完成，直接 drop 即归池：不经过 spawn，运行时关闭时也不会丢失归还或 panic
        if !inner.return_needs_async() {
            return; // conn 在此 drop，自动归还各自连接池
        }

        // ManualPool 后端的归还包含异步健康检查，需要异步执行
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let watcher_inner = inner.clone();
                let created_at = self.created_at.take();
                // 在 tokio 运行时中，异步归还连接
                let join = handle.spawn(async move {
                    inner.return_connection(conn, created_at).await;
                });
                // 归还任务被取消（运行时关闭等）时连接不会进入空闲队列，
                // 退役该连接以保证池容量守恒
                let _ = handle.spawn(async move {
                    if join.await.is_err() {
                        watcher_inner.close_connection().await;
                    }
                });
            }
            Err(_) => {
                // 不在 tokio 运行时上下文：无法执行归还前的异步健康检查，
                // 直接退役并丢弃连接（retire 仅做原子计数与信号量释放，同步安全），
                // 保证池容量守恒；绝不在 Drop 中 panic 或阻塞等待网络
                eprintln!(
                    "[ormer] pooled connection dropped outside a tokio runtime; \
                     the connection is discarded instead of being returned to the pool"
                );
                futures::executor::block_on(inner.close_connection());
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
        self.inner
            .return_connection(conn, self.created_at.take())
            .await;
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

    /// 连接的实际数据库类型（含 QuestDB 复用 PostgreSQL 连接的运行时判定，
    /// 与统一层 `Database::db_type` 口径一致；供 save 等池连接入口使用）。
    pub(crate) fn db_type(&self) -> DbType {
        db_type_for_connection(self.get_connection())
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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => {
                CreateTableExecutor::InfluxDB(db, PhantomData)
            }
        }
    }

    /// 验证表结构
    ///
    /// 以 [`crate::Capabilities::schema_introspection`] 为准：ClickHouse/InfluxDB
    /// 统一拒绝；QuestDB 为 true，由 PostgreSQL 后端按运行时 db_type 分流到
    /// `table_columns()` 专用校验。
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        let db_type = db_type_for_connection(self.get_connection());
        crate::Capabilities::ensure(db_type, |caps| caps.schema_introspection, "validate_table")?;
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
            // 矩阵兜底：正常不可达（schema_introspection=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
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

    /// 部分字段插入 - 返回执行器（L24：与 [`crate::Database::insert_partial`]
    /// 对齐的池连接入口）。
    pub fn insert_partial<T: WritableModel + Send + Sync>(&self) -> InsertPartialExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                InsertPartialExecutor::Sqlite(db.insert_partial::<T>(), PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                InsertPartialExecutor::PostgreSQL(db.insert_partial::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => InsertPartialExecutor::MySQL(db.insert_partial::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => InsertPartialExecutor::MSSQL(db.insert_partial::<T>()),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                InsertPartialExecutor::DuckDB(db.insert_partial::<T>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(_) => InsertPartialExecutor::Unsupported {
                backend: DbType::ClickHouse,
                feature: "partial Model insert on ClickHouse",
                _marker: PhantomData,
            },
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(_) => InsertPartialExecutor::Unsupported {
                backend: DbType::InfluxDB,
                feature: "insert_partial",
                _marker: PhantomData,
            },
        }
    }

    /// 保存 tracked 模型的 dirty 变更 + 图关系同步（L24：与
    /// [`crate::Database::save`] 对齐的池连接入口；写核心在池连接自开的
    /// 事务上执行，结束时提交/回滚）。
    pub fn save<'s, T: WritableModel + crate::model::GraphWritable>(
        &'s self,
        model: &'s mut Tracked<T>,
    ) -> SaveExecutor<'s, T> {
        SaveExecutor::from_pooled(self, model)
    }

    /// 清空表数据 - 返回执行器（`TRUNCATE TABLE`；L24：QuestDB 不支持
    /// 行级 DELETE，这是池连接清空表数据的唯一受支持手段）。
    ///
    /// 以 [`crate::Capabilities::truncate`] 为准，QuestDB 复用 PostgreSQL
    /// 连接、按运行时 db_type 判定（与 [`crate::Database::truncate_table`]
    /// 一致），仍走 PostgreSQL 执行器分支。
    pub fn truncate_table<T: WritableModel>(&self) -> TruncateTableExecutor<'_, T> {
        let db_type = db_type_for_connection(self.get_connection());
        if !crate::Capabilities::of(db_type).truncate {
            return TruncateTableExecutor::Unsupported {
                backend: db_type,
                feature: "truncate_table",
                _marker: PhantomData,
            };
        }
        match self.get_connection() {
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                TruncateTableExecutor::PostgreSQL(db.truncate_table::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                TruncateTableExecutor::MySQL(db.truncate_table::<T>())
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                TruncateTableExecutor::MSSQL(db.truncate_table::<T>())
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                TruncateTableExecutor::DuckDB(db.truncate_table::<T>())
            }
            // 矩阵兜底：正常不可达（truncate=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => TruncateTableExecutor::Unsupported {
                backend: db_type,
                feature: "truncate_table",
                _marker: PhantomData,
            },
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
                super::unified::ClickHouseSelectBackend::ClickHouse(db),
                crate::query::builder::Select::default(),
            ),
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => super::unified::SelectExecutor::ClickHouse(
                super::unified::ClickHouseSelectBackend::Influx(db),
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
    ///
    /// 以 [`crate::Capabilities::row_delete`] 为准：ClickHouse/InfluxDB/QuestDB
    /// 统一落入 `Unsupported` 变体；QuestDB 复用 PostgreSQL 连接，按运行时
    /// db_type 判定。
    pub fn delete<T: WritableModel>(&self) -> super::unified::DeleteExecutor<'_, T> {
        let db_type = db_type_for_connection(self.get_connection());
        if !crate::Capabilities::of(db_type).row_delete {
            return super::unified::DeleteExecutor::Unsupported {
                backend: db_type,
                feature: "row delete",
                _marker: PhantomData,
            };
        }
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
            // 矩阵兜底：正常不可达（row_delete=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => super::unified::DeleteExecutor::Unsupported {
                backend: db_type,
                feature: "row delete",
                _marker: PhantomData,
            },
        }
    }

    /// 创建按块删除执行器（时序数据整块清理）
    pub fn delete_blocks<T: WritableModel>(&self) -> super::unified::BlockDeleteExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => super::unified::BlockDeleteExecutor::fallback(
                DbType::Sqlite,
                super::unified::DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData),
            ),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::BlockDeleteExecutor::PostgreSQL(db.delete_blocks::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::BlockDeleteExecutor::fallback(
                DbType::MySQL,
                super::unified::DeleteExecutor::MySQL(db.delete::<T>()),
            ),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::BlockDeleteExecutor::fallback(
                DbType::MSSQL,
                super::unified::DeleteExecutor::MSSQL(db.delete::<T>()),
            ),
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => super::unified::BlockDeleteExecutor::fallback(
                DbType::DuckDB,
                super::unified::DeleteExecutor::DuckDB(db.delete::<T>()),
            ),
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => super::unified::BlockDeleteExecutor::ClickHouse(
                crate::abstract_layer::clickhouse_backend::BlockDeleteExecutor::new(db),
            ),
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => super::unified::BlockDeleteExecutor::InfluxDB(
                influxdb_backend::BlockDeleteExecutor::new(db),
            ),
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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(_) => super::unified::UpdateExecutor::Unsupported {
                backend: DbType::InfluxDB,
                feature: "row update",
                _marker: PhantomData,
            },
        }
    }

    /// 创建 Related 查询执行器
    ///
    /// 与 `select::<T>().from::<R>()` 是同一功能的重复入口（L30）：
    /// 请改用 `conn.select::<T>().from::<R>()`。
    #[deprecated(
        since = "0.2.12",
        note = "use `select::<T>().from::<R>()` instead"
    )]
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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(_) => {
                super::unified::RelatedSelectExecutor::Unsupported {
                    backend: DbType::InfluxDB,
                    feature: "relation select",
                    _marker: PhantomData,
                }
            }
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<super::unified::Transaction<'_>> {
        self.begin_opts(Default::default()).await
    }

    /// 按各后端默认方式开启事务（不应用任何选项）
    ///
    /// 以 [`crate::Capabilities::transactions`] 为准：ClickHouse/InfluxDB/QuestDB
    /// 统一拒绝，QuestDB 复用 PostgreSQL 连接、按运行时 db_type 判定。
    async fn begin_raw(&self) -> crate::Result<super::unified::Transaction<'_>> {
        let db_type = db_type_for_connection(self.get_connection());
        crate::Capabilities::ensure(db_type, |caps| caps.transactions, "transactions")?;
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
            // 矩阵兜底：正常不可达（transactions=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "transactions",
            }),
        }
    }

    /// 开始事务并按后端语义应用事务选项（MySQL 的选项在 BEGIN 前下发）。
    async fn begin_opts(
        &self,
        options: super::unified::TransactionOptions,
    ) -> crate::Result<super::unified::Transaction<'_>> {
        // MySQL 的选项必须在 BEGIN 前下发（begin_with_opts），其余后端
        // 走"先 BEGIN 再应用选项、失败回滚"的公共路径（L20：与
        // Database::begin_opts 共用 apply_transaction_options_or_rollback）。
        #[cfg(feature = "mysql")]
        if let ConnectionWrapper::MySQL(db) = self.get_connection() {
            let txn = crate::utils::FutureTraceExt::trace(db.begin_with_opts(options)).await?;
            return Ok(super::unified::Transaction::MySQL(txn));
        }
        apply_transaction_options_or_rollback(self.begin_raw().await?, options).await
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
        // 选项已在 begin_opts 阶段按后端语义下发（MySQL 必须在 BEGIN 前）；
        // 收尾（提交/回滚，回滚失败走统一日志）与 Database 路径共用
        // run_txn_closure（L20）。
        let txn = self.begin_opts(options).await?;
        run_txn_closure(txn, f).await
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
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(db) => DropTableExecutor::InfluxDB(db, PhantomData),
        }
    }

    /// 池连接上的原生 SQL select（R3：与 Database / Transaction 路径共用
    /// [`super::unified::RawSelectExecutor`]，经 [`super::unified::ConnRef::Pooled`]
    /// 分派；返回类型的旧名 `PooledRawSelectExecutor` 保留为过渡别名）。
    pub fn select_sql<T>(&self, sql: impl IntoRawSql) -> super::unified::RawSelectExecutor<'_, T> {
        super::unified::RawSelectExecutor::from_conn(
            super::unified::ConnRef::Pooled(self.get_connection()),
            sql.into_raw_sql(),
        )
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    ///
    /// 注意：ClickHouse 与 InfluxDB 的执行通道（HTTP）不报告 affected rows，
    /// 这两个后端恒返回 `Ok(0)`——即使语句实际修改了大量数据。调用方不能
    /// 以返回值 0 区分"未影响任何行"与"后端不统计行数"；需要精确行数时
    /// 应改用 `SELECT count()`（前后各查一次）或选择支持行数统计的后端。
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        // L21：Database / 事务 / 池连接三路径共用一份后端分派。
        exec_raw_sql_on(ConnRefMut::Pooled(self.get_connection()), sql.into_raw_sql()).await
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
        find_by_id_with_executor(self.select::<T>(), key).await
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
        find_related_with_executor(&self.select::<T>(), owner, &relation).await
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
        preload_with_executor(&self.select::<T>(), owners, relation).await
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
    ) -> super::unified::ProjectionSelectExecutor<'_, T, V> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::ProjectionSelectExecutor::Sqlite(db.select_column::<T, V>())
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::ProjectionSelectExecutor::PostgreSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                super::unified::ProjectionSelectExecutor::MySQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                super::unified::ProjectionSelectExecutor::MSSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "duckdb")]
            ConnectionWrapper::DuckDB(db) => {
                super::unified::ProjectionSelectExecutor::DuckDB(db.select_column::<T, V>())
            }
            #[cfg(feature = "clickhouse")]
            ConnectionWrapper::ClickHouse(db) => match super::unified::clickhouse_projection_gate(
                DbType::ClickHouse,
            ) {
                // L19：与 Database::select_column 共用同一判定与文案——
                // ClickHouse 支持聚合投影（advanced_grouping = true），放行
                // 构造分组聚合执行分支，不再在池层提前拒绝。
                None => super::unified::ProjectionSelectExecutor::ClickHouse(
                    db,
                    crate::query::builder::ProjectionSelect::new(),
                ),
                Some(feature) => super::unified::ProjectionSelectExecutor::Unsupported {
                    backend: DbType::ClickHouse,
                    feature,
                    _marker: PhantomData,
                },
            },
            #[cfg(feature = "influxdb")]
            ConnectionWrapper::InfluxDB(_) => {
                super::unified::ProjectionSelectExecutor::Unsupported {
                    backend: DbType::InfluxDB,
                    feature: super::unified::clickhouse_projection_gate(DbType::InfluxDB)
                        .unwrap_or("GROUP BY aggregation"),
                    _marker: PhantomData,
                }
            }
        }
    }
}
