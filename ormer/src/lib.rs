pub mod abstract_layer;
pub mod model;
pub mod query;

pub use abstract_layer::{
    Database, DbType, MappedCollectFuture, MappedSelectExecutor, Transaction,
};

// 连接池类型 - 当启用任一数据库 feature 时可用
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use abstract_layer::{ConnectionPool, PooledConnection};
pub use model::{Error, FromValue, Insertable, Model, Row, Value, generate_create_table_sql};
pub use ormer_derive::Model;
pub use query::builder::{
    AgeColumn, InnerJoinedSelect, IsInValue, IsInValues, LeftJoinedSelect, MappedSelect,
    MultiTableSelect, NumericColumn, RelatedSelect, RightJoinedSelect, Select, SubqueryParam,
    WhereColumn, WhereExpr,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
