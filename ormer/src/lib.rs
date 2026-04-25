pub mod abstract_layer;
pub mod model;
pub mod query;

// 编译时检查：至少启用一个数据库特性
#[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
compile_error!("At least one database feature must be enabled: turso, postgresql, or mysql");

pub use abstract_layer::DbType;

// 数据库相关类型 - 当启用任一数据库 feature 时可用
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use abstract_layer::{
    Database, MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture, Transaction,
};

// 连接池类型 - 当启用任一数据库 feature 时可用
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use abstract_layer::{ConnectionPool, PooledConnection};
pub use model::{
    Error, FromRowValues, FromSingleValue, FromValue, Insertable, Model, Row, Value,
    generate_create_table_sql,
};
pub use ormer_derive::Model;
pub use query::builder::{
    AgeColumn, InnerJoinedSelect, IsInValue, IsInValues, LeftJoinedSelect, MapToResult,
    MappedSelect, MultiTableSelect, NumericColumn, RelatedSelect, RightJoinedSelect, Select,
    SubqueryParam, WhereColumn, WhereExpr,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
