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
    DoubleIncludedCollectFuture, DoubleIncludedSelectExecutor, DropTableExecutor,
    GroupedCollectFuture, GroupedSelectExecutor, IncludedCollectFuture, IncludedSelectExecutor,
    InsertExecutor, InsertOrIgnoreExecutor, InsertOrUpdateExecutor, InsertPartialExecutor,
    LeftJoinCollectFuture, LeftJoinedSelectExecutor, MappedCollectFuture, MappedSelectExecutor,
    ModelCollectWithFuture, NestedInclude, RawCollectFuture, RawSelectExecutor,
    RelatedCollectFuture, RelatedSelectExecutor, RelationNestedLoader, SelectExecutor,
    SelectStream, SelectStreamIterator, Transaction, TransactionInsertExecutor,
    TransactionInsertOrIgnoreExecutor, TransactionInsertOrUpdateExecutor,
    TransactionRawCollectFuture, TransactionRawSelectExecutor, UpdateExecutor,
};

// 连接池类型 - 根据启用的 feature 导出
pub use connection_pool::{ConnectionPool, PooledConnection, PooledRawSelectExecutor};

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

    fn to_sql(&self) -> crate::Result<SqlStatement>;

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output>;

    async fn execute(self) -> crate::Result<Self::Output> {
        let sql = self.to_sql()?;
        self.execute_with_sql(sql).await
    }
}

/// 统一的数据库执行入口。
///
/// 这个 trait 只覆盖 repository/service 最常用的读写方法，返回现有执行器类型，
/// 避免再包一层 Box/Rc 之类的间接层。
pub trait DbExecutor {
    fn select<T: crate::model::Model>(&self) -> SelectExecutor<'_, T>;

    fn select_column<T: crate::model::Model, V>(&self) -> GroupedSelectExecutor<'_, T, V>;
}
