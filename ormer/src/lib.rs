pub mod abstract_layer;
pub mod model;
pub mod query;

pub use model::{Error, FromValue, Model, Row, Value, generate_create_table_sql};
pub use query::builder::{
    AgeColumn, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect, NumericColumn, RelatedSelect,
    RightJoinedSelect, Select, WhereColumn, WhereExpr,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};

// 重新导出 derive 宏
pub use ormer_derive::Model;

// 预导入 Model trait，使其在所有使用 ormer 的 crate 中自动可用
#[doc(hidden)]
pub mod prelude {
    pub use crate::Model;
}

// 条件编译: 根据启用的 feature 导出 Database 和 DbType
#[cfg(feature = "turso")]
pub use abstract_layer::{Database, DbType, Transaction};

#[cfg(all(feature = "postgresql", not(feature = "turso")))]
pub use abstract_layer::{Database, DbType, Transaction};

#[cfg(all(feature = "mysql", not(feature = "turso"), not(feature = "postgresql")))]
pub use abstract_layer::{Database, DbType, Transaction};
