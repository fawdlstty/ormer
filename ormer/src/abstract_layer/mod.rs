/// 数据库抽象层模块
/// 根据运行时指定的数据库类型选择对应的数据库后端
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
use crate::model::DbBackendTypeMapper;

#[cfg(feature = "turso")]
pub mod turso_backend;

#[cfg(feature = "postgresql")]
pub mod postgresql_backend;

#[cfg(feature = "mysql")]
pub mod mysql_backend;

/// 公共模块 - 包含共享辅助函数、宏定义、连接池和统一接口
pub mod common;

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
    /// 空变体，当没有启用任何特性时使用（仅用于编译通过）
    #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
    None,
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
            #[cfg(feature = "turso")]
            DbType::Turso => crate::abstract_layer::turso_backend::TursoTypeMapper::sql_type(
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
            #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
            DbType::None => {
                // 当没有启用任何特性时，返回空字符串（仅用于编译通过）
                String::new()
            }
        }
    }
}

// 统一使用 common 模块提供接口，当启用任一数据库 feature 时可用
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use common::{
    AggregateFuture, CollectFuture, CreateTableExecutor, Database, DeleteExecutor,
    DropTableExecutor, GroupedCollectFuture, GroupedSelectExecutor, LeftJoinCollectFuture,
    LeftJoinedSelectExecutor, MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture,
    RelatedCollectFuture, RelatedSelectExecutor, SelectExecutor, SelectStream,
    SelectStreamIterator, Transaction, UpdateExecutor,
};

// 连接池类型 - 根据启用的 feature 导出
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use common::{ConnectionPool, PooledConnection};
