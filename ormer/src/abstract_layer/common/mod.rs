/// 公共模块 - 包含共享辅助函数、宏定义、连接池和统一接口
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
pub mod connection_pool;

pub mod common_helpers;

/// 宏定义模块 - 用于减少重复代码
#[macro_use]
pub mod macros;

/// 流式查询连接管理模块
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
pub mod stream_connection;
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
pub use stream_connection::StreamConnection;

/// 统一使用 unified 模块提供接口，当启用任一数据库 feature 时可用
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
mod unified;
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
pub use unified::{
    AggregateFuture, CollectFuture, CreateTableExecutor, Database, DeleteExecutor,
    DropTableExecutor, GroupedCollectFuture, GroupedSelectExecutor, InsertExecutor,
    InsertOrUpdateExecutor, LeftJoinCollectFuture, LeftJoinedSelectExecutor, MappedCollectFuture,
    MappedSelectExecutor, ModelCollectWithFuture, RelatedCollectFuture, RelatedSelectExecutor,
    SelectExecutor, SelectStream, SelectStreamIterator, Transaction, TransactionInsertExecutor,
    TransactionInsertOrUpdateExecutor, UpdateExecutor,
};

// 连接池类型 - 根据启用的 feature 导出
#[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]
pub use connection_pool::{ConnectionPool, PooledConnection};
