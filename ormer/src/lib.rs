// 让 crate 内部（含单元测试）也能使用 `::ormer::` 绝对路径，
// 与派生宏（ormer-derive）生成的代码路径保持一致。
extern crate self as ormer;

pub mod abstract_layer;
pub mod db_first;
pub mod error;
pub mod hooks;
pub mod migration;
pub mod model;
pub mod query;
pub mod raw_sql;
pub mod sql_trace;
mod time;
pub mod utils;

#[cfg(not(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "questdb",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
)))]
compile_error!(
    "At least one database feature must be enabled: sqlite, postgresql, questdb, mysql, mssql, duckdb, clickhouse, or influxdb"
);

pub use abstract_layer::DbType;
pub use abstract_layer::capabilities::Capabilities;
pub use db_first::{
    DbFirstColumn, DbFirstForeignKey, DbFirstIndex, DbFirstIndexColumn, DbFirstTable,
};
pub use migration::{
    MIGRATION_TABLE_NAME, Migration, MigrationDryRun, MigrationDryRunStep,
    MigrationExecutionStatus, MigrationInfo, MigrationPlan, MigrationRunner, MigrationStep,
    TableEnsureOutcome, TableMigration,
};

// 数据库相关类型 - 当启用任一数据库 feature 时可用
#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "questdb",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "influxdb"
))]
pub use abstract_layer::{
    AggregateFuture, BatchFuture, BatchManyFuture, BatchQueries, BatchQuery, BatchQueryFuture,
    BlockDeleteExecutor, BlockDeleteResult, CollectFuture, ConnectionPool, CreateTableExecutor,
    Database, DatabaseScope, DbExecutor, DeleteExecutor, DerivedTableCollectFuture,
    DerivedTableSelectExecutor, DoubleIncludedCollectFuture, DoubleIncludedSelectExecutor,
    DropTableExecutor, FirstFuture, FourTableCountFuture, IncludedCollectFuture,
    IncludedSelectExecutor, InnerJoinedSelectExecutor, InsertExecutor, InsertGraphExecutor,
    InsertOrIgnoreExecutor, InsertOrUpdateExecutor, InsertPartialExecutor, IsolationLevel,
    LeftJoinCollectFuture, LeftJoinedSelectExecutor, ModelCollectWithFuture,
    MultiTableCountFuture, NestedInclude, PooledConnection, PooledDatabaseScope,
    ProjectionCollectFuture, ProjectionSelectExecutor, RawCollectFuture, RawSelectExecutor,
    RelatedCollectFuture, RelatedCountFuture, RelatedSelectExecutor, RelationNestedLoader,
    ReplicatedConnectionPool, ReplicatedDatabase, ReplicatedDatabaseBuilder, ReplicatedPoolBuilder,
    RightJoinedSelectExecutor, SaveExecutor, ScopedDeleteExecutor, ScopedUpdateExecutor,
    SelectExecutor, SelectStream, SelectStreamIterator, SingleSqlStatement, SqlExecutor,
    SqlStatement, Transaction, TransactionFuture, TransactionOptions, TruncateTableExecutor,
    UnionSelectExecutor, UpdateExecutor, UpdateGraphExecutor, WithoutHooksExecutor,
};
// 旧类型名过渡别名（已合并，保留 re-export 以兼容现有导入路径）：
// - Mapped/Grouped* → Projection*（R2）
// - Transaction*Insert* / TransactionRaw* / TransactionSave* → 合并入对应普通执行器（R3）
// - PooledRawSelectExecutor → RawSelectExecutor（R3）
#[allow(deprecated)]
pub use abstract_layer::{
    GroupedCollectFuture, GroupedSelectExecutor, MappedCollectFuture, MappedSelectExecutor,
    PooledRawSelectExecutor, TransactionInsertExecutor, TransactionInsertOrIgnoreExecutor,
    TransactionInsertOrUpdateExecutor, TransactionRawCollectFuture, TransactionRawSelectExecutor,
    TransactionSaveExecutor,
};
pub use error::{ConstraintKind, DatabaseErrorKind, OrmerError, Result};
pub use hooks::{HookContext, HookOperation};
pub use model::{
    ActiveValue, AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate,
    CompressionAlgorithm, DbValue, Embed, EmbedWhere, FieldType, FieldTypeProvider, FromRowValues,
    FromValue, GraphWritable, InsertModel, Insertable, Model, NoInclude, PrimaryFields, PrimaryKey,
    Relation, RelationHandle, RelationInfo, RelationKind, RelationPathInfo, RelationQuery,
    RelationSelection, Row, TableOptions, TableRoute, TableRouteValue, ThroughInfo,
    ThroughRelation, TrackableModel, Tracked, Value, ViewModel, WritableModel,
    effective_primary_key_columns, generate_create_table_sql, generate_create_table_sql_with_name,
};
#[cfg(feature = "clickhouse")]
pub use model::{
    generate_clickhouse_create_table_sql, generate_clickhouse_create_table_sql_with_name,
};
pub use ormer_derive::{DbValue, Embed, FieldType, InsertModel, Model, ModelEnum, ViewModel, raw};
pub use query::builder::{
    AggregateSelect, CursorPage, DerivedSelect, DerivedTableSelect, DynamicColumn,
    DynamicColumnSet, FilterQuery, FourTableSelect, GroupByColumns, InnerJoinedSelect,
    IntoArrayValue, IntoGroupingSets, IntoJsonPath, IntoJsonScalar, IsInValue, IsInValues,
    LeftJoinedSelect, MapToResult, MultiTableSelect, NamedFilterQuery, NumericColumn, PageCursor,
    ProjectionSelect, RecursiveColumns, RelatedSelect, RightJoinedSelect, RowValueCompare, Select,
    SelectColumnResult, SetOp, StaticJsonArrayExpr, StaticJsonExpr, StaticJsonUpdate,
    SubqueryParam, TypedColumn, UnionSelect, WhereColumn, WhereExpr, WithoutFilterQuery,
    from_derived,
};
// 旧类型名过渡别名（已合并为 ProjectionSelect，保留 re-export 以兼容现有导入路径）
#[allow(deprecated)]
pub use query::builder::{GroupedSelect, MappedSelect};
pub use query::expr::{
    CaseMatchBuilder, IntervalExpr, IntoRowExpr, IntoSqlExpr, IntoTypedExpr, JsonScalarKind,
    NowExpr, RawExpr, RawExprSegment, RawSqlExpr, SqlExpr, TimePart, TimeUnit, TypedExpr,
    WindowSpecBuilder, case_match, days, hours, minutes, now, raw, row, seconds, value,
};
pub use query::filter::{FilterExpr, OrderBy, OrderDirection};
pub use query::filter::{FullTextMode, FullTextQuery, FullTextRank};
pub use query::insert::{
    ConflictColumns, InsertAssignment, InsertConflict, InsertConflictAction, InsertConflictTarget,
    InsertValue, IntoInsertAssignment, IntoInsertConflictTarget, IntoInsertDefaultColumn,
};
pub use query::update::{
    UpdateAssignment, UpdateBinaryOp, UpdateExpr, UpdateField, UpdateFields, UpdateValue,
};
pub use raw_sql::{IntoRawSql, RawSql, sql};
pub use sql_trace::{SqlTrace, SqlTraceBuilder, SqlTraceEvent, global_sql_trace};

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
