pub mod abstract_layer;
pub mod hooks;
pub mod migration;
pub mod model;
pub mod query;
pub mod raw_sql;
mod time;
pub mod utils;

#[cfg(not(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
)))]
compile_error!(
    "At least one database feature must be enabled: sqlite, postgresql, mysql, or mssql"
);

pub use abstract_layer::DbType;
pub use migration::{
    MIGRATION_TABLE_NAME, Migration, MigrationInfo, MigrationPlan, MigrationRunner, MigrationStep,
    TableMigration,
};

// 数据库相关类型 - 当启用任一数据库 feature 时可用
#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
pub use abstract_layer::{
    ConnectionPool, CreateTableExecutor, Database, DeleteExecutor, DropTableExecutor,
    MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture, PooledConnection,
    PooledRawSelectExecutor, RawCollectFuture, RawSelectExecutor, SelectStream,
    SelectStreamIterator, SingleSqlStatement, SqlExecutor, SqlStatement, Transaction,
    TransactionInsertOrIgnoreExecutor, TransactionRawCollectFuture, TransactionRawSelectExecutor,
};
pub use anyhow::Result;
pub use hooks::{HookContext, HookOperation};
pub use model::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate, FromRowValues,
    FromSingleValue, FromValue, Insertable, Model, ModelEnum, ModelEnumProvider, PrimaryKey,
    Relation, RelationInfo, RelationKind, Row, Value, generate_create_table_sql,
    generate_create_table_sql_with_name,
};
pub use ormer_derive::{Model, ModelEnum};
pub use query::builder::{
    AgeColumn, GroupByColumns, GroupedSelect, InnerJoinedSelect, IsInValue, IsInValues,
    LeftJoinedSelect, MapToResult, MappedSelect, MultiTableSelect, NumericColumn, RelatedSelect,
    RightJoinedSelect, Select, SelectColumnResult, SetOp, SubqueryParam, UnionSelect, WhereColumn,
    WhereExpr,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
pub use raw_sql::{IntoRawSql, RawSql, sql};
