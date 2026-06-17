/// 公共模块 - 包含共享辅助函数、宏定义、连接池和统一接口
pub mod connection_pool;

pub mod common_helpers;

use crate::abstract_layer::DbType;
use crate::model::Value;

/// 宏定义模块 - 用于减少重复代码
#[macro_use]
pub mod macros;

/// 流式查询连接管理模块
pub mod stream_connection;
pub use stream_connection::StreamConnection;

/// 统一使用 unified 模块提供接口，当启用任一数据库 feature 时可用
mod unified;
pub use unified::{
    AggregateFuture, CollectFuture, CreateTableExecutor, Database, DeleteExecutor,
    DropTableExecutor, GroupedCollectFuture, GroupedSelectExecutor, InsertExecutor,
    InsertOrUpdateExecutor, LeftJoinCollectFuture, LeftJoinedSelectExecutor, MappedCollectFuture,
    MappedSelectExecutor, ModelCollectWithFuture, RelatedCollectFuture, RelatedSelectExecutor,
    SelectExecutor, SelectStream, SelectStreamIterator, Transaction, TransactionInsertExecutor,
    TransactionInsertOrIgnoreExecutor, TransactionInsertOrUpdateExecutor, UpdateExecutor,
};

// 连接池类型 - 根据启用的 feature 导出
pub use connection_pool::{ConnectionPool, PooledConnection};

#[derive(Debug, Clone)]
pub struct SingleSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub param_rust_types: Option<Vec<&'static str>>,
}

impl SingleSqlStatement {
    pub fn new(sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            sql: sql.into(),
            params,
            param_rust_types: None,
        }
    }

    pub fn with_param_rust_types(mut self, param_rust_types: Vec<&'static str>) -> Self {
        self.param_rust_types = Some(param_rust_types);
        self
    }
}

#[derive(Debug, Clone)]
pub struct SqlStatement {
    pub db_type: DbType,
    pub statements: Vec<SingleSqlStatement>,
}

impl SqlStatement {
    pub fn single(db_type: DbType, sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            db_type,
            statements: vec![SingleSqlStatement::new(sql, params)],
        }
    }

    pub fn batch(db_type: DbType, statements: Vec<SingleSqlStatement>) -> Self {
        Self {
            db_type,
            statements,
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait SqlExecutor: Sized {
    type Output;

    fn to_sql(&self) -> anyhow::Result<SqlStatement>;

    async fn execute_with_sql(self, sql: SqlStatement) -> anyhow::Result<Self::Output>;

    async fn execute(self) -> anyhow::Result<Self::Output> {
        let sql = self.to_sql()?;
        self.execute_with_sql(sql).await
    }
}
