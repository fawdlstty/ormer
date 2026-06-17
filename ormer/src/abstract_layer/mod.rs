/// 数据库抽象层模块
/// 根据运行时指定的数据库类型选择对应的数据库后端
use crate::model::DbBackendTypeMapper;

#[cfg(feature = "sqlite")]
pub mod sqlite_backend;

#[cfg(feature = "postgresql")]
pub mod postgresql_backend;

#[cfg(feature = "mysql")]
pub mod mysql_backend;

#[cfg(feature = "mssql")]
pub mod mssql_backend;

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
    /// MySQL 数据库
    #[cfg(feature = "mysql")]
    MySQL,
    /// MSSQL 数据库
    #[cfg(feature = "mssql")]
    MSSQL,
}

impl DbType {
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
        }
    }
}

pub use common::{
    AggregateFuture, CollectFuture, CreateTableExecutor, Database, DeleteExecutor,
    DropTableExecutor, GroupedCollectFuture, GroupedSelectExecutor, InsertExecutor,
    InsertOrUpdateExecutor, LeftJoinCollectFuture, LeftJoinedSelectExecutor, MappedCollectFuture,
    MappedSelectExecutor, ModelCollectWithFuture, RelatedCollectFuture, RelatedSelectExecutor,
    SelectExecutor, SelectStream, SelectStreamIterator, SingleSqlStatement, SqlExecutor,
    SqlStatement,
    Transaction, TransactionInsertExecutor, TransactionInsertOrIgnoreExecutor,
    TransactionInsertOrUpdateExecutor, UpdateExecutor,
};

pub use common::{ConnectionPool, PooledConnection};
