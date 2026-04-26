pub mod abstract_layer;
pub mod model;
pub mod query;

// 编译时检查：至少启用一个数据库特性（已移除，允许无特性编译）
// #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
// compile_error!("At least one database feature must be enabled: turso, postgresql, or mysql");

pub use abstract_layer::DbType;

// 数据库相关类型 - 当启用任一数据库 feature 时可用
#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
pub use abstract_layer::{
    ConnectionPool, CreateTableExecutor, Database, DeleteExecutor, DropTableExecutor,
    MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture, PooledConnection,
    Transaction,
};
pub use model::{
    Error, FromRowValues, FromSingleValue, FromValue, Insertable, Model, Row, Value,
    generate_create_table_sql, generate_create_table_sql_with_name,
};
pub use ormer_derive::Model;
pub use query::builder::{
    AgeColumn, InnerJoinedSelect, IsInValue, IsInValues, LeftJoinedSelect, MapToResult,
    MappedSelect, MultiTableSelect, NumericColumn, RelatedSelect, RightJoinedSelect, Select,
    SubqueryParam, WhereColumn, WhereExpr,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
