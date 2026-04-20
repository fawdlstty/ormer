/// 数据库抽象层模块
/// 根据运行时指定的数据库类型选择对应的数据库后端
use crate::model::DbBackendTypeMapper;

#[cfg(feature = "turso")]
pub mod turso_backend;

#[cfg(feature = "postgresql")]
pub mod postgresql_backend;

#[cfg(feature = "mysql")]
pub mod mysql_backend;

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
    CollectFuture, Database, DeleteExecutor, LeftJoinCollectFuture, LeftJoinedSelectExecutor,
    RelatedCollectFuture, RelatedSelectExecutor, SelectExecutor, Transaction, UpdateExecutor,
};
