pub mod abstract_layer;
pub mod error;
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
    ConnectionPool, CreateTableExecutor, Database, DbExecutor, DeleteExecutor,
    DerivedTableCollectFuture, DerivedTableSelectExecutor, DoubleIncludedCollectFuture,
    DoubleIncludedSelectExecutor, DropTableExecutor, InsertGraphExecutor, InsertPartialExecutor,
    MappedCollectFuture, MappedSelectExecutor, ModelCollectWithFuture, NestedInclude, PooledConnection,
    PooledRawSelectExecutor, RawCollectFuture, RawSelectExecutor, RelationNestedLoader,
    ReplicatedConnectionPool, ReplicatedDatabase, ReplicatedDatabaseBuilder, ReplicatedPoolBuilder,
    SelectStream, SelectStreamIterator, SingleSqlStatement, SqlExecutor, SqlStatement, Transaction,
    TransactionInsertOrIgnoreExecutor, TransactionRawCollectFuture, TransactionRawSelectExecutor,
    UpdateGraphExecutor,
};
pub use error::{ConstraintKind, DatabaseErrorKind, OrmerError, Result};
pub use hooks::{HookContext, HookOperation};
pub use model::{
    ActiveValue, AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate,
    Embed, EmbedWhere, FromRowValues, FromSingleValue, FromValue, GraphWritable, InsertModel,
    Insertable, Model, ModelEnum, ModelEnumProvider, NoInclude, PrimaryKey, Relation,
    RelationHandle, RelationInfo, RelationKind, RelationPathInfo, RelationQuery, RelationSelection,
    Row, TableRoute, TableRouteValue, ThroughInfo, ThroughRelation, Value, ViewModel,
    WritableModel, generate_create_table_sql, generate_create_table_sql_with_name,
};
pub use ormer_derive::{Embed, InsertModel, Model, ModelEnum, ViewModel};
pub use query::builder::{
    AgeColumn, CursorPage, DerivedSelect, DerivedTableSelect, DynamicColumn, DynamicColumnSet,
    FilterQuery, GroupByColumns, GroupedSelect, InnerJoinedSelect, IntoGroupingSets, IsInValue,
    IsInValues, LeftJoinedSelect, MapToResult, MappedSelect, MultiTableSelect, NumericColumn,
    PageCursor, RecursiveColumns, RelatedSelect, RightJoinedSelect, RowValueCompare, Select,
    SelectColumnResult, SetOp, SubqueryParam, UnionSelect, WhereColumn, WhereExpr, from_derived,
};
pub use query::expr::{
    CaseMatchBuilder, IntoRowExpr, IntoSqlExpr, IntoTypedExpr, SqlExpr, TypedExpr,
    WindowSpecBuilder, case_match, raw, row, value,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
pub use query::insert::{
    ConflictColumns, InsertAssignment, InsertConflict, InsertConflictAction, InsertConflictTarget,
    InsertValue, IntoInsertAssignment, IntoInsertConflictTarget, IntoInsertDefaultColumn,
};
pub use query::update::{
    UpdateAssignment, UpdateBinaryOp, UpdateExpr, UpdateField, UpdateFields, UpdateValue,
};
pub use raw_sql::{IntoRawSql, RawSql, sql};

#[doc(hidden)]
#[macro_export]
macro_rules! ormer_error {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        $crate::OrmerError::other(format!($fmt $(, $arg)*))
    };
    ($msg:expr $(,)?) => {
        $crate::OrmerError::other($msg)
    };
}

#[macro_export]
macro_rules! expr {
    (match $obj:ident . $field:ident { $($pat:literal => $value:expr,)* _ => $default:expr $(,)? }) => {{
        let mut builder = $crate::query::expr::case_match($obj.$field);
        $(
            builder = builder.when($pat, $value);
        )*
        builder.otherwise($default)
    }};
    (match ($expr:expr) { $($pat:literal => $value:expr,)* _ => $default:expr $(,)? }) => {{
        let mut builder = $crate::query::expr::case_match($expr);
        $(
            builder = builder.when($pat, $value);
        )*
        builder.otherwise($default)
    }};
}
