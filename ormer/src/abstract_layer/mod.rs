/// 数据库抽象层模块
/// 根据运行时指定的数据库类型选择对应的数据库后端
// 该 trait 为下方 sql_type 各 cfg 分支的完全限定调用提供作用域；
// 仅启用不实现 TypeMapper 的后端（如 influxdb-only）时未被使用。
#[cfg_attr(
    not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql",
        feature = "clickhouse"
    )),
    allow(unused_imports)
)]
use crate::model::DbBackendTypeMapper;

#[cfg(feature = "sqlite")]
pub mod sqlite_backend;

#[cfg(feature = "postgresql")]
pub mod postgresql_backend;

#[cfg(feature = "questdb")]
pub mod questdb_backend;

#[cfg(feature = "mysql")]
pub mod mysql_backend;

#[cfg(feature = "mssql")]
pub mod mssql_backend;

#[cfg(feature = "duckdb")]
pub mod duckdb_backend;

#[cfg(feature = "clickhouse")]
pub(crate) mod clickhouse_backend;

#[cfg(feature = "influxdb")]
pub(crate) mod influxdb_backend;

pub mod capabilities;

/// 公共模块 - 包含共享辅助函数、宏定义、连接池和统一接口
pub mod common;

/// 数据库类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbType {
    /// Sqlite 数据库
    #[cfg(feature = "sqlite")]
    Sqlite,
    /// PostgreSQL 数据库
    #[cfg(feature = "postgresql")]
    PostgreSQL,
    /// QuestDB database
    #[cfg(feature = "questdb")]
    QuestDB,
    /// MySQL 数据库
    #[cfg(feature = "mysql")]
    MySQL,
    /// MSSQL 数据库
    #[cfg(feature = "mssql")]
    MSSQL,
    /// DuckDB 数据库
    #[cfg(feature = "duckdb")]
    DuckDB,
    /// ClickHouse 数据库
    #[cfg(feature = "clickhouse")]
    ClickHouse,
    /// InfluxDB 2.x database
    #[cfg(feature = "influxdb")]
    InfluxDB,
}

impl DbType {
    pub(crate) fn is_questdb(&self) -> bool {
        match self {
            #[cfg(feature = "questdb")]
            DbType::QuestDB => true,
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// Whether the backend provides transactional migration execution.
    pub fn is_transactional(&self) -> bool {
        #[cfg(feature = "clickhouse")]
        if matches!(self, DbType::ClickHouse) {
            return false;
        }
        #[cfg(feature = "questdb")]
        if matches!(self, DbType::QuestDB) {
            return false;
        }
        #[cfg(feature = "influxdb")]
        if matches!(self, DbType::InfluxDB) {
            return false;
        }
        true
    }

    /// 根据 Rust 类型和数据库类型获取 SQL 类型
    pub fn sql_type(
        &self,
        _rust_type: &str,
        _is_primary: bool,
        _is_auto_increment: bool,
        _is_nullable: bool,
        _enum_variants: Option<&[&str]>,
    ) -> String {
        match self {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => crate::abstract_layer::sqlite_backend::SqliteTypeMapper::sql_type(
                _rust_type,
                _is_primary,
                _is_auto_increment,
                _is_nullable,
                _enum_variants,
            ),
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                crate::abstract_layer::postgresql_backend::PostgreSQLTypeMapper::sql_type(
                    _rust_type,
                    _is_primary,
                    _is_auto_increment,
                    _is_nullable,
                    _enum_variants,
                )
            }
            #[cfg(feature = "questdb")]
            DbType::QuestDB => crate::abstract_layer::questdb_backend::QuestDBTypeMapper::sql_type(
                _rust_type,
                _is_primary,
                _is_auto_increment,
                _is_nullable,
                _enum_variants,
            ),
            #[cfg(feature = "mysql")]
            DbType::MySQL => crate::abstract_layer::mysql_backend::MySQLTypeMapper::sql_type(
                _rust_type,
                _is_primary,
                _is_auto_increment,
                _is_nullable,
                _enum_variants,
            ),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => crate::abstract_layer::mssql_backend::MSSQLTypeMapper::sql_type(
                _rust_type,
                _is_primary,
                _is_auto_increment,
                _is_nullable,
                _enum_variants,
            ),
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => crate::abstract_layer::duckdb_backend::DuckDBTypeMapper::sql_type(
                _rust_type,
                _is_primary,
                _is_auto_increment,
                _is_nullable,
                _enum_variants,
            ),
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => {
                crate::abstract_layer::clickhouse_backend::ClickHouseTypeMapper::sql_type(
                    _rust_type,
                    _is_primary,
                    _is_auto_increment,
                    _is_nullable,
                    _enum_variants,
            )
        }
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => String::new(),
        }
    }
}

pub use common::{
    AggregateFuture, BatchFuture, BatchManyFuture, BatchQueries, BatchQuery, BatchQueryFuture,
    BlockDeleteExecutor, BlockDeleteResult, CollectFuture, CreateTableExecutor, Database,
    DatabaseScope, DbExecutor, DeleteExecutor, DerivedTableCollectFuture,
    DerivedTableSelectExecutor, DoubleIncludedCollectFuture, DoubleIncludedSelectExecutor,
    DropTableExecutor, FirstFuture, FourTableCountFuture, IncludedCollectFuture,
    IncludedSelectExecutor, InnerJoinedSelectExecutor, InsertExecutor, InsertGraphExecutor,
    InsertOrIgnoreExecutor, InsertOrUpdateExecutor, InsertPartialExecutor, IsolationLevel,
    LeftJoinCollectFuture, LeftJoinedSelectExecutor, ModelCollectWithFuture,
    MultiTableCountFuture, NestedInclude, ProjectionCollectFuture, ProjectionSelectExecutor,
    RawCollectFuture, RawSelectExecutor, RelatedCollectFuture, RelatedCountFuture,
    RelatedSelectExecutor, RelationNestedLoader, ReplicatedDatabase, ReplicatedDatabaseBuilder,
    RightJoinedSelectExecutor, SaveExecutor, ScopedDeleteExecutor, ScopedUpdateExecutor,
    SelectExecutor, SelectStream, SelectStreamIterator, SingleSqlStatement, SqlExecutor,
    SqlStatement, Transaction, TransactionFuture, TransactionOptions, TruncateTableExecutor,
    UnionSelectExecutor, UpdateExecutor, UpdateGraphExecutor, WithoutHooksExecutor,
};

// 旧类型名过渡别名（已合并，保留 re-export 以兼容现有导入路径）：
// - Mapped/Grouped* → Projection*（R2）
// - Transaction*Insert* / TransactionRaw* / TransactionSave* → 合并入对应普通执行器（R3）
// - PooledRawSelectExecutor → RawSelectExecutor（R3）
#[allow(deprecated)]
pub use common::{
    GroupedCollectFuture, GroupedSelectExecutor, MappedCollectFuture, MappedSelectExecutor,
    PooledRawSelectExecutor, TransactionInsertExecutor, TransactionInsertOrIgnoreExecutor,
    TransactionInsertOrUpdateExecutor, TransactionRawCollectFuture, TransactionRawSelectExecutor,
    TransactionSaveExecutor,
};

pub use common::{
    ConnectionPool, PooledConnection, PooledDatabaseScope, ReplicatedConnectionPool,
    ReplicatedPoolBuilder,
};
