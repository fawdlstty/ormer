/// 数据库抽象层模块
/// 根据运行时指定的数据库类型选择对应的数据库后端
use crate::model::DbBackendTypeMapper;

pub mod turso_backend;

#[cfg(feature = "postgresql")]
pub mod postgresql_backend;

#[cfg(feature = "mysql")]
pub mod mysql_backend;

/// 连接池模块 - 始终可用
pub mod connection_pool;

/// 公共辅助函数模块
pub mod common_helpers;

/// 宏定义模块 - 用于减少重复代码
#[macro_use]
pub mod macros;

/// 数据库类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbType {
    /// Turso (SQLite) 数据库
    #[cfg(feature = "turso")]
    Turso,
    /// PostgreSQL 数据库
    #[cfg(feature = "postgresql")]
    PostgreSQL,
    /// MySQL 数据库
    #[cfg(feature = "mysql")]
    MySQL,
}

impl DbType {
    /// 根据 Rust 类型和数据库类型获取 SQL 类型
    pub fn sql_type(
        &self,
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
    ) -> String {
        match self {
            #[cfg(feature = "turso")]
            DbType::Turso => crate::abstract_layer::turso_backend::TursoTypeMapper::sql_type(
                rust_type,
                is_primary,
                is_auto_increment,
                is_nullable,
            ),
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                crate::abstract_layer::postgresql_backend::PostgreSQLTypeMapper::sql_type(
                    rust_type,
                    is_primary,
                    is_auto_increment,
                    is_nullable,
                )
            }
            #[cfg(feature = "mysql")]
            DbType::MySQL => crate::abstract_layer::mysql_backend::MySQLTypeMapper::sql_type(
                rust_type,
                is_primary,
                is_auto_increment,
                is_nullable,
            ),
        }
    }
}

// 统一使用 unified 模块提供接口，无论启用哪些 feature
mod unified;
pub use unified::{
    AggregateFuture, CollectFuture, Database, DeleteExecutor, LeftJoinCollectFuture,
    LeftJoinedSelectExecutor, MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture,
    RelatedCollectFuture, RelatedSelectExecutor, SelectExecutor, Transaction, UpdateExecutor,
};

// 连接池类型 - 根据启用的 feature 导出
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use connection_pool::{ConnectionPool, PooledConnection};
