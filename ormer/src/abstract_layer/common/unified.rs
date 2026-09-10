#![allow(clippy::upper_case_acronyms)]

#[cfg(feature = "clickhouse")]
use super::SingleSqlStatement;
/// 统一的数据库抽象层
/// 使用枚举包装不同数据库后端,对外提供统一接口
/// 通过条件编译控制枚举变体
use super::connection_pool;
use super::{SqlStatement, common_helpers};
use super::super::capabilities::Capabilities;
use crate::db_first;
use crate::model::{
    Model, NoInclude, Relation, RelationHandle, RelationInfo, RelationPathInfo, RelationQuery,
    RelationSelection, TableRouteValue, ThroughRelation, Tracked, Value, WritableModel,
    routed_model_table_name_for_db,
};
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
use crate::query::builder::Select;
#[cfg(feature = "clickhouse")]
use crate::query::builder::ProjectionSelect;
use crate::query::builder::{
    ContextFilter, DerivedSelect, DerivedTableSelect, FilterQuery, NamedFilterQuery, WhereExpr,
    WithoutFilterQuery,
};
use crate::query::filter::FilterExpr;
use crate::query::insert::{IntoInsertAssignment, IntoInsertDefaultColumn};
use crate::raw_sql::{IntoRawSql, RawSql};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

pub type TransactionFuture<'a, R> = Pin<Box<dyn Future<Output = crate::Result<R>> + Send + 'a>>;
pub type BatchQueryFuture<'a, T> = Pin<Box<dyn Future<Output = crate::Result<T>> + Send + 'a>>;

static SAVEPOINT_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactionOptions {
    pub isolation: Option<IsolationLevel>,
    pub read_only: bool,
}

impl TransactionOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn isolation(mut self, isolation: IsolationLevel) -> Self {
        self.isolation = Some(isolation);
        self
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn serializable() -> Self {
        Self::new().isolation(IsolationLevel::Serializable)
    }
}

pub struct BatchFuture<'a, B> {
    batch: B,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, B> BatchFuture<'a, B> {
    pub(crate) fn new(batch: B) -> Self {
        Self {
            batch,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, B> std::future::IntoFuture for BatchFuture<'a, B>
where
    B: BatchQueries<'a> + Send + 'a,
{
    type Output = crate::Result<B::Output>;
    type IntoFuture = BatchQueryFuture<'a, B::Output>;

    fn into_future(self) -> Self::IntoFuture {
        self.batch.into_batch_future()
    }
}

pub struct BatchManyFuture<'a, Q> {
    queries: Vec<Q>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, Q> BatchManyFuture<'a, Q> {
    pub(crate) fn new<I>(queries: I) -> Self
    where
        I: IntoIterator<Item = Q>,
    {
        Self {
            queries: queries.into_iter().collect(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, Q> std::future::IntoFuture for BatchManyFuture<'a, Q>
where
    Q: BatchQuery<'a> + Send + 'a,
{
    type Output = crate::Result<Vec<Q::Output>>;
    type IntoFuture = BatchQueryFuture<'a, Vec<Q::Output>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let mut results = Vec::with_capacity(self.queries.len());
            for query in self.queries {
                results.push(query.into_batch_future().await?);
            }
            Ok(results)
        })
    }
}

pub trait BatchQuery<'a>: Sized {
    type Output: Send + 'a;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output>;
}

pub trait BatchQueries<'a>: Sized {
    type Output: Send + 'a;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output>;
}

macro_rules! impl_batch_tuple {
    ($($name:ident => $var:ident),+ $(,)?) => {
        impl<'a, $($name,)+> BatchQueries<'a> for ($($name,)+)
        where
            $($name: BatchQuery<'a> + Send + 'a,)+
        {
            type Output = ($($name::Output,)+);

            fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
                let ($($var,)+) = self;
                Box::pin(async move {
                    $(
                        let $var = $var.into_batch_future().await?;
                    )+
                    Ok(($($var,)+))
                })
            }
        }
    };
}

impl<'a> BatchQueries<'a> for () {
    type Output = ();

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async { Ok(()) })
    }
}

impl_batch_tuple!(A => a);
impl_batch_tuple!(A => a, B => b);
impl_batch_tuple!(A => a, B => b, C => c);
impl_batch_tuple!(A => a, B => b, C => c, D => d);
impl_batch_tuple!(A => a, B => b, C => c, D => d, E => e);
impl_batch_tuple!(A => a, B => b, C => c, D => d, E => e, F => f);
impl_batch_tuple!(A => a, B => b, C => c, D => d, E => e, F => f, G => g);
impl_batch_tuple!(A => a, B => b, C => c, D => d, E => e, F => f, G => g, H => h);

// 根据启用的 feature 导入后端实现
#[cfg(feature = "sqlite")]
use super::super::sqlite_backend;

#[cfg(feature = "postgresql")]
use super::super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::super::mysql_backend;

#[cfg(feature = "mssql")]
use super::super::mssql_backend;

#[cfg(feature = "duckdb")]
use super::super::duckdb_backend;

#[cfg(feature = "clickhouse")]
use super::super::clickhouse_backend;

#[cfg(feature = "influxdb")]
use super::super::influxdb_backend;

fn relation_filter_values(values: Vec<Value>) -> Vec<crate::query::filter::Value> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| !matches!(value, Value::Null))
        .filter(|value| seen.insert(common_helpers::model_value_key(value)))
        .map(Into::into)
        .collect()
}

#[allow(dead_code)]
fn unsupported_feature(backend: super::super::DbType, feature: &'static str) -> crate::OrmerError {
    crate::OrmerError::UnsupportedFeature { backend, feature }
}

/// 计算待应用迁移：按版本排序、重复版本报错、已应用迁移的 checksum 漂移报错。
/// ClickHouse 与 InfluxDB 的 HTTP 迁移路径共用（两者均无事务，历史由各自端点维护）。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
pub(crate) fn compute_pending_migrations<M: crate::migration::Migration>(
    applied: Vec<crate::migration::MigrationInfo>,
    migrations: &[M],
) -> crate::Result<Vec<crate::migration::MigrationInfo>> {
    let applied = applied
        .into_iter()
        .map(|migration| (migration.version, migration.checksum))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut sorted = migrations.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|migration| migration.version());
    let mut seen = std::collections::BTreeSet::new();
    let mut pending = Vec::new();
    for migration in sorted {
        if !seen.insert(migration.version()) {
            return Err(crate::ormer_error!(
                "Duplicate migration version {}",
                migration.version()
            ));
        }
        if let Some(checksum) = applied.get(&migration.version()) {
            if *checksum != migration.checksum() {
                return Err(crate::ormer_error!(
                    "Migration {} checksum changed after it was applied",
                    migration.version()
                ));
            }
            continue;
        }
        pending.push(crate::migration::MigrationInfo {
            version: migration.version(),
            name: migration.name().to_string(),
            checksum: migration.checksum(),
        });
    }
    Ok(pending)
}

pub(crate) fn primary_key_filter<T: Model>(
    key: impl crate::model::PrimaryKey,
) -> crate::Result<WhereExpr> {
    let pk_columns = T::primary_key_columns();
    let pk_values = key.into_values();

    if pk_columns.is_empty() {
        return Err(crate::ormer_error!(
            "Model {} does not have a primary key",
            T::TABLE_NAME
        ));
    }
    if pk_columns.len() != pk_values.len() {
        return Err(crate::ormer_error!(
            "Primary key column count ({}) does not match value count ({})",
            pk_columns.len(),
            pk_values.len()
        ));
    }

    let filters = common_helpers::primary_key_filter_exprs(pk_columns, pk_values);
    let Some(filter) = common_helpers::and_filter_exprs(filters) else {
        return Err(crate::ormer_error!(
            "Model {} does not have a primary key filter",
            T::TABLE_NAME
        ));
    };

    Ok(WhereExpr::from_filter(filter))
}

pub(crate) fn relation_owner_key(path: RelationPathInfo) -> &'static RelationInfo {
    match path {
        RelationPathInfo::Direct { relation } => relation,
        RelationPathInfo::Through { via_relation, .. } => via_relation,
    }
}

/// find_by_id 家族的公共尾部（L26）：主键过滤 → `range(..1)` → collect →
/// 取首行。Database / DatabaseScope / Transaction / TransactionScope /
/// 池连接 PooledDatabaseScope 五个入口共用，各入口只负责构造带各自
/// context filters 的 [`SelectExecutor`]。
pub(crate) async fn find_by_id_with_executor<T>(
    exec: SelectExecutor<'_, T>,
    key: impl crate::model::PrimaryKey,
) -> crate::Result<Option<T>>
where
    T: Model + 'static + Send + Sync,
{
    let where_expr = primary_key_filter::<T>(key)?;
    let results = exec
        .filter(|_| where_expr)
        .range(..1)
        .collect::<Vec<T>>()
        .await?;
    Ok(results.into_iter().next())
}

/// find_related 的公共尾部（L26）：解析 owner key 后走关联查询执行器。
pub(crate) async fn find_related_with_executor<'a, T, S>(
    exec: &SelectExecutor<'a, T>,
    owner: &T,
    relation: &S,
) -> crate::Result<Vec<S::Target>>
where
    T: Model + 'static + Send + Sync,
    S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync,
    S::Target: Send + Sync,
    S::Via: Send + Sync,
{
    let path = relation.path_info()?;
    let key = owner.relation_key_value(relation_owner_key(path))?;
    exec.select_related_with_selection(vec![key], relation).await
}

/// preload 的公共尾部（L26）：批量预加载关联对象，避免 N+1。
pub(crate) async fn preload_with_executor<'a, T, S>(
    exec: &SelectExecutor<'a, T>,
    owners: &mut [T],
    relation: S,
) -> crate::Result<()>
where
    T: Model + 'static + Send + Sync,
    S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync,
    S::Target: Send + Sync,
    S::Via: Send + Sync,
{
    exec.preload_models_with_selection(owners, relation).await
}

/// select_column 家族（[`Database::select_column`] / 池连接
/// `DbExecutor::select_column` / [`SelectExecutor::select_column`]）对
/// ClickHouse 协议后端的统一能力判定（L19：三个入口共用同一判定与文案）。
///
/// 以 `Capabilities::advanced_grouping` 为准：ClickHouse 支持聚合投影
/// （矩阵为 true，放行走分组聚合执行分支）；InfluxDB 不支持（InfluxQL
/// 的 GROUP BY 仅支持时间桶与 tag）。返回 `None` 表示放行，`Some(feature)`
/// 为统一的拒绝文案。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
pub(crate) fn clickhouse_projection_gate(
    db_type: super::super::DbType,
) -> Option<&'static str> {
    if Capabilities::of(db_type).advanced_grouping {
        return None;
    }
    // DbType::InfluxDB 变体仅在 influxdb feature 下存在，需同步门控。
    #[cfg(feature = "influxdb")]
    if db_type == super::super::DbType::InfluxDB {
        return Some(
            "GROUP BY aggregation on InfluxDB (InfluxQL GROUP BY only supports time buckets and tags)",
        );
    }
    Some("GROUP BY aggregation")
}

pub trait NestedInclude<'a, Owner: Model>: Clone {
    fn load_nested_include<'b>(
        self,
        executor: &'b SelectExecutor<'a, Owner>,
        owners: &'b mut [Owner],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        'a: 'b;
}

impl<'a, Owner: Model> NestedInclude<'a, Owner> for NoInclude {
    fn load_nested_include<'b>(
        self,
        _executor: &'b SelectExecutor<'a, Owner>,
        _owners: &'b mut [Owner],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        'a: 'b,
    {
        Box::pin(async { Ok(()) })
    }
}

impl<'a, Owner, Target> NestedInclude<'a, Owner> for Relation<Owner, Target>
where
    Owner: Model + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
{
    fn load_nested_include<'b>(
        self,
        executor: &'b SelectExecutor<'a, Owner>,
        owners: &'b mut [Owner],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        'a: 'b,
    {
        Box::pin(async move { executor.preload_models_with_selection(owners, self).await })
    }
}

impl<'a, Owner, Via, Target> NestedInclude<'a, Owner> for ThroughRelation<Owner, Via, Target>
where
    Owner: Model + 'static + Send + Sync,
    Via: Model + Clone + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
{
    fn load_nested_include<'b>(
        self,
        executor: &'b SelectExecutor<'a, Owner>,
        owners: &'b mut [Owner],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        'a: 'b,
    {
        Box::pin(async move { executor.preload_models_with_selection(owners, self).await })
    }
}

impl<'a, Owner, Target, Handle, Nested> NestedInclude<'a, Owner>
    for RelationQuery<Owner, Target, Handle, Nested>
where
    Owner: Model + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
    Handle: RelationHandle<Owner, Target> + Clone + Send + Sync + 'static,
    Handle::Via: Send + Sync,
    Nested: NestedInclude<'a, Target> + Clone + Send + Sync + 'static,
{
    fn load_nested_include<'b>(
        self,
        executor: &'b SelectExecutor<'a, Owner>,
        owners: &'b mut [Owner],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        'a: 'b,
    {
        Box::pin(async move { executor.preload_models_with_selection(owners, self).await })
    }
}

pub trait RelationNestedLoader<'a, Owner: Model>: RelationSelection<Owner> {
    fn load_nested<'b>(
        &'b self,
        executor: &'b SelectExecutor<'a, Self::Target>,
        related: &'b mut [Self::Target],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        Self::Target: Send + Sync,
        'a: 'b;
}

impl<'a, Owner, Target> RelationNestedLoader<'a, Owner> for Relation<Owner, Target>
where
    Owner: Model + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
{
    fn load_nested<'b>(
        &'b self,
        _executor: &'b SelectExecutor<'a, Self::Target>,
        _related: &'b mut [Self::Target],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        Self::Target: Send + Sync,
        'a: 'b,
    {
        Box::pin(async { Ok(()) })
    }
}

impl<'a, Owner, Via, Target> RelationNestedLoader<'a, Owner> for ThroughRelation<Owner, Via, Target>
where
    Owner: Model + 'static + Send + Sync,
    Via: Model + Clone + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
{
    fn load_nested<'b>(
        &'b self,
        _executor: &'b SelectExecutor<'a, Self::Target>,
        _related: &'b mut [Self::Target],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        Self::Target: Send + Sync,
        'a: 'b,
    {
        Box::pin(async { Ok(()) })
    }
}

impl<'a, Owner, Target, Handle, Nested> RelationNestedLoader<'a, Owner>
    for RelationQuery<Owner, Target, Handle, Nested>
where
    Owner: Model + 'static + Send + Sync,
    Target: Model + Clone + 'static + Send + Sync,
    Handle: RelationHandle<Owner, Target> + Clone + Send + Sync + 'static,
    Handle::Via: Send + Sync,
    Nested: NestedInclude<'a, Target> + Clone + Send + Sync + 'static,
{
    fn load_nested<'b>(
        &'b self,
        executor: &'b SelectExecutor<'a, Self::Target>,
        related: &'b mut [Self::Target],
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'b>>
    where
        Owner: 'static + Send + Sync,
        Self::Target: Send + Sync,
        'a: 'b,
    {
        let nested = self.nested().clone();
        Box::pin(async move { nested.load_nested_include(executor, related).await })
    }
}

/// 统一的 Database 枚举
pub enum Database {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Database),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::Database),
    #[cfg(feature = "clickhouse")]
    ClickHouse(super::super::clickhouse_backend::Database),
    #[cfg(feature = "influxdb")]
    InfluxDB(super::super::influxdb_backend::Database),
}

pub struct ReplicatedDatabaseBuilder {
    db_type: super::super::DbType,
    write_connection: Option<String>,
    read_connections: Vec<String>,
}

pub struct ReplicatedDatabase {
    db_type: super::super::DbType,
    write: Database,
    reads: Vec<Database>,
    next_read: AtomicUsize,
}

impl ReplicatedDatabaseBuilder {
    pub(crate) fn new(db_type: super::super::DbType) -> Self {
        Self {
            db_type,
            write_connection: None,
            read_connections: Vec::new(),
        }
    }

    pub fn write(mut self, connection_string: impl Into<String>) -> Self {
        self.write_connection = Some(connection_string.into());
        self
    }

    pub fn read(mut self, connection_string: impl Into<String>) -> Self {
        self.read_connections.push(connection_string.into());
        self
    }

    pub async fn connect(self) -> crate::Result<ReplicatedDatabase> {
        let Some(write_connection) = self.write_connection else {
            return Err(crate::ormer_error!(
                "replicated database requires a write connection"
            ));
        };

        let write = Database::connect(self.db_type, &write_connection).await?;
        let mut reads = Vec::with_capacity(self.read_connections.len());
        for connection in self.read_connections {
            reads.push(Database::connect(self.db_type, &connection).await?);
        }

        Ok(ReplicatedDatabase {
            db_type: self.db_type,
            write,
            reads,
            next_read: AtomicUsize::new(0),
        })
    }
}

impl ReplicatedDatabase {
    pub fn db_type(&self) -> super::super::DbType {
        self.db_type
    }

    pub fn sql_trace(&self) -> crate::SqlTraceBuilder {
        crate::global_sql_trace().builder()
    }

    pub fn write(&self) -> &Database {
        &self.write
    }

    pub fn read(&self) -> &Database {
        if self.reads.is_empty() {
            return &self.write;
        }
        let index = self.next_read.fetch_add(1, Ordering::Relaxed) % self.reads.len();
        &self.reads[index]
    }

    pub fn scope(&self) -> DatabaseScope<'_> {
        self.write().scope()
    }

    pub async fn transaction<R, F>(&self, f: F) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_>) -> TransactionFuture<'tx, R>,
    {
        self.write.transaction(f).await
    }

    pub async fn transaction_opts<R, F>(
        &self,
        options: TransactionOptions,
        f: F,
    ) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_>) -> TransactionFuture<'tx, R>,
    {
        self.write.transaction_opts(options, f).await
    }
}

pub struct DerivedTableSelectExecutor<'a, R: Model> {
    db: &'a Database,
    select: DerivedTableSelect<R>,
}

#[derive(Clone)]
pub struct DatabaseScope<'a> {
    db: &'a Database,
    context_filters: Vec<ContextFilter>,
}

impl<'a> DatabaseScope<'a> {
    pub fn select<T: Model>(&self) -> SelectExecutor<'a, T> {
        self.db
            .select::<T>()
            .with_context_filters(self.context_filters.clone())
    }

    pub async fn find_by_id<T: Model + 'static + Send + Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> crate::Result<Option<T>> {
        find_by_id_with_executor(self.select::<T>(), key).await
    }

    pub async fn find_related<T: Model + 'static + Send + Sync, S: RelationSelection<T>>(
        &self,
        owner: &T,
        relation: S,
    ) -> crate::Result<Vec<S::Target>>
    where
        for<'b> S: RelationNestedLoader<'b, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
    {
        find_related_with_executor(&self.select::<T>(), owner, &relation).await
    }

    pub async fn preload<T: Model + 'static + Send + Sync, S: RelationSelection<T>>(
        &self,
        owners: &mut [T],
        relation: S,
    ) -> crate::Result<()>
    where
        for<'b> S: RelationNestedLoader<'b, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
    {
        preload_with_executor(&self.select::<T>(), owners, relation).await
    }

    pub fn delete<T: WritableModel>(&self) -> ScopedDeleteExecutor<'a, T> {
        ScopedDeleteExecutor {
            inner: self.db.delete::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }

    pub fn update<T: WritableModel>(&self) -> ScopedUpdateExecutor<'a, T> {
        ScopedUpdateExecutor {
            inner: self.db.update::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }
}

#[cfg(feature = "clickhouse")]
async fn clickhouse_select_models<T, C>(
    db: &super::super::clickhouse_backend::Database,
    select: Select<T>,
) -> crate::Result<C>
where
    T: Model,
    C: FromIterator<T>,
{
    let (sql, params) = select.try_to_sql_with_params(super::super::DbType::ClickHouse)?;
    let columns = T::columns();
    let rows = db
        .select_values(RawSql::new(sql).with_params(params), Some(&columns))
        .await?;
    rows.iter()
        .map(|values| T::from_row_values(values))
        .collect()
}

/// 渲染 InfluxQL 并解析结果为模型集合（与 ClickHouse 路径共用统一查询构建器）。
#[cfg(feature = "influxdb")]
async fn influx_select_models<T, C>(
    db: &influxdb_backend::Database,
    select: Select<T>,
) -> crate::Result<C>
where
    T: Model,
    C: FromIterator<T>,
{
    let (sql, params) = select.try_to_sql_with_params(super::super::DbType::InfluxDB)?;
    let columns = T::columns();
    let rows = db
        .select_values(RawSql::new(sql).with_params(params), Some(&columns))
        .await?;
    rows.iter()
        .map(|values| T::from_row_values(values))
        .collect()
}

/// 在 ClickHouse 协议后端上执行聚合 SELECT，返回单列聚合值。
///
/// InfluxQL 没有 `AVG`，对应语义是 `MEAN`，在此按前缀替换；
/// COUNT/SUM/MIN/MAX 两边语法一致。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
async fn clickhouse_aggregate_on_backend<T, R>(
    db: ClickHouseSelectBackend<'_>,
    aggregate: crate::query::builder::AggregateSelect<T, R>,
) -> crate::Result<R>
where
    T: Model,
    R: crate::model::FromValue,
{
    let backend = clickhouse_select_backend_db_type(db);
    // InfluxQL 没有 AVG，对应语义是 MEAN（仅 influxdb 构建需要改写 sql）
    #[cfg_attr(not(feature = "influxdb"), allow(unused_mut))]
    let (mut sql, params) = aggregate.try_to_sql_with_params(backend)?;
    #[cfg(feature = "influxdb")]
    if backend == super::super::DbType::InfluxDB && sql.starts_with("SELECT AVG(") {
        sql = sql.replacen("SELECT AVG(", "SELECT MEAN(", 1);
    }
    let rows = match db {
        #[cfg(feature = "clickhouse")]
        ClickHouseSelectBackend::ClickHouse(db) => {
            db.select_values(RawSql::new(sql).with_params(params), None).await?
        }
        #[cfg(feature = "influxdb")]
        ClickHouseSelectBackend::Influx(db) => {
            db.raw_select_values(RawSql::new(sql).with_params(params), None).await?
        }
    };
    let value = rows
        .first()
        .and_then(|row| row.first().cloned())
        .unwrap_or(Value::Null);
    R::from_value(&value)
}

#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
async fn clickhouse_select_models_on_backend<T, C>(
    db: ClickHouseSelectBackend<'_>,
    select: Select<T>,
) -> crate::Result<C>
where
    T: Model,
    C: FromIterator<T>,
{
    match db {
        #[cfg(feature = "clickhouse")]
        ClickHouseSelectBackend::ClickHouse(db) => {
            clickhouse_select_models::<T, C>(db, select).await
        }
        #[cfg(feature = "influxdb")]
        ClickHouseSelectBackend::Influx(db) => influx_select_models::<T, C>(db, select).await,
    }
}

#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
async fn clickhouse_select_first_on_backend<T>(
    db: ClickHouseSelectBackend<'_>,
    select: Select<T>,
) -> crate::Result<Option<T>>
where
    T: Model + Send + Sync,
{
    Ok(
        clickhouse_select_models_on_backend::<T, Vec<T>>(db, select)
            .await?
            .into_iter()
            .next(),
    )
}

#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
async fn clickhouse_fetch_page_on_backend<T>(
    db: ClickHouseSelectBackend<'_>,
    select: Select<T>,
) -> crate::Result<crate::query::builder::CursorPage<T>>
where
    T: Model + Send + Sync,
{
    let (select, cursor_columns) = select.prepare_cursor_page()?;
    let items =
        clickhouse_select_models_on_backend::<T, Vec<T>>(db, select.clone()).await?;
    select.finish_cursor_page(items, &cursor_columns)
}

impl<'a, T: Model> NamedFilterQuery<T> for DatabaseScope<'a> {
    fn apply_named_filter(mut self, name: &'static str, expr: WhereExpr) -> Self {
        self.context_filters
            .push(ContextFilter::new::<T>(name, expr));
        self
    }
}

/// 统一的 CreateTableExecutor 枚举
pub enum CreateTableExecutor<'a, T: crate::model::WritableModel> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::CreateTableExecutor<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
    #[cfg(feature = "influxdb")]
    InfluxDB(
        &'a influxdb_backend::Database,
        std::marker::PhantomData<&'a T>,
    ),
}

impl<'a, T: crate::model::WritableModel> CreateTableExecutor<'a, T> {
    // table_name 仅由本地后端分支消费（InfluxDB measurement 名称由模型固定）
    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unused_variables)
    )]
    pub fn with_table_name(self, table_name: &str) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            CreateTableExecutor::Sqlite(exec) => {
                CreateTableExecutor::Sqlite(exec.with_table_name(table_name))
            }
            #[cfg(feature = "postgresql")]
            CreateTableExecutor::PostgreSQL(exec) => {
                CreateTableExecutor::PostgreSQL(exec.with_table_name(table_name))
            }
            #[cfg(feature = "mysql")]
            CreateTableExecutor::MySQL(exec) => {
                CreateTableExecutor::MySQL(exec.with_table_name(table_name))
            }
            #[cfg(feature = "mssql")]
            CreateTableExecutor::MSSQL(exec) => {
                CreateTableExecutor::MSSQL(exec.with_table_name(table_name))
            }
            #[cfg(feature = "duckdb")]
            CreateTableExecutor::DuckDB(exec) => {
                CreateTableExecutor::DuckDB(exec.with_table_name(table_name))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ CreateTableExecutor::Unsupported { .. } => unsupported,
            // InfluxDB measurement 名称由模型固定，表路由不生效
            #[cfg(feature = "influxdb")]
            unsupported @ CreateTableExecutor::InfluxDB(..) => unsupported,
        }
    }

    pub fn route_table(self, key: impl Into<String>, value: impl TableRouteValue) -> Self {
        let mut route = crate::model::TableRoute::new();
        route.insert(key, value);
        self.with_table_route(route)
    }

    pub fn with_table_route(self, route: crate::model::TableRoute) -> Self {
        let db_type = match &self {
            #[cfg(feature = "sqlite")]
            CreateTableExecutor::Sqlite(_) => crate::abstract_layer::DbType::Sqlite,
            #[cfg(feature = "postgresql")]
            CreateTableExecutor::PostgreSQL(_) => crate::abstract_layer::DbType::PostgreSQL,
            #[cfg(feature = "mysql")]
            CreateTableExecutor::MySQL(_) => crate::abstract_layer::DbType::MySQL,
            #[cfg(feature = "mssql")]
            CreateTableExecutor::MSSQL(_) => crate::abstract_layer::DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            CreateTableExecutor::DuckDB(_) => crate::abstract_layer::DbType::DuckDB,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            CreateTableExecutor::Unsupported { backend, .. } => *backend,
            #[cfg(feature = "influxdb")]
            CreateTableExecutor::InfluxDB(..) => super::super::DbType::InfluxDB,
        };
        let table_name = routed_model_table_name_for_db::<T>(db_type, &route)
            .unwrap_or_else(|err| panic!("Failed to render table route: {}", err));
        self.with_table_name(&table_name)
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            CreateTableExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            CreateTableExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            CreateTableExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            CreateTableExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            CreateTableExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            CreateTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
            #[cfg(feature = "influxdb")]
            CreateTableExecutor::InfluxDB(..) => Err(unsupported_feature(
                super::super::DbType::InfluxDB,
                "create_table to_sql (retention policy is applied over HTTP)",
            )),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            CreateTableExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            CreateTableExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            CreateTableExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            CreateTableExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            CreateTableExecutor::DuckDB(exec) => exec.execute().await,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            CreateTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
            #[cfg(feature = "influxdb")]
            CreateTableExecutor::InfluxDB(db, _) => db.create_table::<T>().await,
        }?;
        crate::model::clear_version_snapshots::<T>();
        Ok(())
    }
}

/// 统一的 DropTableExecutor 枚举
pub enum DropTableExecutor<'a, T: crate::model::WritableModel> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(
        &'a super::super::clickhouse_backend::Database,
        std::marker::PhantomData<T>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
    #[cfg(feature = "influxdb")]
    InfluxDB(
        &'a influxdb_backend::Database,
        std::marker::PhantomData<&'a T>,
    ),
}

impl<'a, T: crate::model::WritableModel> DropTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            DropTableExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            DropTableExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            DropTableExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            DropTableExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            DropTableExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(feature = "clickhouse")]
            DropTableExecutor::ClickHouse(_, _) => {
                let table = crate::model::quote_qualified_identifier(
                    super::super::DbType::ClickHouse,
                    T::table_name_for_db(super::super::DbType::ClickHouse),
                );
                Ok(SqlStatement::single(
                    super::super::DbType::ClickHouse,
                    format!("DROP TABLE IF EXISTS {table}"),
                    Vec::new(),
                ))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            DropTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
            #[cfg(feature = "influxdb")]
            DropTableExecutor::InfluxDB(..) => {
                let table = crate::model::quote_qualified_identifier(
                    super::super::DbType::InfluxDB,
                    T::table_name_for_db(super::super::DbType::InfluxDB),
                );
                Ok(SqlStatement::single(
                    super::super::DbType::InfluxDB,
                    format!("DROP MEASUREMENT {table}"),
                    Vec::new(),
                ))
            }
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            DropTableExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            DropTableExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            DropTableExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            DropTableExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            DropTableExecutor::DuckDB(exec) => exec.execute().await,
            #[cfg(feature = "clickhouse")]
            DropTableExecutor::ClickHouse(db, _) => db.drop_table::<T>().await,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            DropTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
            #[cfg(feature = "influxdb")]
            DropTableExecutor::InfluxDB(db, _) => db.drop_table::<T>().await,
        }?;
        crate::model::clear_version_snapshots::<T>();
        Ok(())
    }
}

/// 统一的 TruncateTableExecutor 枚举：`TRUNCATE TABLE t`。
///
/// QuestDB 不支持行级 DELETE，`truncate_table` 是唯一的清空表数据手段；
/// PostgreSQL/MySQL/MSSQL/DuckDB 同样原生支持；SQLite 无 TRUNCATE 语法
/// （能力矩阵为 false），ClickHouse/InfluxDB 未接入执行器。
pub enum TruncateTableExecutor<'a, T: crate::model::WritableModel> {
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TruncateTableExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TruncateTableExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TruncateTableExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::TruncateTableExecutor<'a, T>),
    /// 能力矩阵门控产物：`truncate: false` 的后端在
    /// [`Database::truncate_table`] 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
}

impl<'a, T: crate::model::WritableModel> TruncateTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "postgresql")]
            TruncateTableExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            TruncateTableExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            TruncateTableExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            TruncateTableExecutor::DuckDB(exec) => exec.to_sql(),
            TruncateTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "postgresql")]
            TruncateTableExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TruncateTableExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TruncateTableExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            TruncateTableExecutor::DuckDB(exec) => exec.execute().await,
            TruncateTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }?;
        crate::model::clear_version_snapshots::<T>();
        Ok(())
    }
}

/// 统一的 InsertExecutor 枚举
pub enum InsertExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "sqlite")]
    /// 事务路径变体：持有后端事务插入执行器，与 `Sqlite` 变体共享同一套
    /// 链式配置方法（原 `TransactionInsertExecutor` 已合并入本枚举）。
    SqliteTxn(sqlite_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    /// 事务路径变体（原 `TransactionInsertExecutor` 的 PostgreSQL 分支）。
    PostgreSQLTxn(postgresql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    /// 事务路径变体（原 `TransactionInsertExecutor` 的 MySQL 分支）。
    MySQLTxn(mysql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    /// 事务路径变体（原 `TransactionInsertExecutor` 的 MSSQL 分支）。
    MSSQLTxn(mssql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    /// 事务路径变体（原 `TransactionInsertExecutor` 的 DuckDB 分支）。
    DuckDBTxn(duckdb_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(
        &'a clickhouse_backend::Database,
        I,
        Option<crate::query::insert::InsertConflict>,
        std::marker::PhantomData<I::Model>,
    ),
    #[cfg(feature = "influxdb")]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        models: I,
        _marker: std::marker::PhantomData<I::Model>,
        _lifetime: std::marker::PhantomData<&'a ()>,
    },
    #[cfg(feature = "influxdb")]
    InfluxDB(
        &'a influxdb_backend::Database,
        I,
        Option<crate::query::insert::InsertConflict>,
        std::marker::PhantomData<I::Model>,
    ),
}

/// 旧事务插入执行器名过渡别名（已合并为 [`InsertExecutor`]，保留
/// re-export 以兼容现有导入路径；事务入口 [`Transaction::insert`]
/// 返回的就是 [`InsertExecutor`] 的事务变体）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionInsertExecutor 已合并为 InsertExecutor，请改用 InsertExecutor"
)]
pub type TransactionInsertExecutor<'a, I> = InsertExecutor<'a, I>;

pub enum InsertPartialExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::InsertPartialExecutor<'a, T>,
        std::marker::PhantomData<&'a T>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertPartialExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertPartialExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertPartialExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertPartialExecutor<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
}

impl<'a, T: Model + Send + Sync> InsertPartialExecutor<'a, T> {
    // f 仅由本地后端分支消费（ClickHouse/InfluxDB 不支持 partial insert）
    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unused_variables)
    )]
    pub fn set<F, A>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> A,
        A: IntoInsertAssignment<T>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertPartialExecutor::Sqlite(exec, phantom) => {
                InsertPartialExecutor::Sqlite(exec.set(f), phantom)
            }
            #[cfg(feature = "postgresql")]
            InsertPartialExecutor::PostgreSQL(exec) => {
                InsertPartialExecutor::PostgreSQL(exec.set(f))
            }
            #[cfg(feature = "mysql")]
            InsertPartialExecutor::MySQL(exec) => InsertPartialExecutor::MySQL(exec.set(f)),
            #[cfg(feature = "mssql")]
            InsertPartialExecutor::MSSQL(exec) => InsertPartialExecutor::MSSQL(exec.set(f)),
            #[cfg(feature = "duckdb")]
            InsertPartialExecutor::DuckDB(exec) => InsertPartialExecutor::DuckDB(exec.set(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ InsertPartialExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn default<F, C>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> C,
        C: IntoInsertDefaultColumn<T>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertPartialExecutor::Sqlite(exec, phantom) => {
                InsertPartialExecutor::Sqlite(exec.default(f), phantom)
            }
            #[cfg(feature = "postgresql")]
            InsertPartialExecutor::PostgreSQL(exec) => {
                InsertPartialExecutor::PostgreSQL(exec.default(f))
            }
            #[cfg(feature = "mysql")]
            InsertPartialExecutor::MySQL(exec) => InsertPartialExecutor::MySQL(exec.default(f)),
            #[cfg(feature = "mssql")]
            InsertPartialExecutor::MSSQL(exec) => InsertPartialExecutor::MSSQL(exec.default(f)),
            #[cfg(feature = "duckdb")]
            InsertPartialExecutor::DuckDB(exec) => InsertPartialExecutor::DuckDB(exec.default(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ InsertPartialExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertPartialExecutor::Sqlite(exec, _) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertPartialExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertPartialExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertPartialExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertPartialExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            InsertPartialExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<<T as Model>::AutoIncrementKeyType> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertPartialExecutor::Sqlite(exec, _) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertPartialExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertPartialExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertPartialExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertPartialExecutor::DuckDB(exec) => exec.execute().await,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            InsertPartialExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }
}

/// ClickHouse/InfluxDB 变体的 insert conflict 配置落地（L11）：两后端不支持
/// conflict 处理，配置存储到变体字段，由 to_sql / execute 入口统一返回
/// `UnsupportedFeature`，与 to_sql 的拒绝行为一致，不再静默丢弃。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
fn insert_conflict_or_default(
    conflict: &mut Option<crate::query::insert::InsertConflict>,
) -> &mut crate::query::insert::InsertConflict {
    conflict.get_or_insert_with(crate::query::insert::InsertConflict::default)
}

/// 已配置的 insert conflict（ClickHouse/InfluxDB execute 入口的拒绝判定）。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
fn insert_conflict_is_configured(
    conflict: Option<&crate::query::insert::InsertConflict>,
) -> bool {
    conflict.is_some_and(|conflict| conflict.is_configured())
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    /// ClickHouse/InfluxDB 变体的 conflict 配置分支收敛（L22：14 处同构
    /// match 分支合一）。原宏在 match 臂位置展开多条臂的写法非法（宏
    /// 无法生成 match 臂），改为方法：两个后端变体的解构、conflict
    /// 初始化与执行器重建在此统一，调用方分支体以 `FnOnce` 闭包传入
    /// （match 分支互斥，捕获的参数可安全移动）。其余变体（含
    /// Unsupported 哨兵）原样返回。
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    fn with_insert_conflict<F>(self, apply: F) -> Self
    where
        F: FnOnce(&mut crate::query::insert::InsertConflict),
    {
        match self {
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(db, models, mut conflict, marker) => {
                apply(insert_conflict_or_default(&mut conflict));
                InsertExecutor::ClickHouse(db, models, conflict, marker)
            }
            #[cfg(feature = "influxdb")]
            InsertExecutor::InfluxDB(db, models, mut conflict, marker) => {
                apply(insert_conflict_or_default(&mut conflict));
                InsertExecutor::InfluxDB(db, models, conflict, marker)
            }
            other => other,
        }
    }

    pub fn on_conflict<F, C>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> C,
        C: crate::query::insert::ConflictColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.on_conflict(f)),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.on_conflict(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.on_conflict(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.on_conflict(f))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.on_conflict(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => InsertExecutor::MySQLTxn(exec.on_conflict(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.on_conflict(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => InsertExecutor::MSSQLTxn(exec.on_conflict(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.on_conflict(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => InsertExecutor::DuckDBTxn(exec.on_conflict(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                conflict.target = Some(crate::query::insert::InsertConflictTarget::Columns(
                    f(<I::Model as Model>::Where::default()).conflict_columns(),
                ));
            }),
        }
    }

    pub fn on_constraint<Target>(self, target: Target) -> Self
    where
        Target: crate::query::insert::IntoInsertConflictTarget<I::Model>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.on_constraint(target)),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => {
                InsertExecutor::SqliteTxn(exec.on_constraint(target))
            }
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => {
                InsertExecutor::PostgreSQL(exec.on_constraint(target))
            }
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.on_constraint(target))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.on_constraint(target)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => {
                InsertExecutor::MySQLTxn(exec.on_constraint(target))
            }
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.on_constraint(target)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => {
                InsertExecutor::MSSQLTxn(exec.on_constraint(target))
            }
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.on_constraint(target)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => {
                InsertExecutor::DuckDBTxn(exec.on_constraint(target))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                conflict.target = Some(target.into_insert_conflict_target());
            }),
        }
    }

    pub fn conflict_where<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> W,
        W: Into<WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.conflict_where(f)),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.conflict_where(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.conflict_where(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.conflict_where(f))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.conflict_where(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => {
                InsertExecutor::MySQLTxn(exec.conflict_where(f))
            }
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.conflict_where(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => {
                InsertExecutor::MSSQLTxn(exec.conflict_where(f))
            }
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.conflict_where(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => {
                InsertExecutor::DuckDBTxn(exec.conflict_where(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                conflict.target_filter = Some(crate::query::insert::where_expr_to_filter(f(
                    <I::Model as Model>::Where::default(),
                )));
            }),
        }
    }

    pub fn do_nothing(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.do_nothing()),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.do_nothing()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_nothing()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.do_nothing())
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_nothing()),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => InsertExecutor::MySQLTxn(exec.do_nothing()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_nothing()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => InsertExecutor::MSSQLTxn(exec.do_nothing()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_nothing()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => InsertExecutor::DuckDBTxn(exec.do_nothing()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                conflict.action = Some(crate::query::insert::InsertConflictAction::DoNothing);
            }),
        }
    }

    pub fn do_update(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.do_update()),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.do_update()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_update()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.do_update())
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_update()),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => InsertExecutor::MySQLTxn(exec.do_update()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_update()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => InsertExecutor::MSSQLTxn(exec.do_update()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_update()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => InsertExecutor::DuckDBTxn(exec.do_update()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                conflict.action = Some(crate::query::insert::InsertConflictAction::DoUpdate);
            }),
        }
    }

    pub fn do_update_if<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> W,
        W: Into<WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.do_update_if(f)),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.do_update_if(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_update_if(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.do_update_if(f))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_update_if(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => InsertExecutor::MySQLTxn(exec.do_update_if(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_update_if(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => InsertExecutor::MSSQLTxn(exec.do_update_if(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_update_if(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => InsertExecutor::DuckDBTxn(exec.do_update_if(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                let update_filter = crate::query::insert::where_expr_to_filter(f(
                    <I::Model as Model>::Where::default(),
                ));
                conflict.action = Some(crate::query::insert::InsertConflictAction::DoUpdate);
                conflict.update_filter = Some(update_filter);
            }),
        }
    }

    pub fn set<F>(self, f: F) -> Self
    where
        F: FnOnce(&mut <I::Model as Model>::Update),
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.set(f)),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => InsertExecutor::SqliteTxn(exec.set(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.set(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => {
                InsertExecutor::PostgreSQLTxn(exec.set(f))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.set(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => InsertExecutor::MySQLTxn(exec.set(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.set(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => InsertExecutor::MSSQLTxn(exec.set(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.set(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => InsertExecutor::DuckDBTxn(exec.set(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            exec => exec.with_insert_conflict(|conflict| {
                let mut update = <I::Model as Model>::Update::default();
                f(&mut update);
                conflict
                    .action
                    .get_or_insert(crate::query::insert::InsertConflictAction::DoUpdate);
                conflict.assignments.extend(
                    <<I::Model as Model>::Update as crate::query::update::UpdateFields>::assignments(
                        &update,
                    ),
                );
            }),
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => exec.to_sql(),
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(_, models, conflict, _) => {
                // to_sql 渲染等价的多行 VALUES 文本（可观测性用途）；
                // 实际执行走 insert_model_rows 的原生流式 INSERT。
                let refs = models.as_refs();
                let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
                    super::super::DbType::ClickHouse,
                    &refs,
                    conflict.as_ref(),
                )?;
                Ok(SqlStatement::batch(
                    super::super::DbType::ClickHouse,
                    statements
                        .into_iter()
                        .map(|statement| SingleSqlStatement::new(statement.sql, statement.params))
                        .collect(),
                ))
            }
            #[cfg(feature = "influxdb")]
            InsertExecutor::Unsupported {
                backend,
                feature,
                ..
            } => Err(unsupported_feature(*backend, *feature)),
            #[cfg(feature = "influxdb")]
            InsertExecutor::InfluxDB(..) => Err(unsupported_feature(
                super::super::DbType::InfluxDB,
                "insert to_sql (InfluxDB writes use Line Protocol over HTTP)",
            )),
        }
    }

    pub async fn execute(
        self,
    ) -> crate::Result<<I::Model as crate::model::Model>::AutoIncrementKeyType> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(exec) => exec.execute().await,
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(db, mut models, conflict, _) => {
                // ClickHouse 不支持 insert conflict：已配置的 conflict 配置在
                // 入口直接拒绝（与 to_sql 行为一致），不再静默丢弃
                if insert_conflict_is_configured(conflict.as_ref()) {
                    return Err(unsupported_feature(
                        super::super::DbType::ClickHouse,
                        "insert conflict handling",
                    ));
                }
                if models.as_refs().is_empty() {
                    return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
                }

                let ctx = crate::HookContext::new(crate::HookOperation::Insert);
                // 钩子先于写入执行：钩子可修改模型字段，载荷必须在
                // before_insert 之后序列化
                models.run_before_insert(ctx).await?;
                {
                    let refs = models.as_refs();
                    db.insert_model_rows::<I::Model>(&refs).await?;
                }
                models.run_after_insert(ctx).await?;
                Ok(<I::Model as Model>::AutoIncrementKeyType::default())
            }
            #[cfg(feature = "influxdb")]
            InsertExecutor::Unsupported {
                backend,
                feature,
                models,
                ..
            } => {
                drop(models);
                Err(unsupported_feature(backend, feature))
            }
            #[cfg(feature = "influxdb")]
            InsertExecutor::InfluxDB(db, mut models, conflict, _) => {
                // InfluxDB 不支持 insert conflict：已配置的 conflict 配置在
                // 入口直接拒绝（与 to_sql 行为一致），不再静默丢弃
                if insert_conflict_is_configured(conflict.as_ref()) {
                    return Err(unsupported_feature(
                        super::super::DbType::InfluxDB,
                        "insert conflict handling",
                    ));
                }
                if models.as_refs().is_empty() {
                    return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
                }

                crate::abstract_layer::influxdb_backend::validate_influx_model::<I::Model>(
                    super::super::DbType::InfluxDB,
                )?;
                let ctx = crate::HookContext::new(crate::HookOperation::Insert);
                // 钩子可修改模型字段，Line Protocol 必须在 before_insert 之后渲染
                models.run_before_insert(ctx).await?;
                {
                    let refs = models.as_refs();
                    let lines = crate::abstract_layer::influxdb_backend::render_line_protocol::<
                        I::Model,
                    >(&refs)?;
                    db.write_lines_with_policy(
                        &lines,
                        crate::abstract_layer::influxdb_backend::model_retention_policy_name::<
                            I::Model,
                        >()
                        .as_deref(),
                    )
                    .await?;
                }
                models.run_after_insert(ctx).await?;
                Ok(<I::Model as Model>::AutoIncrementKeyType::default())
            }
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }

    pub async fn returning(self) -> crate::Result<Vec<I::Model>> {
        // 能力矩阵优先：dml_returning=false 的后端（MySQL/QuestDB/ClickHouse/
        // InfluxDB）统一以 "DML RETURNING" 拒绝；PostgreSQL 变体经后端
        // db_type() 按运行时类型判定（QuestDB 复用 PG 连接）。
        let backend = match &self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(_) => super::super::DbType::Sqlite,
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(_) => super::super::DbType::Sqlite,
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.db_type(),
            // 事务只存在于 transactions=true 的真实 PostgreSQL 连接上，
            // 无 QuestDB 运行时分支。
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(_) => super::super::DbType::PostgreSQL,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(_) => super::super::DbType::MySQL,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(_) => super::super::DbType::MySQL,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(_) => super::super::DbType::MSSQL,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(_) => super::super::DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(_) => super::super::DbType::DuckDB,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(_) => super::super::DbType::DuckDB,
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(..) => super::super::DbType::ClickHouse,
            #[cfg(feature = "influxdb")]
            InsertExecutor::Unsupported { backend, .. } => *backend,
            #[cfg(feature = "influxdb")]
            InsertExecutor::InfluxDB(..) => super::super::DbType::InfluxDB,
        };
        Capabilities::ensure(backend, |caps| caps.dml_returning, "DML RETURNING")?;
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => exec.returning().await,
            #[cfg(feature = "sqlite")]
            InsertExecutor::SqliteTxn(_) => Err(unsupported_feature(
                super::super::DbType::Sqlite,
                "DML RETURNING inside transactions",
            )),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.returning().await,
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQLTxn(_) => Err(unsupported_feature(
                super::super::DbType::PostgreSQL,
                "DML RETURNING inside transactions",
            )),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.returning().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQLTxn(_) => Err(unsupported_feature(
                super::super::DbType::MySQL,
                "DML RETURNING inside transactions",
            )),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.returning().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQLTxn(_) => Err(unsupported_feature(
                super::super::DbType::MSSQL,
                "DML RETURNING inside transactions",
            )),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.returning().await,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDBTxn(_) => Err(unsupported_feature(
                super::super::DbType::DuckDB,
                "DML RETURNING inside transactions",
            )),
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(..) => Err(unsupported_feature(
                super::super::DbType::ClickHouse,
                "DML RETURNING",
            )),
            #[cfg(feature = "influxdb")]
            InsertExecutor::Unsupported {
                backend,
                feature,
                ..
            } => Err(unsupported_feature(backend, feature)),
            #[cfg(feature = "influxdb")]
            InsertExecutor::InfluxDB(..) => Err(unsupported_feature(
                super::super::DbType::InfluxDB,
                "DML RETURNING",
            )),
        }
    }
}

/// `db.insert(&models).await` 直接执行（委托 [`InsertExecutor::execute`]），
/// 与后端层执行器和文档示例的写法保持一致。
/// 注意：Delete/Update 执行器的 `IntoFuture` 由 `impl_unified_delete_executor!`
/// / `impl_unified_update_executor!` 宏生成，此处不再重复实现。
impl<'a, I> std::future::IntoFuture for InsertExecutor<'a, I>
where
    I: crate::model::Insertable + Send + Sync,
    <I::Model as crate::model::Model>::AutoIncrementKeyType: Send,
    <I as crate::model::Insertable>::Model: Send + Sync,
    Self: 'a,
{
    type Output = crate::Result<<I::Model as crate::model::Model>::AutoIncrementKeyType>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.execute())
    }
}

/// 统一的 InsertOrUpdateExecutor 枚举
pub enum InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertOrUpdateExecutor<'a, I>),
    /// 事务路径变体：原 `TransactionInsertOrUpdateExecutor` 已合并入本枚举。
    #[cfg(feature = "sqlite")]
    SqliteTxn(sqlite_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrUpdateExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrUpdateExecutor` 的 PostgreSQL 分支）。
    #[cfg(feature = "postgresql")]
    PostgreSQLTxn(postgresql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrUpdateExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrUpdateExecutor` 的 MySQL 分支）。
    #[cfg(feature = "mysql")]
    MySQLTxn(mysql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrUpdateExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrUpdateExecutor` 的 MSSQL 分支）。
    #[cfg(feature = "mssql")]
    MSSQLTxn(mssql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertOrUpdateExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrUpdateExecutor` 的 DuckDB 分支）。
    #[cfg(feature = "duckdb")]
    DuckDBTxn(duckdb_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    /// 能力矩阵门控产物：`insert_conflict: false` 的后端在
    /// [`Database::insert_or_update`] 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a I>,
    },
}

/// 旧事务插入或更新执行器名过渡别名（已合并为 [`InsertOrUpdateExecutor`]，
/// 保留 re-export 以兼容现有导入路径）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionInsertOrUpdateExecutor 已合并为 InsertOrUpdateExecutor，请改用 InsertOrUpdateExecutor"
)]
pub type TransactionInsertOrUpdateExecutor<'a, I> = InsertOrUpdateExecutor<'a, I>;

pub struct InsertGraphExecutor<'a, T: crate::model::GraphWritable> {
    db: &'a Database,
    model: &'a mut T,
}

pub struct UpdateGraphExecutor<'a, T: crate::model::GraphWritable> {
    db: &'a Database,
    model: &'a mut T,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::SqliteTxn(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDBTxn(exec) => exec.to_sql(),
            InsertOrUpdateExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::SqliteTxn(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDB(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDBTxn(exec) => exec.execute().await,
            InsertOrUpdateExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

/// 错误处理兜底路径中的回滚：失败不能完全静默，至少记录日志，
/// 避免连接带着未关闭的事务回池而无任何痕迹。
///
/// L20：Database / 事务 / 池连接三条路径的回滚失败日志统一走此处
/// （savepoint 的 ROLLBACK TO / RELEASE 失败同样）。
/// 泛型化返回值：调用方既有 `Result<()>`（事务回滚）也有 `Result<u64>`
/// （savepoint 的 execute_sql），仅日志用途，不关心成功值。
pub(crate) fn log_rollback_failure<T>(result: crate::Result<T>) {
    if let Err(err) = result {
        eprintln!("[ormer] rollback failed during error handling: {err}");
    }
}

/// 事务闭包的公共收尾（L20）：成功提交、失败回滚（回滚失败走统一日志，
/// 不覆盖主错误）。[`Database::transaction_opts`] 与池连接的
/// `PooledConnection::transaction_opts` 共用。
pub(crate) async fn run_txn_closure<R, F>(txn: Transaction<'_>, f: F) -> crate::Result<R>
where
    F: for<'tx> FnOnce(&'tx mut Transaction<'_>) -> TransactionFuture<'tx, R>,
{
    let mut txn = txn;
    match f(&mut txn).await {
        Ok(value) => {
            txn.commit().await?;
            Ok(value)
        }
        Err(err) => {
            log_rollback_failure(txn.rollback().await);
            Err(err)
        }
    }
}

/// "先 BEGIN 再应用事务选项"的公共路径（L20）：应用失败时回滚并返回
/// 原错误（回滚失败走统一日志）。MySQL 以外的后端共用；
/// [`Database::begin_opts`] 与池连接的 `begin_then_apply` 均委托此处。
pub(crate) async fn apply_transaction_options_or_rollback(
    txn: Transaction<'_>,
    options: TransactionOptions,
) -> crate::Result<Transaction<'_>> {
    let mut txn = txn;
    if let Err(err) = apply_transaction_options(&mut txn, options).await {
        log_rollback_failure(txn.rollback().await);
        return Err(err);
    }
    Ok(txn)
}

impl<'a, T> InsertGraphExecutor<'a, T>
where
    T: crate::model::GraphWritable + Send + Sync + 'a,
    <T as Model>::AutoIncrementKeyType: Into<crate::model::Value>,
{
    pub async fn execute(self) -> crate::Result<()> {
        let mut tx = self.db.begin().await?;
        if let Err(err) = async {
            let key = tx.insert(&*self.model).execute().await?;
            let key_value = crate::model::graph_auto_increment_key_value(key);
            if !crate::model::graph_is_no_auto_increment_key(&key_value) {
                self.model
                    .assign_column_value(<T as Model>::primary_key_columns()[0], key_value)?;
            }
            <T as crate::model::GraphWritable>::insert_graph_relations(&mut tx, self.model).await
        }
        .await
        {
            log_rollback_failure(tx.rollback().await);
            return Err(err);
        }
        tx.commit().await
    }
}

impl<'a, T> UpdateGraphExecutor<'a, T>
where
    T: crate::model::GraphWritable + Send + Sync + 'a,
{
    pub async fn execute(self) -> crate::Result<u64> {
        let mut tx = self.db.begin().await?;
        let affected = match async {
            #[cfg(feature = "duckdb")]
            let affected = if tx.db_type() == super::super::DbType::DuckDB {
                match common_helpers::model_update_plan(&*self.model, None) {
                    Some(plan) => {
                        let statement =
                            common_helpers::build_duckdb_graph_update_sql::<T>(&*self.model, &plan)?;
                        match &mut tx {
                            Transaction::DuckDB(txn) => {
                                let executor = txn.update::<T>();
                                <duckdb_backend::UpdateExecutor<T> as super::SqlExecutor>::execute_with_sql(
                                    executor,
                                    SqlStatement::single(
                                        super::super::DbType::DuckDB,
                                        statement.sql,
                                        statement.params,
                                    ),
                                )
                                .await?
                            }
                            // db_type() 已判定为 DuckDB，其余变体不应出现；
                            // 返回错误而非 panic，防御事务枚举被混用。
                            _ => {
                                return Err(crate::OrmerError::invalid_operation(
                                    "DuckDB graph update dispatched to a non-DuckDB transaction",
                                ));
                            }
                        }
                    }
                    None => 0,
                }
            } else {
                tx.update::<T>().set_model(&*self.model).execute().await?
            };
            #[cfg(not(feature = "duckdb"))]
            let affected = tx.update::<T>().set_model(&*self.model).execute().await?;
            <T as crate::model::GraphWritable>::update_graph_relations(&mut tx, self.model).await?;
            Ok::<u64, crate::OrmerError>(affected)
        }
        .await
        {
            Ok(affected) => affected,
            Err(err) => {
                log_rollback_failure(tx.rollback().await);
                return Err(err);
            }
        };
        tx.commit().await?;
        Ok(affected)
    }
}

/// Save 执行器：按 dirty 列更新 + 图关系同步（R3 合并）。
///
/// [`Database::save`] 走自开事务（结束时提交/回滚），[`Transaction::save`]
/// 使用外部事务（写完不提交不回滚，由调用方决定事务边界）。
pub struct SaveExecutor<'a, T: WritableModel + crate::model::GraphWritable> {
    conn: SaveConn<'a, T>,
    model: &'a mut Tracked<T>,
}

/// Save 执行器的连接句柄（R3 合并；Database / 池连接 / 事务三条构造路径）。
///
/// 事务路径的写核心（[`save_dirty_columns_and_relations`] → 图同步链路）要求
/// `&mut Transaction<'tx>`；`'tx` 与外层借用生命周期不同，且 `&'op mut
/// Transaction<'a>` 无法收缩成 `&'op mut Transaction<'op>`（可变引用不变性）。
/// 因此在 [`Transaction::save`] 构造时把事务句柄装进擦除 `'tx` 的运行闭包，
/// [`SaveExecutor`] 的公开形状保持 `SaveExecutor<'a, T>` 不变；
/// SQL 预览同样在构造时渲染（dirty 列集合在执行器存续期被独占借用，不可再变，
/// 尽早渲染与调用时惰性渲染结果一致）。
///
/// 池连接路径（L24）与事务路径同构（预渲染 SQL + 擦除启动闭包，池连接的
/// 内层生命周期同样需要擦除），但语义与 `Db` 一致：写核心自开事务、
/// 结束时提交/回滚。
enum SaveConn<'a, T: WritableModel + crate::model::GraphWritable> {
    Db(&'a Database),
    Pooled {
        sql: crate::Result<SqlStatement>,
        /// 在池连接自开的事务上执行写核心，并把模型句柄归还调用方。
        run: SaveTxnRun<'a, T>,
    },
    Txn {
        sql: crate::Result<SqlStatement>,
        /// 在给定事务上执行写核心，并把模型句柄归还调用方
        /// （后续 accept_changes / after_update 仍由执行器统一调度）。
        run: SaveTxnRun<'a, T>,
    },
}

/// 事务路径写核心的擦除启动器：消费 `&'a mut Tracked<T>`，
/// 返回 (写结果, 模型句柄)。
///
/// 闭包只捕获事务句柄（模型是参数），因此外层 `+ Send` 仅要求
/// `Transaction: Send`；返回的 future 不标注 `Send`，与合并前
/// `TransactionSaveExecutor::execute` 的条件 Send 语义一致
/// （`SaveExecutor` 仍在 `T: Send` 时自动满足 `Send`）。
type SaveTxnRun<'a, T> = Box<
    dyn FnOnce(
            &'a mut Tracked<T>,
        ) -> Pin<
            Box<
                dyn Future<
                    Output = (crate::Result<u64>, &'a mut Tracked<T>),
                > + 'a,
            >,
        > + Send
        + 'a,
>;

/// Save 执行器两条路径共用的收尾：写成功后接受 dirty 变更。
fn finish_save<T: crate::model::Model>(affected: u64, model: &mut Tracked<T>) -> crate::Result<u64> {
    if affected > 0 {
        model.accept_changes();
    }
    Ok(affected)
}

impl<'a, T: WritableModel + crate::model::Model + crate::model::GraphWritable>
    SaveExecutor<'a, T>
{
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match &self.conn {
            SaveConn::Db(db) => {
                let fields = self.model.dirty_columns();
                if fields.is_empty() {
                    return Ok(SqlStatement::batch(db.db_type(), Vec::new()));
                }
                db.update::<T>()
                    .set_model_columns(self.model.as_model(), &fields)
                    .to_sql()
            }
            // 事务 / 池连接路径构造时已渲染，dirty 列在执行器存续期不可再变
            SaveConn::Txn { sql, .. } | SaveConn::Pooled { sql, .. } => sql.clone(),
        }
    }

    pub async fn execute(self) -> crate::Result<u64> {
        let SaveExecutor { conn, mut model } = self;
        let affected = match conn {
            // 自开事务：写入核心共享（R3 合并），结束时提交/回滚
            SaveConn::Db(db) => {
                let mut tx = db.begin().await?;
                match save_dirty_columns_and_relations(&mut tx, &mut model).await {
                    Ok(affected) => {
                        tx.commit().await?;
                        affected
                    }
                    Err(err) => {
                        log_rollback_failure(tx.rollback().await);
                        return Err(err);
                    }
                }
            }
            // 外部事务：写完不提交不回滚，由调用方决定事务边界；
            // 池连接：闭包内自开事务并提交/回滚（L24）
            SaveConn::Txn { run, .. } | SaveConn::Pooled { run, .. } => {
                let (result, m) = run(model).await;
                model = m;
                result?
            }
        };
        finish_save(affected, &mut model)
    }

    /// 同义词，等价于 [`Self::execute`]。
    #[deprecated(since = "0.2.11", note = "use `execute()` instead")]
    pub async fn exec(self) -> crate::Result<u64> {
        self.execute().await
    }

    pub async fn execute_with_hooks(self) -> crate::Result<u64>
    where
        T: crate::BeforeUpdate + crate::AfterUpdate + Send + Sync,
    {
        let SaveExecutor { conn, mut model } = self;
        let mut ctx = crate::HookContext::new(crate::HookOperation::Update);
        // 事务路径携带 `.transaction()` 标记（与合并前事务版语义一致）
        if matches!(conn, SaveConn::Txn { .. }) {
            ctx = ctx.transaction();
        }
        // 与 Update/Delete 执行器同语义：受全局 without_hooks 范围开关控制
        if ctx.hooks_enabled() {
            crate::BeforeUpdate::before_update(model.as_model_mut(), &mut ctx).await?;
        }

        let affected = match conn {
            // 自开事务：after_update 在提交前执行，失败会回滚（与合并前一致）
            SaveConn::Db(db) => {
                let mut tx = db.begin().await?;
                let result = async {
                    let affected =
                        save_dirty_columns_and_relations(&mut tx, &mut model).await?;
                    if affected > 0 && ctx.hooks_enabled() {
                        crate::AfterUpdate::after_update(model.as_model(), &mut ctx).await?;
                    }
                    Ok::<u64, crate::OrmerError>(affected)
                }
                .await;
                match result {
                    Ok(affected) => {
                        tx.commit().await?;
                        affected
                    }
                    Err(err) => {
                        log_rollback_failure(tx.rollback().await);
                        return Err(err);
                    }
                }
            }
            // 外部事务：写完不提交不回滚，由调用方决定事务边界；
            // 池连接：闭包内自开事务并提交/回滚（L24）
            SaveConn::Txn { run, .. } | SaveConn::Pooled { run, .. } => {
                let (result, m) = run(model).await;
                model = m;
                let affected = result?;
                if affected > 0 && ctx.hooks_enabled() {
                    crate::AfterUpdate::after_update(model.as_model(), &mut ctx).await?;
                }
                affected
            }
        };
        finish_save(affected, &mut model)
    }

    /// 跳过本次保存的钩子（等价于在 [`Self::execute_with_hooks`] 外层
    /// 包 `without_hooks` 范围），与其他写入执行器的语义一致。
    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

/// 统一的 InsertOrIgnoreExecutor 枚举
pub enum InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrIgnoreExecutor` 的 SQLite 分支）。
    #[cfg(feature = "sqlite")]
    SqliteTxn(sqlite_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrIgnoreExecutor` 的 PostgreSQL 分支）。
    #[cfg(feature = "postgresql")]
    PostgreSQLTxn(postgresql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrIgnoreExecutor` 的 MySQL 分支）。
    #[cfg(feature = "mysql")]
    MySQLTxn(mysql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrIgnoreExecutor` 的 MSSQL 分支）。
    #[cfg(feature = "mssql")]
    MSSQLTxn(mssql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 事务路径变体（原 `TransactionInsertOrIgnoreExecutor` 的 DuckDB 分支）。
    #[cfg(feature = "duckdb")]
    DuckDBTxn(duckdb_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    /// 能力矩阵门控产物：`insert_ignore: false` 的后端在
    /// [`Database::insert_or_ignore`] 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a I>,
    },
}

/// 旧事务插入或忽略执行器名过渡别名（已合并为 [`InsertOrIgnoreExecutor`]，
/// 保留 re-export 以兼容现有导入路径）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionInsertOrIgnoreExecutor 已合并为 InsertOrIgnoreExecutor，请改用 InsertOrIgnoreExecutor"
)]
pub type TransactionInsertOrIgnoreExecutor<'a, I> = InsertOrIgnoreExecutor<'a, I>;

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::SqliteTxn(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQLTxn(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDBTxn(exec) => exec.to_sql(),
            InsertOrIgnoreExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::SqliteTxn(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQLTxn(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDB(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDBTxn(exec) => exec.execute().await,
            InsertOrIgnoreExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

impl Database {
    pub fn replicated(db_type: super::super::DbType) -> ReplicatedDatabaseBuilder {
        ReplicatedDatabaseBuilder::new(db_type)
    }

    pub fn sql_trace(&self) -> crate::SqlTraceBuilder {
        crate::global_sql_trace().builder()
    }

    /// 连接到数据库,根据 DbType 选择后端
    pub async fn connect(
        db_type: super::super::DbType,
        connection_string: &str,
    ) -> crate::Result<Self> {
        match db_type {
            #[cfg(feature = "sqlite")]
            super::super::DbType::Sqlite => {
                let db = sqlite_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::Sqlite(db))
            }
            #[cfg(feature = "postgresql")]
            super::super::DbType::PostgreSQL => {
                let db = postgresql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::PostgreSQL(db))
            }
            #[cfg(feature = "questdb")]
            super::super::DbType::QuestDB => {
                let db = postgresql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::PostgreSQL(db))
            }
            #[cfg(feature = "mysql")]
            super::super::DbType::MySQL => {
                let db = mysql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::MySQL(db))
            }
            #[cfg(feature = "mssql")]
            super::super::DbType::MSSQL => {
                let db = mssql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::MSSQL(db))
            }
            #[cfg(feature = "duckdb")]
            super::super::DbType::DuckDB => {
                let db = duckdb_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::DuckDB(db))
            }
            #[cfg(feature = "clickhouse")]
            super::super::DbType::ClickHouse => {
                let db = super::super::clickhouse_backend::Database::connect(connection_string)?;
                Ok(Database::ClickHouse(db))
            }
            #[cfg(feature = "influxdb")]
            super::super::DbType::InfluxDB => {
                let db = super::super::influxdb_backend::Database::connect(connection_string)?;
                Ok(Database::InfluxDB(db))
            }
        }
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => CreateTableExecutor::Sqlite(db.create_table::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => CreateTableExecutor::PostgreSQL(db.create_table::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => CreateTableExecutor::MySQL(db.create_table::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => CreateTableExecutor::MSSQL(db.create_table::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => CreateTableExecutor::DuckDB(db.create_table::<T>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => CreateTableExecutor::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "CREATE TABLE without explicit ClickHouse engine settings",
                _marker: std::marker::PhantomData,
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => {
                CreateTableExecutor::InfluxDB(db, std::marker::PhantomData)
            }
        }
    }

    /// 验证表结构
    ///
    /// 以 [`Capabilities::schema_introspection`] 为准：矩阵为 false 的后端
    /// （ClickHouse/InfluxDB）统一拒绝；QuestDB 为 true，走
    /// `table_columns()` 专用校验路径。
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        if !Capabilities::of(self.db_type()).schema_introspection {
            return Err(unsupported_feature(self.db_type(), "validate_table"));
        }
        match self {
            #[cfg(feature = "questdb")]
            Database::PostgreSQL(db) if db.db_type().is_questdb() => {
                let _ = db;
                self.validate_table_questdb::<T>().await
            }
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.validate_table::<T>().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.validate_table::<T>().await,
            // 矩阵兜底：正常不可达（schema_introspection=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(unsupported_feature(self.db_type(), "validate_table")),
        }
    }

    /// Generate Rust model definitions from the database schema.
    ///
    /// 注意：db-first 实体生成不受 [`Capabilities::schema_introspection`] 门控
    /// （ClickHouse 的 db-first 可用而该字段为 false），InfluxDB/QuestDB 的
    /// 拒绝由本入口与 `postgresql_backend::db_first_tables` 分别硬编码。
    pub async fn generate_entities(&self, schema: Option<&str>) -> crate::Result<String> {
        #[cfg(feature = "influxdb")]
        #[allow(irrefutable_let_patterns)]
        if let Database::InfluxDB(_) = self {
            return Err(unsupported_feature(
                super::super::DbType::InfluxDB,
                "generate_entities",
            ));
        }
        let tables = match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => db.db_first_tables(schema).await?,
            // 不可达：InfluxDB 已在函数开头提前报错，仅为 match 穷尽性保留
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => Vec::new(),
        };
        db_first::generate_entities(self.db_type(), &tables)
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => InsertExecutor::Sqlite(db.insert::<I>(models)),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => InsertExecutor::PostgreSQL(db.insert::<I>(models)),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertExecutor::MySQL(db.insert::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertExecutor::MSSQL(db.insert::<I>(models)),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => InsertExecutor::DuckDB(db.insert::<I>(models)),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                InsertExecutor::ClickHouse(db, models, None, std::marker::PhantomData)
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => InsertExecutor::InfluxDB(
                db,
                models,
                None,
                std::marker::PhantomData,
            ),
        }
    }

    pub fn insert_partial<T: WritableModel + Send + Sync>(&self) -> InsertPartialExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertPartialExecutor::Sqlite(db.insert_partial::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => InsertPartialExecutor::PostgreSQL(db.insert_partial::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertPartialExecutor::MySQL(db.insert_partial::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertPartialExecutor::MSSQL(db.insert_partial::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => InsertPartialExecutor::DuckDB(db.insert_partial::<T>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => InsertPartialExecutor::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "partial Model insert on ClickHouse",
                _marker: std::marker::PhantomData,
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => InsertPartialExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: "insert_partial",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn insert_model<T>(
        &self,
        model: impl crate::model::InsertModel<T>,
    ) -> InsertPartialExecutor<'_, T>
    where
        T: WritableModel + Send + Sync,
    {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertPartialExecutor::Sqlite(db.insert_model::<T>(model), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                InsertPartialExecutor::PostgreSQL(db.insert_model::<T>(model))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertPartialExecutor::MySQL(db.insert_model::<T>(model)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertPartialExecutor::MSSQL(db.insert_model::<T>(model)),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => InsertPartialExecutor::DuckDB(db.insert_model::<T>(model)),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => InsertPartialExecutor::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "partial Model insert on ClickHouse",
                _marker: std::marker::PhantomData,
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => InsertPartialExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: "insert_model",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn insert_graph<'a, T>(&'a self, model: &'a mut T) -> InsertGraphExecutor<'a, T>
    where
        T: crate::model::GraphWritable,
    {
        InsertGraphExecutor { db: self, model }
    }

    /// 插入或更新记录 - 返回执行器
    ///
    /// 以 [`Capabilities::insert_conflict`] 为准：矩阵为 false 的后端
    /// （ClickHouse/InfluxDB/QuestDB）统一拒绝；QuestDB 复用 PostgreSQL 连接，
    /// 按运行时 `db_type` 判定，不能按 `Database` 枚举变体判定。
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        if !Capabilities::of(self.db_type()).insert_conflict {
            return InsertOrUpdateExecutor::Unsupported {
                backend: self.db_type(),
                feature: "insert conflict handling",
                _marker: std::marker::PhantomData,
            };
        }
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertOrUpdateExecutor::Sqlite(db.insert_or_update::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                InsertOrUpdateExecutor::PostgreSQL(db.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertOrUpdateExecutor::MySQL(db.insert_or_update::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertOrUpdateExecutor::MSSQL(db.insert_or_update::<I>(models)),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => {
                InsertOrUpdateExecutor::DuckDB(db.insert_or_update::<I>(models))
            }
            // 矩阵兜底：正常不可达（insert_conflict=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => InsertOrUpdateExecutor::Unsupported {
                backend: self.db_type(),
                feature: "insert conflict handling",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn upsert<I: crate::model::Insertable>(&self, models: I) -> InsertOrUpdateExecutor<'_, I> {
        self.insert_or_update(models)
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    ///
    /// 以 [`Capabilities::insert_ignore`] 为准：矩阵为 false 的后端
    /// （ClickHouse/InfluxDB/QuestDB）统一拒绝，QuestDB 按运行时 `db_type` 判定。
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        if !Capabilities::of(self.db_type()).insert_ignore {
            return InsertOrIgnoreExecutor::Unsupported {
                backend: self.db_type(),
                feature: "insert ignore",
                _marker: std::marker::PhantomData,
            };
        }
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertOrIgnoreExecutor::Sqlite(db.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                InsertOrIgnoreExecutor::PostgreSQL(db.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertOrIgnoreExecutor::MySQL(db.insert_or_ignore::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertOrIgnoreExecutor::MSSQL(db.insert_or_ignore::<I>(models)),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => {
                InsertOrIgnoreExecutor::DuckDB(db.insert_or_ignore::<I>(models))
            }
            // 矩阵兜底：正常不可达（insert_ignore=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => InsertOrIgnoreExecutor::Unsupported {
                backend: self.db_type(),
                feature: "insert ignore",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn batch<'a, B>(&'a self, batch: B) -> BatchFuture<'a, B>
    where
        B: BatchQueries<'a>,
    {
        BatchFuture::new(batch)
    }

    pub fn batch_many<'a, I, Q>(&'a self, queries: I) -> BatchManyFuture<'a, Q>
    where
        I: IntoIterator<Item = Q>,
        Q: BatchQuery<'a>,
    {
        BatchManyFuture::new(queries)
    }

    /// 根据主键查找单条记录
    /// 支持单主键和复合主键
    /// ```ignore
    /// // 单主键
    /// let user: Option<User> = db.find_by_id::<User>(1).await?;
    /// // 复合主键
    /// let record: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 2)).await?;
    /// ```
    pub async fn find_by_id<T: Model + 'static + std::marker::Send + std::marker::Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> crate::Result<Option<T>> {
        find_by_id_with_executor(self.select::<T>(), key).await
    }

    /// 查找单个模型的关联对象。
    pub async fn find_related<
        T: Model + 'static + std::marker::Send + std::marker::Sync,
        S: RelationSelection<T>,
    >(
        &self,
        owner: &T,
        relation: S,
    ) -> crate::Result<Vec<S::Target>>
    where
        for<'b> S: RelationNestedLoader<'b, T> + std::marker::Send + std::marker::Sync,
        S::Target: std::marker::Send + std::marker::Sync,
        S::Via: std::marker::Send + std::marker::Sync,
    {
        find_related_with_executor(&self.select::<T>(), owner, &relation).await
    }

    /// 批量预加载关联对象，避免循环查询产生 N+1。
    pub async fn preload<
        T: Model + 'static + std::marker::Send + std::marker::Sync,
        S: RelationSelection<T>,
    >(
        &self,
        owners: &mut [T],
        relation: S,
    ) -> crate::Result<()>
    where
        for<'b> S: RelationNestedLoader<'b, T> + std::marker::Send + std::marker::Sync,
        S::Target: std::marker::Send + std::marker::Sync,
        S::Via: std::marker::Send + std::marker::Sync,
    {
        preload_with_executor(&self.select::<T>(), owners, relation).await
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => SelectExecutor::Sqlite(db.select::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => SelectExecutor::PostgreSQL(db.select::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => SelectExecutor::MySQL(db.select::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => SelectExecutor::MSSQL(db.select::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => SelectExecutor::DuckDB(db.select::<T>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                SelectExecutor::ClickHouse(
                    ClickHouseSelectBackend::ClickHouse(db),
                    crate::query::builder::Select::default(),
                )
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => SelectExecutor::ClickHouse(
                ClickHouseSelectBackend::Influx(db),
                crate::query::builder::Select::default(),
            ),
        }
    }

    pub fn scope(&self) -> DatabaseScope<'_> {
        DatabaseScope {
            db: self,
            context_filters: Vec::new(),
        }
    }

    pub fn from_derived<R: Model>(
        &self,
        derived: DerivedSelect<R>,
    ) -> DerivedTableSelectExecutor<'_, R> {
        DerivedTableSelectExecutor {
            db: self,
            select: crate::query::builder::from_derived(derived),
        }
    }

    /// 执行 UNION / UNION ALL / INTERSECT / EXCEPT 集合查询。
    ///
    /// ```ignore
    /// let union = Select::new()
    ///     .filter(|p| p.age.gt(30))
    ///     .union(Select::new().filter(|p| p.name.like("%admin%")));
    /// let rows: Vec<User> = db.select_union(union).collect().await?;
    /// ```
    pub fn select_union<T: Model>(
        &self,
        union: crate::query::builder::UnionSelect<T>,
    ) -> UnionSelectExecutor<'_, T> {
        UnionSelectExecutor { db: self, select: union }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                ProjectionSelectExecutor::Sqlite(db.select_column::<T, V>())
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                ProjectionSelectExecutor::PostgreSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => ProjectionSelectExecutor::MySQL(db.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => ProjectionSelectExecutor::MSSQL(db.select_column::<T, V>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => ProjectionSelectExecutor::DuckDB(db.select_column::<T, V>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => match clickhouse_projection_gate(
                super::super::DbType::ClickHouse,
            ) {
                None => ProjectionSelectExecutor::ClickHouse(db, ProjectionSelect::new()),
                Some(feature) => ProjectionSelectExecutor::Unsupported {
                    backend: super::super::DbType::ClickHouse,
                    feature,
                    _marker: std::marker::PhantomData,
                },
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => ProjectionSelectExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: clickhouse_projection_gate(super::super::DbType::InfluxDB)
                    .unwrap_or("GROUP BY aggregation"),
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 创建 Delete 执行器
    ///
    /// 以 [`Capabilities::row_delete`] 为准：矩阵为 false 的后端
    /// （ClickHouse/InfluxDB/QuestDB）统一拒绝；QuestDB 复用 PostgreSQL 连接，
    /// 按运行时 `db_type` 判定，不能按 `Database` 枚举变体判定。
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        if !Capabilities::of(self.db_type()).row_delete {
            return DeleteExecutor::Unsupported {
                backend: self.db_type(),
                feature: "row delete",
                _marker: std::marker::PhantomData,
            };
        }
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => DeleteExecutor::PostgreSQL(db.delete::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => DeleteExecutor::MySQL(db.delete::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => DeleteExecutor::MSSQL(db.delete::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => DeleteExecutor::DuckDB(db.delete::<T>()),
            // 矩阵兜底：正常不可达（row_delete=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => DeleteExecutor::Unsupported {
                backend: self.db_type(),
                feature: "row delete",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 创建按块删除执行器（时序数据整块清理，各后端语义见各执行器文档）
    ///
    /// 不做矩阵门控：[`Capabilities::block_delete`] 当前对所有后端为 true
    /// （ClickHouse/InfluxDB 走原生块删除，QuestDB 走 `DROP PARTITION`，
    /// 其余 OLTP 后端回退为对齐时间边界的行删除）；该字段保留为未来新增
    /// 后端的统一门控入口。
    pub fn delete_blocks<T: WritableModel>(&self) -> BlockDeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => BlockDeleteExecutor::fallback(
                super::super::DbType::Sqlite,
                DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData),
            ),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => BlockDeleteExecutor::PostgreSQL(db.delete_blocks::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => BlockDeleteExecutor::fallback(
                super::super::DbType::MySQL,
                DeleteExecutor::MySQL(db.delete::<T>()),
            ),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => BlockDeleteExecutor::fallback(
                super::super::DbType::MSSQL,
                DeleteExecutor::MSSQL(db.delete::<T>()),
            ),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => BlockDeleteExecutor::fallback(
                super::super::DbType::DuckDB,
                DeleteExecutor::DuckDB(db.delete::<T>()),
            ),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                BlockDeleteExecutor::ClickHouse(clickhouse_backend::BlockDeleteExecutor::new(db))
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => {
                BlockDeleteExecutor::InfluxDB(influxdb_backend::BlockDeleteExecutor::new(db))
            }
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                UpdateExecutor::Sqlite(db.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => UpdateExecutor::PostgreSQL(db.update::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => UpdateExecutor::MySQL(db.update::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => UpdateExecutor::MSSQL(db.update::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => UpdateExecutor::DuckDB(db.update::<T>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => UpdateExecutor::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "row update on ClickHouse; use execute_sql",
                _marker: std::marker::PhantomData,
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => UpdateExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: "row update",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn save<'a, T: WritableModel + crate::model::GraphWritable>(
        &'a self,
        model: &'a mut Tracked<T>,
    ) -> SaveExecutor<'a, T> {
        SaveExecutor {
            conn: SaveConn::Db(self),
            model,
        }
    }

    pub fn update_graph<'a, T>(&'a self, model: &'a mut T) -> UpdateGraphExecutor<'a, T>
    where
        T: crate::model::GraphWritable,
    {
        UpdateGraphExecutor { db: self, model }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn from<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                RelatedSelectExecutor::Sqlite(db.related::<T, R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => RelatedSelectExecutor::PostgreSQL(db.related::<T, R>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => RelatedSelectExecutor::MySQL(db.related::<T, R>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => RelatedSelectExecutor::MSSQL(db.related::<T, R>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => RelatedSelectExecutor::DuckDB(db.related::<T, R>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => RelatedSelectExecutor::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "related (multi-table) select",
                _marker: std::marker::PhantomData,
            },
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => RelatedSelectExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: "relation select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 开始事务
    ///
    /// 以 [`Capabilities::of`] 声明的事务能力为准：`transactions: false`
    /// 的后端（QuestDB/ClickHouse/InfluxDB）在此直接返回
    /// `UnsupportedFeature`，而不是透传底层协议的伪事务。
    pub async fn begin(&self) -> crate::Result<Transaction<'_>> {
        if !Capabilities::of(self.db_type()).transactions {
            return Err(unsupported_feature(self.db_type(), "transactions"));
        }
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::Sqlite(txn))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::PostgreSQL(txn))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::MySQL(txn))
            }
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::MSSQL(txn))
            }
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::DuckDB(txn))
            }
            // 矩阵兜底：正常不可达（transactions=false 已在上面拦截）。
            #[allow(unreachable_patterns)]
            _ => Err(unsupported_feature(self.db_type(), "transactions")),
        }
    }

    pub async fn transaction<R, F>(&self, f: F) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_>) -> TransactionFuture<'tx, R>,
    {
        self.transaction_opts(TransactionOptions::new(), f).await
    }

    pub async fn transaction_opts<R, F>(
        &self,
        options: TransactionOptions,
        f: F,
    ) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_>) -> TransactionFuture<'tx, R>,
    {
        // 选项必须在 begin 阶段按后端语义下发（MySQL 的 SET TRANSACTION
        // 只作用于"下一个事务"，begin 之后设置无效）；收尾（提交/回滚）
        // 与池层共用 run_txn_closure。
        let txn = self.begin_opts(options).await?;
        run_txn_closure(txn, f).await
    }

    async fn begin_opts(&self, options: TransactionOptions) -> crate::Result<Transaction<'_>> {
        // MySQL 的选项必须在 BEGIN 前下发（begin_with_opts），其余后端
        // 走"先 BEGIN 再应用选项、失败回滚"的公共路径。
        #[cfg(feature = "mysql")]
        if let Database::MySQL(db) = self {
            let txn = db.begin_with_opts(options).await?;
            return Ok(Transaction::MySQL(txn));
        }
        apply_transaction_options_or_rollback(self.begin().await?, options).await
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => DropTableExecutor::Sqlite(db.drop_table::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => DropTableExecutor::PostgreSQL(db.drop_table::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => DropTableExecutor::MySQL(db.drop_table::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => DropTableExecutor::MSSQL(db.drop_table::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => DropTableExecutor::DuckDB(db.drop_table::<T>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => DropTableExecutor::ClickHouse(db, std::marker::PhantomData),
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => {
                DropTableExecutor::InfluxDB(db, std::marker::PhantomData)
            }
        }
    }

    /// 清空表数据 - 返回执行器（`TRUNCATE TABLE`）
    ///
    /// 以 [`Capabilities::truncate`] 为准：QuestDB 不支持行级 DELETE，
    /// 这是清空表数据的唯一受支持手段（QuestDB 复用 PostgreSQL 连接，
    /// 按运行时 `db_type` 判定，仍走 PostgreSQL 执行器分支）；
    /// MySQL/MSSQL/DuckDB 原生支持；SQLite 无 TRUNCATE 语法（保持 false）。
    pub fn truncate_table<T: WritableModel>(&self) -> TruncateTableExecutor<'_, T> {
        if !Capabilities::of(self.db_type()).truncate {
            return TruncateTableExecutor::Unsupported {
                backend: self.db_type(),
                feature: "truncate_table",
                _marker: std::marker::PhantomData,
            };
        }
        match self {
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                TruncateTableExecutor::PostgreSQL(db.truncate_table::<T>())
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => TruncateTableExecutor::MySQL(db.truncate_table::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => TruncateTableExecutor::MSSQL(db.truncate_table::<T>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => TruncateTableExecutor::DuckDB(db.truncate_table::<T>()),
            // 矩阵兜底：正常不可达（truncate=false 已在上面拦截），
            // 保留防御性拒绝以避免矩阵漂移时 panic。
            #[allow(unreachable_patterns)]
            _ => TruncateTableExecutor::Unsupported {
                backend: self.db_type(),
                feature: "truncate_table",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn select_sql<T>(&self, sql: impl IntoRawSql) -> RawSelectExecutor<'_, T> {
        RawSelectExecutor::from_conn(ConnRef::Db(self), sql.into_raw_sql())
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    ///
    /// 注意：ClickHouse 与 InfluxDB 的执行通道（HTTP）不报告 affected rows，
    /// 这两个后端恒返回 `Ok(0)`——即使语句实际修改了大量数据。调用方不能
    /// 以返回值 0 区分"未影响任何行"与"后端不统计行数"；需要精确行数时
    /// 应改用 `SELECT count()`（前后各查一次）或选择支持行数统计的后端。
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        // L21：Database / 事务 / 池连接三路径共用一份后端分派。
        exec_raw_sql_on(ConnRefMut::Db(self), sql.into_raw_sql()).await
    }

    /// Count rows in a table using backend-specific identifier quoting.
    pub async fn table_row_count(&self, table_name: &str) -> crate::Result<u64> {
        let sql = format!(
            "SELECT COUNT(*) FROM {}",
            common_helpers::quote_table_name_with_schema(self.db_type(), table_name)
        );
        let rows = self.select_sql::<i64>(sql).collect::<Vec<i64>>().await?;
        Ok(rows.into_iter().next().unwrap_or(0).max(0) as u64)
    }

    /// 创建连接池
    #[cfg(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql",
        feature = "duckdb",
        feature = "clickhouse"
    ))]
    pub fn create_pool(
        db_type: super::super::DbType,
        connection_string: &str,
    ) -> super::connection_pool::PoolBuilder {
        super::connection_pool::PoolBuilder::new(db_type, connection_string)
    }
}

impl super::DbExecutor for Database {
    fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        Database::select::<T>(self)
    }

    fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        Database::select_column::<T, V>(self)
    }
}

impl<'a, R: Model> DerivedTableSelectExecutor<'a, R> {
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(R::Where) -> W,
        W: Into<WhereExpr>,
    {
        self.select = self.select.filter(f);
        self
    }

    pub fn order_by<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(R::Where) -> O,
        O: Into<crate::OrderBy>,
    {
        self.select = self.select.order_by(f);
        self
    }

    pub fn order_by_desc<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(R::Where) -> O,
        O: Into<crate::OrderBy>,
    {
        self.select = self.select.order_by_desc(f);
        self
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(mut self, range: RR) -> Self {
        self.select = self.select.range(range);
        self
    }

    pub fn collect<C>(self) -> DerivedTableCollectFuture<'a, R, C>
    where
        R: crate::model::FromRowValues + 'static,
        C: FromIterator<R> + 'static,
    {
        DerivedTableCollectFuture {
            db: self.db,
            select: self.select,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let db_type = self.db.db_type();
        #[cfg(feature = "postgresql")]
        if matches!(db_type, crate::DbType::PostgreSQL) {
            let (sql, params, rust_types) =
                self.select.try_to_sql_with_params_and_types(db_type)?;
            return Ok(SqlStatement::batch(
                db_type,
                vec![super::SingleSqlStatement::new(sql, params).with_param_rust_types(rust_types)],
            ));
        }

        let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
        Ok(SqlStatement::single(db_type, sql, params))
    }
}

pub struct DerivedTableCollectFuture<'a, R: Model, C> {
    db: &'a Database,
    select: DerivedTableSelect<R>,
    _marker: std::marker::PhantomData<C>,
}

impl<'a, R, C> std::future::IntoFuture for DerivedTableCollectFuture<'a, R, C>
where
    R: Model + crate::model::FromRowValues + 'static + std::marker::Send,
    C: FromIterator<R> + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let db_type = self.db.db_type();
            match self.db {
                #[cfg(feature = "sqlite")]
                Database::Sqlite(db) => {
                    let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(db) => {
                    let (sql, params, rust_types) =
                        self.select.try_to_sql_with_params_and_types(db_type)?;
                    db.select_raw_with_types::<R, C>(&sql, params, rust_types)
                        .await
                }
                #[cfg(feature = "mysql")]
                Database::MySQL(db) => {
                    let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                Database::MSSQL(db) => {
                    let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                Database::DuckDB(db) => {
                    let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
                    let rows = db
                        .select_values(RawSql::new(sql).with_params(params), R::row_columns())
                        .await?;
                    rows.into_iter()
                        .map(|values| <R as crate::model::FromRowValues>::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                #[cfg(feature = "influxdb")]
                Database::InfluxDB(_) => Err(unsupported_feature(
                    super::super::DbType::InfluxDB,
                    "derived table select",
                )),
            }
        })
    }
}

/// UNION/INTERSECT/EXCEPT 集合查询执行器（P1-1）。
///
/// SQL 由 [`crate::query::builder::UnionSelect::try_to_sql_with_params`] 生成：操作数以括号包装，
/// 避免操作数自带 ORDER BY/LIMIT 时产生非法 SQL；MSSQL 例外——T-SQL
/// 不支持括号化操作数，按非括号形态拼接。
pub struct UnionSelectExecutor<'a, T: Model> {
    db: &'a Database,
    select: crate::query::builder::UnionSelect<T>,
}

impl<'a, T: Model> UnionSelectExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let db_type = self.db.db_type();
        // MSSQL 的 T-SQL 不支持括号化集合操作数，按非括号形态拼接
        #[cfg(feature = "mssql")]
        let (sql, params) = if db_type == super::super::DbType::MSSQL {
            self.select.to_sql_with_params_unparenthesized(db_type)
        } else {
            self.select.try_to_sql_with_params(db_type)?
        };
        #[cfg(not(feature = "mssql"))]
        let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
        Ok(SqlStatement::single(db_type, sql, params))
    }

    pub fn collect<C>(self) -> UnionCollectFuture<'a, T, C>
    where
        T: crate::model::FromRowValues + 'static,
        C: FromIterator<T> + 'static,
    {
        UnionCollectFuture {
            db: self.db,
            select: self.select,
            _marker: std::marker::PhantomData,
        }
    }

    /// 取第一行（等价于 `collect::<Vec<_>>()` 后取首元素）。
    pub async fn first(self) -> crate::Result<Option<T>>
    where
        T: crate::model::FromRowValues + 'static + Send,
    {
        Ok(self
            .collect::<Vec<T>>()
            .await?
            .into_iter()
            .next())
    }
}

pub struct UnionCollectFuture<'a, T: Model, C> {
    db: &'a Database,
    select: crate::query::builder::UnionSelect<T>,
    _marker: std::marker::PhantomData<C>,
}

impl<'a, T, C> std::future::IntoFuture for UnionCollectFuture<'a, T, C>
where
    T: Model + crate::model::FromRowValues + 'static + std::marker::Send,
    C: FromIterator<T> + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let db_type = self.db.db_type();
            // MSSQL 的 T-SQL 不支持括号化集合操作数
            #[cfg(feature = "mssql")]
            let (sql, params) = if db_type == super::super::DbType::MSSQL {
                self.select.to_sql_with_params_unparenthesized(db_type)
            } else {
                self.select.try_to_sql_with_params(db_type)?
            };
            #[cfg(not(feature = "mssql"))]
            let (sql, params) = self.select.try_to_sql_with_params(db_type)?;
            match self.db {
                #[cfg(feature = "sqlite")]
                Database::Sqlite(db) => db.select_raw::<T, C>(&sql, params).await,
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(db) => db.select_raw::<T, C>(&sql, params).await,
                #[cfg(feature = "mysql")]
                Database::MySQL(db) => db.select_raw::<T, C>(&sql, params).await,
                #[cfg(feature = "duckdb")]
                Database::DuckDB(db) => db.select_raw::<T, C>(&sql, params).await,
                #[cfg(feature = "mssql")]
                Database::MSSQL(db) => db.select_raw::<T, C>(&sql, params).await,
                #[cfg(feature = "clickhouse")]
                Database::ClickHouse(db) => {
                    let rows = db
                        .select_values(RawSql::new(sql).with_params(params), T::row_columns())
                        .await?;
                    rows.into_iter()
                        .map(|values| <T as crate::model::FromRowValues>::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                #[cfg(feature = "influxdb")]
                Database::InfluxDB(_) => Err(unsupported_feature(
                    super::super::DbType::InfluxDB,
                    "union select",
                )),
            }
        })
    }
}

/// 统一层内部连接句柄（R3）：Database / Transaction / 池连接三条执行路径
/// 共用一份执行器实现的关键。
///
/// `Txn` 持共享引用：raw select 路径的后端 `select_raw` 均为 `&self` 接收器，
/// 且 `&Transaction` 的协变性允许把 `&'op Transaction<'a>` 收缩为
/// `&'op Transaction<'op>`，避免 `&'a mut Transaction<'a>` 的不变性陷阱。
/// 写路径（Save）需要 `&mut Transaction`，无法共享本句柄，见 [`SaveConn`]。
#[derive(Clone, Copy)]
pub(crate) enum ConnRef<'a> {
    Db(&'a Database),
    Txn(&'a Transaction<'a>),
    /// 池连接路径（`PooledConnection::select_sql`）。
    Pooled(&'a connection_pool::ConnectionWrapper),
}

/// 写路径连接句柄：与 [`ConnRef`] 同构，但事务变体持 `&mut Transaction`
/// ——后端事务 `exec_raw` 为 `&mut self` 接收器，共享引用无法调用；
/// 读路径（select_raw）无此需求，继续用 [`ConnRef`]。仅被
/// [`exec_raw_sql_on`] 消费。`'tx` 独立于引用生命周期 `'a`：`&mut T`
/// 对 `T` 不变，`&'a mut Transaction<'a>` 会阻断 `Transaction` 协变性
/// 允许的生命周期收缩（见 `Transaction::execute_sql`）。
pub(crate) enum ConnRefMut<'a, 'tx> {
    Db(&'a Database),
    Txn(&'a mut Transaction<'tx>),
    Pooled(&'a connection_pool::ConnectionWrapper),
}

/// 原生非查询 SQL 的公共分派（L21）：[`Database::execute_sql`] /
/// `Transaction::execute_sql` / `PooledConnection::execute_sql` 三条路径
/// 共用一份后端 match（与 raw select 的 [`RawCollectFuture`] 三路合一
/// 先例同构）。ClickHouse/InfluxDB 的执行通道（HTTP）不报告 affected
/// rows，恒返回 `Ok(0)`。写路径：事务变体需 `&mut`，见 [`ConnRefMut`]。
pub(crate) async fn exec_raw_sql_on(
    conn: ConnRefMut<'_, '_>,
    sql: RawSql,
) -> crate::Result<u64> {
    match conn {
        #[cfg(feature = "sqlite")]
        ConnRefMut::Db(Database::Sqlite(db)) => {
            let (sql, params) = sql.render(super::super::DbType::Sqlite)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "sqlite")]
        ConnRefMut::Txn(Transaction::Sqlite(txn)) => {
            let (sql, params) = sql.render(super::super::DbType::Sqlite)?;
            txn.exec_raw(&sql, params).await
        }
        #[cfg(feature = "postgresql")]
        ConnRefMut::Db(Database::PostgreSQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::PostgreSQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "postgresql")]
        ConnRefMut::Txn(Transaction::PostgreSQL(txn)) => {
            let (sql, params) = sql.render(super::super::DbType::PostgreSQL)?;
            txn.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mysql")]
        ConnRefMut::Db(Database::MySQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::MySQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mysql")]
        ConnRefMut::Txn(Transaction::MySQL(txn)) => {
            let (sql, params) = sql.render(super::super::DbType::MySQL)?;
            txn.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mssql")]
        ConnRefMut::Db(Database::MSSQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::MSSQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mssql")]
        ConnRefMut::Txn(Transaction::MSSQL(txn)) => {
            let (sql, params) = sql.render(super::super::DbType::MSSQL)?;
            txn.exec_raw(&sql, params).await
        }
        #[cfg(feature = "duckdb")]
        ConnRefMut::Db(Database::DuckDB(db)) => {
            let (sql, params) = sql.render(super::super::DbType::DuckDB)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "duckdb")]
        ConnRefMut::Txn(Transaction::DuckDB(txn)) => {
            let (sql, params) = sql.render(super::super::DbType::DuckDB)?;
            txn.exec_raw(&sql, params).await
        }
        #[cfg(feature = "postgresql")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::PostgreSQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::PostgreSQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mysql")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::MySQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::MySQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "mssql")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::MSSQL(db)) => {
            let (sql, params) = sql.render(super::super::DbType::MSSQL)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "duckdb")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::DuckDB(db)) => {
            let (sql, params) = sql.render(super::super::DbType::DuckDB)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "sqlite")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::Sqlite(db)) => {
            let (sql, params) = sql.render(super::super::DbType::Sqlite)?;
            db.exec_raw(&sql, params).await
        }
        #[cfg(feature = "clickhouse")]
        ConnRefMut::Db(Database::ClickHouse(db)) => {
            db.execute_sql(sql).await?;
            Ok(0)
        }
        #[cfg(feature = "clickhouse")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::ClickHouse(db)) => {
            db.execute_sql(sql).await?;
            Ok(0)
        }
        #[cfg(feature = "influxdb")]
        ConnRefMut::Db(Database::InfluxDB(db)) => {
            db.execute_sql(sql).await?;
            Ok(0)
        }
        #[cfg(feature = "influxdb")]
        ConnRefMut::Pooled(connection_pool::ConnectionWrapper::InfluxDB(db)) => {
            db.execute_sql(sql).await?;
            Ok(0)
        }
        // 事务路径不含 ClickHouse/InfluxDB（transactions: false），
        // 仅剩 Transaction 的哨兵变体，做 void 消除。
        ConnRefMut::Txn(Transaction::_Phantom(infallible, _)) => match *infallible {},
    }
}

pub struct RawSelectExecutor<'a, T> {
    conn: ConnRef<'a>,
    sql: RawSql,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T> RawSelectExecutor<'a, T> {
    pub(crate) fn from_conn(conn: ConnRef<'a>, sql: RawSql) -> Self {
        Self {
            conn,
            sql,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn collect<C>(self) -> RawCollectFuture<'a, T, C>
    where
        T: crate::model::FromRowValues + 'static,
        C: FromIterator<T> + 'static,
    {
        RawCollectFuture {
            conn: self.conn,
            sql: self.sql,
            _marker: std::marker::PhantomData,
        }
    }
}

/// 旧事务 raw select 执行器名过渡别名（已合并为 [`RawSelectExecutor`]，
/// 保留 re-export 以兼容现有导入路径；`'tx` 参数仅为兼容旧签名保留）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionRawSelectExecutor 已合并为 RawSelectExecutor，请改用 RawSelectExecutor"
)]
pub type TransactionRawSelectExecutor<'a, 'tx, T> = RawSelectExecutor<'a, T>;

/// 旧池 raw select 执行器名过渡别名（已合并为 [`RawSelectExecutor`]，
/// 保留 re-export 以兼容现有导入路径；`'pool` 参数仅为兼容旧签名保留）。
#[deprecated(
    since = "0.2.12",
    note = "PooledRawSelectExecutor 已合并为 RawSelectExecutor，请改用 RawSelectExecutor"
)]
pub type PooledRawSelectExecutor<'conn, 'pool, T> = RawSelectExecutor<'conn, T>;

pub struct RawCollectFuture<'a, T, C> {
    conn: ConnRef<'a>,
    sql: RawSql,
    _marker: std::marker::PhantomData<(T, C)>,
}

/// 旧事务 raw collect future 名过渡别名（已合并为 [`RawCollectFuture`]）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionRawCollectFuture 已合并为 RawCollectFuture，请改用 RawCollectFuture"
)]
pub type TransactionRawCollectFuture<'a, 'tx, T, C> = RawCollectFuture<'a, T, C>;

impl<'a, T, C> std::future::IntoFuture for RawCollectFuture<'a, T, C>
where
    T: crate::model::FromRowValues + 'static + std::marker::Send,
    C: FromIterator<T> + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            // Database / Transaction / 池连接三路径在此共用一份后端分派
            // （R3 合并；各路径的 render 与解码行为与合并前逐字一致）。
            match self.conn {
                #[cfg(feature = "sqlite")]
                ConnRef::Db(Database::Sqlite(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::Sqlite)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "sqlite")]
                ConnRef::Txn(Transaction::Sqlite(txn)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::Sqlite)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                ConnRef::Db(Database::PostgreSQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::PostgreSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                ConnRef::Txn(Transaction::PostgreSQL(txn)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::PostgreSQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mysql")]
                ConnRef::Db(Database::MySQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MySQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mysql")]
                ConnRef::Txn(Transaction::MySQL(txn)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MySQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                ConnRef::Db(Database::MSSQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MSSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                ConnRef::Txn(Transaction::MSSQL(txn)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MSSQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                ConnRef::Db(Database::DuckDB(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::DuckDB)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                ConnRef::Txn(Transaction::DuckDB(txn)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::DuckDB)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::PostgreSQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::PostgreSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mysql")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::MySQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MySQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::MSSQL(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MSSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::DuckDB(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::DuckDB)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "sqlite")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::Sqlite(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::Sqlite)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "clickhouse")]
                ConnRef::Db(Database::ClickHouse(db)) => {
                    let (sql, params) = self.sql.render(super::super::DbType::ClickHouse)?;
                    let rows = db
                        .select_values(
                            crate::raw_sql::RawSql::new(sql).with_params(params),
                            <T as crate::model::FromRowValues>::row_columns(),
                        )
                        .await?;
                    rows.into_iter()
                        .map(|values| T::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                #[cfg(feature = "clickhouse")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::ClickHouse(db)) => {
                    let rows = db
                        .select_values(self.sql, <T as crate::model::FromRowValues>::row_columns())
                        .await?;
                    rows.into_iter()
                        .map(|values| T::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                #[cfg(feature = "influxdb")]
                ConnRef::Db(Database::InfluxDB(db)) => {
                    let rows = db
                        .select_values(
                            self.sql,
                            <T as crate::model::FromRowValues>::row_columns(),
                        )
                        .await?;
                    rows.into_iter()
                        .map(|values| T::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                #[cfg(feature = "influxdb")]
                ConnRef::Pooled(connection_pool::ConnectionWrapper::InfluxDB(db)) => {
                    let rows = db
                        .select_values(self.sql, <T as crate::model::FromRowValues>::row_columns())
                        .await?;
                    rows.into_iter()
                        .map(|values| T::from_row_values(&values))
                        .collect::<crate::Result<C>>()
                }
                // 事务路径不含 ClickHouse/InfluxDB（transactions: false），
                // 仅剩 Transaction 的哨兵变体，做 void 消除。
                ConnRef::Txn(Transaction::_Phantom(infallible, _)) => match *infallible {},
            }
        })
    }
}


/// 统一的 SelectExecutor 枚举
pub enum SelectExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::SelectExecutor<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(crate::abstract_layer::common::unified::ClickHouseSelectBackend<'a>, crate::query::builder::Select<T>),
}

/// ClickHouse protocol backend handle; InfluxDB reuses it only as a hidden
/// unsupported placeholder so builder APIs remain compilable without a DB.
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
#[derive(Clone, Copy)]
pub enum ClickHouseSelectBackend<'a> {
    #[cfg(feature = "clickhouse")]
    ClickHouse(&'a clickhouse_backend::Database),
    #[cfg(feature = "influxdb")]
    #[doc(hidden)]
    Influx(&'a influxdb_backend::Database),
}

#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
fn clickhouse_select_backend_db_type(
    db: ClickHouseSelectBackend<'_>,
) -> super::super::DbType {
    match db {
        #[cfg(feature = "clickhouse")]
        ClickHouseSelectBackend::ClickHouse(_) => super::super::DbType::ClickHouse,
        #[cfg(feature = "influxdb")]
        ClickHouseSelectBackend::Influx(_) => super::super::DbType::InfluxDB,
    }
}

crate::impl_unified_select_executor_methods!(SelectExecutor);

impl<'a, T: Model> SelectExecutor<'a, T> {
    pub fn fields<F, G>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.fields(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.fields(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.fields(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.fields(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.fields(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                SelectExecutor::ClickHouse(db, select.fields(f))
            }
        }
    }

    pub fn query(self, query: impl Into<String>) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.query(query)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.query(query)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.query(query)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.query(query)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.query(query)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                SelectExecutor::ClickHouse(db, select.query(query))
            }
        }
    }

    pub fn mode(self, mode: crate::query::filter::FullTextMode) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.mode(mode)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.mode(mode)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.mode(mode)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.mode(mode)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.mode(mode)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                SelectExecutor::ClickHouse(db, select.mode(mode))
            }
        }
    }

    pub fn language(self, language: impl Into<String>) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.language(language)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.language(language)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.language(language)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.language(language)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.language(language)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                SelectExecutor::ClickHouse(db, select.language(language))
            }
        }
    }

    pub fn rank(self, rank: crate::query::filter::FullTextRank) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.rank(rank)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.rank(rank)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.rank(rank)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.rank(rank)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.rank(rank)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                SelectExecutor::ClickHouse(db, select.rank(rank))
            }
        }
    }
}

impl<'a, T: Model> FilterQuery<T> for SelectExecutor<'a, T> {
    fn append_filter_expr(self, expr: WhereExpr) -> Self {
        SelectExecutor::append_filter_expr(self, expr)
    }
}

impl<'a, T: Model> NamedFilterQuery<T> for SelectExecutor<'a, T> {
    /// 命名过滤器以 `ContextFilter` 形式下发到内部 Select 构建器：
    /// name 与 scope 级 context filter 共用同一份 `(model_table, name)` 键，
    /// `without_filter(name)` 因此可同时撤销查询级与 scope 级同名过滤器。
    fn apply_named_filter(self, name: &'static str, expr: WhereExpr) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(
                exec.with_context_filters(vec![ContextFilter::new::<T>(name, expr)]),
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(
                exec.with_context_filters(vec![ContextFilter::new::<T>(name, expr)]),
            ),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(
                exec.with_context_filters(vec![ContextFilter::new::<T>(name, expr)]),
            ),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(
                exec.with_context_filters(vec![ContextFilter::new::<T>(name, expr)]),
            ),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(
                exec.with_context_filters(vec![ContextFilter::new::<T>(name, expr)]),
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => SelectExecutor::ClickHouse(
                db,
                NamedFilterQuery::<T>::apply_named_filter(select, name, expr),
            ),
        }
    }
}

impl<'a, T: Model> WithoutFilterQuery<T> for SelectExecutor<'a, T> {
    fn without_filter(self, name: &'static str) -> Self {
        SelectExecutor::without_filter(self, name)
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    fn select_model<R: Model>(&self) -> SelectExecutor<'a, R> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectExecutor::Sqlite(exec.select_model::<R>()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                SelectExecutor::PostgreSQL(exec.select_model::<R>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.select_model::<R>()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectExecutor::MSSQL(exec.select_model::<R>()),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectExecutor::DuckDB(exec.select_model::<R>()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => SelectExecutor::ClickHouse(
                *db,
                Select::default().with_context_filters(select.context_filters()),
            ),
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => exec.to_sql(),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                let backend = clickhouse_select_backend_db_type(*db);
                let (sql, params) = select.try_to_sql_with_params(backend)?;
                Ok(SqlStatement::single(backend, sql, params))
            }
        }
    }

    pub fn include<F, S>(self, f: F) -> IncludedSelectExecutor<'a, T, S>
    where
        F: FnOnce(T::Where) -> S,
        S: RelationSelection<T>,
    {
        let where_obj = T::Where::default();
        IncludedSelectExecutor {
            select: self,
            selection: f(where_obj),
            _marker: std::marker::PhantomData,
        }
    }

    pub(crate) async fn select_related_with_selection<S>(
        &self,
        keys: Vec<Value>,
        selection: &S,
    ) -> crate::Result<Vec<S::Target>>
    where
        S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
        T: 'static + Send + Sync,
    {
        match selection.path_info()? {
            RelationPathInfo::Direct { relation } => {
                self.select_target_models::<S>(relation.target_key, keys, selection)
                    .await
            }
            RelationPathInfo::Through {
                via_relation,
                target_relation,
                ..
            } => {
                let via_items = self.select_via_models::<S>(via_relation, keys).await?;
                let target_keys = via_items
                    .iter()
                    .filter_map(|item| item.column_value(target_relation.local_key))
                    .collect();
                self.select_target_models::<S>(target_relation.target_key, target_keys, selection)
                    .await
            }
        }
    }

    async fn select_target_models<S>(
        &self,
        target_key: &str,
        keys: Vec<Value>,
        selection: &S,
    ) -> crate::Result<Vec<S::Target>>
    where
        S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
        T: 'static + Send + Sync,
    {
        let values = relation_filter_values(keys);
        if values.is_empty() {
            return Ok(Vec::new());
        }

        let mut exec = self.select_model::<S::Target>().filter(|_| {
            WhereExpr::from_filter(FilterExpr::In {
                column: target_key.to_string(),
                values,
            })
        });

        for filter in selection.filters().iter().cloned() {
            exec = exec.filter(|_| WhereExpr::from_filter(filter));
        }

        for order in selection.order_by().iter().cloned() {
            exec = exec.order_by(|_| order);
        }

        if selection.range_start().is_some() || selection.range_end().is_some() {
            exec = exec.range(crate::query::builder::RangeBounds {
                start: selection.range_start(),
                end: selection.range_end(),
            });
        }

        let mut related = exec.collect::<Vec<S::Target>>().await?;
        let target_select = self.select_model::<S::Target>();
        selection.load_nested(&target_select, &mut related).await?;
        Ok(related)
    }

    async fn select_via_models<S>(
        &self,
        via_relation: &RelationInfo,
        keys: Vec<Value>,
    ) -> crate::Result<Vec<S::Via>>
    where
        S: RelationSelection<T>,
        S::Via: Send + Sync,
    {
        let values = relation_filter_values(keys);
        if values.is_empty() {
            return Ok(Vec::new());
        }

        self.select_model::<S::Via>()
            .filter(|_| {
                WhereExpr::from_filter(FilterExpr::In {
                    column: via_relation.target_key.to_string(),
                    values,
                })
            })
            .collect::<Vec<S::Via>>()
            .await
    }

    pub(crate) async fn preload_models_with_selection<S>(
        &self,
        owners: &mut [T],
        selection: S,
    ) -> crate::Result<()>
    where
        S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync,
        S::Target: Send + Sync,
        S::Via: Send + Sync,
        T: 'static + Send + Sync,
    {
        let path = selection.path_info()?;
        let owner_relation = relation_owner_key(path);
        let owner_keys = owners
            .iter()
            .map(|owner| owner.relation_key_value(owner_relation))
            .collect::<crate::Result<Vec<_>>>()?;

        match path {
            RelationPathInfo::Direct { relation } => {
                let related = self
                    .select_target_models::<S>(relation.target_key, owner_keys, &selection)
                    .await?;
                let mut grouped: std::collections::HashMap<String, Vec<S::Target>> =
                    std::collections::HashMap::new();
                for item in related {
                    if let Some(key) = item.column_value(relation.target_key) {
                        grouped
                            .entry(common_helpers::model_value_key(&key))
                            .or_default()
                            .push(item);
                    }
                }

                for owner in owners {
                    let key = owner.relation_key_value(relation)?;
                    let values = grouped
                        .get(&common_helpers::model_value_key(&key))
                        .cloned()
                        .unwrap_or_default();
                    owner.assign_relation(relation.name, values)?;
                }
            }
            RelationPathInfo::Through {
                relation,
                via_relation,
                target_relation,
            } => {
                let via_items = self
                    .select_via_models::<S>(via_relation, owner_keys)
                    .await?;
                let mut target_keys_by_owner: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();

                for item in &via_items {
                    if let (Some(owner_key), Some(target_key)) = (
                        item.column_value(via_relation.target_key),
                        item.column_value(target_relation.local_key),
                    ) {
                        let target_key = common_helpers::model_value_key(&target_key);
                        target_keys_by_owner
                            .entry(common_helpers::model_value_key(&owner_key))
                            .or_default()
                            .push(target_key.clone());
                    }
                }

                let target_key_values = via_items
                    .iter()
                    .filter_map(|item| item.column_value(target_relation.local_key))
                    .collect();
                let related = self
                    .select_target_models::<S>(
                        target_relation.target_key,
                        target_key_values,
                        &selection,
                    )
                    .await?;
                let mut targets_by_key: std::collections::HashMap<String, Vec<S::Target>> =
                    std::collections::HashMap::new();
                for item in &related {
                    if let Some(key) = item.column_value(target_relation.target_key) {
                        targets_by_key
                            .entry(common_helpers::model_value_key(&key))
                            .or_default()
                            .push(item.clone());
                    }
                }

                for owner in owners {
                    let key = owner.relation_key_value(via_relation)?;
                    let key = common_helpers::model_value_key(&key);
                    let values = target_keys_by_owner
                        .get(&key)
                        .map(|target_keys| {
                            target_keys
                                .iter()
                                .flat_map(|target_key| {
                                    targets_by_key.get(target_key).into_iter().flatten()
                                })
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    owner.assign_relation(relation.name, values)?;
                }
            }
        }

        Ok(())
    }

    /// 添加关联表查询
    ///
    /// 与构建器层 `Select::from::<R>()` 保持单泛型形态：关联表类型由 R 指定，
    /// 主表类型就是当前执行器的 T（`select::<User>().from::<Role>()`）。
    pub fn from<R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T: Model + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                RelatedSelectExecutor::Sqlite(exec.from::<R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RelatedSelectExecutor::PostgreSQL(exec.from::<R>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => RelatedSelectExecutor::MySQL(exec.from::<R>()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => RelatedSelectExecutor::MSSQL(exec.from::<R>()),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => RelatedSelectExecutor::DuckDB(exec.from::<R>()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => RelatedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "related (multi-table) select on ClickHouse",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => MultiTableSelectExecutor::Sqlite(
                exec.from3::<R1, R2>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                MultiTableSelectExecutor::PostgreSQL(exec.from3::<R1, R2>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                MultiTableSelectExecutor::MySQL(exec.from3::<R1, R2>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                MultiTableSelectExecutor::MSSQL(exec.from3::<R1, R2>())
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => MultiTableSelectExecutor::DuckDB(
                exec.from3::<R1, R2>(),
                std::marker::PhantomData,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => MultiTableSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "related (multi-table) select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => FourTableSelectExecutor::Sqlite(
                exec.from4::<R1, R2, R3>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                FourTableSelectExecutor::PostgreSQL(exec.from4::<R1, R2, R3>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                FourTableSelectExecutor::MySQL(exec.from4::<R1, R2, R3>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                FourTableSelectExecutor::MSSQL(exec.from4::<R1, R2, R3>())
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => FourTableSelectExecutor::DuckDB(
                exec.from4::<R1, R2, R3>(),
                std::marker::PhantomData,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => FourTableSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "related (multi-table) select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                LeftJoinedSelectExecutor::Sqlite(exec.left_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.left_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => LeftJoinedSelectExecutor::MySQL(exec.left_join::<J>(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => LeftJoinedSelectExecutor::MSSQL(exec.left_join::<J>(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                LeftJoinedSelectExecutor::DuckDB(exec.left_join::<J>(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => LeftJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "LEFT JOIN select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                InnerJoinedSelectExecutor::Sqlite(exec.inner_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                InnerJoinedSelectExecutor::PostgreSQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                InnerJoinedSelectExecutor::MySQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                InnerJoinedSelectExecutor::MSSQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                InnerJoinedSelectExecutor::DuckDB(exec.inner_join::<J>(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => InnerJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "INNER JOIN select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                RightJoinedSelectExecutor::Sqlite(exec.right_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RightJoinedSelectExecutor::PostgreSQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                RightJoinedSelectExecutor::MySQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                RightJoinedSelectExecutor::MSSQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                RightJoinedSelectExecutor::DuckDB(exec.right_join::<J>(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => RightJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "RIGHT JOIN select",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn left_join_derived<J: Model>(
        self,
        derived: DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => LeftJoinedSelectExecutor::Sqlite(
                exec.left_join_derived::<J>(derived, f),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.left_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                LeftJoinedSelectExecutor::MySQL(exec.left_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                LeftJoinedSelectExecutor::MSSQL(exec.left_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                LeftJoinedSelectExecutor::DuckDB(exec.left_join_derived::<J>(derived, f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => LeftJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "JOIN select with a derived table",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn inner_join_derived<J: Model>(
        self,
        derived: DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => InnerJoinedSelectExecutor::Sqlite(
                exec.inner_join_derived::<J>(derived, f),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                InnerJoinedSelectExecutor::PostgreSQL(exec.inner_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                InnerJoinedSelectExecutor::MySQL(exec.inner_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                InnerJoinedSelectExecutor::MSSQL(exec.inner_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                InnerJoinedSelectExecutor::DuckDB(exec.inner_join_derived::<J>(derived, f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => InnerJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "JOIN select with a derived table",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn right_join_derived<J: Model>(
        self,
        derived: DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => RightJoinedSelectExecutor::Sqlite(
                exec.right_join_derived::<J>(derived, f),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RightJoinedSelectExecutor::PostgreSQL(exec.right_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                RightJoinedSelectExecutor::MySQL(exec.right_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                RightJoinedSelectExecutor::MSSQL(exec.right_join_derived::<J>(derived, f))
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                RightJoinedSelectExecutor::DuckDB(exec.right_join_derived::<J>(derived, f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => RightJoinedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "JOIN select with a derived table",
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C>
    where
        T: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => CollectFuture::Sqlite(exec.clone().collect::<C>()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                CollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                CollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                CollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => CollectFuture::DuckDB(exec.clone().collect::<C>()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                CollectFuture::ClickHouse(
                    db.clone(),
                    select.clone(),
                    std::marker::PhantomData,
                )
            }
        }
    }

    /// 执行查询并返回第一条记录
    pub fn first(self) -> FirstFuture<'a, T>
    where
        T: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => FirstFuture::Sqlite(exec.first()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => FirstFuture::PostgreSQL(exec.first()),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => FirstFuture::MySQL(exec.first()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => FirstFuture::MSSQL(exec.first()),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => FirstFuture::DuckDB(exec.first()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => FirstFuture::ClickHouse(db, select),
        }
    }

    /// COUNT 聚合函数。
    ///
    /// 返回行数（`usize`）；关联/多表查询的同谓词行数统计
    /// （`RelatedSelectExecutor::count` 等）在统一层强转为同一 `usize`
    /// 类型，分页场景两个入口可直接混用。
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.count(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.count(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.count(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.count(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => AggregateFuture::DuckDB(exec.count(f)),
            // ClickHouse/InfluxDB 都是聚合引擎：聚合 SELECT 走既有
            // select_values 单列执行路径（InfluxQL 的 AVG 映射为 MEAN）
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                AggregateFuture::ClickHouse(db, select.count(f), std::marker::PhantomData)
            }
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.sum(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.sum(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.sum(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.sum(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => AggregateFuture::DuckDB(exec.sum(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                AggregateFuture::ClickHouse(db, select.sum(f), std::marker::PhantomData)
            }
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.avg(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.avg(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.avg(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.avg(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => AggregateFuture::DuckDB(exec.avg(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                AggregateFuture::ClickHouse(db, select.avg(f), std::marker::PhantomData)
            }
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.max(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.max(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.max(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.max(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => AggregateFuture::DuckDB(exec.max(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                AggregateFuture::ClickHouse(db, select.max(f), std::marker::PhantomData)
            }
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.min(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.min(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.min(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.min(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => AggregateFuture::DuckDB(exec.min(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                AggregateFuture::ClickHouse(db, select.min(f), std::marker::PhantomData)
            }
        }
    }
}

/// 统一的 DeleteExecutor 枚举
pub enum DeleteExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::DeleteExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::DeleteExecutor<T>),
    /// 能力矩阵门控产物：`row_delete: false` 的后端在 [`Database::delete`] /
    /// `PooledConnection::delete` 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
}

crate::impl_unified_delete_executor!(DeleteExecutor);

impl<'a, T: Model> NamedFilterQuery<T> for DeleteExecutor<'a, T> {
    fn apply_named_filter(self, _name: &'static str, expr: WhereExpr) -> Self {
        self.filter(|_| expr)
    }
}

impl<'a, T: Model> super::SqlExecutor for DeleteExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        match self {
            #[cfg(feature = "sqlite")]
            DeleteExecutor::Sqlite(exec, _) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "postgresql")]
            DeleteExecutor::PostgreSQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "mysql")]
            DeleteExecutor::MySQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "mssql")]
            DeleteExecutor::MSSQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "duckdb")]
            DeleteExecutor::DuckDB(exec) => exec.execute_with_sql(sql).await,
            DeleteExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }
}

/// 按块删除结果。
///
/// `blocks_dropped` 尽力而为（回退路径为 0）；`rows_deleted` 仅回退路径
/// （行删除实现）能报告。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockDeleteResult {
    /// 删除的块数。
    pub blocks_dropped: u64,
    /// 影响行数（仅回退路径有值）。
    pub rows_deleted: Option<u64>,
}

impl BlockDeleteResult {
    /// 回退路径结果：仅报告行数。
    pub(crate) fn from_row_count(rows: u64) -> Self {
        Self {
            blocks_dropped: 0,
            rows_deleted: Some(rows),
        }
    }
}

/// 统一的按块删除执行器。
///
/// - `PostgreSQL` 变体同时服务 PostgreSQL/TimescaleDB（`drop_chunks`，无扩展时
///   回退行删除）与 QuestDB（`DROP PARTITION`）。
/// - `ClickHouse` 变体枚举既有分区后合并为一条 `ALTER TABLE ... DROP PARTITION`。
/// - 其余 OLTP 后端走 `Fallback`：按对齐边界执行行删除。
pub enum BlockDeleteExecutor<'a, T: Model> {
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::BlockDeleteExecutor<'a, T>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(clickhouse_backend::BlockDeleteExecutor<'a, T>),
    #[cfg(feature = "influxdb")]
    InfluxDB(influxdb_backend::BlockDeleteExecutor<'a, T>),
    /// OLTP 后端回退路径：`DELETE FROM t WHERE <对齐时间边界>`。
    #[cfg(any(
        feature = "sqlite",
        feature = "mysql",
        feature = "mssql",
        feature = "duckdb"
    ))]
    Fallback {
        db_type: super::super::DbType,
        key: Option<crate::abstract_layer::common::common_helpers::BlockKey>,
        range: Option<crate::abstract_layer::common::common_helpers::BlockRange>,
        delete: DeleteExecutor<'a, T>,
    },
}

crate::impl_unified_block_delete_executor!(BlockDeleteExecutor);

#[cfg(any(
    feature = "sqlite",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb"
))]
impl<'a, T: Model> BlockDeleteExecutor<'a, T> {
    pub(crate) fn fallback(
        db_type: super::super::DbType,
        delete: DeleteExecutor<'a, T>,
    ) -> Self {
        BlockDeleteExecutor::Fallback {
            db_type,
            key: crate::abstract_layer::common::common_helpers::resolve_block_key::<T>(db_type)
                .ok(),
            range: None,
            delete,
        }
    }
}

impl<'a, T: Model> super::SqlExecutor for BlockDeleteExecutor<'a, T> {
    type Output = BlockDeleteResult;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        BlockDeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        match self {
            #[cfg(feature = "postgresql")]
            BlockDeleteExecutor::PostgreSQL(exec) => exec
                .execute_with_sql(sql)
                .await
                .map(BlockDeleteResult::from_row_count),
            #[cfg(feature = "clickhouse")]
            BlockDeleteExecutor::ClickHouse(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "influxdb")]
            BlockDeleteExecutor::InfluxDB(_) => Err(unsupported_feature(
                super::super::DbType::InfluxDB,
                "block delete execute_with_sql (the native backend uses the HTTP delete API)",
            )),
            #[cfg(any(
                feature = "sqlite",
                feature = "mysql",
                feature = "mssql",
                feature = "duckdb"
            ))]
            BlockDeleteExecutor::Fallback { delete, .. } => delete
                .execute_with_sql(sql)
                .await
                .map(BlockDeleteResult::from_row_count),
        }
    }

    async fn execute(self) -> crate::Result<Self::Output> {
        BlockDeleteExecutor::execute(self).await
    }
}

/// 统一的 UpdateExecutor 枚举
pub enum UpdateExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::UpdateExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::UpdateExecutor<T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
}

crate::impl_unified_update_executor!(UpdateExecutor);

impl<'a, T: Model> UpdateExecutor<'a, T> {
    pub(crate) fn set_model_columns(self, model: &T, fields: &[String]) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            UpdateExecutor::Sqlite(exec, phantom) => {
                UpdateExecutor::Sqlite(exec.set_model_fields(model, fields), phantom)
            }
            #[cfg(feature = "postgresql")]
            UpdateExecutor::PostgreSQL(exec) => {
                UpdateExecutor::PostgreSQL(exec.set_model_fields(model, fields))
            }
            #[cfg(feature = "mysql")]
            UpdateExecutor::MySQL(exec) => {
                UpdateExecutor::MySQL(exec.set_model_fields(model, fields))
            }
            #[cfg(feature = "mssql")]
            UpdateExecutor::MSSQL(exec) => {
                UpdateExecutor::MSSQL(exec.set_model_fields(model, fields))
            }
            #[cfg(feature = "duckdb")]
            UpdateExecutor::DuckDB(exec) => {
                UpdateExecutor::DuckDB(exec.set_model_fields(model, fields))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ UpdateExecutor::Unsupported { .. } => unsupported,
        }
    }
}

impl<'a, T: Model> NamedFilterQuery<T> for UpdateExecutor<'a, T> {
    fn apply_named_filter(self, _name: &'static str, expr: WhereExpr) -> Self {
        self.filter(|_| expr)
    }
}

impl<'a, T: Model> super::SqlExecutor for UpdateExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        UpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        match self {
            #[cfg(feature = "sqlite")]
            UpdateExecutor::Sqlite(exec, _) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "postgresql")]
            UpdateExecutor::PostgreSQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "mysql")]
            UpdateExecutor::MySQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "mssql")]
            UpdateExecutor::MSSQL(exec) => exec.execute_with_sql(sql).await,
            #[cfg(feature = "duckdb")]
            UpdateExecutor::DuckDB(exec) => exec.execute_with_sql(sql).await,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            UpdateExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }
}

pub struct ScopedDeleteExecutor<'a, T: Model> {
    pub(crate) inner: DeleteExecutor<'a, T>,
    pub(crate) context_filters: Vec<ContextFilter>,
    pub(crate) disabled_filters: Vec<&'static str>,
}

pub struct ScopedUpdateExecutor<'a, T: Model> {
    pub(crate) inner: UpdateExecutor<'a, T>,
    pub(crate) context_filters: Vec<ContextFilter>,
    pub(crate) disabled_filters: Vec<&'static str>,
}

fn scoped_filter_exprs<T: Model>(
    context_filters: &[ContextFilter],
    disabled_filters: &[&'static str],
) -> Vec<FilterExpr> {
    context_filters
        .iter()
        .filter(|filter| !disabled_filters.iter().any(|name| *name == filter.name()))
        .filter_map(ContextFilter::filter_for::<T>)
        .collect()
}

fn append_scoped_filters<T: Model>(
    statement: &mut SqlStatement,
    context_filters: &[ContextFilter],
    disabled_filters: &[&'static str],
) -> crate::Result<()> {
    let filters = scoped_filter_exprs::<T>(context_filters, disabled_filters);
    if filters.is_empty() {
        return Ok(());
    }

    for single in &mut statement.statements {
        // scope 条件的参数占位符追加在既有参数之后，与占位符编号保持一致
        let mut scope_sql = String::new();
        let mut param_idx = single.params.len() + 1;
        for (index, filter) in filters.iter().enumerate() {
            if index > 0 {
                scope_sql.push_str(" AND ");
            }
            common_helpers::format_filter_with_params(
                filter,
                &mut scope_sql,
                &mut param_idx,
                &mut single.params,
                statement.db_type,
            )?;
        }

        match find_toplevel_where(&single.sql) {
            // 已有 WHERE：把既有条件整体加括号再 AND，防止
            // "WHERE pk = ? OR pk = ?" 这类 OR 分支绕过租户过滤
            Some(insert_at) => {
                single.sql.insert_str(insert_at, "(");
                single.sql.push_str(") AND ");
                single.sql.push_str(&scope_sql);
            }
            None => {
                single.sql.push_str(" WHERE ");
                single.sql.push_str(&scope_sql);
            }
        }
    }

    Ok(())
}

/// 定位语句中顶层 WHERE 子句的条件起始位置（"WHERE" 后的偏移）。
///
/// 扫描时跳过单引号字符串字面量与括号内的子查询，避免把
/// `SET x = (SELECT ... WHERE ...)` 或含 "WHERE" 的字面量误认为顶层 WHERE。
fn find_toplevel_where(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_string = false;
            }
        } else {
            match b {
                b'\'' => in_string = true,
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'W' if depth == 0 => {
                    let is_word_start = i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\n';
                    if is_word_start && sql[i..].starts_with("WHERE") {
                        let mut j = i + "WHERE".len();
                        while j < bytes.len() && bytes[j] == b' ' {
                            j += 1;
                        }
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

impl<'a, T: Model> ScopedDeleteExecutor<'a, T> {
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        self.inner = self.inner.filter(f);
        self
    }

    pub fn model(mut self, model: &T) -> Self {
        self.inner = self.inner.model(model);
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let mut statement = self.inner.to_sql()?;
        append_scoped_filters::<T>(
            &mut statement,
            &self.context_filters,
            &self.disabled_filters,
        )?;
        Ok(statement)
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as super::SqlExecutor>::execute(self).await
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }

    /// 同义词，等价于 [`Self::execute`]。
    #[deprecated(since = "0.2.11", note = "use `execute()` instead")]
    pub async fn exec(self) -> crate::Result<u64> {
        self.execute().await
    }
}

impl<'a, T: Model> NamedFilterQuery<T> for ScopedDeleteExecutor<'a, T> {
    fn apply_named_filter(self, _name: &'static str, expr: WhereExpr) -> Self {
        self.filter(|_| expr)
    }
}

impl<'a, T: Model> WithoutFilterQuery<T> for ScopedDeleteExecutor<'a, T> {
    fn without_filter(mut self, name: &'static str) -> Self {
        if !self.disabled_filters.iter().any(|item| *item == name) {
            self.disabled_filters.push(name);
        }
        self
    }
}

impl<'a, T: Model> super::SqlExecutor for ScopedDeleteExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        ScopedDeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        self.inner.execute_with_sql(sql).await
    }
}

impl<'a, T: Model> ScopedUpdateExecutor<'a, T> {
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        self.inner = self.inner.filter(f);
        self
    }

    pub fn set<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut T::Update),
    {
        self.inner = self.inner.set(f);
        self
    }

    pub fn set_model<I: crate::model::Insertable<Model = T>>(mut self, models: I) -> Self {
        self.inner = self.inner.set_model(models);
        self
    }

    pub fn set_model_fields<I, F, M>(mut self, models: I, fields_fn: F) -> Self
    where
        I: crate::model::Insertable<Model = T>,
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        self.inner = self.inner.set_model_fields(models, fields_fn);
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let mut statement = self.inner.to_sql()?;
        append_scoped_filters::<T>(
            &mut statement,
            &self.context_filters,
            &self.disabled_filters,
        )?;
        Ok(statement)
    }

    pub async fn execute(self) -> crate::Result<u64> {
        <Self as super::SqlExecutor>::execute(self).await
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

impl<'a, T: Model> NamedFilterQuery<T> for ScopedUpdateExecutor<'a, T> {
    fn apply_named_filter(self, _name: &'static str, expr: WhereExpr) -> Self {
        self.filter(|_| expr)
    }
}

impl<'a, T: Model> WithoutFilterQuery<T> for ScopedUpdateExecutor<'a, T> {
    fn without_filter(mut self, name: &'static str) -> Self {
        if !self.disabled_filters.iter().any(|item| *item == name) {
            self.disabled_filters.push(name);
        }
        self
    }
}

impl<'a, T: Model> super::SqlExecutor for ScopedUpdateExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        ScopedUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        self.inner.execute_with_sql(sql).await
    }
}

/// 统一的 CollectFuture 枚举
pub enum CollectFuture<'a, T: Model, C: FromIterator<T>> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::CollectFuture<'a, T, C>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(
        ClickHouseSelectBackend<'a>,
        crate::query::builder::Select<T>,
        std::marker::PhantomData<C>,
    ),
}

/// 统一的 FirstFuture 枚举
pub enum FirstFuture<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::FirstFuture<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::FirstFuture<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::FirstFuture<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::FirstFuture<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::FirstFuture<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(
        ClickHouseSelectBackend<'a>,
        crate::query::builder::Select<T>,
    ),
}

/// 统一的 AggregateFuture 枚举
pub enum AggregateFuture<'a, T: Model, R> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::AggregateFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::AggregateFuture<T, R>),
    /// ClickHouse/InfluxDB 的聚合 SELECT：渲染为单列聚合查询后走
    /// select_values/raw_select_values 执行路径。
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(
        ClickHouseSelectBackend<'a>,
        crate::query::builder::AggregateSelect<T, R>,
        std::marker::PhantomData<&'a R>,
    ),
}

crate::impl_unified_aggregate_future!(AggregateFuture);

/// 统一层"主表 + N 个关联表"执行器家族（RelatedSelectExecutor /
/// MultiTableSelectExecutor / FourTableSelectExecutor）的公共生成宏：
/// 统一执行器枚举、同谓词 COUNT Future 枚举、Future 的 IntoFuture 委托与
/// `count()` 入口一次生成，三个调用点只差类型名与关联表参数个数。
///
/// DuckDB 变体存在历史形态差异：RelatedSelectExecutor 为单字段，
/// Multi/Four 为 `(executor, PhantomData)` 双字段——通过可选的
/// `duckdb_extra` 注入额外字段以保持各自公开形状不变；`count()` 的
/// 解构使用剩余模式 `(exec, ..)` 同时兼容两种形态（PhantomData 为零
/// 大小，丢弃后重建语义等价）。
///
/// "主表行收集"Future 与 `collect()` 入口由
/// [`impl_unified_multi_table_collect`] 单独生成：宏转录中 `$($r),+`
/// 无法嵌套进可选段 `$(...)?`（可选段只有 1 个绑定，`+` 段有 N 个，
/// 转录深度无法对齐），拆成独立宏后各自在顶层重绑关联表参数。
/// Related 的 collect 由 `impl_unified_related_select_executor!` 生成，
/// 不使用本宏族。
macro_rules! impl_unified_multi_table_select_family {
    (
        $exec:ident, $future:ident,
        $exec_doc:literal, $future_doc:literal, $count_doc:literal,
        ($($r:ident),+)
        $(, duckdb_extra { $($duck_extra:tt)* })?
    ) => {
        #[doc = $exec_doc]
        pub enum $exec<'a, T: Model, $($r: Model),+> {
            #[cfg(feature = "sqlite")]
            Sqlite(
                sqlite_backend::$exec<T, $($r),+>,
                std::marker::PhantomData<&'a ()>,
            ),
            #[cfg(feature = "postgresql")]
            PostgreSQL(postgresql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mysql")]
            MySQL(mysql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mssql")]
            MSSQL(mssql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "duckdb")]
            DuckDB(
                duckdb_backend::$exec<T, $($r),+>
                $(, $($duck_extra)*)?
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            #[doc(hidden)]
            Unsupported {
                backend: super::super::DbType,
                feature: &'static str,
                _marker: std::marker::PhantomData<&'a (T, $($r),+)>,
            },
        }

        #[doc = $future_doc]
        pub enum $future<'a, T: Model, $($r: Model),+> {
            #[cfg(feature = "sqlite")]
            Sqlite(
                sqlite_backend::$exec<T, $($r),+>,
                std::marker::PhantomData<&'a ()>,
            ),
            #[cfg(feature = "postgresql")]
            PostgreSQL(postgresql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mysql")]
            MySQL(mysql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mssql")]
            MSSQL(mssql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "duckdb")]
            DuckDB(
                duckdb_backend::$exec<T, $($r),+>,
                std::marker::PhantomData<&'a ()>,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            #[doc(hidden)]
            Unsupported {
                backend: super::super::DbType,
                feature: &'static str,
                _marker: std::marker::PhantomData<&'a (T, $($r),+)>,
            },
        }

        crate::impl_unified_related_count_future!(
            $future,
            count,
            [
                'a,
                T: crate::Model + 'static + std::marker::Send + std::marker::Sync,
                $($r: crate::Model + 'static + std::marker::Send + std::marker::Sync),+
            ],
            ['a, T, $($r),+]
        );

        impl<'a, T: Model + 'static, $($r: Model + 'static),+> $exec<'a, T, $($r),+> {
            #[doc = $count_doc]
            pub fn count(self) -> $future<'a, T, $($r),+> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $exec::Sqlite(exec, phantom) => $future::Sqlite(exec, phantom),
                    #[cfg(feature = "postgresql")]
                    $exec::PostgreSQL(exec) => $future::PostgreSQL(exec),
                    #[cfg(feature = "mysql")]
                    $exec::MySQL(exec) => $future::MySQL(exec),
                    #[cfg(feature = "mssql")]
                    $exec::MSSQL(exec) => $future::MSSQL(exec),
                    #[cfg(feature = "duckdb")]
                    $exec::DuckDB(exec, ..) => {
                        $future::DuckDB(exec, std::marker::PhantomData)
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $exec::Unsupported {
                        backend,
                        feature,
                        ..
                    } => $future::Unsupported {
                        backend,
                        feature,
                        _marker: std::marker::PhantomData,
                    },
                }
            }
        }
    };
}

/// "主表行收集"生成宏（L18：from3/from4 此前只能 count、无法取回行数据；
/// SQL 只选择主表列，解码路径与 related collect 相同）：生成 `collect()`
/// 方法、收集 Future 枚举及其 IntoFuture 委托。与
/// [`impl_unified_multi_table_select_family`] 分离定义的原因见其文档。
macro_rules! impl_unified_multi_table_collect {
    (
        $exec:ident, $collect_future:ident,
        $method_doc:literal, $future_doc:literal,
        ($($r:ident),+)
    ) => {
        impl<'a, T: Model + 'static, $($r: Model + 'static),+> $exec<'a, T, $($r),+> {
            #[doc = $method_doc]
            pub fn collect(self) -> $collect_future<'a, T, $($r),+> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $exec::Sqlite(exec, phantom) => {
                        $collect_future::Sqlite(exec, phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $exec::PostgreSQL(exec) => $collect_future::PostgreSQL(exec),
                    #[cfg(feature = "mysql")]
                    $exec::MySQL(exec) => $collect_future::MySQL(exec),
                    #[cfg(feature = "mssql")]
                    $exec::MSSQL(exec) => $collect_future::MSSQL(exec),
                    #[cfg(feature = "duckdb")]
                    $exec::DuckDB(exec, ..) => {
                        $collect_future::DuckDB(exec, std::marker::PhantomData)
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $exec::Unsupported {
                        backend,
                        feature,
                        ..
                    } => $collect_future::Unsupported {
                        backend,
                        feature,
                        _marker: std::marker::PhantomData,
                    },
                }
            }
        }

        #[doc = $future_doc]
        pub enum $collect_future<'a, T: Model, $($r: Model),+> {
            #[cfg(feature = "sqlite")]
            Sqlite(
                sqlite_backend::$exec<T, $($r),+>,
                std::marker::PhantomData<&'a ()>,
            ),
            #[cfg(feature = "postgresql")]
            PostgreSQL(postgresql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mysql")]
            MySQL(mysql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "mssql")]
            MSSQL(mssql_backend::$exec<'a, T, $($r),+>),
            #[cfg(feature = "duckdb")]
            DuckDB(
                duckdb_backend::$exec<T, $($r),+>,
                std::marker::PhantomData<&'a ()>,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            #[doc(hidden)]
            Unsupported {
                backend: super::super::DbType,
                feature: &'static str,
                _marker: std::marker::PhantomData<&'a (T, $($r),+)>,
            },
        }

        impl<
            'a,
            T: crate::Model + 'static + std::marker::Send + std::marker::Sync,
            $($r: crate::Model + 'static + std::marker::Send + std::marker::Sync),+
        > std::future::IntoFuture for $collect_future<'a, T, $($r),+>
        where
            Self: 'a,
        {
            type Output = crate::Result<Vec<T>>;
            type IntoFuture = std::pin::Pin<
                Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>,
            >;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $collect_future::Sqlite(exec, _) => {
                        Box::pin(async move { exec.collect_rows().await })
                    }
                    #[cfg(feature = "postgresql")]
                    $collect_future::PostgreSQL(exec) => {
                        Box::pin(async move { exec.collect_rows().await })
                    }
                    #[cfg(feature = "mysql")]
                    $collect_future::MySQL(exec) => {
                        Box::pin(async move { exec.collect_rows().await })
                    }
                    #[cfg(feature = "mssql")]
                    $collect_future::MSSQL(exec) => {
                        Box::pin(async move { exec.collect_rows().await })
                    }
                    #[cfg(feature = "duckdb")]
                    $collect_future::DuckDB(exec, _) => {
                        Box::pin(async move { exec.collect_rows().await })
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $collect_future::Unsupported {
                        backend,
                        feature,
                        ..
                    } => Box::pin(async move {
                        Err(unsupported_feature(backend, feature))
                    }),
                }
            }
        }
    };
}

impl_unified_multi_table_select_family!(
    RelatedSelectExecutor,
    RelatedCountFuture,
    "统一的 RelatedSelectExecutor 枚举",
    "统一的关联查询同谓词行数统计 Future（page.md 缺失一：分页 total_count 场景）。\n\n结果为与原子查询同谓词的 `SELECT COUNT(*)`，不受 range()/order_by() 影响。",
    "统计同谓词总行数（列表分页 total_count 用）：\n生成 `SELECT COUNT(*) FROM (<原子查询>)`，不受 range()/order_by() 影响。\n\n返回 `usize`（统一层把后端的 `i64` 行数强转），与\n[`SelectExecutor::count`] 的返回类型一致。",
    (R)
);

impl_unified_multi_table_select_family!(
    MultiTableSelectExecutor,
    MultiTableCountFuture,
    "统一的 MultiTableSelectExecutor 枚举",
    "统一的三表关联查询同谓词行数统计 Future。",
    "统计同谓词总行数（列表分页 total_count 用）。\n\n返回 `usize`（统一层把后端的 `i64` 行数强转），与\n[`SelectExecutor::count`] 的返回类型一致。",
    (R1, R2),
    duckdb_extra { std::marker::PhantomData<&'a ()> }
);

impl_unified_multi_table_collect!(
    MultiTableSelectExecutor,
    MultiTableCollectFuture,
    "执行查询并收集主表行（L18）：多表关联 SQL 只选择主表列，\n返回 `Vec<T>`；关联表仅用于过滤。",
    "统一的三表关联查询主表行收集 Future（L18：from3 此前只能 count）。\n\nSQL 只选择主表列，返回 `Vec<T>`。",
    (R1, R2)
);

impl_unified_multi_table_select_family!(
    FourTableSelectExecutor,
    FourTableCountFuture,
    "统一的 FourTableSelectExecutor 枚举",
    "统一的四表关联查询同谓词行数统计 Future。",
    "统计同谓词总行数（列表分页 total_count 用）。\n\n返回 `usize`（统一层把后端的 `i64` 行数强转），与\n[`SelectExecutor::count`] 的返回类型一致。",
    (R1, R2, R3),
    duckdb_extra { std::marker::PhantomData<&'a ()> }
);

impl_unified_multi_table_collect!(
    FourTableSelectExecutor,
    FourTableCollectFuture,
    "执行查询并收集主表行（L18）：多表关联 SQL 只选择主表列，\n返回 `Vec<T>`；关联表仅用于过滤。",
    "统一的四表关联查询主表行收集 Future（L18：from4 此前只能 count）。\n\nSQL 只选择主表列，返回 `Vec<T>`。",
    (R1, R2, R3)
);

/// 统一的 InnerJoinedSelectExecutor 枚举
pub enum InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::InnerJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InnerJoinedSelectExecutor<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

/// 统一的 RightJoinedSelectExecutor 枚举
pub enum RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RightJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::RightJoinedSelectExecutor<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

/// 统一的 LeftJoinedSelectExecutor 枚举
pub enum LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::LeftJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::LeftJoinedSelectExecutor<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

/// 统一的 LeftJoinCollectFuture 枚举
pub enum LeftJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::LeftJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::LeftJoinCollectFuture<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

/// 统一的 InnerJoinCollectFuture 枚举
pub enum InnerJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::InnerJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InnerJoinCollectFuture<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

/// 统一的 RightJoinCollectFuture 枚举
pub enum RightJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RightJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::RightJoinCollectFuture<T, J>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, J)>,
    },
}

crate::impl_unified_collect_future!(CollectFuture);

impl<'a, T: Model + 'static + std::marker::Send + std::marker::Sync> std::future::IntoFuture
    for FirstFuture<'a, T>
{
    type Output = crate::Result<Option<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            FirstFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            FirstFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            FirstFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            FirstFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "duckdb")]
            FirstFuture::DuckDB(future) => Box::pin(future.into_future()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            FirstFuture::ClickHouse(db, select) => {
                Box::pin(async move { clickhouse_select_first_on_backend(db, select).await })
            }
        }
    }
}

crate::impl_unified_related_select_executor!(RelatedSelectExecutor);

/// 统一的 RelatedCollectFuture 枚举
pub enum RelatedCollectFuture<'a, T: Model, R: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RelatedCollectFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::RelatedCollectFuture<T, R>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R)>,
    },
}

crate::impl_unified_related_collect_future!(RelatedCollectFuture);

/// 统一的 Transaction 枚举
pub enum Transaction<'a> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Transaction),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Transaction<'a>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Transaction<'a>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Transaction<'a>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::Transaction),
    // 哨兵变体：仅在无携带生命周期的后端变体（如仅启用 sqlite/duckdb/clickhouse/
    // influxdb）时锚定 `'a`，使枚举在所有 feature 组合下可编译。
    // 携带 [`std::convert::Infallible`]（无任何值）保证该变体在安全 Rust 中
    // 无法被构造——外部代码与内部代码均构造不出；match 分支对它做
    // void 消除（`match *infallible {}`）而非 `unreachable!()` panic。
    #[doc(hidden)]
    _Phantom(std::convert::Infallible, std::marker::PhantomData<&'a ()>),
}

/// 旧事务保存执行器名过渡别名（已合并为 [`SaveExecutor`]，保留 re-export
/// 以兼容现有导入路径；事务入口 [`Transaction::save`] 返回的就是
/// [`SaveExecutor`] 的事务变体，`'tx` 参数仅为兼容旧签名保留）。
#[deprecated(
    since = "0.2.12",
    note = "TransactionSaveExecutor 已合并为 SaveExecutor，请改用 SaveExecutor"
)]
pub type TransactionSaveExecutor<'a, 'tx, T> = SaveExecutor<'a, T>;

/// Save 执行器的共享核心：dirty 列更新 + 图关系同步，写路径在给定事务上执行。
/// [`SaveExecutor`] 的自开事务路径与外部事务路径共用本函数，图保存语义只维护一份。
async fn save_dirty_columns_and_relations<T>(
    txn: &mut Transaction<'_>,
    model: &mut Tracked<T>,
) -> crate::Result<u64>
where
    T: WritableModel + crate::model::Model + crate::model::GraphWritable,
{
    let fields = model.dirty_columns();
    let mut affected = 0u64;
    if !fields.is_empty() {
        affected += txn
            .update::<T>()
            .set_model_columns(model.as_model(), &fields)
            .execute()
            .await?;
    }
    affected += model.sync_graph_relations(txn).await?;
    Ok(affected)
}

/// 池连接 save 的写核心（L24）：在池连接上自开事务执行
/// [`save_dirty_columns_and_relations`]，结束时提交/回滚（回滚失败走统一日志），
/// 与 [`Database::save`] 的自开事务路径语义一致。
async fn save_on_pooled_conn<T>(
    conn: &connection_pool::PooledConnection<'_>,
    model: &mut Tracked<T>,
) -> crate::Result<u64>
where
    T: WritableModel + crate::model::Model + crate::model::GraphWritable,
{
    let mut tx = conn.begin().await?;
    match save_dirty_columns_and_relations(&mut tx, model).await {
        Ok(affected) => {
            tx.commit().await?;
            Ok(affected)
        }
        Err(err) => {
            log_rollback_failure(tx.rollback().await);
            Err(err)
        }
    }
}

impl<'a, T: WritableModel + crate::model::GraphWritable> SaveExecutor<'a, T> {
    /// 池连接路径构造（L24）：`PooledConnection::save` 委托此处。
    ///
    /// SQL 预渲染与启动闭包擦除同事务路径（池连接的内层生命周期 `'pool`
    /// 与外层借用 `'a` 不同，同样需要擦除）；写核心自开事务并提交/回滚，
    /// 语义与 [`Database::save`] 一致。
    pub(crate) fn from_pooled<'pool>(
        conn: &'a connection_pool::PooledConnection<'pool>,
        model: &'a mut Tracked<T>,
    ) -> Self
    where
        'pool: 'a,
    {
        // SQL 预览在构造时渲染（dirty 列在执行器存续期被独占借用，不可再变）
        let sql = {
            let fields = model.dirty_columns();
            if fields.is_empty() {
                Ok(SqlStatement::batch(conn.db_type(), Vec::new()))
            } else {
                conn.update::<T>()
                    .set_model_columns(model.as_model(), &fields)
                    .to_sql()
            }
        };
        SaveExecutor {
            conn: SaveConn::Pooled {
                sql,
                run: Box::new(
                    move |model: &'a mut Tracked<T>| -> Pin<
                        Box<
                            dyn Future<
                                    Output = (crate::Result<u64>, &'a mut Tracked<T>),
                                > + 'a,
                        >,
                    > {
                        Box::pin(async move {
                            let result = save_on_pooled_conn(conn, model).await;
                            (result, model)
                        })
                    },
                ),
            },
            model,
        }
    }
}

#[cfg(any(feature = "postgresql", feature = "mysql", feature = "mssql"))]
pub(crate) fn isolation_level_sql(isolation: IsolationLevel) -> &'static str {
    match isolation {
        IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
        IsolationLevel::ReadCommitted => "READ COMMITTED",
        IsolationLevel::RepeatableRead => "REPEATABLE READ",
        IsolationLevel::Serializable => "SERIALIZABLE",
    }
}

/// 在事务开始后应用事务选项。
///
/// 可达路径仅为 SQLite/PostgreSQL/MSSQL/DuckDB（`begin_opts` 把 MySQL 固定
/// 路由到 `begin_with_opts`（选项必须在 BEGIN 前下发），QuestDB/ClickHouse/
/// InfluxDB 在 `begin()` 即被 `transactions: false` 拦截），其余后端在此
/// 防御性拒绝，避免矩阵漂移时静默忽略选项或在 BEGIN 后错误地下发
/// `SET TRANSACTION`。
#[cfg_attr(
    not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mssql",
        feature = "duckdb"
    )),
    allow(unused_variables)
)]
pub(crate) async fn apply_transaction_options(
    txn: &mut Transaction<'_>,
    options: TransactionOptions,
) -> crate::Result<()> {
    match txn.db_type() {
        #[cfg(feature = "sqlite")]
        super::super::DbType::Sqlite => {
            if options.isolation.is_some() || options.read_only {
                return Err(unsupported_feature(
                    super::super::DbType::Sqlite,
                    "transaction options on SQLite",
                ));
            }
            Ok(())
        }
        #[cfg(feature = "postgresql")]
        super::super::DbType::PostgreSQL => {
            if let Some(isolation) = options.isolation {
                txn.execute_sql(format!(
                    "SET TRANSACTION ISOLATION LEVEL {}",
                    isolation_level_sql(isolation)
                ))
                .await?;
            }
            if options.read_only {
                txn.execute_sql("SET TRANSACTION READ ONLY").await?;
            }
            Ok(())
        }
        #[cfg(feature = "mssql")]
        super::super::DbType::MSSQL => {
            if let Some(isolation) = options.isolation {
                txn.execute_sql(format!(
                    "SET TRANSACTION ISOLATION LEVEL {}",
                    isolation_level_sql(isolation)
                ))
                .await?;
            }
            if options.read_only {
                return Err(unsupported_feature(
                    super::super::DbType::MSSQL,
                    "read-only transactions",
                ));
            }
            Ok(())
        }
        #[cfg(feature = "duckdb")]
        super::super::DbType::DuckDB => {
            if options.isolation.is_some() || options.read_only {
                return Err(unsupported_feature(
                    super::super::DbType::DuckDB,
                    "transaction options on DuckDB",
                ));
            }
            Ok(())
        }
        // MySQL/QuestDB/ClickHouse/InfluxDB 正常不可达（见函数文档）。
        #[allow(unreachable_patterns)]
        _ => Err(unsupported_feature(txn.db_type(), "transaction options")),
    }
}

/// 事务内的作用域查询入口：与 [`DatabaseScope`] 一致地自动附加
/// context filters（多租户/软删除等）。写路径同样受到保护，
/// 避免事务内的更新/删除绕过租户隔离。
pub struct TransactionScope<'a, 'tx> {
    txn: &'tx Transaction<'a>,
    context_filters: Vec<ContextFilter>,
}

impl<'a, 'tx> TransactionScope<'a, 'tx> {
    /// 附加 context filter（如租户条件），对后续经此作用域发起的查询生效。
    pub fn with_context_filter<T: Model>(mut self, name: &'static str, expr: WhereExpr) -> Self {
        self.context_filters
            .push(ContextFilter::new::<T>(name, expr));
        self
    }

    /// 撤销同名 context filter（scope 级或查询级命名过滤器）。
    pub fn without_filter<T: Model>(mut self, name: &'static str) -> Self {
        self.context_filters
            .retain(|filter| filter.name() != name);
        self
    }

    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        self.txn
            .select::<T>()
            .with_context_filters(self.context_filters.clone())
    }

    pub fn delete<T: WritableModel>(&self) -> ScopedDeleteExecutor<'_, T> {
        ScopedDeleteExecutor {
            inner: self.txn.delete::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }

    pub fn update<T: WritableModel>(&self) -> ScopedUpdateExecutor<'_, T> {
        ScopedUpdateExecutor {
            inner: self.txn.update::<T>(),
            context_filters: self.context_filters.clone(),
            disabled_filters: Vec::new(),
        }
    }

    pub async fn find_by_id<T: Model + 'static + Send + Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> crate::Result<Option<T>> {
        find_by_id_with_executor(self.select::<T>(), key).await
    }
}

impl<'a> Transaction<'a> {
    /// 事务内的作用域查询入口（对齐 [`Database::scope`]）。
    pub fn scope<'tx>(&'tx self) -> TransactionScope<'a, 'tx> {
        TransactionScope {
            txn: self,
            context_filters: Vec::new(),
        }
    }

    pub fn db_type(&self) -> super::super::DbType {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(_) => super::super::DbType::Sqlite,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(_) => super::super::DbType::PostgreSQL,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(_) => super::super::DbType::MySQL,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(_) => super::super::DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(_) => super::super::DbType::DuckDB,
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub fn select_sql<T>(&mut self, sql: impl IntoRawSql) -> RawSelectExecutor<'_, T> {
        // 共享引用 + Transaction 协变性：`&mut Transaction<'a>` 在此降级为
        // `&Transaction<'_>`，与 Database / 池路径共用一份 RawSelectExecutor。
        RawSelectExecutor::from_conn(ConnRef::Txn(self), sql.into_raw_sql())
    }

    /// 执行原生非查询 SQL 并返回影响的行数。
    ///
    /// 注意：事务后端不含 ClickHouse/InfluxDB（`transactions: false`），
    /// 本方法返回值在各可达后端上均有精确行数语义。
    pub async fn execute_sql(&mut self, sql: impl IntoRawSql) -> crate::Result<u64> {
        // L21：Database / 事务 / 池连接三路径共用一份后端分派。
        // 写路径：后端事务 exec_raw 为 &mut 接收器，走 ConnRefMut（见
        // select_sql 的读路径注释；Transaction 协变性允许 &mut 收缩）。
        exec_raw_sql_on(ConnRefMut::Txn(self), sql.into_raw_sql()).await
    }

    /// 在事务内执行一段受 SAVEPOINT 保护的闭包，失败时回滚到 SAVEPOINT。
    ///
    /// 以 [`Capabilities::savepoints`] 为准：DuckDB 不支持 SAVEPOINT 语法，
    /// 在此直接返回 `UnsupportedFeature` 而不是透传驱动层 Parser 错误。
    pub async fn savepoint<R, F>(&mut self, f: F) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'a>) -> TransactionFuture<'tx, R>,
    {
        Capabilities::ensure(self.db_type(), |caps| caps.savepoints, "savepoints")?;
        let name = format!(
            "__ormer_savepoint_{}",
            SAVEPOINT_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        #[cfg(feature = "mssql")]
        let is_mssql = matches!(self.db_type(), super::super::DbType::MSSQL);
        #[cfg(not(feature = "mssql"))]
        let is_mssql = false;

        if is_mssql {
            self.execute_sql(format!("SAVE TRANSACTION {name}")).await?;
        } else {
            self.execute_sql(format!("SAVEPOINT {name}")).await?;
        }

        match f(self).await {
            Ok(value) => {
                if !is_mssql {
                    self.execute_sql(format!("RELEASE SAVEPOINT {name}"))
                        .await?;
                }
                Ok(value)
            }
            Err(err) => {
                // 回滚到 SAVEPOINT / RELEASE 的失败不覆盖主错误，但不能
                // 静默吞掉（L23：统一走回滚失败日志）。
                if is_mssql {
                    log_rollback_failure(
                        self.execute_sql(format!("ROLLBACK TRANSACTION {name}")).await,
                    );
                } else {
                    log_rollback_failure(
                        self.execute_sql(format!("ROLLBACK TO SAVEPOINT {name}")).await,
                    );
                    log_rollback_failure(
                        self.execute_sql(format!("RELEASE SAVEPOINT {name}")).await,
                    );
                }
                Err(err)
            }
        }
    }

    /// 提交事务
    pub async fn commit(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => txn.commit().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.commit().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.commit().await,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => txn.commit().await,
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => txn.commit().await,
            Transaction::_Phantom(infallible, _) => match infallible {},
        }
    }

    /// 回滚事务
    pub async fn rollback(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => txn.rollback().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.rollback().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.rollback().await,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => txn.rollback().await,
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => txn.rollback().await,
            Transaction::_Phantom(infallible, _) => match infallible {},
        }
    }

    /// 关闭并回滚事务
    pub async fn close(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => txn.close().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.close().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.close().await,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => txn.close().await,
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => txn.close().await,
            Transaction::_Phantom(infallible, _) => match infallible {},
        }
    }

    /// 根据主键查找单条记录（事务中）
    pub async fn find_by_id<T: Model + 'static + std::marker::Send + std::marker::Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> crate::Result<Option<T>> {
        find_by_id_with_executor(self.select::<T>(), key).await
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => SelectExecutor::Sqlite(txn.select::<T>()),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => SelectExecutor::PostgreSQL(txn.select::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => SelectExecutor::MySQL(txn.select::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => SelectExecutor::MSSQL(txn.select::<T>()),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => SelectExecutor::DuckDB(txn.select::<T>()),
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub fn batch<'b, B>(&'b self, batch: B) -> BatchFuture<'b, B>
    where
        B: BatchQueries<'b>,
    {
        BatchFuture::new(batch)
    }

    pub fn batch_many<'b, I, Q>(&'b self, queries: I) -> BatchManyFuture<'b, Q>
    where
        I: IntoIterator<Item = Q>,
        Q: BatchQuery<'b>,
    {
        BatchManyFuture::new(queries)
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                ProjectionSelectExecutor::Sqlite(txn.select_column::<T, V>())
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                ProjectionSelectExecutor::PostgreSQL(txn.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => ProjectionSelectExecutor::MySQL(txn.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => ProjectionSelectExecutor::MSSQL(txn.select_column::<T, V>()),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                ProjectionSelectExecutor::DuckDB(txn.select_column::<T, V>())
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                DeleteExecutor::Sqlite(txn.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => DeleteExecutor::PostgreSQL(txn.delete::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => DeleteExecutor::MySQL(txn.delete::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => DeleteExecutor::MSSQL(txn.delete::<T>()),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => DeleteExecutor::DuckDB(txn.delete::<T>()),
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                UpdateExecutor::Sqlite(txn.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => UpdateExecutor::PostgreSQL(txn.update::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => UpdateExecutor::MySQL(txn.update::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => UpdateExecutor::MSSQL(txn.update::<T>()),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => UpdateExecutor::DuckDB(txn.update::<T>()),
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    /// 保存 tracked 模型的 dirty 变更（在当前事务上执行，写完不提交不回滚）。
    ///
    /// 返回合并后的 [`SaveExecutor`] 事务路径（旧名 [`TransactionSaveExecutor`]
    /// 保留为过渡别名）。
    pub fn save<'op, T: WritableModel + crate::model::GraphWritable>(
        &'op mut self,
        model: &'op mut Tracked<T>,
    ) -> SaveExecutor<'op, T> {
        // SQL 预览在构造时渲染（dirty 列在执行器存续期被独占借用，不可再变）
        let sql = {
            let fields = model.dirty_columns();
            if fields.is_empty() {
                Ok(SqlStatement::batch(self.db_type(), Vec::new()))
            } else {
                self.update::<T>()
                    .set_model_columns(model.as_model(), &fields)
                    .to_sql()
            }
        };
        // 写核心启动闭包：`&'op mut Transaction<'a>` 在此装箱擦除 `'a`，
        // SaveExecutor 的公开形状保持单生命周期
        let txn = self;
        SaveExecutor {
            conn: SaveConn::Txn {
                sql,
                run: Box::new(
                    move |model: &'op mut Tracked<T>| -> Pin<
                        Box<
                            dyn Future<
                                    Output = (crate::Result<u64>, &'op mut Tracked<T>),
                                > + 'op,
                        >,
                    > {
                        Box::pin(async move {
                            let result = save_dirty_columns_and_relations(txn, model).await;
                            (result, model)
                        })
                    },
                ),
            },
            model,
        }
    }

    /// 插入记录 - 返回执行器（合并后返回 [`InsertExecutor`] 的事务变体，
    /// 旧名 [`TransactionInsertExecutor`] 保留为过渡别名）。
    pub fn insert<I: crate::model::Insertable>(&mut self, models: I) -> InsertExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => InsertExecutor::SqliteTxn(txn.insert::<I>(models)),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                InsertExecutor::PostgreSQLTxn(txn.insert::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => InsertExecutor::MySQLTxn(txn.insert::<I>(models)),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => InsertExecutor::MSSQLTxn(txn.insert::<I>(models)),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => InsertExecutor::DuckDBTxn(txn.insert::<I>(models)),
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    /// 插入或更新记录 - 返回执行器（合并后返回 [`InsertOrUpdateExecutor`]
    /// 的事务变体，旧名 [`TransactionInsertOrUpdateExecutor`] 保留为过渡别名）。
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                InsertOrUpdateExecutor::SqliteTxn(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                InsertOrUpdateExecutor::PostgreSQLTxn(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                InsertOrUpdateExecutor::MySQLTxn(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                InsertOrUpdateExecutor::MSSQLTxn(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                InsertOrUpdateExecutor::DuckDBTxn(txn.insert_or_update::<I>(models))
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub fn upsert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        self.insert_or_update(models)
    }

    /// 插入或忽略记录 - 返回执行器（合并后返回 [`InsertOrIgnoreExecutor`]
    /// 的事务变体，旧名 [`TransactionInsertOrIgnoreExecutor`] 保留为过渡别名）。
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                InsertOrIgnoreExecutor::SqliteTxn(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                InsertOrIgnoreExecutor::PostgreSQLTxn(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                InsertOrIgnoreExecutor::MySQLTxn(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                InsertOrIgnoreExecutor::MSSQLTxn(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                InsertOrIgnoreExecutor::DuckDBTxn(txn.insert_or_ignore::<I>(models))
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }
}

impl<'a> super::DbExecutor for Transaction<'a> {
    fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        Transaction::select::<T>(self)
    }

    fn select_column<T: Model, V>(&self) -> ProjectionSelectExecutor<'_, T, V> {
        Transaction::select_column::<T, V>(self)
    }
}

crate::impl_unified_join_executor!(LeftJoinedSelectExecutor);

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            LeftJoinedSelectExecutor::Sqlite(exec, phantom) => {
                LeftJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            LeftJoinedSelectExecutor::MSSQL(exec) => {
                LeftJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            LeftJoinedSelectExecutor::DuckDB(exec) => {
                LeftJoinCollectFuture::DuckDB(exec.collect::<C>())
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            LeftJoinedSelectExecutor::Unsupported {
                backend, feature, ..
            } => LeftJoinCollectFuture::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

crate::impl_unified_join_executor!(InnerJoinedSelectExecutor);

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InnerJoinedSelectExecutor::Sqlite(exec, phantom) => {
                InnerJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            InnerJoinedSelectExecutor::PostgreSQL(exec) => {
                InnerJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            InnerJoinedSelectExecutor::MySQL(exec) => {
                InnerJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            InnerJoinedSelectExecutor::MSSQL(exec) => {
                InnerJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            InnerJoinedSelectExecutor::DuckDB(exec) => {
                InnerJoinCollectFuture::DuckDB(exec.collect::<C>())
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            InnerJoinedSelectExecutor::Unsupported {
                backend, feature, ..
            } => InnerJoinCollectFuture::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

crate::impl_unified_join_executor!(RightJoinedSelectExecutor);

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            RightJoinedSelectExecutor::Sqlite(exec, phantom) => {
                RightJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            RightJoinedSelectExecutor::PostgreSQL(exec) => {
                RightJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            RightJoinedSelectExecutor::MySQL(exec) => {
                RightJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            RightJoinedSelectExecutor::MSSQL(exec) => {
                RightJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            RightJoinedSelectExecutor::DuckDB(exec) => {
                RightJoinCollectFuture::DuckDB(exec.collect::<C>())
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            RightJoinedSelectExecutor::Unsupported {
                backend, feature, ..
            } => RightJoinCollectFuture::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

crate::impl_unified_join_collect_future!(LeftJoinCollectFuture, crate::Result<Vec<(T, Option<J>)>>);

crate::impl_unified_join_collect_future!(InnerJoinCollectFuture, crate::Result<Vec<(T, J)>>);

crate::impl_unified_join_collect_future!(
    RightJoinCollectFuture,
    crate::Result<Vec<(Option<T>, J)>>
);

/// 统一的 Projection 查询执行器枚举（字段投影与分组聚合合一）
///
/// 由原 `MappedSelectExecutor`（字段投影）与 `GroupedSelectExecutor`（分组聚合）
/// 合并而来：`ProjectionSelect::grouping` 决定实际走的 SQL 路径。
pub enum ProjectionSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::ProjectionSelectExecutor<'a, T, V>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::ProjectionSelectExecutor<'a, T, V>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::ProjectionSelectExecutor<'a, T, V>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::ProjectionSelectExecutor<'a, T, V>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::ProjectionSelectExecutor<'a, T, V>),
    /// ClickHouse 分组聚合：渲染 GROUP BY 聚合 SQL 后走
    /// `select_named_values`（JSONEachRowWithNames）按投影顺序解码。
    #[cfg(feature = "clickhouse")]
    ClickHouse(&'a clickhouse_backend::Database, ProjectionSelect<T, V>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V)>,
    },
}

#[deprecated(
    since = "0.2.12",
    note = "MappedSelectExecutor 已合并为 ProjectionSelectExecutor，请改用 ProjectionSelectExecutor"
)]
pub type MappedSelectExecutor<'a, T, V> = ProjectionSelectExecutor<'a, T, V>;

#[deprecated(
    since = "0.2.12",
    note = "GroupedSelectExecutor 已合并为 ProjectionSelectExecutor，请改用 ProjectionSelectExecutor"
)]
pub type GroupedSelectExecutor<'a, T, V> = ProjectionSelectExecutor<'a, T, V>;

impl<'a, T: Model, V> ProjectionSelectExecutor<'a, T, V> {
    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => {
                ProjectionSelectExecutor::Sqlite(exec.group_by(f))
            }
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.group_by(f))
            }
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => ProjectionSelectExecutor::MySQL(exec.group_by(f)),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => ProjectionSelectExecutor::MSSQL(exec.group_by(f)),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => {
                ProjectionSelectExecutor::DuckDB(exec.group_by(f))
            }
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(db, select) => {
                ProjectionSelectExecutor::ClickHouse(db, select.group_by(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ ProjectionSelectExecutor::Unsupported { .. } => unsupported,
        }
    }

    /// 添加 HAVING 条件
    pub fn having<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => {
                ProjectionSelectExecutor::Sqlite(exec.having(f))
            }
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.having(f))
            }
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => ProjectionSelectExecutor::MySQL(exec.having(f)),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => ProjectionSelectExecutor::MSSQL(exec.having(f)),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => {
                ProjectionSelectExecutor::DuckDB(exec.having(f))
            }
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(db, select) => {
                ProjectionSelectExecutor::ClickHouse(db, select.having(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ ProjectionSelectExecutor::Unsupported { .. } => unsupported,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F, W>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => {
                ProjectionSelectExecutor::Sqlite(exec.filter(f))
            }
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.filter(f))
            }
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => ProjectionSelectExecutor::MySQL(exec.filter(f)),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => ProjectionSelectExecutor::MSSQL(exec.filter(f)),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => {
                ProjectionSelectExecutor::DuckDB(exec.filter(f))
            }
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(db, select) => {
                ProjectionSelectExecutor::ClickHouse(db, select.filter(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ ProjectionSelectExecutor::Unsupported { .. } => unsupported,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C>(&self) -> ProjectionCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => {
                ProjectionCollectFuture::Sqlite(exec.collect::<C>())
            }
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => {
                ProjectionCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => {
                ProjectionCollectFuture::MySQL(exec.collect::<C>())
            }
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => {
                ProjectionCollectFuture::MSSQL(exec.collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => {
                ProjectionCollectFuture::DuckDB(exec.collect::<C>())
            }
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(db, select) => ProjectionCollectFuture::ClickHouse(
                *db,
                select.clone(),
                std::marker::PhantomData,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionSelectExecutor::Unsupported {
                backend, feature, ..
            } => ProjectionCollectFuture::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }

    pub fn as_model<R: Model>(self) -> crate::Result<DerivedSelect<R>>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(_, _) => Err(unsupported_feature(
                super::super::DbType::ClickHouse,
                "Model select_column on ClickHouse; use select_sql",
            )),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionSelectExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }
}

impl<'a, T: Model, V> Clone for ProjectionSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => ProjectionSelectExecutor::Sqlite(exec.clone()),
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.clone_with_client())
            }
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => {
                ProjectionSelectExecutor::MySQL(exec.clone_with_pool())
            }
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => {
                ProjectionSelectExecutor::MSSQL(exec.clone_with_pool())
            }
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => ProjectionSelectExecutor::DuckDB(exec.clone()),
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(db, select) => {
                ProjectionSelectExecutor::ClickHouse(db, select.clone())
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionSelectExecutor::Unsupported {
                backend, feature, ..
            } => ProjectionSelectExecutor::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

/// 统一的 Projection Collect Future 枚举（字段投影与分组聚合合一）
pub enum ProjectionCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::ProjectionCollectFuture<'a, T, V, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::ProjectionCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::ProjectionCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::ProjectionCollectFuture<'a, T, V, C>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::ProjectionCollectFuture<'a, T, V, C>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(
        &'a clickhouse_backend::Database,
        ProjectionSelect<T, V>,
        std::marker::PhantomData<C>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V, C)>,
    },
}

#[deprecated(
    since = "0.2.12",
    note = "MappedCollectFuture 已合并为 ProjectionCollectFuture，请改用 ProjectionCollectFuture"
)]
pub type MappedCollectFuture<'a, T, V, C> = ProjectionCollectFuture<'a, T, V, C>;

#[deprecated(
    since = "0.2.12",
    note = "GroupedCollectFuture 已合并为 ProjectionCollectFuture，请改用 ProjectionCollectFuture"
)]
pub type GroupedCollectFuture<'a, T, V, C> = ProjectionCollectFuture<'a, T, V, C>;

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for ProjectionCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionCollectFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            ProjectionCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            ProjectionCollectFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            ProjectionCollectFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "duckdb")]
            ProjectionCollectFuture::DuckDB(future) => Box::pin(future.into_future()),
            // 分组投影的输出列名由服务端返回（JSONEachRowWithNames 首行），
            // 按投影顺序解码为 V
            #[cfg(feature = "clickhouse")]
            ProjectionCollectFuture::ClickHouse(db, select, _) => Box::pin(async move {
                let (sql, params) = select.try_to_sql_with_params(super::super::DbType::ClickHouse)?;
                let (_, rows) = db
                    .select_named_values(RawSql::new(sql).with_params(params))
                    .await?;
                rows.iter()
                    .map(|values| V::from_row_values(values))
                    .collect()
            }),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionCollectFuture::Unsupported {
                backend, feature, ..
            } => Box::pin(async move { Err(unsupported_feature(backend, feature)) }),
        }
    }
}

/// 统一的 ModelCollectWithFuture 枚举
pub enum ModelCollectWithFuture<'a, T: Model + 'static, V: 'static, C, M, F> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::ModelCollectWithFuture<'a, T, V, C, M, F>),
    #[cfg(feature = "postgresql")]
    PostgreSQLCollect(
        postgresql_backend::ProjectionCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mysql")]
    MySQLCollect(
        mysql_backend::ProjectionCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mssql")]
    MSSQLCollect(
        mssql_backend::ProjectionCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::ModelCollectWithFuture<'a, T, V, C, M, F>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V, C, M, F)>,
    },
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持：
    /// - 单字段：map_to(|r| r.uid) -> ProjectionSelectExecutor<'a, T, i32>
    /// - 元组：map_to(|r| (r.uid, r.id)) -> ProjectionSelectExecutor<'a, T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> ProjectionSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => ProjectionSelectExecutor::Sqlite(exec.map_to(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.map_to(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => ProjectionSelectExecutor::MySQL(exec.map_to(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => ProjectionSelectExecutor::MSSQL(exec.map_to(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => ProjectionSelectExecutor::DuckDB(exec.map_to(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => ProjectionSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "select capability on ClickHouse",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> ProjectionSelectExecutor<'a, T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                ProjectionSelectExecutor::Sqlite(exec.select_column(f))
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                ProjectionSelectExecutor::PostgreSQL(exec.select_column(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => ProjectionSelectExecutor::MySQL(exec.select_column(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => ProjectionSelectExecutor::MSSQL(exec.select_column(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => {
                ProjectionSelectExecutor::DuckDB(exec.select_column(f))
            }
            // 能力矩阵门控与 Database / 池连接入口共用同一判定与文案
            // （L19：clickhouse_projection_gate）。
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                let backend = clickhouse_select_backend_db_type(db);
                if let Some(feature) = clickhouse_projection_gate(backend) {
                    return ProjectionSelectExecutor::Unsupported {
                        backend,
                        feature,
                        _marker: std::marker::PhantomData,
                    };
                }
                #[cfg(feature = "clickhouse")]
                #[cfg_attr(not(feature = "influxdb"), allow(irrefutable_let_patterns))]
                if let ClickHouseSelectBackend::ClickHouse(db) = db {
                    return ProjectionSelectExecutor::ClickHouse(db, select.select_column(f));
                }
                #[cfg(not(feature = "clickhouse"))]
                {
                    // influxdb-only 构建时能力门控已在上方返回，仅为
                    // 穷尽性保留
                    let _ = (select, f);
                }
                ProjectionSelectExecutor::Unsupported {
                    backend,
                    feature: "GROUP BY aggregation",
                    _marker: std::marker::PhantomData,
                }
            }
        }
    }
}

pub struct IncludedSelectExecutor<'a, T: Model, S: RelationSelection<T>> {
    select: SelectExecutor<'a, T>,
    selection: S,
    _marker: std::marker::PhantomData<S>,
}

impl<'a, T: Model, S: RelationSelection<T>> IncludedSelectExecutor<'a, T, S> {
    pub fn include<F, S2>(self, f: F) -> DoubleIncludedSelectExecutor<'a, T, S, S2>
    where
        F: FnOnce(T::Where) -> S2,
        S2: RelationSelection<T>,
    {
        DoubleIncludedSelectExecutor {
            select: self.select,
            first: self.selection,
            second: f(T::Where::default()),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn collect<C>(self) -> IncludedCollectFuture<'a, T, S, C>
    where
        T: 'static,
        S: 'static,
        S::Target: Clone + 'static,
        C: FromIterator<T> + 'static,
    {
        IncludedCollectFuture {
            select: self.select,
            selection: self.selection,
            _marker: std::marker::PhantomData,
        }
    }
}

pub struct DoubleIncludedSelectExecutor<
    'a,
    T: Model,
    S1: RelationSelection<T>,
    S2: RelationSelection<T>,
> {
    select: SelectExecutor<'a, T>,
    first: S1,
    second: S2,
    _marker: std::marker::PhantomData<(S1, S2)>,
}

impl<'a, T, S1, S2> DoubleIncludedSelectExecutor<'a, T, S1, S2>
where
    T: Model,
    S1: RelationSelection<T>,
    S2: RelationSelection<T>,
{
    pub fn collect<C>(self) -> DoubleIncludedCollectFuture<'a, T, S1, S2, C>
    where
        T: 'static,
        S1: 'static,
        S2: 'static,
        S1::Target: Clone + 'static,
        S2::Target: Clone + 'static,
        C: FromIterator<T> + 'static,
    {
        DoubleIncludedCollectFuture {
            select: self.select,
            first: self.first,
            second: self.second,
            _marker: std::marker::PhantomData,
        }
    }
}

pub struct IncludedCollectFuture<'a, T: Model, S: RelationSelection<T>, C> {
    select: SelectExecutor<'a, T>,
    selection: S,
    _marker: std::marker::PhantomData<C>,
}

pub struct DoubleIncludedCollectFuture<
    'a,
    T: Model,
    S1: RelationSelection<T>,
    S2: RelationSelection<T>,
    C,
> {
    select: SelectExecutor<'a, T>,
    first: S1,
    second: S2,
    _marker: std::marker::PhantomData<C>,
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    S: RelationSelection<T> + RelationNestedLoader<'a, T> + std::marker::Send + std::marker::Sync + 'a,
    C: FromIterator<T> + 'static,
> std::future::IntoFuture for IncludedCollectFuture<'a, T, S, C>
where
    S::Target: std::marker::Send + std::marker::Sync,
    S::Via: std::marker::Send + std::marker::Sync,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let mut owners = self.select.collect::<Vec<T>>().await?;
            self.select
                .preload_models_with_selection(&mut owners, self.selection)
                .await?;
            Ok(owners.into_iter().collect())
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    S1: RelationSelection<T> + RelationNestedLoader<'a, T> + std::marker::Send + std::marker::Sync + 'a,
    S2: RelationSelection<T> + RelationNestedLoader<'a, T> + std::marker::Send + std::marker::Sync + 'a,
    C: FromIterator<T> + 'static,
> std::future::IntoFuture for DoubleIncludedCollectFuture<'a, T, S1, S2, C>
where
    S1::Target: std::marker::Send + std::marker::Sync,
    S1::Via: std::marker::Send + std::marker::Sync,
    S2::Target: std::marker::Send + std::marker::Sync,
    S2::Via: std::marker::Send + std::marker::Sync,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let mut owners = self.select.collect::<Vec<T>>().await?;
            self.select
                .preload_models_with_selection(&mut owners, self.first)
                .await?;
            self.select
                .preload_models_with_selection(&mut owners, self.second)
                .await?;
            Ok(owners.into_iter().collect())
        })
    }
}

/// collect_with 的克隆式后端分支（L27：PostgreSQL/MySQL/MSSQL 三份同构
/// 代码收敛）：`clone_with_*` → `collect::<Vec<V>>` → 装入对应 Collect 变体。
/// `V` 由变体构造器的期望类型反向推断，宏内无需引用泛型参数。
macro_rules! collect_with_clone_branch {
    ($exec:ident, $f:ident, [$($variant:ident)::+], $clone:ident) => {{
        let future = $exec.$clone().collect::<Vec<_>>();
        $($variant)::+(future, $f, std::marker::PhantomData)
    }};
}

impl<'a, T: Model, V> ProjectionSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果，同时应用转换函数
    /// 用于将查询结果转换为其他类型（如Model）
    /// 示例：collect_with(|v| Uids { id: v })
    pub fn collect_with<C, F, M>(self, f: F) -> ModelCollectWithFuture<'a, T, V, C, M, F>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<M> + 'static,
        F: Fn(V) -> M + Clone + 'static,
        M: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => {
                ModelCollectWithFuture::Sqlite(exec.collect_with::<C, F, M>(f))
            }
            // 三个克隆式后端共用同一份组合逻辑（L27）
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => collect_with_clone_branch!(
                exec,
                f,
                [ModelCollectWithFuture::PostgreSQLCollect],
                clone_with_client
            ),
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => collect_with_clone_branch!(
                exec,
                f,
                [ModelCollectWithFuture::MySQLCollect],
                clone_with_pool
            ),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => collect_with_clone_branch!(
                exec,
                f,
                [ModelCollectWithFuture::MSSQLCollect],
                clone_with_pool
            ),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => {
                ModelCollectWithFuture::DuckDB(exec.collect_with::<C, F, M>(f))
            }
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(_, _) => ModelCollectWithFuture::Unsupported {
                backend: super::super::DbType::ClickHouse,
                feature: "collect_with on ClickHouse; use select_sql",
                _marker: std::marker::PhantomData,
            },
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionSelectExecutor::Unsupported {
                backend, feature, ..
            } => ModelCollectWithFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

impl<'a, T: Model, V> ProjectionSelectExecutor<'a, T, V> {
    fn into_in_filter(self, column: String) -> crate::query::filter::FilterExpr {
        let in_subquery = |subquery: crate::Result<(String, Vec<crate::model::Value>)>| {
            crate::query::filter::FilterExpr::InSubqueryDynamic {
                column,
                subquery: crate::query::filter::DynamicSubquery::new(move |_| subquery.clone()),
            }
        };

        match self {
            #[cfg(feature = "sqlite")]
            ProjectionSelectExecutor::Sqlite(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "postgresql")]
            ProjectionSelectExecutor::PostgreSQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "mysql")]
            ProjectionSelectExecutor::MySQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "mssql")]
            ProjectionSelectExecutor::MSSQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "duckdb")]
            ProjectionSelectExecutor::DuckDB(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "clickhouse")]
            ProjectionSelectExecutor::ClickHouse(_, _) => in_subquery(Err(
                crate::OrmerError::UnsupportedFeature {
                    backend: super::super::DbType::ClickHouse,
                    feature: "subquery on ClickHouse; use select_sql",
                },
            )),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ProjectionSelectExecutor::Unsupported {
                backend, feature, ..
            } => in_subquery(Err(crate::OrmerError::UnsupportedFeature {
                backend,
                feature,
            })),
        }
    }
}

// 为 ProjectionSelectExecutor 实现 IsInValues trait
impl<'a, T: Model, V: crate::query::builder::ColumnValueType> crate::query::builder::IsInValues<V>
    for ProjectionSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        crate::query::builder::WhereExpr::from_filter(self.into_in_filter(column))
    }
}

// 为 &ProjectionSelectExecutor 实现 IsInValues trait（引用版本）
impl<'a, 'b, T: Model, V: crate::query::builder::ColumnValueType>
    crate::query::builder::IsInValues<V> for &'b ProjectionSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        crate::query::builder::WhereExpr::from_filter(self.clone().into_in_filter(column))
    }
}

/// 克隆式 collect_with（PostgreSQL/MySQL/MSSQL Collect 变体）的公共
/// 收尾（L27）：等待投影结果后逐项应用转换函数，三份同构循环收敛为一份。
/// 后端 Collect 变体只实现 `IntoFuture`，调用方需先 `.into_future()`。
async fn collect_with_map<V, M, C, F, Fut>(future: Fut, mapper: F) -> crate::Result<C>
where
    Fut: Future<Output = crate::Result<Vec<V>>>,
    F: Fn(V) -> M,
    C: FromIterator<M>,
{
    Ok(future.await?.into_iter().map(mapper).collect())
}

impl<'a, T, V, C, M, F> std::future::IntoFuture for ModelCollectWithFuture<'a, T, V, C, M, F>
where
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<M> + 'static,
    M: 'static + std::marker::Send,
    F: Fn(V) -> M + Clone + Send + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            ModelCollectWithFuture::Sqlite(future) => Box::pin(future.into_future()),
            // 三个克隆式后端共用同一份 mapper 收尾（L27）
            #[cfg(feature = "postgresql")]
            ModelCollectWithFuture::PostgreSQLCollect(future, mapper, _) => {
                Box::pin(collect_with_map(future.into_future(), mapper))
            }
            #[cfg(feature = "mysql")]
            ModelCollectWithFuture::MySQLCollect(future, mapper, _) => {
                Box::pin(collect_with_map(future.into_future(), mapper))
            }
            #[cfg(feature = "mssql")]
            ModelCollectWithFuture::MSSQLCollect(future, mapper, _) => {
                Box::pin(collect_with_map(future.into_future(), mapper))
            }
            #[cfg(feature = "duckdb")]
            ModelCollectWithFuture::DuckDB(future) => Box::pin(future.into_future()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            ModelCollectWithFuture::Unsupported {
                backend, feature, ..
            } => Box::pin(async move { Err(unsupported_feature(backend, feature)) }),
        }
    }
}

impl<'a, T> BatchQuery<'a> for SelectExecutor<'a, T>
where
    T: Model + 'static + Send + Sync,
{
    type Output = Vec<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<T>>().await })
    }
}

impl<'a, T> BatchQuery<'a> for FirstFuture<'a, T>
where
    T: Model + 'static + Send + Sync,
{
    type Output = Option<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        std::future::IntoFuture::into_future(self)
    }
}

impl<'a, T, R> BatchQuery<'a> for AggregateFuture<'a, T, R>
where
    T: Model + 'static + Send,
    R: crate::model::FromValue + 'static + Send,
{
    type Output = R;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        std::future::IntoFuture::into_future(self)
    }
}

impl<'a, R> BatchQuery<'a> for DerivedTableSelectExecutor<'a, R>
where
    R: Model + crate::model::FromRowValues + 'static + Send,
{
    type Output = Vec<R>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<R>>().await })
    }
}

impl<'a, T> BatchQuery<'a> for RawSelectExecutor<'a, T>
where
    T: crate::model::FromRowValues + 'static + Send,
{
    type Output = Vec<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<T>>().await })
    }
}

impl<'a, T, R> BatchQuery<'a> for RelatedSelectExecutor<'a, T, R>
where
    T: Model + 'static + Send + Sync,
    R: Model + 'static + Send + Sync,
{
    type Output = Vec<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<T>>().await })
    }
}

impl<'a, T, J> BatchQuery<'a> for LeftJoinedSelectExecutor<'a, T, J>
where
    T: Model + 'static + Send + Sync,
    J: Model + 'static + Send + Sync,
{
    type Output = Vec<(T, Option<J>)>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<(T, Option<J>)>>().await })
    }
}

impl<'a, T, J> BatchQuery<'a> for InnerJoinedSelectExecutor<'a, T, J>
where
    T: Model + 'static + Send + Sync,
    J: Model + 'static + Send + Sync,
{
    type Output = Vec<(T, J)>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<(T, J)>>().await })
    }
}

impl<'a, T, J> BatchQuery<'a> for RightJoinedSelectExecutor<'a, T, J>
where
    T: Model + 'static + Send + Sync,
    J: Model + 'static + Send + Sync,
{
    type Output = Vec<(Option<T>, J)>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<(Option<T>, J)>>().await })
    }
}

impl<'a, T, V> BatchQuery<'a> for ProjectionSelectExecutor<'a, T, V>
where
    T: Model + 'static + Send + Sync,
    V: crate::model::FromRowValues + 'static + Send + Sync,
{
    type Output = Vec<V>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<V>>().await })
    }
}

impl<'a, T, S> BatchQuery<'a> for IncludedSelectExecutor<'a, T, S>
where
    T: Model + 'static + Send + Sync,
    S: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync + 'a + 'static,
    S::Target: Clone + 'static + Send + Sync,
    S::Via: Send + Sync,
{
    type Output = Vec<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<T>>().await })
    }
}

impl<'a, T, S1, S2> BatchQuery<'a> for DoubleIncludedSelectExecutor<'a, T, S1, S2>
where
    T: Model + 'static + Send + Sync,
    S1: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync + 'a + 'static,
    S2: RelationSelection<T> + RelationNestedLoader<'a, T> + Send + Sync + 'a + 'static,
    S1::Target: Clone + 'static + Send + Sync,
    S1::Via: Send + Sync,
    S2::Target: Clone + 'static + Send + Sync,
    S2::Via: Send + Sync,
{
    type Output = Vec<T>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<T>>().await })
    }
}

/// 统一的 SelectStream 枚举
pub enum SelectStream<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectStream<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectStream<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectStream<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectStream<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::SelectStream<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(
        ClickHouseSelectBackend<'a>,
        crate::query::builder::Select<T>,
    ),
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectStream::Sqlite(exec.stream()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectStream::PostgreSQL(exec.stream()),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectStream::MySQL(exec.stream()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectStream::MSSQL(exec.stream()),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => SelectStream::DuckDB(exec.stream()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => SelectStream::ClickHouse(db, select),
        }
    }
}

/// 统一的 SelectStreamIterator 枚举
pub enum SelectStreamIterator<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::SelectStreamIterator<'a, T>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    ClickHouse(ClickHouseStreamState<T>, std::marker::PhantomData<&'a ()>),
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器
    pub async fn into_iter(self) -> crate::Result<SelectStreamIterator<'a, T>> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectStream::Sqlite(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::Sqlite(iter))
            }
            #[cfg(feature = "postgresql")]
            SelectStream::PostgreSQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::PostgreSQL(iter))
            }
            #[cfg(feature = "mysql")]
            SelectStream::MySQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::MySQL(iter))
            }
            #[cfg(feature = "mssql")]
            SelectStream::MSSQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::MSSQL(iter))
            }
            #[cfg(feature = "duckdb")]
            SelectStream::DuckDB(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::DuckDB(iter))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectStream::ClickHouse(db, select) => {
                // ClickHouse：走 JSONEachRow 字节游标，边拉边解码（真流式）；
                // InfluxDB：无游标 API，保持缓冲实现
                #[cfg(feature = "clickhouse")]
                #[cfg_attr(not(feature = "influxdb"), allow(irrefutable_let_patterns))]
                if let ClickHouseSelectBackend::ClickHouse(db) = db {
                    let (sql, params) =
                        select.try_to_sql_with_params(super::super::DbType::ClickHouse)?;
                    let cursor = db.select_json_stream(RawSql::new(sql).with_params(params))?;
                    return Ok(SelectStreamIterator::ClickHouse(
                        ClickHouseStreamState::Streaming(ClickHouseJsonStreamDecoder {
                            cursor,
                            columns: T::columns(),
                            buffer: Vec::new(),
                            finished: false,
                            _marker: std::marker::PhantomData,
                        }),
                        std::marker::PhantomData,
                    ));
                }
                #[cfg(feature = "influxdb")]
                #[allow(irrefutable_let_patterns)]
                if let ClickHouseSelectBackend::Influx(db) = db {
                    let rows = influx_select_models::<T, Vec<T>>(db, select).await?;
                    return Ok(SelectStreamIterator::ClickHouse(
                        ClickHouseStreamState::Buffered(rows.into_iter()),
                        std::marker::PhantomData,
                    ));
                }
                // cfg 门控保证上方两个 if-let 覆盖所有可构造的后端变体；
                // 落到这里只能是手工混用构造的值，返回错误而非 panic。
                return Err(crate::OrmerError::invalid_operation(
                    "SelectStream::ClickHouse backend matched neither arm",
                ));
            }
        }
    }
}

/// ClickHouse/InfluxDB 流式迭代器的内部状态（P1-3）。
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
pub enum ClickHouseStreamState<T: Model> {
    /// InfluxDB 无游标 API：先缓冲全量再逐行吐出（非真流式，
    /// 大结果集有内存压力）。
    Buffered(std::vec::IntoIter<T>),
    /// ClickHouse：包装后端 [`clickhouse_backend::Database::select_json_stream`]
    /// 的字节游标，按行解码 JSONEachRow，内存占用与结果集规模无关。
    #[cfg(feature = "clickhouse")]
    Streaming(ClickHouseJsonStreamDecoder<T>),
}

/// ClickHouse JSONEachRow 字节游标解码器：跨 chunk 维护行缓冲，
/// 每凑齐一行即按列名解码为模型行值。
#[cfg(feature = "clickhouse")]
pub struct ClickHouseJsonStreamDecoder<T: Model> {
    cursor: clickhouse::query::BytesCursor,
    columns: Vec<&'static str>,
    buffer: Vec<u8>,
    finished: bool,
    _marker: std::marker::PhantomData<T>,
}

#[cfg(feature = "clickhouse")]
impl<T: Model> ClickHouseJsonStreamDecoder<T> {
    async fn next_row(&mut self) -> Option<crate::Result<T>> {
        loop {
            if let Some(pos) = self.buffer.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                if line.trim().is_empty() {
                    continue;
                }
                let row: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(row) => row,
                    Err(error) => {
                        return Some(Err(crate::ormer_error!(
                            "Invalid ClickHouse JSONEachRow row: {error}"
                        )))
                    }
                };
                let values = match super::super::clickhouse_backend::named_json_row_values(
                    &row,
                    &self.columns,
                ) {
                    Ok(values) => values,
                    Err(error) => return Some(Err(error)),
                };
                return Some(T::from_row_values(&values));
            }
            if self.finished {
                return None;
            }
            match self.cursor.next().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => {
                    self.finished = true;
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(crate::OrmerError::from_external(
                        "clickhouse::BytesCursor::next",
                        error,
                    )));
                }
            }
        }
    }
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<crate::Result<T>> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectStreamIterator::Sqlite(iter) => iter.next().await,
            #[cfg(feature = "postgresql")]
            SelectStreamIterator::PostgreSQL(iter) => iter.next().await,
            #[cfg(feature = "mysql")]
            SelectStreamIterator::MySQL(iter) => iter.next().await,
            #[cfg(feature = "mssql")]
            SelectStreamIterator::MSSQL(iter) => iter.next().await,
            #[cfg(feature = "duckdb")]
            SelectStreamIterator::DuckDB(iter) => iter.next().await,
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectStreamIterator::ClickHouse(state, _) => match state {
                ClickHouseStreamState::Buffered(rows) => rows.next().map(Ok),
                #[cfg(feature = "clickhouse")]
                ClickHouseStreamState::Streaming(decoder) => decoder.next_row().await,
            },
        }
    }
}
