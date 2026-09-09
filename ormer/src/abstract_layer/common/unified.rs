#![allow(clippy::upper_case_acronyms)]

#[cfg(feature = "clickhouse")]
use super::SingleSqlStatement;
/// 统一的数据库抽象层
/// 使用枚举包装不同数据库后端,对外提供统一接口
/// 通过条件编译控制枚举变体
use super::{SqlStatement, common_helpers};
use super::super::capabilities::Capabilities;
use crate::db_first;
use crate::model::{
    Model, NoInclude, Relation, RelationHandle, RelationInfo, RelationPathInfo, RelationQuery,
    RelationSelection, TableRouteValue, ThroughRelation, Tracked, Value, WritableModel,
    normalize_table_name_for_db, routed_model_table_name_for_db,
};
#[cfg(any(feature = "clickhouse", feature = "influxdb"))]
use crate::query::builder::Select;
#[cfg(feature = "clickhouse")]
use crate::query::builder::GroupedSelect;
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

fn quote_table_name(db_type: super::super::DbType, table_name: &str) -> String {
    let normalized = normalize_table_name_for_db(db_type, table_name);
    match db_type {
        #[cfg(feature = "postgresql")]
        super::super::DbType::PostgreSQL => {
            let (schema, table) = crate::model::split_schema_table_name(normalized, "public");
            if schema == "public" {
                crate::model::quote_identifier(db_type, table)
            } else {
                format!(
                    "{}.{}",
                    crate::model::quote_identifier(db_type, schema),
                    crate::model::quote_identifier(db_type, table)
                )
            }
        }
        #[cfg(feature = "mssql")]
        super::super::DbType::MSSQL => {
            let (schema, table) = crate::model::split_schema_table_name(normalized, "dbo");
            if schema == "dbo" {
                crate::model::quote_identifier(db_type, table)
            } else {
                format!(
                    "{}.{}",
                    crate::model::quote_identifier(db_type, schema),
                    crate::model::quote_identifier(db_type, table)
                )
            }
        }
        #[cfg(feature = "questdb")]
        super::super::DbType::QuestDB => {
            let (_, table) = crate::model::split_schema_table_name(normalized, "public");
            crate::model::quote_identifier(db_type, table)
        }
        #[cfg(feature = "sqlite")]
        super::super::DbType::Sqlite => crate::model::quote_identifier(db_type, normalized),
        #[cfg(feature = "mysql")]
        super::super::DbType::MySQL => crate::model::quote_identifier(db_type, normalized),
        #[cfg(any(
            feature = "duckdb",
            feature = "clickhouse",
            feature = "influxdb"
        ))]
        _ => crate::model::quote_identifier(db_type, normalized),
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
        let where_expr = primary_key_filter::<T>(key)?;
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .collect::<Vec<T>>()
            .await?;
        Ok(results.into_iter().next())
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
        let path = relation.path_info()?;
        let key = owner.relation_key_value(relation_owner_key(path))?;
        self.select::<T>()
            .select_related_with_selection(vec![key], &relation)
            .await
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
        self.select::<T>()
            .preload_models_with_selection(owners, relation)
            .await
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
    let (mut sql, params) = aggregate.to_sql_with_params(backend);
    // InfluxQL 没有 AVG，对应语义是 MEAN
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
/// PostgreSQL 同样支持。其余后端暂未提供该执行器。
pub enum TruncateTableExecutor<'a, T: crate::model::WritableModel> {
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TruncateTableExecutor<'a, T>),
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
            TruncateTableExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "postgresql")]
            TruncateTableExecutor::PostgreSQL(exec) => exec.execute().await,
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
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertExecutor<'a, I>),
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
        std::marker::PhantomData<I::Model>,
    ),
}

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

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    pub fn on_conflict<F, C>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> C,
        C: crate::query::insert::ConflictColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.on_conflict(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.on_conflict(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.on_conflict(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.on_conflict(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.on_conflict(f)),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn on_constraint<Target>(self, target: Target) -> Self
    where
        Target: crate::query::insert::IntoInsertConflictTarget<I::Model>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.on_constraint(target)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => {
                InsertExecutor::PostgreSQL(exec.on_constraint(target))
            }
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.on_constraint(target)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.on_constraint(target)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.on_constraint(target)),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
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
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.conflict_where(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.conflict_where(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.conflict_where(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.conflict_where(f)),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn do_nothing(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.do_nothing()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_nothing()),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_nothing()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_nothing()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_nothing()),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn do_update(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.do_update()),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_update()),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_update()),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_update()),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_update()),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
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
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.do_update_if(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.do_update_if(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.do_update_if(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.do_update_if(f)),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn set<F>(self, f: F) -> Self
    where
        F: FnOnce(&mut <I::Model as Model>::Update),
    {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => InsertExecutor::Sqlite(exec.set(f)),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => InsertExecutor::PostgreSQL(exec.set(f)),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => InsertExecutor::MySQL(exec.set(f)),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => InsertExecutor::MSSQL(exec.set(f)),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => InsertExecutor::DuckDB(exec.set(f)),
            #[cfg(feature = "clickhouse")]
            unsupported @ InsertExecutor::ClickHouse(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::InfluxDB(..) => unsupported,
            #[cfg(feature = "influxdb")]
            unsupported @ InsertExecutor::Unsupported { .. } => unsupported,
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.to_sql(),
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
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.execute().await,
            #[cfg(feature = "clickhouse")]
            InsertExecutor::ClickHouse(db, mut models, _conflict, _) => {
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
            InsertExecutor::InfluxDB(db, mut models, _) => {
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
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.db_type(),
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(_) => super::super::DbType::MySQL,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(_) => super::super::DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(_) => super::super::DbType::DuckDB,
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
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.returning().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.returning().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.returning().await,
            #[cfg(feature = "duckdb")]
            InsertExecutor::DuckDB(exec) => exec.returning().await,
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

/// 统一的 InsertOrUpdateExecutor 枚举
pub enum InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertOrUpdateExecutor<'a, I>),
    /// 能力矩阵门控产物：`insert_conflict: false` 的后端在
    /// [`Database::insert_or_update`] 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a I>,
    },
}

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
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDB(exec) => exec.to_sql(),
            InsertOrUpdateExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "duckdb")]
            InsertOrUpdateExecutor::DuckDB(exec) => exec.execute().await.map(|_| ()),
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
fn log_rollback_failure(result: crate::Result<()>) {
    if let Err(err) = result {
        eprintln!("[ormer] rollback failed during error handling: {err}");
    }
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

pub struct SaveExecutor<'a, T: WritableModel + crate::model::GraphWritable> {
    db: &'a Database,
    model: &'a mut Tracked<T>,
}

impl<'a, T: WritableModel + crate::model::Model + crate::model::GraphWritable> SaveExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let fields = self.model.dirty_columns();
        if fields.is_empty() {
            return Ok(SqlStatement::batch(self.db.db_type(), Vec::new()));
        }
        self.db
            .update::<T>()
            .set_model_columns(self.model.as_model(), &fields)
            .to_sql()
    }

    pub async fn execute(self) -> crate::Result<u64> {
        let SaveExecutor { db, model } = self;
        let fields = model.dirty_columns();
        let mut tx = db.begin().await?;
        let result = async {
            let mut affected = 0u64;
            if !fields.is_empty() {
                affected += tx
                    .update::<T>()
                    .set_model_columns(model.as_model(), &fields)
                    .execute()
                    .await?;
            }
            affected += model.sync_graph_relations(&mut tx).await?;
            Ok::<u64, crate::OrmerError>(affected)
        }
        .await;

        match result {
            Ok(affected) => {
                tx.commit().await?;
                if affected > 0 {
                    model.accept_changes();
                }
                Ok(affected)
            }
            Err(err) => {
                log_rollback_failure(tx.rollback().await);
                Err(err)
            }
        }
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
        let SaveExecutor { db, model } = self;
        let mut ctx = crate::HookContext::new(crate::HookOperation::Update);
        // 与 Update/Delete 执行器同语义：受全局 without_hooks 范围开关控制
        if ctx.hooks_enabled() {
            crate::BeforeUpdate::before_update(model.as_model_mut(), &mut ctx).await?;
        }

        let fields = model.dirty_columns();
        let mut tx = db.begin().await?;
        let result = async {
            let mut affected = 0u64;
            if !fields.is_empty() {
                affected += tx
                    .update::<T>()
                    .set_model_columns(model.as_model(), &fields)
                    .execute()
                    .await?;
            }
            affected += model.sync_graph_relations(&mut tx).await?;
            if affected > 0 && ctx.hooks_enabled() {
                crate::AfterUpdate::after_update(model.as_model(), &mut ctx).await?;
            }
            Ok::<u64, crate::OrmerError>(affected)
        }
        .await;

        match result {
            Ok(affected) => {
                tx.commit().await?;
                if affected > 0 {
                    model.accept_changes();
                }
                Ok(affected)
            }
            Err(err) => {
                log_rollback_failure(tx.rollback().await);
                Err(err)
            }
        }
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
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::InsertOrIgnoreExecutor<'a, I>),
    /// 能力矩阵门控产物：`insert_ignore: false` 的后端在
    /// [`Database::insert_or_ignore`] 构造时落入该变体。
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a I>,
    },
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDB(exec) => exec.to_sql(),
            InsertOrIgnoreExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(*backend, *feature)),
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
            #[cfg(feature = "duckdb")]
            InsertOrIgnoreExecutor::DuckDB(exec) => exec.execute().await.map(|_| ()),
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
        let where_expr = primary_key_filter::<T>(key)?;

        // 执行查询并取第一条
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .collect::<Vec<T>>()
            .await?;

        Ok(results.into_iter().next())
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
        let path = relation.path_info()?;
        let key = owner.relation_key_value(relation_owner_key(path))?;
        self.select::<T>()
            .select_related_with_selection(vec![key], &relation)
            .await
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
        self.select::<T>()
            .preload_models_with_selection(owners, relation)
            .await
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
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => GroupedSelectExecutor::Sqlite(db.select_column::<T, V>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                GroupedSelectExecutor::PostgreSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => GroupedSelectExecutor::MySQL(db.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => GroupedSelectExecutor::MSSQL(db.select_column::<T, V>()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => GroupedSelectExecutor::DuckDB(db.select_column::<T, V>()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                // 能力矩阵门控：advanced_grouping 为 true 才走分组聚合执行分支
                if Capabilities::of(super::super::DbType::ClickHouse).advanced_grouping {
                    GroupedSelectExecutor::ClickHouse(db, GroupedSelect::new())
                } else {
                    GroupedSelectExecutor::Unsupported {
                        backend: super::super::DbType::ClickHouse,
                        feature: "GROUP BY aggregation on ClickHouse",
                        _marker: std::marker::PhantomData,
                    }
                }
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => GroupedSelectExecutor::Unsupported {
                backend: super::super::DbType::InfluxDB,
                feature: "GROUP BY aggregation on InfluxDB (InfluxQL GROUP BY only supports time buckets and tags)",
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
        SaveExecutor { db: self, model }
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
        // 只作用于"下一个事务"，begin 之后设置无效）。
        let mut txn = self.begin_opts(options).await?;

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

    async fn begin_opts(&self, options: TransactionOptions) -> crate::Result<Transaction<'_>> {
        // 两个互补 cfg 的通配臂在单后端编译组合下会触发 unreachable 警告
        #[allow(unreachable_patterns)]
        match self {
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => {
                let txn = db.begin_with_opts(options).await?;
                Ok(Transaction::MySQL(txn))
            }
            #[cfg(not(feature = "mysql"))]
            _ => {
                let mut txn = self.begin().await?;
                if let Err(err) = apply_transaction_options(&mut txn, options).await {
                    log_rollback_failure(txn.rollback().await);
                    return Err(err);
                }
                Ok(txn)
            }
            #[cfg(feature = "mysql")]
            _ => {
                let mut txn = self.begin().await?;
                if let Err(err) = apply_transaction_options(&mut txn, options).await {
                    log_rollback_failure(txn.rollback().await);
                    return Err(err);
                }
                Ok(txn)
            }
        }
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
    /// 按运行时 `db_type` 判定，仍走 PostgreSQL 执行器分支）。
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
        RawSelectExecutor {
            db: self,
            sql: sql.into_raw_sql(),
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                let (sql, params) = sql.render(super::super::DbType::Sqlite)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                let (sql, params) = sql.render(super::super::DbType::PostgreSQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => {
                let (sql, params) = sql.render(super::super::DbType::MySQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => {
                let (sql, params) = sql.render(super::super::DbType::MSSQL)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => {
                let (sql, params) = sql.render(super::super::DbType::DuckDB)?;
                db.exec_raw(&sql, params).await
            }
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                db.execute_sql(sql).await?;
                Ok(0)
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => {
                db.execute_sql(sql).await?;
                Ok(0)
            }
        }
    }

    /// Count rows in a table using backend-specific identifier quoting.
    pub async fn table_row_count(&self, table_name: &str) -> crate::Result<u64> {
        let sql = format!(
            "SELECT COUNT(*) FROM {}",
            quote_table_name(self.db_type(), table_name)
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

    fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
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
            let (sql, params, rust_types) = self.select.to_sql_with_params_and_types(db_type);
            return Ok(SqlStatement::batch(
                db_type,
                vec![super::SingleSqlStatement::new(sql, params).with_param_rust_types(rust_types)],
            ));
        }

        let (sql, params) = self.select.to_sql_with_params(db_type);
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
                    let (sql, params) = self.select.to_sql_with_params(db_type);
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(db) => {
                    let (sql, params, rust_types) =
                        self.select.to_sql_with_params_and_types(db_type);
                    db.select_raw_with_types::<R, C>(&sql, params, rust_types)
                        .await
                }
                #[cfg(feature = "mysql")]
                Database::MySQL(db) => {
                    let (sql, params) = self.select.to_sql_with_params(db_type);
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                Database::MSSQL(db) => {
                    let (sql, params) = self.select.to_sql_with_params(db_type);
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                Database::DuckDB(db) => {
                    let (sql, params) = self.select.to_sql_with_params(db_type);
                    db.select_raw::<R, C>(&sql, params).await
                }
                #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
                let (sql, params) = self.select.to_sql_with_params(db_type);
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

pub struct RawSelectExecutor<'a, T> {
    db: &'a Database,
    sql: RawSql,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T> RawSelectExecutor<'a, T> {
    pub fn collect<C>(self) -> RawCollectFuture<'a, T, C>
    where
        T: crate::model::FromRowValues + 'static,
        C: FromIterator<T> + 'static,
    {
        RawCollectFuture {
            db: self.db,
            sql: self.sql,
            _marker: std::marker::PhantomData,
        }
    }
}

pub struct RawCollectFuture<'a, T, C> {
    db: &'a Database,
    sql: RawSql,
    _marker: std::marker::PhantomData<(T, C)>,
}

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
            match self.db {
                #[cfg(feature = "sqlite")]
                Database::Sqlite(db) => {
                    let (sql, params) = self.sql.render(super::super::DbType::Sqlite)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(db) => {
                    let (sql, params) = self.sql.render(super::super::DbType::PostgreSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mysql")]
                Database::MySQL(db) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MySQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                Database::MSSQL(db) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MSSQL)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                Database::DuckDB(db) => {
                    let (sql, params) = self.sql.render(super::super::DbType::DuckDB)?;
                    db.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => {
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
                #[cfg(feature = "influxdb")]
                Database::InfluxDB(db) => {
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
            }
        })
    }
}

pub struct TransactionRawSelectExecutor<'a, 'tx, T> {
    txn: &'a mut Transaction<'tx>,
    sql: RawSql,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, 'tx, T> TransactionRawSelectExecutor<'a, 'tx, T> {
    pub fn collect<C>(self) -> TransactionRawCollectFuture<'a, 'tx, T, C>
    where
        T: crate::model::FromRowValues + 'static,
        C: FromIterator<T> + 'static,
    {
        TransactionRawCollectFuture {
            txn: self.txn,
            sql: self.sql,
            _marker: std::marker::PhantomData,
        }
    }
}

pub struct TransactionRawCollectFuture<'a, 'tx, T, C> {
    txn: &'a mut Transaction<'tx>,
    sql: RawSql,
    _marker: std::marker::PhantomData<(T, C)>,
}

impl<'a, 'tx, T, C> std::future::IntoFuture for TransactionRawCollectFuture<'a, 'tx, T, C>
where
    T: crate::model::FromRowValues + 'static + std::marker::Send,
    C: FromIterator<T> + 'static,
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            match self.txn {
                #[cfg(feature = "sqlite")]
                Transaction::Sqlite(txn) => {
                    let (sql, params) = self.sql.render(super::super::DbType::Sqlite)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "postgresql")]
                Transaction::PostgreSQL(txn) => {
                    let (sql, params) = self.sql.render(super::super::DbType::PostgreSQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mysql")]
                Transaction::MySQL(txn) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MySQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "mssql")]
                Transaction::MSSQL(txn) => {
                    let (sql, params) = self.sql.render(super::super::DbType::MSSQL)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                #[cfg(feature = "duckdb")]
                Transaction::DuckDB(txn) => {
                    let (sql, params) = self.sql.render(super::super::DbType::DuckDB)?;
                    txn.select_raw::<T, C>(&sql, params).await
                }
                Transaction::_Phantom(infallible, _) => match *infallible {},
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

    /// COUNT 聚合函数
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
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, C)>,
    },
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
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a T>,
    },
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
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R)>,
    },
}

crate::impl_unified_aggregate_future!(AggregateFuture);

/// 统一的 RelatedSelectExecutor 枚举
pub enum RelatedSelectExecutor<'a, T: Model, R: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RelatedSelectExecutor<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::RelatedSelectExecutor<T, R>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R)>,
    },
}

/// 统一的 MultiTableSelectExecutor 枚举
pub enum MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "duckdb")]
    DuckDB(
        duckdb_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R1, R2)>,
    },
}

/// 统一的 FourTableSelectExecutor 枚举
pub enum FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "duckdb")]
    DuckDB(
        duckdb_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R1, R2, R3)>,
    },
}

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
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            FirstFuture::Unsupported {
                backend, feature, ..
            } => Box::pin(async move { Err(unsupported_feature(backend, feature)) }),
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

/// 统一的关联查询同谓词行数统计 Future（page.md 缺失一：分页 total_count 场景）。
///
/// 结果为与原子查询同谓词的 `SELECT COUNT(*)`，不受 range()/order_by() 影响。
pub enum RelatedCountFuture<'a, T: Model, R: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RelatedSelectExecutor<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "duckdb")]
    DuckDB(
        duckdb_backend::RelatedSelectExecutor<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R)>,
    },
}

/// 统一的三表关联查询同谓词行数统计 Future。
pub enum MultiTableCountFuture<'a, T: Model, R1: Model, R2: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "duckdb")]
    DuckDB(
        duckdb_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R1, R2)>,
    },
}

/// 统一的四表关联查询同谓词行数统计 Future。
pub enum FourTableCountFuture<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "duckdb")]
    DuckDB(
        duckdb_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, R1, R2, R3)>,
    },
}

crate::impl_unified_related_count_future!(
    RelatedCountFuture,
    count,
    [
        'a,
        T: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R: crate::Model + 'static + std::marker::Send + std::marker::Sync
    ],
    ['a, T, R]
);
crate::impl_unified_related_count_future!(
    MultiTableCountFuture,
    count,
    [
        'a,
        T: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R1: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R2: crate::Model + 'static + std::marker::Send + std::marker::Sync
    ],
    ['a, T, R1, R2]
);
crate::impl_unified_related_count_future!(
    FourTableCountFuture,
    count,
    [
        'a,
        T: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R1: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R2: crate::Model + 'static + std::marker::Send + std::marker::Sync,
        R3: crate::Model + 'static + std::marker::Send + std::marker::Sync
    ],
    ['a, T, R1, R2, R3]
);

impl<'a, T: Model + 'static, R: Model + 'static> RelatedSelectExecutor<'a, T, R> {
    /// 统计同谓词总行数（列表分页 total_count 用）：
    /// 生成 `SELECT COUNT(*) FROM (<原子查询>)`，不受 range()/order_by() 影响。
    pub fn count(self) -> RelatedCountFuture<'a, T, R> {
        match self {
            #[cfg(feature = "sqlite")]
            RelatedSelectExecutor::Sqlite(exec, phantom) => {
                RelatedCountFuture::Sqlite(exec, phantom)
            }
            #[cfg(feature = "postgresql")]
            RelatedSelectExecutor::PostgreSQL(exec) => RelatedCountFuture::PostgreSQL(exec),
            #[cfg(feature = "mysql")]
            RelatedSelectExecutor::MySQL(exec) => RelatedCountFuture::MySQL(exec),
            #[cfg(feature = "mssql")]
            RelatedSelectExecutor::MSSQL(exec) => RelatedCountFuture::MSSQL(exec),
            #[cfg(feature = "duckdb")]
            RelatedSelectExecutor::DuckDB(exec) => {
                RelatedCountFuture::DuckDB(exec, std::marker::PhantomData)
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            RelatedSelectExecutor::Unsupported {
                backend,
                feature,
                ..
            } => RelatedCountFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

impl<'a, T: Model + 'static, R1: Model + 'static, R2: Model + 'static>
    MultiTableSelectExecutor<'a, T, R1, R2>
{
    /// 统计同谓词总行数（列表分页 total_count 用）。
    pub fn count(self) -> MultiTableCountFuture<'a, T, R1, R2> {
        match self {
            #[cfg(feature = "sqlite")]
            MultiTableSelectExecutor::Sqlite(exec, phantom) => {
                MultiTableCountFuture::Sqlite(exec, phantom)
            }
            #[cfg(feature = "postgresql")]
            MultiTableSelectExecutor::PostgreSQL(exec) => {
                MultiTableCountFuture::PostgreSQL(exec)
            }
            #[cfg(feature = "mysql")]
            MultiTableSelectExecutor::MySQL(exec) => MultiTableCountFuture::MySQL(exec),
            #[cfg(feature = "mssql")]
            MultiTableSelectExecutor::MSSQL(exec) => MultiTableCountFuture::MSSQL(exec),
            #[cfg(feature = "duckdb")]
            MultiTableSelectExecutor::DuckDB(exec, phantom) => {
                MultiTableCountFuture::DuckDB(exec, phantom)
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MultiTableSelectExecutor::Unsupported {
                backend,
                feature,
                ..
            } => MultiTableCountFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

impl<'a, T: Model + 'static, R1: Model + 'static, R2: Model + 'static, R3: Model + 'static>
    FourTableSelectExecutor<'a, T, R1, R2, R3>
{
    /// 统计同谓词总行数（列表分页 total_count 用）。
    pub fn count(self) -> FourTableCountFuture<'a, T, R1, R2, R3> {
        match self {
            #[cfg(feature = "sqlite")]
            FourTableSelectExecutor::Sqlite(exec, phantom) => {
                FourTableCountFuture::Sqlite(exec, phantom)
            }
            #[cfg(feature = "postgresql")]
            FourTableSelectExecutor::PostgreSQL(exec) => {
                FourTableCountFuture::PostgreSQL(exec)
            }
            #[cfg(feature = "mysql")]
            FourTableSelectExecutor::MySQL(exec) => FourTableCountFuture::MySQL(exec),
            #[cfg(feature = "mssql")]
            FourTableSelectExecutor::MSSQL(exec) => FourTableCountFuture::MSSQL(exec),
            #[cfg(feature = "duckdb")]
            FourTableSelectExecutor::DuckDB(exec, phantom) => {
                FourTableCountFuture::DuckDB(exec, phantom)
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            FourTableSelectExecutor::Unsupported {
                backend,
                feature,
                ..
            } => FourTableCountFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}


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

pub struct TransactionSaveExecutor<'a, 'tx, T: WritableModel + crate::model::GraphWritable> {
    txn: &'a mut Transaction<'tx>,
    model: &'a mut Tracked<T>,
}

impl<'a, 'tx, T: WritableModel + crate::model::Model + crate::model::GraphWritable>
    TransactionSaveExecutor<'a, 'tx, T>
{
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let fields = self.model.dirty_columns();
        if fields.is_empty() {
            return Ok(SqlStatement::batch(self.txn.db_type(), Vec::new()));
        }
        self.txn
            .update::<T>()
            .set_model_columns(self.model.as_model(), &fields)
            .to_sql()
    }

    pub async fn execute(self) -> crate::Result<u64> {
        let TransactionSaveExecutor { txn, model } = self;
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
        if affected > 0 {
            model.accept_changes();
        }
        Ok(affected)
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
        let TransactionSaveExecutor { txn, model } = self;
        let mut ctx = crate::HookContext::new(crate::HookOperation::Update).transaction();
        crate::BeforeUpdate::before_update(model.as_model_mut(), &mut ctx).await?;

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
        if affected > 0 {
            crate::AfterUpdate::after_update(model.as_model(), &mut ctx).await?;
            model.accept_changes();
        }
        Ok(affected)
    }
}

/// 事务中的插入执行器
pub enum TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::TransactionInsertExecutor<'a, I>),
    // 哨兵变体：仅在未启用任何事务型后端（如仅 influxdb）时锚定 `'a` 与 `I`。
    // [`std::convert::Infallible`] 无任何值，该变体在安全 Rust 中无法被构造，
    // match 分支对它做 void 消除（`match *infallible {}`）而非 panic。
    #[doc(hidden)]
    _Phantom(std::convert::Infallible, std::marker::PhantomData<&'a I>),
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertExecutor<'a, I> {
    pub fn on_conflict<F, C>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> C,
        C: crate::query::insert::ConflictColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.on_conflict(f))
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.on_conflict(f))
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.on_conflict(f))
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.on_conflict(f))
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.on_conflict(f))
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn on_constraint<Target>(self, target: Target) -> Self
    where
        Target: crate::query::insert::IntoInsertConflictTarget<I::Model>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.on_constraint(target))
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.on_constraint(target))
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.on_constraint(target))
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.on_constraint(target))
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.on_constraint(target))
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn conflict_where<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> W,
        W: Into<WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.conflict_where(f))
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.conflict_where(f))
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.conflict_where(f))
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.conflict_where(f))
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.conflict_where(f))
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn do_nothing(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.do_nothing())
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.do_nothing())
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.do_nothing())
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.do_nothing())
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.do_nothing())
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn do_update(self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.do_update())
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.do_update())
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.do_update())
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.do_update())
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.do_update())
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn do_update_if<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<I::Model as Model>::Where) -> W,
        W: Into<WhereExpr>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.do_update_if(f))
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.do_update_if(f))
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => {
                TransactionInsertExecutor::MySQL(exec.do_update_if(f))
            }
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => {
                TransactionInsertExecutor::MSSQL(exec.do_update_if(f))
            }
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.do_update_if(f))
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn set<F>(self, f: F) -> Self
    where
        F: FnOnce(&mut <I::Model as Model>::Update),
    {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => {
                TransactionInsertExecutor::Sqlite(exec.set(f))
            }
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => {
                TransactionInsertExecutor::PostgreSQL(exec.set(f))
            }
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => TransactionInsertExecutor::MySQL(exec.set(f)),
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => TransactionInsertExecutor::MSSQL(exec.set(f)),
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => {
                TransactionInsertExecutor::DuckDB(exec.set(f))
            }
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => exec.to_sql(),
            TransactionInsertExecutor::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub async fn execute(
        self,
    ) -> crate::Result<<I::Model as crate::model::Model>::AutoIncrementKeyType> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            TransactionInsertExecutor::DuckDB(exec) => exec.execute().await,
            TransactionInsertExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

/// 事务中的插入或更新执行器
pub enum TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    // 哨兵变体：同 [`TransactionInsertExecutor::_Phantom`]，`Infallible`
    // 使其在安全 Rust 中不可构造，match 分支做 void 消除而非 panic。
    #[doc(hidden)]
    _Phantom(std::convert::Infallible, std::marker::PhantomData<&'a I>),
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertOrUpdateExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            TransactionInsertOrUpdateExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            TransactionInsertOrUpdateExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            TransactionInsertOrUpdateExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            TransactionInsertOrUpdateExecutor::DuckDB(exec) => exec.to_sql(),
            TransactionInsertOrUpdateExecutor::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertOrUpdateExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            TransactionInsertOrUpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TransactionInsertOrUpdateExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TransactionInsertOrUpdateExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            TransactionInsertOrUpdateExecutor::DuckDB(exec) => exec.execute().await,
            TransactionInsertOrUpdateExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
    }
}

/// 事务中的插入或忽略执行器
pub enum TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::TransactionInsertOrIgnoreExecutor<'a, I>),
    // 哨兵变体：同 [`TransactionInsertExecutor::_Phantom`]，`Infallible`
    // 使其在安全 Rust 中不可构造，match 分支做 void 消除而非 panic。
    #[doc(hidden)]
    _Phantom(std::convert::Infallible, std::marker::PhantomData<&'a I>),
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertOrIgnoreExecutor::Sqlite(exec) => exec.to_sql(),
            #[cfg(feature = "postgresql")]
            TransactionInsertOrIgnoreExecutor::PostgreSQL(exec) => exec.to_sql(),
            #[cfg(feature = "mysql")]
            TransactionInsertOrIgnoreExecutor::MySQL(exec) => exec.to_sql(),
            #[cfg(feature = "mssql")]
            TransactionInsertOrIgnoreExecutor::MSSQL(exec) => exec.to_sql(),
            #[cfg(feature = "duckdb")]
            TransactionInsertOrIgnoreExecutor::DuckDB(exec) => exec.to_sql(),
            TransactionInsertOrIgnoreExecutor::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub async fn execute(self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertOrIgnoreExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            TransactionInsertOrIgnoreExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TransactionInsertOrIgnoreExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TransactionInsertOrIgnoreExecutor::MSSQL(exec) => exec.execute().await,
            #[cfg(feature = "duckdb")]
            TransactionInsertOrIgnoreExecutor::DuckDB(exec) => exec.execute().await,
            TransactionInsertOrIgnoreExecutor::_Phantom(infallible, _) => match infallible {},
        }
    }

    pub fn without_hooks(self) -> crate::WithoutHooksExecutor<Self> {
        crate::WithoutHooksExecutor(self)
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
        #[cfg(feature = "questdb")]
        super::super::DbType::QuestDB => Err(unsupported_feature(
            super::super::DbType::QuestDB,
            "transaction options",
        )),
        #[cfg(feature = "mysql")]
        super::super::DbType::MySQL => {
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
        #[cfg(feature = "clickhouse")]
        super::super::DbType::ClickHouse => Err(unsupported_feature(
            super::super::DbType::ClickHouse,
            "transactions on ClickHouse",
        )),
        #[cfg(feature = "influxdb")]
        super::super::DbType::InfluxDB => Err(unsupported_feature(
            super::super::DbType::InfluxDB,
            "transactions",
        )),
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
        let where_expr = primary_key_filter::<T>(key)?;
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .collect::<Vec<T>>()
            .await?;
        Ok(results.into_iter().next())
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

    pub fn select_sql<T>(
        &mut self,
        sql: impl IntoRawSql,
    ) -> TransactionRawSelectExecutor<'_, 'a, T> {
        TransactionRawSelectExecutor {
            txn: self,
            sql: sql.into_raw_sql(),
            _marker: std::marker::PhantomData,
        }
    }

    pub async fn execute_sql(&mut self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                let (sql, params) = sql.render(super::super::DbType::Sqlite)?;
                txn.exec_raw(&sql, params).await
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                let (sql, params) = sql.render(super::super::DbType::PostgreSQL)?;
                txn.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                let (sql, params) = sql.render(super::super::DbType::MySQL)?;
                txn.exec_raw(&sql, params).await
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                let (sql, params) = sql.render(super::super::DbType::MSSQL)?;
                txn.exec_raw(&sql, params).await
            }
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                let (sql, params) = sql.render(super::super::DbType::DuckDB)?;
                txn.exec_raw(&sql, params).await
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub async fn savepoint<R, F>(&mut self, f: F) -> crate::Result<R>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'a>) -> TransactionFuture<'tx, R>,
    {
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
                if is_mssql {
                    let _ = self
                        .execute_sql(format!("ROLLBACK TRANSACTION {name}"))
                        .await;
                } else {
                    let _ = self
                        .execute_sql(format!("ROLLBACK TO SAVEPOINT {name}"))
                        .await;
                    let _ = self.execute_sql(format!("RELEASE SAVEPOINT {name}")).await;
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
        let where_expr = primary_key_filter::<T>(key)?;

        // 执行查询并取第一条
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .collect::<Vec<T>>()
            .await?;

        Ok(results.into_iter().next())
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
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => GroupedSelectExecutor::Sqlite(txn.select_column::<T, V>()),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                GroupedSelectExecutor::PostgreSQL(txn.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => GroupedSelectExecutor::MySQL(txn.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => GroupedSelectExecutor::MSSQL(txn.select_column::<T, V>()),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => GroupedSelectExecutor::DuckDB(txn.select_column::<T, V>()),
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

    pub fn save<'op, T: WritableModel + crate::model::GraphWritable>(
        &'op mut self,
        model: &'op mut Tracked<T>,
    ) -> TransactionSaveExecutor<'op, 'a, T> {
        TransactionSaveExecutor { txn: self, model }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => TransactionInsertExecutor::Sqlite(txn.insert::<I>(models)),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                TransactionInsertExecutor::PostgreSQL(txn.insert::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => TransactionInsertExecutor::MySQL(txn.insert::<I>(models)),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => TransactionInsertExecutor::MSSQL(txn.insert::<I>(models)),
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => TransactionInsertExecutor::DuckDB(txn.insert::<I>(models)),
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                TransactionInsertOrUpdateExecutor::Sqlite(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                TransactionInsertOrUpdateExecutor::PostgreSQL(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                TransactionInsertOrUpdateExecutor::MySQL(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                TransactionInsertOrUpdateExecutor::MSSQL(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                TransactionInsertOrUpdateExecutor::DuckDB(txn.insert_or_update::<I>(models))
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }

    pub fn upsert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        self.insert_or_update(models)
    }

    /// 插入或忽略记录 - 返回执行器
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                TransactionInsertOrIgnoreExecutor::Sqlite(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                TransactionInsertOrIgnoreExecutor::PostgreSQL(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                TransactionInsertOrIgnoreExecutor::MySQL(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                TransactionInsertOrIgnoreExecutor::MSSQL(txn.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "duckdb")]
            Transaction::DuckDB(txn) => {
                TransactionInsertOrIgnoreExecutor::DuckDB(txn.insert_or_ignore::<I>(models))
            }
            Transaction::_Phantom(infallible, _) => match *infallible {},
        }
    }
}

impl<'a> super::DbExecutor for Transaction<'a> {
    fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        Transaction::select::<T>(self)
    }

    fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
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

/// 统一的 MappedSelectExecutor 枚举
pub enum MappedSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V)>,
    },
}

/// 统一的 GroupedSelectExecutor 枚举
pub enum GroupedSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::GroupedSelectExecutor<'a, T, V>),
    /// ClickHouse 分组聚合：渲染 GROUP BY 聚合 SQL 后走
    /// `select_named_values`（JSONEachRowWithNames）按投影顺序解码。
    #[cfg(feature = "clickhouse")]
    ClickHouse(&'a clickhouse_backend::Database, GroupedSelect<T, V>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V)>,
    },
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.group_by(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.group_by(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.group_by(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.group_by(f)),
            #[cfg(feature = "duckdb")]
            GroupedSelectExecutor::DuckDB(exec) => GroupedSelectExecutor::DuckDB(exec.group_by(f)),
            #[cfg(feature = "clickhouse")]
            GroupedSelectExecutor::ClickHouse(db, select) => {
                GroupedSelectExecutor::ClickHouse(db, select.group_by(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ GroupedSelectExecutor::Unsupported { .. } => unsupported,
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
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.having(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.having(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.having(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.having(f)),
            #[cfg(feature = "duckdb")]
            GroupedSelectExecutor::DuckDB(exec) => GroupedSelectExecutor::DuckDB(exec.having(f)),
            #[cfg(feature = "clickhouse")]
            GroupedSelectExecutor::ClickHouse(db, select) => {
                GroupedSelectExecutor::ClickHouse(db, select.having(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ GroupedSelectExecutor::Unsupported { .. } => unsupported,
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
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.filter(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.filter(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.filter(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.filter(f)),
            #[cfg(feature = "duckdb")]
            GroupedSelectExecutor::DuckDB(exec) => GroupedSelectExecutor::DuckDB(exec.filter(f)),
            #[cfg(feature = "clickhouse")]
            GroupedSelectExecutor::ClickHouse(db, select) => {
                GroupedSelectExecutor::ClickHouse(db, select.filter(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            unsupported @ GroupedSelectExecutor::Unsupported { .. } => unsupported,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => {
                GroupedCollectFuture::Sqlite(exec.collect::<C>())
            }
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedCollectFuture::MySQL(exec.collect::<C>()),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedCollectFuture::MSSQL(exec.collect::<C>()),
            #[cfg(feature = "duckdb")]
            GroupedSelectExecutor::DuckDB(exec) => {
                GroupedCollectFuture::DuckDB(exec.collect::<C>())
            }
            #[cfg(feature = "clickhouse")]
            GroupedSelectExecutor::ClickHouse(db, select) => GroupedCollectFuture::ClickHouse(
                *db,
                select.clone(),
                std::marker::PhantomData,
            ),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            GroupedSelectExecutor::Unsupported {
                backend, feature, ..
            } => GroupedCollectFuture::Unsupported {
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
            GroupedSelectExecutor::Sqlite(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "duckdb")]
            GroupedSelectExecutor::DuckDB(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "clickhouse")]
            GroupedSelectExecutor::ClickHouse(_, _) => Err(unsupported_feature(
                super::super::DbType::ClickHouse,
                "Model select_column on ClickHouse; use select_sql",
            )),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            GroupedSelectExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }
}

impl<'a, T: Model, V> Clone for MappedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => MappedSelectExecutor::Sqlite(exec.clone()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                MappedSelectExecutor::PostgreSQL(exec.clone_with_client())
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                MappedSelectExecutor::MySQL(exec.clone_with_pool())
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                MappedSelectExecutor::MSSQL(exec.clone_with_pool())
            }
            #[cfg(feature = "duckdb")]
            MappedSelectExecutor::DuckDB(exec) => MappedSelectExecutor::DuckDB(exec.clone()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedSelectExecutor::Unsupported {
                backend, feature, ..
            } => MappedSelectExecutor::Unsupported {
                backend: *backend,
                feature: *feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

/// 统一的 MappedCollectFuture 枚举
pub enum MappedCollectFuture<'a, T: Model + 'static, V: 'static, C: FromIterator<V> + 'static> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
    #[doc(hidden)]
    Unsupported {
        backend: super::super::DbType,
        feature: &'static str,
        _marker: std::marker::PhantomData<&'a (T, V, C)>,
    },
}

/// 统一的 GroupedCollectFuture 枚举
pub enum GroupedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "duckdb")]
    DuckDB(duckdb_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "clickhouse")]
    ClickHouse(
        &'a clickhouse_backend::Database,
        GroupedSelect<T, V>,
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

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for GroupedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedCollectFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            GroupedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            GroupedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            GroupedCollectFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "duckdb")]
            GroupedCollectFuture::DuckDB(future) => Box::pin(future.into_future()),
            // 分组投影的输出列名由服务端返回（JSONEachRowWithNames 首行），
            // 按投影顺序解码为 V
            #[cfg(feature = "clickhouse")]
            GroupedCollectFuture::ClickHouse(db, select, _) => Box::pin(async move {
                let (sql, params) = select.try_to_sql_with_params(super::super::DbType::ClickHouse)?;
                let (_, rows) = db
                    .select_named_values(RawSql::new(sql).with_params(params))
                    .await?;
                rows.iter()
                    .map(|values| V::from_row_values(values))
                    .collect()
            }),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            GroupedCollectFuture::Unsupported {
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
        postgresql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mysql")]
    MySQLCollect(
        mysql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mssql")]
    MSSQLCollect(
        mssql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
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
    /// - 单字段：map_to(|r| r.uid) -> MappedSelectExecutor<'a, T, i32>
    /// - 元组：map_to(|r| (r.uid, r.id)) -> MappedSelectExecutor<'a, T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => MappedSelectExecutor::Sqlite(exec.map_to(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => MappedSelectExecutor::PostgreSQL(exec.map_to(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => MappedSelectExecutor::MySQL(exec.map_to(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => MappedSelectExecutor::MSSQL(exec.map_to(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => MappedSelectExecutor::DuckDB(exec.map_to(f)),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, _) => MappedSelectExecutor::Unsupported {
                backend: clickhouse_select_backend_db_type(db),
                feature: "select capability on ClickHouse",
                _marker: std::marker::PhantomData,
            },
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.select_column(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.select_column(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.select_column(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.select_column(f)),
            #[cfg(feature = "duckdb")]
            SelectExecutor::DuckDB(exec) => GroupedSelectExecutor::DuckDB(exec.select_column(f)),
            // 能力矩阵门控：advanced_grouping 为 true 的后端走分组聚合执行分支
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            SelectExecutor::ClickHouse(db, select) => {
                let backend = clickhouse_select_backend_db_type(db);
                if !Capabilities::of(backend).advanced_grouping {
                    // 能力矩阵门控：InfluxQL 的 GROUP BY 仅支持时间桶与 tag
                    #[cfg(feature = "influxdb")]
                    let feature = if backend == super::super::DbType::InfluxDB {
                        "GROUP BY aggregation on InfluxDB (InfluxQL GROUP BY only supports time buckets and tags)"
                    } else {
                        "GROUP BY aggregation"
                    };
                    #[cfg(not(feature = "influxdb"))]
                    let feature = "GROUP BY aggregation";
                    return GroupedSelectExecutor::Unsupported {
                        backend,
                        feature,
                        _marker: std::marker::PhantomData,
                    };
                }
                #[cfg(feature = "clickhouse")]
                if let ClickHouseSelectBackend::ClickHouse(db) = db {
                    return GroupedSelectExecutor::ClickHouse(db, select.select_column(f));
                }
                #[cfg(not(feature = "clickhouse"))]
                {
                    // influxdb-only 构建时能力门控已在上方返回，仅为
                    // 穷尽性保留
                    let _ = (select, f);
                }
                GroupedSelectExecutor::Unsupported {
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

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    pub fn as_model<R: Model>(self) -> crate::Result<DerivedSelect<R>>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => Ok(exec.as_model::<R>()),
            #[cfg(feature = "duckdb")]
            MappedSelectExecutor::DuckDB(exec) => Ok(exec.as_model::<R>()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedSelectExecutor::Unsupported {
                backend, feature, ..
            } => Err(unsupported_feature(backend, feature)),
        }
    }

    pub fn collect<C>(self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => MappedCollectFuture::Sqlite(exec.collect::<C>()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                MappedCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                MappedCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                MappedCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "duckdb")]
            MappedSelectExecutor::DuckDB(exec) => MappedCollectFuture::DuckDB(exec.collect::<C>()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedSelectExecutor::Unsupported {
                backend, feature, ..
            } => MappedCollectFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }

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
            MappedSelectExecutor::Sqlite(exec) => {
                ModelCollectWithFuture::Sqlite(exec.collect_with::<C, F, M>(f))
            }
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                // PostgreSQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_client();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::PostgreSQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                // MySQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_pool();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::MySQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                // MSSQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_pool();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::MSSQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(feature = "duckdb")]
            MappedSelectExecutor::DuckDB(exec) => {
                ModelCollectWithFuture::DuckDB(exec.collect_with::<C, F, M>(f))
            }
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedSelectExecutor::Unsupported {
                backend, feature, ..
            } => ModelCollectWithFuture::Unsupported {
                backend,
                feature,
                _marker: std::marker::PhantomData,
            },
        }
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    fn into_in_filter(self, column: String) -> crate::query::filter::FilterExpr {
        let in_subquery = |subquery: crate::Result<(String, Vec<crate::model::Value>)>| {
            crate::query::filter::FilterExpr::InSubqueryDynamic {
                column,
                subquery: crate::query::filter::DynamicSubquery::new(move |_| subquery.clone()),
            }
        };

        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(feature = "duckdb")]
            MappedSelectExecutor::DuckDB(exec) => in_subquery(exec.to_subquery_sql()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedSelectExecutor::Unsupported {
                backend, feature, ..
            } => in_subquery(Err(crate::OrmerError::UnsupportedFeature {
                backend,
                feature,
            })),
        }
    }
}

// 为 MappedSelectExecutor 实现 IsInValues trait
impl<'a, T: Model, V: crate::query::builder::ColumnValueType> crate::query::builder::IsInValues<V>
    for MappedSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        crate::query::builder::WhereExpr::from_filter(self.into_in_filter(column))
    }
}

// 为 &MappedSelectExecutor 实现 IsInValues trait（引用版本）
impl<'a, 'b, T: Model, V: crate::query::builder::ColumnValueType>
    crate::query::builder::IsInValues<V> for &'b MappedSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        crate::query::builder::WhereExpr::from_filter(self.clone().into_in_filter(column))
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            MappedCollectFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            MappedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            MappedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            MappedCollectFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "duckdb")]
            MappedCollectFuture::DuckDB(future) => Box::pin(future.into_future()),
            #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
            MappedCollectFuture::Unsupported {
                backend, feature, ..
            } => Box::pin(async move { Err(unsupported_feature(backend, feature)) }),
        }
    }
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
            #[cfg(feature = "postgresql")]
            ModelCollectWithFuture::PostgreSQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
            #[cfg(feature = "mysql")]
            ModelCollectWithFuture::MySQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
            #[cfg(feature = "mssql")]
            ModelCollectWithFuture::MSSQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
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

impl<'a, T, V> BatchQuery<'a> for GroupedSelectExecutor<'a, T, V>
where
    T: Model + 'static + Send + Sync,
    V: crate::model::FromRowValues + 'static + Send + Sync,
{
    type Output = Vec<V>;

    fn into_batch_future(self) -> BatchQueryFuture<'a, Self::Output> {
        Box::pin(async move { self.collect::<Vec<V>>().await })
    }
}

impl<'a, T, V> BatchQuery<'a> for MappedSelectExecutor<'a, T, V>
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
