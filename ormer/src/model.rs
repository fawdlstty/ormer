use crate::time::{naive_local_to_utc, utc_to_naive_local};
use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Mutex, OnceLock};

pub type RustDecimal = rust_decimal::Decimal;

pub type BigDecimal = bigdecimal::BigDecimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionInfo {
    pub column: &'static str,
    pub rust_type: &'static str,
    pub initial: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VersionSnapshotKey {
    table: &'static str,
    values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VersionObjectKey {
    table: &'static str,
    address: usize,
}

#[derive(Debug, Clone)]
pub struct VersionSnapshotUpdate {
    key: VersionSnapshotKey,
    object_key: VersionObjectKey,
    version: u64,
}

impl VersionSnapshotUpdate {
    pub fn apply(&self) {
        store_version_snapshot(&self.key, self.object_key, self.version);
    }
}

fn version_snapshots() -> &'static Mutex<HashMap<VersionSnapshotKey, u64>> {
    static SNAPSHOTS: OnceLock<Mutex<HashMap<VersionSnapshotKey, u64>>> = OnceLock::new();
    SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn version_object_snapshots() -> &'static Mutex<HashMap<VersionObjectKey, u64>> {
    static SNAPSHOTS: OnceLock<Mutex<HashMap<VersionObjectKey, u64>>> = OnceLock::new();
    SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn version_snapshot_key<T: Model>(model: &T) -> VersionSnapshotKey {
    let version_column = T::version_info().map(|info| info.column);
    VersionSnapshotKey {
        table: T::TABLE_NAME,
        values: T::columns()
            .iter()
            .filter(|column| Some(**column) != version_column)
            .filter_map(|column| model.column_value(column))
            .map(|value| value.to_table_route_value())
            .collect(),
    }
}

fn version_object_key<T: Model>(model: &T) -> VersionObjectKey {
    VersionObjectKey {
        table: T::TABLE_NAME,
        address: model as *const T as usize,
    }
}

fn version_snapshot_keys<T: Model>(model: &T) -> (VersionSnapshotKey, VersionObjectKey) {
    (version_snapshot_key(model), version_object_key(model))
}

fn store_version_snapshot(key: &VersionSnapshotKey, object_key: VersionObjectKey, version: u64) {
    if let Ok(mut snapshots) = version_snapshots().lock() {
        snapshots.insert(key.clone(), version);
    }
    if let Ok(mut snapshots) = version_object_snapshots().lock() {
        snapshots.insert(object_key, version);
    }
}

fn snapshot_version(key: &VersionSnapshotKey) -> Option<u64> {
    version_snapshots()
        .lock()
        .ok()
        .and_then(|snapshots| snapshots.get(key).copied())
}

fn object_snapshot_version(object_key: VersionObjectKey) -> Option<u64> {
    version_object_snapshots()
        .lock()
        .ok()
        .and_then(|snapshots| snapshots.get(&object_key).copied())
}

pub fn clear_version_snapshots<T: Model>() {
    if let Ok(mut snapshots) = version_snapshots().lock() {
        snapshots.retain(|key, _| key.table != T::TABLE_NAME);
    }
    if let Ok(mut snapshots) = version_object_snapshots().lock() {
        snapshots.retain(|key, _| key.table != T::TABLE_NAME);
    }
}

pub fn record_version_snapshot<T: Model>(model: &T, version: u64) {
    if T::version_info().is_none() {
        return;
    }
    let (key, object_key) = version_snapshot_keys(model);
    store_version_snapshot(&key, object_key, version);
}

pub fn model_version<T: Model>(model: &T) -> u64 {
    let Some(info) = T::version_info() else {
        return 0;
    };
    let (key, object_key) = version_snapshot_keys(model);
    if let Some(version) = object_snapshot_version(object_key) {
        return version;
    }

    let version = snapshot_version(&key).unwrap_or(info.initial);
    if let Ok(mut snapshots) = version_object_snapshots().lock() {
        snapshots.insert(object_key, version);
    }
    version
}

pub fn version_snapshot_update<T: Model>(
    model: &T,
    old_version: u64,
) -> Option<VersionSnapshotUpdate> {
    if T::version_info().is_none() {
        return None;
    }
    let (key, object_key) = version_snapshot_keys(model);
    Some(VersionSnapshotUpdate {
        key,
        object_key,
        version: old_version.saturating_add(1),
    })
}

/// 为 Duration 扩展 PostgreSQL INTERVAL 格式化能力
pub trait DurationToInterval {
    /// 将 Duration 转换为 PostgreSQL INTERVAL 字符串
    /// 支持小数形式，自动选择最适配的单位（milliseconds/seconds/minutes/hours/days）
    fn to_interval_string(&self) -> String;
}

impl DurationToInterval for std::time::Duration {
    fn to_interval_string(&self) -> String {
        let total_secs = self.as_secs_f64();
        let millis = self.subsec_millis();

        if total_secs < 1.0 && millis > 0 {
            format!("{} milliseconds", millis)
        } else if total_secs < 60.0 {
            format_interval_unit(total_secs, "seconds")
        } else if total_secs < 3600.0 {
            format_interval_unit(total_secs / 60.0, "minutes")
        } else if total_secs < 86400.0 {
            format_interval_unit(total_secs / 3600.0, "hours")
        } else {
            format_interval_unit(total_secs / 86400.0, "days")
        }
    }
}

fn format_interval_unit(value: f64, unit: &str) -> String {
    if value.fract() == 0.0 {
        format!("{} {}", value as u64, unit)
    } else {
        format!("{:.3} {}", value, unit)
    }
}

/// 字段元数据
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub rust_name: &'static str,
    pub name: &'static str,
    pub rust_type: &'static str,
    pub is_primary: bool,
    pub is_auto_increment: bool,
    pub is_nullable: bool,
    pub unique_group: Option<i32>, // None表示不唯一，Some(group_id)表示属于哪个唯一键组
    pub unique_name: Option<&'static str>,
    pub is_indexed: bool,
    pub index_group: Option<i32>,
    pub index_name: Option<&'static str>,
    pub index_order: Option<&'static str>,
    pub index_where: Option<&'static str>,
    pub foreign_key: Option<ForeignKeyInfo>, // 外键信息
    pub enum_variants: Option<&'static [&'static str]>, // 枚举类型的变体列表
    pub data_type: Option<&'static str>,     // 数据库类型覆盖
    pub db_value_type: Option<fn(crate::abstract_layer::DbType) -> &'static str>, // 自定义数据库类型
    pub default: Option<ColumnDefault>,                                           // 数据库端默认值
    pub check: Option<CheckConstraint>,                                           // CHECK 约束
    pub hypertable: Option<std::time::Duration>, // TimescaleDB hypertable 分片时长
    pub compress: bool,                          // 是否启用数据库级压缩
    pub compression: Option<CompressionAlgorithm>, // 压缩算法
    pub index_method: Option<&'static str>,      // fulltext、gin 等索引方法
    pub index_expression: Option<&'static str>,  // 函数索引或全文向量表达式
    pub index_columns: Option<&'static str>,     // 复合索引列清单
}

/// 数据库专用表级 DDL 选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableOptions {
    pub mysql_engine: Option<&'static str>,
    pub mysql_charset: Option<&'static str>,
    pub mysql_collation: Option<&'static str>,
    pub postgresql_storage: Option<&'static str>,
    pub postgresql_fillfactor: Option<u8>,
    pub mssql_filegroup: Option<&'static str>,
    pub clickhouse_engine: Option<&'static str>,
    pub clickhouse_order_by: Option<&'static str>,
    pub clickhouse_partition_by: Option<&'static str>,
    pub clickhouse_ttl: Option<&'static str>,
    pub clickhouse_settings: Option<&'static str>,
}

impl Default for TableOptions {
    fn default() -> Self {
        Self::empty()
    }
}

impl TableOptions {
    pub const fn empty() -> Self {
        Self {
            mysql_engine: None,
            mysql_charset: None,
            mysql_collation: None,
            postgresql_storage: None,
            postgresql_fillfactor: None,
            mssql_filegroup: None,
            clickhouse_engine: None,
            clickhouse_order_by: None,
            clickhouse_partition_by: None,
            clickhouse_ttl: None,
            clickhouse_settings: None,
        }
    }
}

/// Build model-declared MySQL table options.
#[cfg(feature = "mysql")]
pub const fn mysql_table_options(
    engine: Option<&'static str>,
    charset: Option<&'static str>,
    collation: Option<&'static str>,
) -> Option<TableOptions> {
    Some(TableOptions {
        mysql_engine: engine,
        mysql_charset: charset,
        mysql_collation: collation,
        ..TableOptions::empty()
    })
}

/// Build model-declared MySQL table options.
///
/// The deprecated marker is intentional: a configured `#[mysql(...)]`
/// attribute must warn when the MySQL backend feature is disabled.
#[cfg(not(feature = "mysql"))]
#[deprecated(note = "#[mysql(...)] is ignored because the ormer `mysql` feature is disabled")]
pub const fn mysql_table_options(
    engine: Option<&'static str>,
    charset: Option<&'static str>,
    collation: Option<&'static str>,
) -> Option<TableOptions> {
    let _ = (engine, charset, collation);
    None
}

/// Build model-declared PostgreSQL table options.
#[cfg(feature = "postgresql")]
pub const fn postgresql_table_options(
    storage: Option<&'static str>,
    fillfactor: Option<u8>,
) -> Option<TableOptions> {
    Some(TableOptions {
        postgresql_storage: storage,
        postgresql_fillfactor: fillfactor,
        ..TableOptions::empty()
    })
}

/// Build model-declared PostgreSQL table options.
///
/// A configured `#[postgresql(...)]` attribute warns without compiling out
/// the model when the PostgreSQL backend feature is disabled.
#[cfg(not(feature = "postgresql"))]
#[deprecated(note = "#[postgresql(...)] is ignored because the ormer `postgresql` feature is disabled")]
pub const fn postgresql_table_options(
    storage: Option<&'static str>,
    fillfactor: Option<u8>,
) -> Option<TableOptions> {
    let _ = (storage, fillfactor);
    None
}

/// Build model-declared MSSQL table options.
#[cfg(feature = "mssql")]
pub const fn mssql_table_options(filegroup: Option<&'static str>) -> Option<TableOptions> {
    Some(TableOptions {
        mssql_filegroup: filegroup,
        ..TableOptions::empty()
    })
}

/// Build model-declared MSSQL table options.
///
/// A configured `#[mssql(...)]` attribute warns without compiling out
/// the model when the MSSQL backend feature is disabled.
#[cfg(not(feature = "mssql"))]
#[deprecated(note = "#[mssql(...)] is ignored because the ormer `mssql` feature is disabled")]
pub const fn mssql_table_options(filegroup: Option<&'static str>) -> Option<TableOptions> {
    let _ = filegroup;
    None
}

/// Build model-declared ClickHouse table options.
#[cfg(feature = "clickhouse")]
pub const fn clickhouse_table_options(
    engine: Option<&'static str>,
    order_by: Option<&'static str>,
    partition_by: Option<&'static str>,
    ttl: Option<&'static str>,
    settings: Option<&'static str>,
) -> Option<TableOptions> {
    Some(TableOptions {
        clickhouse_engine: engine,
        clickhouse_order_by: order_by,
        clickhouse_partition_by: partition_by,
        clickhouse_ttl: ttl,
        clickhouse_settings: settings,
        ..TableOptions::empty()
    })
}

/// Build model-declared ClickHouse table options.
///
/// A configured `#[clickhouse(...)]` attribute warns without compiling out
/// the model when the ClickHouse backend feature is disabled.
#[cfg(not(feature = "clickhouse"))]
#[deprecated(note = "#[clickhouse(...)] is ignored because the ormer `clickhouse` feature is disabled")]
pub const fn clickhouse_table_options(
    engine: Option<&'static str>,
    order_by: Option<&'static str>,
    partition_by: Option<&'static str>,
    ttl: Option<&'static str>,
    settings: Option<&'static str>,
) -> Option<TableOptions> {
    let _ = (engine, order_by, partition_by, ttl, settings);
    None
}

/// Merge dialect-specific model table options into one DDL metadata value.
pub const fn merge_table_options(
    mysql: Option<TableOptions>,
    postgresql: Option<TableOptions>,
    mssql: Option<TableOptions>,
    clickhouse: Option<TableOptions>,
) -> Option<TableOptions> {
    let mut options = TableOptions::empty();
    if let Some(value) = mysql {
        options.mysql_engine = value.mysql_engine;
        options.mysql_charset = value.mysql_charset;
        options.mysql_collation = value.mysql_collation;
    }
    if let Some(value) = postgresql {
        options.postgresql_storage = value.postgresql_storage;
        options.postgresql_fillfactor = value.postgresql_fillfactor;
    }
    if let Some(value) = mssql {
        options.mssql_filegroup = value.mssql_filegroup;
    }
    if let Some(value) = clickhouse {
        options.clickhouse_engine = value.clickhouse_engine;
        options.clickhouse_order_by = value.clickhouse_order_by;
        options.clickhouse_partition_by = value.clickhouse_partition_by;
        options.clickhouse_ttl = value.clickhouse_ttl;
        options.clickhouse_settings = value.clickhouse_settings;
    }

    if options.mysql_engine.is_some()
        || options.mysql_charset.is_some()
        || options.mysql_collation.is_some()
        || options.postgresql_storage.is_some()
        || options.postgresql_fillfactor.is_some()
        || options.mssql_filegroup.is_some()
        || options.clickhouse_engine.is_some()
        || options.clickhouse_order_by.is_some()
        || options.clickhouse_partition_by.is_some()
        || options.clickhouse_ttl.is_some()
        || options.clickhouse_settings.is_some()
    {
        Some(options)
    } else {
        None
    }
}

impl TableOptions {
    fn append_mysql_options(&self, sql: &mut String) {
        let mut parts = Vec::new();
        if let Some(engine) = self.mysql_engine {
            parts.push(format!("ENGINE={}", quote_sql_literal(engine)));
        }
        if let Some(charset) = self.mysql_charset {
            parts.push(format!("DEFAULT CHARSET={}", quote_sql_literal(charset)));
        }
        if let Some(collation) = self.mysql_collation {
            parts.push(format!("COLLATE={}", quote_sql_literal(collation)));
        }
        if !parts.is_empty() {
            sql.push(' ');
            sql.push_str(&parts.join(" "));
        }
    }

    fn append_postgresql_options(&self, sql: &mut String) -> crate::Result<()> {
        let mut parts = Vec::new();
        if let Some(storage) = self.postgresql_storage {
            parts.push(format!("storage = {}", quote_sql_literal(storage)));
        }
        if let Some(fillfactor) = self.postgresql_fillfactor {
            parts.push(format!("fillfactor = {fillfactor}"));
        }
        if parts.is_empty() {
            return Ok(());
        }
        sql.push_str(" WITH (");
        sql.push_str(&parts.join(", "));
        sql.push(')');
        Ok(())
    }

    fn append_mssql_options(&self, sql: &mut String) -> crate::Result<()> {
        if let Some(filegroup) = self.mssql_filegroup {
            if filegroup.contains('\'') {
                return Err(crate::ormer_error!(
                    "MSSQL filegroup must not contain a single quote"
                ));
            }
            sql.push_str(" ON ");
            sql.push_str(filegroup);
        }
        Ok(())
    }

    fn clickhouse_engine_clause(&self) -> Option<&'static str> {
        self.clickhouse_engine
            .map(str::trim)
            .filter(|engine| !engine.is_empty())
    }

    fn append_clickhouse_options(&self, sql: &mut String) -> crate::Result<()> {
        for (label, clause) in [
            ("ORDER BY", self.clickhouse_order_by),
            ("PARTITION BY", self.clickhouse_partition_by),
            ("TTL", self.clickhouse_ttl),
            ("SETTINGS", self.clickhouse_settings),
        ] {
            let Some(clause) = clause.map(str::trim).filter(|clause| !clause.is_empty()) else {
                continue;
            };
            if clause.contains(';') {
                return Err(crate::ormer_error!(
                    "ClickHouse {label} clause must not contain ';'"
                ));
            }
            sql.push(' ');
            sql.push_str(label);
            sql.push(' ');
            sql.push_str(clause);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionAlgorithm {
    Pglz,
    Lz4,
    Zlib,
    Zstd,
}

impl CompressionAlgorithm {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pglz => "pglz",
            Self::Lz4 => "lz4",
            Self::Zlib => "zlib",
            Self::Zstd => "zstd",
        }
    }

    pub const fn as_upper_str(self) -> &'static str {
        match self {
            Self::Pglz => "PGLZ",
            Self::Lz4 => "LZ4",
            Self::Zlib => "ZLIB",
            Self::Zstd => "ZSTD",
        }
    }
}

pub fn column_compression_algorithm(column: &ColumnSchema) -> Option<CompressionAlgorithm> {
    column
        .compression
        .or_else(|| column.compress.then_some(CompressionAlgorithm::Pglz))
}

/// 字段默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnDefault {
    String(&'static str),
    Number(&'static str),
    Boolean(bool),
    Expression(&'static str),
}

impl ColumnDefault {
    pub fn to_sql(self, db_type: crate::abstract_layer::DbType) -> String {
        match self {
            Self::String(value) => quote_sql_literal(value),
            Self::Number(value) => value.to_string(),
            Self::Boolean(value) => match db_type {
                #[cfg(feature = "sqlite")]
                crate::abstract_layer::DbType::Sqlite => if value { "1" } else { "0" }.to_string(),
                #[cfg(feature = "postgresql")]
                crate::abstract_layer::DbType::PostgreSQL => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
                #[cfg(feature = "mysql")]
                crate::abstract_layer::DbType::MySQL => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
                #[cfg(feature = "mssql")]
                crate::abstract_layer::DbType::MSSQL => if value { "1" } else { "0" }.to_string(),
                #[cfg(feature = "duckdb")]
                crate::abstract_layer::DbType::DuckDB => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
                #[cfg(feature = "clickhouse")]
                crate::abstract_layer::DbType::ClickHouse => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
            },
            Self::Expression(expr) => expr.to_string(),
        }
    }
}

/// 字段 CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckConstraint {
    pub name: Option<&'static str>,
    pub expr: &'static str,
}

/// Runtime values used to render table name templates such as `orders_{tenant_id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableRoute {
    values: HashMap<String, String>,
}

impl TableRoute {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, value: impl TableRouteValue) -> Self {
        self.insert(key, value);
        self
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl TableRouteValue) {
        self.values.insert(key.into(), value.to_table_route_value());
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn merge_missing(&mut self, other: TableRoute) {
        for (key, value) in other.values {
            self.values.entry(key).or_insert(value);
        }
    }
}

/// Converts a value into an identifier segment for table routing.
pub trait TableRouteValue {
    fn to_table_route_value(&self) -> String;
}

macro_rules! impl_table_route_value_to_string {
    ($($t:ty),* $(,)?) => {
        $(
            impl TableRouteValue for $t {
                fn to_table_route_value(&self) -> String {
                    self.to_string()
                }
            }
        )*
    };
}

impl_table_route_value_to_string!(
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    bool,
    f32,
    f64,
    rust_decimal::Decimal,
    bigdecimal::BigDecimal,
);

impl TableRouteValue for str {
    fn to_table_route_value(&self) -> String {
        self.to_string()
    }
}

impl TableRouteValue for String {
    fn to_table_route_value(&self) -> String {
        self.clone()
    }
}

impl<T: TableRouteValue + ?Sized> TableRouteValue for &T {
    fn to_table_route_value(&self) -> String {
        (*self).to_table_route_value()
    }
}

impl TableRouteValue for Value {
    fn to_table_route_value(&self) -> String {
        match self {
            Value::Integer(value) => value.to_string(),
            Value::BigInt(value) => value.to_string(),
            Value::Duration(value) => value.as_micros().to_string(),
            Value::Text(value) => value.clone(),
            Value::TextArray(value) => value.join("_"),
            Value::Real(value) => value.to_string().replace('.', "_"),
            Value::Decimal(value) | Value::BigDecimal(value) => value.replace('.', "_"),
            Value::Boolean(value) => {
                if *value {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Value::Bytes(value) => value.iter().map(|byte| format!("{byte:02x}")).collect(),
            Value::IntegerArray(value) => value
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("_"),
            Value::BigIntArray(value) => value
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("_"),
            Value::NullableBigIntArray(value) => value
                .iter()
                .map(|value| {
                    value
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "null".to_string())
                })
                .collect::<Vec<_>>()
                .join("_"),
            Value::DateTime(value) => value.timestamp_millis().to_string(),
            Value::Date(value) => value.format("%Y%m%d").to_string(),
            Value::Time(value) => value.format("%H%M%S").to_string(),
            Value::Json(value) => value.to_string(),
            Value::Uuid(value) => value.to_string().replace('-', "_"),
            Value::Null => "null".to_string(),
        }
    }
}

/// 外键信息
#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub name: Option<&'static str>,                  // 外键约束名
    pub ref_table: &'static str,                     // 引用的表名
    pub ref_column: &'static str,                    // 引用的列名（对于静态指定的情况）
    pub ref_column_fn: Option<fn() -> &'static str>, // 运行时获取列名的函数（对于自动关联主键的情况）
    pub on_delete: Option<ForeignKeyAction>,         // ON DELETE 动作
    pub on_update: Option<ForeignKeyAction>,         // ON UPDATE 动作
}

impl ForeignKeyInfo {
    /// 获取引用列名
    pub fn get_ref_column(&self) -> &'static str {
        if let Some(fn_get) = self.ref_column_fn {
            fn_get()
        } else {
            self.ref_column
        }
    }
}

/// 外键动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignKeyAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ForeignKeyAction {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

/// 关系方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    HasMany,
    BelongsTo,
    HasOne,
    Through,
}

/// through 关系路径元数据。
#[derive(Debug, Clone)]
pub struct ThroughInfo {
    pub via_relation: &'static str,
    pub target_relation: &'static str,
}

/// 关系元数据
#[derive(Debug, Clone)]
pub struct RelationInfo {
    pub name: &'static str,
    pub kind: RelationKind,
    pub target_table: &'static str,
    pub local_key: &'static str,
    pub target_key: &'static str,
    pub through: Option<ThroughInfo>,
}

#[derive(Debug, Clone, Copy)]
pub enum RelationPathInfo {
    Direct {
        relation: &'static RelationInfo,
    },
    Through {
        relation: &'static RelationInfo,
        via_relation: &'static RelationInfo,
        target_relation: &'static RelationInfo,
    },
}

pub trait RelationHandle<Owner: Model, Target: Model>: Clone {
    type Via: Model + Clone + 'static;

    fn path_info(&self) -> crate::Result<RelationPathInfo>;
}

pub trait RelationSelection<Owner: Model>: Clone {
    type Target: Model + Clone + 'static;
    type Via: Model + Clone + 'static;

    fn path_info(&self) -> crate::Result<RelationPathInfo>;

    fn filters(&self) -> &[crate::query::filter::FilterExpr] {
        &[]
    }

    fn order_by(&self) -> &[crate::query::filter::OrderBy] {
        &[]
    }

    fn range_start(&self) -> Option<usize> {
        None
    }

    fn range_end(&self) -> Option<usize> {
        None
    }
}

/// 类型化关系句柄，用于 include/preload/find_related。
#[derive(Debug)]
pub struct Relation<Owner: Model, Target: Model> {
    name: &'static str,
    _marker: PhantomData<(Owner, Target)>,
}

impl<Owner: Model, Target: Model> Copy for Relation<Owner, Target> {}

impl<Owner: Model, Target: Model> Clone for Relation<Owner, Target> {
    fn clone(&self) -> Self {
        *self
    }
}

macro_rules! relation_query_entrypoints {
    () => {
        pub fn filter<F, W>(self, f: F) -> RelationQuery<Owner, Target, Self>
        where
            F: FnOnce(Target::Where) -> W,
            W: Into<crate::query::builder::WhereExpr>,
        {
            RelationQuery::new(self).filter(f)
        }

        pub fn order_by<F, O>(self, f: F) -> RelationQuery<Owner, Target, Self>
        where
            F: FnOnce(Target::Where) -> O,
            O: Into<crate::query::filter::OrderBy>,
        {
            RelationQuery::new(self).order_by(f)
        }

        pub fn order_by_desc<F, O>(self, f: F) -> RelationQuery<Owner, Target, Self>
        where
            F: FnOnce(Target::Where) -> O,
            O: Into<crate::query::filter::OrderBy>,
        {
            RelationQuery::new(self).order_by_desc(f)
        }

        pub fn range<R>(self, range: R) -> RelationQuery<Owner, Target, Self>
        where
            R: Into<crate::query::builder::RangeBounds>,
        {
            RelationQuery::new(self).range(range)
        }

        pub fn include<I, F>(self, f: F) -> RelationQuery<Owner, Target, Self, I>
        where
            F: FnOnce(Target::Where) -> I,
        {
            RelationQuery::new(self).include(f)
        }
    };
}

impl<Owner: Model, Target: Model> Relation<Owner, Target> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _marker: PhantomData,
        }
    }

    pub fn info(&self) -> crate::Result<&'static RelationInfo> {
        Owner::RELATIONS
            .iter()
            .find(|relation| {
                relation.name == self.name && relation.target_table == Target::TABLE_NAME
            })
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Relation {} -> {} not found on {}",
                    self.name,
                    Target::TABLE_NAME,
                    Owner::TABLE_NAME
                )
            })
    }

    pub fn any<F, W>(self, f: F) -> crate::query::builder::WhereExpr
    where
        F: FnOnce(Target::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        let relation = self.info().expect("relation metadata not found");
        crate::query::builder::WhereExpr::from_filter(
            crate::query::filter::FilterExpr::RelationExists {
                owner_table: Owner::TABLE_NAME,
                owner_key: relation.local_key,
                target_table: Target::TABLE_NAME,
                target_key: relation.target_key,
                filter: Some(Box::new(crate::query::filter::FilterExpr::from(
                    f(Target::Where::default()).into(),
                ))),
            },
        )
    }

    relation_query_entrypoints!();
}

impl<Owner: Model, Target: Model + Clone + 'static> RelationHandle<Owner, Target>
    for Relation<Owner, Target>
{
    type Via = Target;

    fn path_info(&self) -> crate::Result<RelationPathInfo> {
        Ok(RelationPathInfo::Direct {
            relation: self.info()?,
        })
    }
}

impl<Owner: Model, Target: Model + Clone + 'static> RelationSelection<Owner>
    for Relation<Owner, Target>
{
    type Target = Target;
    type Via = Target;

    fn path_info(&self) -> crate::Result<RelationPathInfo> {
        <Self as RelationHandle<Owner, Target>>::path_info(self)
    }
}

/// 类型化 through 关系句柄，用于多对多或跨中间模型的关系路径。
#[derive(Debug)]
pub struct ThroughRelation<Owner: Model, Via: Model, Target: Model> {
    name: &'static str,
    via_relation: &'static str,
    target_relation: &'static str,
    _marker: PhantomData<(Owner, Via, Target)>,
}

impl<Owner: Model, Via: Model, Target: Model> Copy for ThroughRelation<Owner, Via, Target> {}

impl<Owner: Model, Via: Model, Target: Model> Clone for ThroughRelation<Owner, Via, Target> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Owner: Model, Via: Model, Target: Model> ThroughRelation<Owner, Via, Target> {
    pub const fn new(
        name: &'static str,
        via_relation: &'static str,
        target_relation: &'static str,
    ) -> Self {
        Self {
            name,
            via_relation,
            target_relation,
            _marker: PhantomData,
        }
    }

    pub fn info(&self) -> crate::Result<&'static RelationInfo> {
        Owner::RELATIONS
            .iter()
            .find(|relation| {
                relation.name == self.name && relation.target_table == Target::TABLE_NAME
            })
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Relation {} -> {} not found on {}",
                    self.name,
                    Target::TABLE_NAME,
                    Owner::TABLE_NAME
                )
            })
    }

    fn via_info(&self) -> crate::Result<&'static RelationInfo> {
        Owner::RELATIONS
            .iter()
            .find(|relation| {
                relation.name == self.via_relation && relation.target_table == Via::TABLE_NAME
            })
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Through relation {} -> {} not found on {}",
                    self.via_relation,
                    Via::TABLE_NAME,
                    Owner::TABLE_NAME
                )
            })
    }

    fn target_info(&self) -> crate::Result<&'static RelationInfo> {
        Via::RELATIONS
            .iter()
            .find(|relation| {
                relation.name == self.target_relation && relation.target_table == Target::TABLE_NAME
            })
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Through target relation {} -> {} not found on {}",
                    self.target_relation,
                    Target::TABLE_NAME,
                    Via::TABLE_NAME
                )
            })
    }

    pub fn any<F, W>(self, f: F) -> crate::query::builder::WhereExpr
    where
        F: FnOnce(Target::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        let via_relation = self
            .via_info()
            .expect("through relation metadata not found");
        let target_relation = self
            .target_info()
            .expect("through target relation metadata not found");
        crate::query::builder::WhereExpr::from_filter(
            crate::query::filter::FilterExpr::ThroughRelationExists {
                owner_table: Owner::TABLE_NAME,
                owner_key: via_relation.local_key,
                via_table: Via::TABLE_NAME,
                via_owner_key: via_relation.target_key,
                via_target_key: target_relation.local_key,
                target_table: Target::TABLE_NAME,
                target_key: target_relation.target_key,
                filter: Some(Box::new(crate::query::filter::FilterExpr::from(
                    f(Target::Where::default()).into(),
                ))),
            },
        )
    }

    relation_query_entrypoints!();
}

impl<Owner: Model, Via: Model + Clone + 'static, Target: Model> RelationHandle<Owner, Target>
    for ThroughRelation<Owner, Via, Target>
{
    type Via = Via;

    fn path_info(&self) -> crate::Result<RelationPathInfo> {
        Ok(RelationPathInfo::Through {
            relation: self.info()?,
            via_relation: self.via_info()?,
            target_relation: self.target_info()?,
        })
    }
}

impl<Owner: Model, Via: Model + Clone + 'static, Target: Model + Clone + 'static>
    RelationSelection<Owner> for ThroughRelation<Owner, Via, Target>
{
    type Target = Target;
    type Via = Via;

    fn path_info(&self) -> crate::Result<RelationPathInfo> {
        <Self as RelationHandle<Owner, Target>>::path_info(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NoInclude;

#[derive(Debug)]
pub struct RelationQuery<Owner: Model, Target: Model, Handle, Nested = NoInclude> {
    handle: Handle,
    filters: Vec<crate::query::filter::FilterExpr>,
    order_by: Vec<crate::query::filter::OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    nested: Nested,
    _marker: PhantomData<(Owner, Target)>,
}

impl<Owner, Target, Handle, Nested> Clone for RelationQuery<Owner, Target, Handle, Nested>
where
    Owner: Model,
    Target: Model,
    Handle: Clone,
    Nested: Clone,
{
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            nested: self.nested.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Owner: Model, Target: Model, Handle> RelationQuery<Owner, Target, Handle, NoInclude> {
    pub fn new(handle: Handle) -> Self {
        Self {
            handle,
            filters: Vec::new(),
            order_by: Vec::new(),
            range_start: None,
            range_end: None,
            nested: NoInclude,
            _marker: PhantomData,
        }
    }
}

impl<Owner: Model, Target: Model, Handle, Nested> RelationQuery<Owner, Target, Handle, Nested> {
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(Target::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        self.filters.push(crate::query::filter::FilterExpr::from(
            f(Target::Where::default()).into(),
        ));
        self
    }

    pub fn order_by<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(Target::Where) -> O,
        O: Into<crate::query::filter::OrderBy>,
    {
        self.order_by.push(f(Target::Where::default()).into());
        self
    }

    pub fn order_by_desc<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(Target::Where) -> O,
        O: Into<crate::query::filter::OrderBy>,
    {
        let mut order = f(Target::Where::default()).into();
        order.direction = crate::query::filter::OrderDirection::Desc;
        self.order_by.push(order);
        self
    }

    pub fn range<R>(mut self, range: R) -> Self
    where
        R: Into<crate::query::builder::RangeBounds>,
    {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    pub fn include<I, F>(self, f: F) -> RelationQuery<Owner, Target, Handle, I>
    where
        F: FnOnce(Target::Where) -> I,
    {
        RelationQuery {
            handle: self.handle,
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            nested: f(Target::Where::default()),
            _marker: PhantomData,
        }
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn nested(&self) -> &Nested {
        &self.nested
    }
}

impl<Owner, Target, Handle, Nested> RelationSelection<Owner>
    for RelationQuery<Owner, Target, Handle, Nested>
where
    Owner: Model,
    Target: Model + Clone + 'static,
    Handle: RelationHandle<Owner, Target> + Clone,
    Nested: Clone,
{
    type Target = Target;
    type Via = Handle::Via;

    fn path_info(&self) -> crate::Result<RelationPathInfo> {
        self.handle.path_info()
    }

    fn filters(&self) -> &[crate::query::filter::FilterExpr] {
        &self.filters
    }

    fn order_by(&self) -> &[crate::query::filter::OrderBy] {
        &self.order_by
    }

    fn range_start(&self) -> Option<usize> {
        self.range_start
    }

    fn range_end(&self) -> Option<usize> {
        self.range_end
    }
}

#[derive(Debug, Clone)]
pub struct EmbedColumnSchema {
    pub rust_name: &'static str,
    pub name: &'static str,
    pub rust_type: &'static str,
    pub is_nullable: bool,
    pub enum_variants: Option<&'static [&'static str]>,
    pub data_type: Option<&'static str>,
    pub db_value_type: Option<fn(crate::abstract_layer::DbType) -> &'static str>,
    pub compress: bool,
    pub compression: Option<CompressionAlgorithm>,
}

pub trait Embed: Sized {
    const COLUMNS: &'static [&'static str];
    const COLUMN_SCHEMA: &'static [EmbedColumnSchema];
    type Where: EmbedWhere + Default;

    fn from_row(row: &Row, prefix: &str) -> crate::Result<Self>;
    fn from_row_values(values: &[Value]) -> crate::Result<Self>;
    fn field_values(&self) -> Vec<Value>;
    fn column_value(&self, column: &str) -> Option<Value>;
    fn assign_column_value(&mut self, column: &str, value: Value) -> crate::Result<()>;
}

pub trait EmbedWhere {
    fn new_with_prefix(prefix: &str) -> Self;
}

/// 数据库后端 trait - 用于 SQL 类型映射
pub trait DbBackendTypeMapper {
    /// 根据 Rust 类型获取 SQL 类型
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String;
}

/// 自定义数据库值类型。
///
/// 自定义类型的运行时绑定和解码仍通过已有 `Value` 分支完成；`db_type`
/// 只用于建表 SQL 的后端类型名。
pub trait DbValue: Sized {
    fn to_value(&self) -> Value;
    fn from_value(value: &Value) -> crate::Result<Self>;
    fn db_type(db_type: crate::abstract_layer::DbType) -> &'static str;
}

/// 主键 trait - 用于 find_by_id 方法
/// 支持单主键和复合主键
pub trait PrimaryKey: Sized {
    fn into_values(self) -> Vec<Value>;
}

impl<T: Into<Value>> PrimaryKey for T {
    fn into_values(self) -> Vec<Value> {
        vec![self.into()]
    }
}

impl PrimaryKey for &uuid::Uuid {
    fn into_values(self) -> Vec<Value> {
        vec![Value::Uuid(*self)]
    }
}

macro_rules! impl_primary_key_tuple {
    ($($name:ident : $idx:tt),+ $(,)?) => {
        impl<$($name),+> PrimaryKey for ($($name,)+)
        where
            $($name: Into<Value>),+
        {
            fn into_values(self) -> Vec<Value> {
                vec![$(self.$idx.into()),+]
            }
        }
    };
}

impl_primary_key_tuple!(A: 0, B: 1);
impl_primary_key_tuple!(A: 0, B: 1, C: 2);

/// 主键 Rust 字段元数据。
pub trait PrimaryFields {
    type Fields;

    fn primary_field_names() -> Vec<&'static str>;
    fn promary_fields(&self) -> Self::Fields;
}

/// 只读模型 trait，用于 view、DTO、raw SQL 结果和查询投影。
pub trait ViewModel: Sized {
    const TABLE_NAME: &'static str;
    const COLUMNS: &'static [&'static str];
    const COLUMN_SCHEMA: &'static [ColumnSchema];
    const TABLE_OPTIONS: Option<TableOptions> = None;

    /// 获取指定数据库后端实际使用的表名。
    fn table_name_for_db(db_type: crate::abstract_layer::DbType) -> &'static str {
        normalize_table_name_for_db(db_type, Self::TABLE_NAME)
    }

    /// 获取 hypertable 时间字段名和分片时长（如果有）
    fn hypertable_info() -> Option<(&'static str, std::time::Duration)> {
        for col in Self::column_schema() {
            if let Some(duration) = col.hypertable {
                return Some((col.name, duration));
            }
        }
        None
    }

    type QueryBuilder;
    type Where: Default;

    fn columns() -> Vec<&'static str> {
        Self::COLUMNS.to_vec()
    }

    fn column_schema() -> Vec<ColumnSchema> {
        Self::COLUMN_SCHEMA.to_vec()
    }

    /// 获取主键 Rust 字段名列表。
    fn primary_field_names() -> Vec<&'static str> {
        Self::COLUMN_SCHEMA
            .iter()
            .filter(|column| column.is_primary)
            .map(|column| column.rust_name)
            .collect()
    }

    fn query() -> Self::QueryBuilder;
    fn select() -> Self::QueryBuilder;
    fn from_row(row: &Row) -> crate::Result<Self>;
    fn from_row_values(values: &[Value]) -> crate::Result<Self>;
}

/// 可写模型 marker，用于表、迁移、写入相关 API。
pub trait WritableModel: Model {}

/// 模型 trait,所有 ORM 模型必须实现
pub trait Model: Sized {
    const TABLE_NAME: &'static str;
    const COLUMNS: &'static [&'static str];
    const COLUMN_SCHEMA: &'static [ColumnSchema];
    const TABLE_OPTIONS: Option<TableOptions> = None;
    const RELATIONS: &'static [RelationInfo] = &[];

    /// 获取指定数据库后端实际使用的表名。
    fn table_name_for_db(db_type: crate::abstract_layer::DbType) -> &'static str {
        normalize_table_name_for_db(db_type, Self::TABLE_NAME)
    }

    /// 从模型实例中提取表路由变量。
    fn table_route(&self) -> crate::Result<TableRoute> {
        let mut variables = table_route_variables(Self::TABLE_NAME);
        if let Some(route_key) = Self::hypertable_route_key() {
            if !variables.iter().any(|variable| variable == route_key) {
                variables.push(route_key.to_string());
            }
        }
        let mut route = TableRoute::new();
        for variable in variables {
            let value = self.column_value(&variable).ok_or_else(|| {
                crate::ormer_error!(
                    "Table route value {} not found on model {}",
                    variable,
                    Self::TABLE_NAME
                )
            })?;
            route.insert(variable, value);
        }
        Ok(route)
    }

    /// 获取 hypertable 时间字段名和分片时长（如果有）
    fn hypertable_info() -> Option<(&'static str, std::time::Duration)> {
        for col in Self::column_schema() {
            if let Some(duration) = col.hypertable {
                return Some((col.name, duration));
            }
        }
        None
    }

    /// 获取 TimescaleDB 字符串拆表路由键（如果有）。
    fn hypertable_route_key() -> Option<&'static str> {
        None
    }

    /// 自增主键类型
    /// 如果模型有自增主键（#[primary(auto)]），此类型为主键的 Rust 类型（如 i32, i64）
    /// 如果没有自增主键，此类型为 ()
    type AutoIncrementKeyType: Default + 'static;

    type QueryBuilder;
    type Where: Default;
    type Update: Default + crate::query::update::UpdateFields;

    fn columns() -> Vec<&'static str> {
        Self::COLUMNS.to_vec()
    }

    fn column_schema() -> Vec<ColumnSchema> {
        Self::COLUMN_SCHEMA.to_vec()
    }

    /// 获取主键 Rust 字段名列表。
    fn primary_field_names() -> Vec<&'static str> {
        Self::COLUMN_SCHEMA
            .iter()
            .filter(|column| column.is_primary)
            .map(|column| column.rust_name)
            .collect()
    }

    fn version_info() -> Option<VersionInfo> {
        None
    }

    fn table_options() -> Option<TableOptions> {
        Self::TABLE_OPTIONS
    }

    fn query() -> Self::QueryBuilder;
    fn select() -> Self::QueryBuilder;
    fn from_row(row: &Row) -> crate::Result<Self>;
    fn from_row_values(values: &[Value]) -> crate::Result<Self>;

    /// 通过 Rust 字段名查找实际 SQL 列名。
    fn column_name_for_field(field: &str) -> Option<&'static str> {
        Self::column_schema()
            .iter()
            .find(|column| column.rust_name == field || column.name == field)
            .map(|column| column.name)
    }

    /// 获取字段值 (用于 INSERT/UPDATE)
    fn field_values(&self) -> Vec<Value>;

    /// 获取指定列的值。
    fn column_value(&self, _column: &str) -> Option<Value> {
        None
    }

    /// 按列名写入字段值。派生模型会为普通字段和嵌入字段生成实现。
    fn assign_column_value(&mut self, _column: &str, _value: Value) -> crate::Result<()> {
        Err(crate::ormer_error!(
            "Column assignment is not supported on {}",
            Self::TABLE_NAME
        ))
    }

    /// 获取关系本地键值。
    fn relation_key_value(&self, relation: &RelationInfo) -> crate::Result<Value> {
        self.column_value(relation.local_key).ok_or_else(|| {
            crate::ormer_error!(
                "Column {} not found on {} for relation {}",
                relation.local_key,
                Self::TABLE_NAME,
                relation.name
            )
        })
    }

    /// 写入预加载后的关系对象。
    fn assign_relation<Target: Model + 'static>(
        &mut self,
        relation_name: &'static str,
        values: Vec<Target>,
    ) -> crate::Result<()> {
        let _ = values;
        Err(crate::ormer_error!(
            "Relation {} is not assignable on {}",
            relation_name,
            Self::TABLE_NAME
        ))
    }

    fn graph_relations_mut(&mut self) -> Vec<GraphRelationMut<'_>> {
        Vec::new()
    }

    /// 获取主键字段名列表（支持单主键和复合主键）
    fn primary_key_columns() -> &'static [&'static str] {
        // 默认实现返回空，要求派生宏生成
        &[]
    }

    /// 获取主键值列表（支持单主键和复合主键）
    fn primary_key_values(&self) -> Vec<Value>;

    /// 获取主键字段名（已废弃，请使用 primary_key_columns）
    #[deprecated(since = "0.2.0", note = "Please use `primary_key_columns()` instead")]
    fn primary_key_column() -> &'static str {
        Self::primary_key_columns()[0]
    }

    /// 获取主键值（已废弃，请使用 primary_key_values）
    #[deprecated(since = "0.2.0", note = "Please use `primary_key_values()` instead")]
    fn primary_key_value(&self) -> Value {
        self.primary_key_values()[0].clone()
    }

    /// 获取非主键字段的 (列名, 值) 对，用于 set_model
    fn non_pk_field_values(&self) -> Vec<(&'static str, Value)> {
        let all_values = self.field_values();
        Self::column_schema()
            .iter()
            .filter(|col| !col.is_primary)
            .filter_map(|col| {
                Self::columns()
                    .iter()
                    .position(|c| *c == col.name)
                    .and_then(|idx| {
                        if idx < all_values.len() {
                            Some((col.name, all_values[idx].clone()))
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    /// 获取指定非主键字段的 (列名, 值) 对，用于 set_model_fields。
    fn non_pk_field_values_for_columns(&self, columns: &[String]) -> Vec<(&'static str, Value)> {
        self.non_pk_field_values()
            .into_iter()
            .filter(|(col, _)| columns.iter().any(|selected| selected == col))
            .collect()
    }

    /// 获取需要插入的列名（排除自增主键）
    fn insert_columns() -> Vec<&'static str> {
        Self::column_schema()
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.name)
            .collect()
    }

    /// 获取需要插入的字段值（排除自增主键）
    fn insert_values(&self) -> Vec<Value> {
        let all_values = self.field_values();
        Self::column_schema()
            .iter()
            .filter(|col| !col.is_auto_increment)
            .filter_map(|col| {
                // 找到原始字段值中对应的索引
                Self::columns()
                    .iter()
                    .position(|c| *c == col.name)
                    .and_then(|original_idx| {
                        if original_idx < all_values.len() {
                            Some(all_values[original_idx].clone())
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Tracked<T: Model> {
    model: T,
    snapshot: Vec<(&'static str, Value)>,
    original: Option<T>,
    clone_model: Option<fn(&T) -> T>,
}

impl<T: Model> Tracked<T> {
    pub fn as_model(&self) -> &T {
        &self.model
    }

    pub fn as_model_mut(&mut self) -> &mut T {
        &mut self.model
    }

    pub fn into_inner(self) -> T {
        self.model
    }

    pub fn dirty_columns(&self) -> Vec<String> {
        tracked_field_values(&self.model)
            .into_iter()
            .filter_map(|(column, value)| {
                let original =
                    self.snapshot
                        .iter()
                        .find_map(|(snapshot_column, snapshot_value)| {
                            (*snapshot_column == column).then_some(snapshot_value)
                        });
                match original {
                    Some(original) if values_equal(original, &value) => None,
                    _ => Some(column.to_string()),
                }
            })
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_columns().is_empty()
    }
}

impl<T: Model> Tracked<T> {
    pub fn new(model: T) -> Self {
        let snapshot = tracked_field_values(&model);
        Self {
            model,
            snapshot,
            original: None,
            clone_model: None,
        }
    }
}

impl<T: Model + Clone> Tracked<T> {
    pub fn new_graph(model: T) -> Self {
        let snapshot = tracked_field_values(&model);
        let original = model.clone();
        Self {
            model,
            snapshot,
            original: Some(original),
            clone_model: Some(clone_model::<T>),
        }
    }
}

impl<T: Model> Tracked<T> {
    pub fn accept_changes(&mut self) {
        self.snapshot = tracked_field_values(&self.model);
        if let Some(clone_model) = self.clone_model {
            self.original = Some(clone_model(&self.model));
        }
    }

    pub(crate) async fn sync_graph_relations<'tx>(
        &mut self,
        tx: &mut crate::abstract_layer::Transaction<'tx>,
    ) -> crate::Result<u64>
    where
        T: GraphWritable,
    {
        let Some(original) = self.original.as_mut() else {
            return Ok(0);
        };
        <T as GraphWritable>::sync_tracked_graph_relations(tx, original, &mut self.model).await
    }
}

fn clone_model<T: Clone>(model: &T) -> T {
    model.clone()
}

impl<T: Model> std::ops::Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

impl<T: Model> std::ops::DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.model
    }
}

pub trait TrackableModel: Model + Clone + Sized {
    fn track(self) -> Tracked<Self> {
        Tracked::new_graph(self)
    }
}

impl<T: Model + Clone> TrackableModel for T {}

fn tracked_field_values<T: Model>(model: &T) -> Vec<(&'static str, Value)> {
    let version_column = T::version_info().map(|info| info.column);
    model
        .non_pk_field_values()
        .into_iter()
        .filter(|(column, _)| Some(*column) != version_column)
        .collect()
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::BigInt(left), Value::BigInt(right)) => left == right,
        (Value::Duration(left), Value::Duration(right)) => left == right,
        (Value::Text(left), Value::Text(right)) => left == right,
        (Value::TextArray(left), Value::TextArray(right)) => left == right,
        (Value::Real(left), Value::Real(right)) => left == right,
        (Value::Decimal(left), Value::Decimal(right)) => left == right,
        (Value::BigDecimal(left), Value::BigDecimal(right)) => left == right,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::Bytes(left), Value::Bytes(right)) => left == right,
        (Value::IntegerArray(left), Value::IntegerArray(right)) => left == right,
        (Value::BigIntArray(left), Value::BigIntArray(right)) => left == right,
        (Value::NullableBigIntArray(left), Value::NullableBigIntArray(right)) => left == right,
        (Value::DateTime(left), Value::DateTime(right)) => left == right,
        (Value::Date(left), Value::Date(right)) => left == right,
        (Value::Time(left), Value::Time(right)) => left == right,
        (Value::Json(left), Value::Json(right)) => left == right,
        (Value::Uuid(left), Value::Uuid(right)) => left == right,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

pub fn model_scalar_values_equal<T: Model>(left: &T, right: &T) -> bool {
    let left_values = left.field_values();
    let right_values = right.field_values();
    left_values.len() == right_values.len()
        && left_values
            .iter()
            .zip(right_values.iter())
            .all(|(left, right)| values_equal(left, right))
}

#[derive(Debug, Clone, Default)]
pub struct GraphRelationDiff {
    pub deleted: Vec<usize>,
    pub inserted: Vec<usize>,
    pub matched: Vec<(usize, usize)>,
}

fn model_primary_key_sort_key<T: Model>(model: &T) -> Vec<String> {
    model
        .primary_key_values()
        .into_iter()
        .map(|value| crate::abstract_layer::common::common_helpers::model_value_key(&value))
        .collect()
}

pub fn graph_relation_diff<T: Model>(original: &mut [T], current: &mut [T]) -> GraphRelationDiff {
    original.sort_by_cached_key(model_primary_key_sort_key);
    current.sort_by_cached_key(model_primary_key_sort_key);
    let original_keys = original
        .iter()
        .map(model_primary_key_sort_key)
        .collect::<Vec<_>>();
    let current_keys = current
        .iter()
        .map(model_primary_key_sort_key)
        .collect::<Vec<_>>();

    let mut diff = GraphRelationDiff::default();
    let mut original_index = 0;
    let mut current_index = 0;
    while original_index < original.len() && current_index < current.len() {
        match original_keys[original_index].cmp(&current_keys[current_index]) {
            Ordering::Less => {
                diff.deleted.push(original_index);
                original_index += 1;
            }
            Ordering::Greater => {
                diff.inserted.push(current_index);
                current_index += 1;
            }
            Ordering::Equal => {
                diff.matched.push((original_index, current_index));
                original_index += 1;
                current_index += 1;
            }
        }
    }
    while original_index < original.len() {
        diff.deleted.push(original_index);
        original_index += 1;
    }
    while current_index < current.len() {
        diff.inserted.push(current_index);
        current_index += 1;
    }
    diff
}

pub enum GraphRelationMut<'a> {
    HasMany {
        relation: &'static RelationInfo,
        items: &'a mut dyn GraphModels,
    },
    HasOne {
        relation: &'static RelationInfo,
        item: Option<&'a mut dyn GraphModel>,
    },
    Through {
        relation: &'static RelationInfo,
        items: &'a mut dyn GraphModels,
    },
}

pub trait GraphModel {
    fn model(&self) -> &dyn Any;
    fn model_mut(&mut self) -> &mut dyn Any;
    fn primary_key_values(&self) -> Vec<Value>;
    fn assign_column_value(&mut self, column: &str, value: Value) -> crate::Result<()>;
}

impl<T: Model + 'static> GraphModel for T {
    fn model(&self) -> &dyn Any {
        self
    }

    fn model_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn primary_key_values(&self) -> Vec<Value> {
        <T as Model>::primary_key_values(self)
    }

    fn assign_column_value(&mut self, column: &str, value: Value) -> crate::Result<()> {
        <T as Model>::assign_column_value(self, column, value)
    }
}

pub trait GraphModels {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn with_each_mut(
        &mut self,
        f: &mut dyn FnMut(&mut dyn GraphModel) -> crate::Result<()>,
    ) -> crate::Result<()>;
}

impl<T: Model + 'static> GraphModels for Vec<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn with_each_mut(
        &mut self,
        f: &mut dyn FnMut(&mut dyn GraphModel) -> crate::Result<()>,
    ) -> crate::Result<()> {
        for item in self {
            f(item)?;
        }
        Ok(())
    }
}

pub trait GraphWritable: WritableModel + Sized {
    fn insert_graph_relations<'tx>(
        _tx: &mut crate::abstract_layer::Transaction<'tx>,
        _owner: &mut Self,
    ) -> impl std::future::Future<Output = crate::Result<()>> {
        async { Ok(()) }
    }

    fn update_graph_relations<'tx>(
        _tx: &mut crate::abstract_layer::Transaction<'tx>,
        _owner: &mut Self,
    ) -> impl std::future::Future<Output = crate::Result<()>> {
        async { Ok(()) }
    }

    fn sync_tracked_graph_relations<'tx>(
        _tx: &mut crate::abstract_layer::Transaction<'tx>,
        _original: &mut Self,
        _current: &mut Self,
    ) -> impl std::future::Future<Output = crate::Result<u64>> {
        async { Ok(0) }
    }
}

pub fn graph_assign_parent_key<Owner, Target>(
    owner: &Owner,
    relation: &RelationInfo,
    target: &mut Target,
) -> crate::Result<()>
where
    Owner: Model,
    Target: Model,
{
    let value = owner.relation_key_value(relation)?;
    target.assign_column_value(relation.target_key, value)
}

pub fn graph_auto_increment_key_value<K: Into<Value>>(key: K) -> Value {
    key.into()
}

pub fn graph_is_no_auto_increment_key(value: &Value) -> bool {
    matches!(value, Value::Null)
}

pub fn graph_model_has_default_auto_increment_key<T>(model: &T) -> bool
where
    T: Model,
    T::AutoIncrementKeyType: Into<Value>,
{
    let Some(column) = T::column_schema()
        .into_iter()
        .find(|column| column.is_auto_increment)
    else {
        return false;
    };
    let Some(value) = model.column_value(column.name) else {
        return false;
    };
    values_equal(
        &value,
        &<T::AutoIncrementKeyType as Default>::default().into(),
    )
}

pub fn graph_relation_info<Owner, Target>(
    relation_name: &'static str,
) -> crate::Result<&'static RelationInfo>
where
    Owner: Model,
    Target: Model,
{
    Owner::RELATIONS
        .iter()
        .find(|relation| {
            relation.name == relation_name && relation.target_table == Target::TABLE_NAME
        })
        .ok_or_else(|| {
            crate::ormer_error!(
                "Relation {} -> {} not found on {}",
                relation_name,
                Target::TABLE_NAME,
                Owner::TABLE_NAME
            )
        })
}

pub fn graph_through_infos<Owner, Via, Target>(
    relation_name: &'static str,
) -> crate::Result<(
    &'static RelationInfo,
    &'static RelationInfo,
    &'static RelationInfo,
)>
where
    Owner: Model,
    Via: Model,
    Target: Model,
{
    let relation = graph_relation_info::<Owner, Target>(relation_name)?;
    let through = relation.through.as_ref().ok_or_else(|| {
        crate::ormer_error!(
            "Relation {} on {} is not a through relation",
            relation_name,
            Owner::TABLE_NAME
        )
    })?;
    let via_relation = graph_relation_info::<Owner, Via>(through.via_relation)?;
    let target_relation = graph_relation_info::<Via, Target>(through.target_relation)?;
    Ok((relation, via_relation, target_relation))
}

pub fn graph_target_key_value<Target>(
    target: &Target,
    target_relation: &RelationInfo,
) -> crate::Result<Value>
where
    Target: Model,
{
    target
        .column_value(target_relation.target_key)
        .ok_or_else(|| {
            crate::ormer_error!(
                "Column {} not found on {}",
                target_relation.target_key,
                Target::TABLE_NAME
            )
        })
        .and_then(|value| {
            if matches!(value, Value::Null) {
                Err(crate::ormer_error!(
                    "Through target {}.{} cannot be NULL",
                    Target::TABLE_NAME,
                    target_relation.target_key
                ))
            } else {
                Ok(value)
            }
        })
}

pub async fn graph_insert_through_link<'tx, Owner, Via>(
    tx: &mut crate::abstract_layer::Transaction<'tx>,
    owner: &Owner,
    via_relation: &RelationInfo,
    target_relation: &RelationInfo,
    target_key: Value,
) -> crate::Result<()>
where
    Owner: Model,
    Via: Model,
{
    let owner_key = owner.relation_key_value(via_relation)?;
    graph_insert_through_link_values::<Via>(
        tx,
        via_relation.target_key,
        owner_key,
        target_relation.local_key,
        target_key,
    )
    .await
}

pub async fn graph_insert_through_link_values<'tx, Via>(
    tx: &mut crate::abstract_layer::Transaction<'tx>,
    owner_column: &'static str,
    owner_value: Value,
    target_column: &'static str,
    target_value: Value,
) -> crate::Result<()>
where
    Via: Model,
{
    let db_type = tx.db_type();
    let table = quote_qualified_identifier(db_type, Via::table_name_for_db(db_type));
    let owner_col = quote_identifier(db_type, owner_column);
    let target_col = quote_identifier(db_type, target_column);

    #[cfg(feature = "sqlite")]
    if matches!(db_type, crate::abstract_layer::DbType::Sqlite) {
        let exists_sql = format!(
            "SELECT COUNT(*) FROM {table} WHERE {owner_col} = {{}} AND {target_col} = {{}}"
        );
        let exists: Vec<i64> = tx
            .select_sql(
                crate::RawSql::new(exists_sql)
                    .bind(owner_value.clone())
                    .bind(target_value.clone()),
            )
            .collect()
            .await?;
        if exists.first().copied().unwrap_or(0) > 0 {
            return Ok(());
        }
    }

    let sql = match db_type {
        #[cfg(feature = "sqlite")]
        crate::abstract_layer::DbType::Sqlite => {
            format!("INSERT INTO {table} ({owner_col}, {target_col}) VALUES ({{}}, {{}})")
        }
        #[cfg(feature = "postgresql")]
        crate::abstract_layer::DbType::PostgreSQL => {
            format!(
                "INSERT INTO {table} ({owner_col}, {target_col}) VALUES ({{}}, {{}}) ON CONFLICT DO NOTHING"
            )
        }
        #[cfg(feature = "mysql")]
        crate::abstract_layer::DbType::MySQL => {
            format!("INSERT IGNORE INTO {table} ({owner_col}, {target_col}) VALUES ({{}}, {{}})")
        }
        #[cfg(feature = "mssql")]
        crate::abstract_layer::DbType::MSSQL => format!(
            "IF NOT EXISTS (SELECT 1 FROM {table} WHERE {owner_col} = {{}} AND {target_col} = {{}}) \
             INSERT INTO {table} ({owner_col}, {target_col}) VALUES ({{}}, {{}})"
        ),
        #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
        _ => {
            return Err(crate::ormer_error!(
                "through-link insertion is not implemented for this backend"
            ));
        }
    };
    let raw = crate::RawSql::new(sql)
        .bind(owner_value.clone())
        .bind(target_value.clone());
    let needs_probe_binds = {
        #[cfg(feature = "mssql")]
        {
            matches!(db_type, crate::abstract_layer::DbType::MSSQL)
        }
        #[cfg(not(feature = "mssql"))]
        {
            let _ = db_type;
            false
        }
    };
    let raw = if needs_probe_binds {
        raw.bind(owner_value).bind(target_value)
    } else {
        raw
    };
    tx.execute_sql(raw).await?;
    Ok(())
}

pub async fn graph_sync_through_links<'tx, Owner, Via>(
    tx: &mut crate::abstract_layer::Transaction<'tx>,
    owner: &Owner,
    via_relation: &RelationInfo,
    target_relation: &RelationInfo,
    target_keys: &[Value],
) -> crate::Result<()>
where
    Owner: Model,
    Via: Model,
{
    let owner_key = owner.relation_key_value(via_relation)?;
    let db_type = tx.db_type();
    let table = quote_qualified_identifier(db_type, Via::table_name_for_db(db_type));
    let owner_col = quote_identifier(db_type, via_relation.target_key);
    let target_col = quote_identifier(db_type, target_relation.local_key);

    if target_keys.is_empty() {
        let sql = format!("DELETE FROM {table} WHERE {owner_col} = {{}}");
        tx.execute_sql(crate::RawSql::new(sql).bind(owner_key))
            .await?;
        return Ok(());
    }

    let placeholders = (0..target_keys.len())
        .map(|_| "{}")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "DELETE FROM {table} WHERE {owner_col} = {{}} AND {target_col} NOT IN ({placeholders})"
    );
    let mut raw = crate::RawSql::new(sql).bind(owner_key);
    for target_key in target_keys {
        raw = raw.bind(target_key.clone());
    }
    tx.execute_sql(raw).await?;
    Ok(())
}

/// 字段类型元数据提供者 trait (可选实现)。
///
/// 枚举类型可提供变体列表以生成 ENUM SQL；包装字段类型可提供内部 Rust 类型，
/// 让后端按内部类型生成 SQL 并解析数据库值。
pub trait FieldTypeProvider {
    const ENUM_VARIANTS: Option<&'static [&'static str]>;
    const DB_VALUE_TYPE: Option<fn(crate::abstract_layer::DbType) -> &'static str> = None;
    const RUST_TYPE: Option<&'static str> = None;

    /// 获取枚举的所有变体名称
    fn enum_variants() -> Option<&'static [&'static str]> {
        Self::ENUM_VARIANTS
    }

    fn db_value_type() -> Option<fn(crate::abstract_layer::DbType) -> &'static str> {
        Self::DB_VALUE_TYPE
    }

    fn rust_type() -> Option<&'static str> {
        Self::RUST_TYPE
    }

    fn model_columns(column: &'static str) -> Vec<&'static str> {
        vec![column]
    }

    fn model_column_schema(column: ColumnSchema) -> Vec<ColumnSchema> {
        vec![column]
    }

    fn model_has_column(
        discriminator_column: &'static str,
        rust_field: &'static str,
        column: &str,
    ) -> bool {
        column == discriminator_column || column == rust_field
    }

    fn model_from_row(
        _rust_field: &'static str,
        discriminator_column: &'static str,
        row: &Row,
    ) -> crate::Result<Self>
    where
        Self: Sized + FromValue,
    {
        row.get(discriminator_column)
    }

    fn model_from_row_values(
        _rust_field: &'static str,
        _discriminator_column: &'static str,
        values: &[Value],
    ) -> crate::Result<Self>
    where
        Self: Sized + FromRowValues,
    {
        <Self as FromRowValues>::from_row_values(values)
    }

    fn model_field_values(&self) -> Vec<Value>
    where
        Self: Clone + Into<Value>,
    {
        vec![self.clone().into()]
    }

    fn model_column_value(
        &self,
        discriminator_column: &'static str,
        rust_field: &'static str,
        column: &str,
    ) -> Option<Value>
    where
        Self: Clone + Into<Value>,
    {
        if Self::model_has_column(discriminator_column, rust_field, column) {
            Some(self.clone().into())
        } else {
            None
        }
    }

    fn model_assign_column_value(
        &mut self,
        discriminator_column: &'static str,
        rust_field: &'static str,
        column: &str,
        value: Value,
    ) -> crate::Result<bool>
    where
        Self: Sized + FromValue,
    {
        if Self::model_has_column(discriminator_column, rust_field, column) {
            *self = <Self as FromValue>::from_value(&value)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// 去掉 schema 前缀，返回最后一段表名。
pub fn table_name_without_schema(table_name: &str) -> &str {
    table_name
        .rsplit_once('.')
        .map(|(_, table)| table)
        .unwrap_or(table_name)
}

/// 按数据库后端返回实际 SQL 中使用的表名。
pub fn normalize_table_name_for_db(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
) -> &str {
    match db_type {
        #[cfg(feature = "sqlite")]
        crate::abstract_layer::DbType::Sqlite => table_name_without_schema(table_name),
        #[cfg(feature = "mysql")]
        crate::abstract_layer::DbType::MySQL => table_name_without_schema(table_name),
        #[cfg(feature = "postgresql")]
        crate::abstract_layer::DbType::PostgreSQL => table_name,
        #[cfg(feature = "mssql")]
        crate::abstract_layer::DbType::MSSQL => table_name,
        #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
        _ => table_name,
    }
}

pub fn table_route_variables(table_name: &str) -> Vec<String> {
    let mut variables = Vec::new();
    let mut rest = table_name;
    while let Some(open_idx) = rest.find('{') {
        rest = &rest[open_idx + 1..];
        let Some(close_idx) = rest.find('}') else {
            break;
        };
        let name = &rest[..close_idx];
        if !name.is_empty() && !variables.iter().any(|existing| existing == name) {
            variables.push(name.to_string());
        }
        rest = &rest[close_idx + 1..];
    }
    variables
}

fn validate_table_route_segment(key: &str, value: &str) -> crate::Result<()> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(crate::ormer_error!(
            "Invalid table route value for {}: {}",
            key,
            value
        ));
    }
    Ok(())
}

pub fn render_table_name_template(table_name: &str, route: &TableRoute) -> crate::Result<String> {
    let mut rendered = String::new();
    let mut rest = table_name;
    while let Some(open_idx) = rest.find('{') {
        rendered.push_str(&rest[..open_idx]);
        rest = &rest[open_idx + 1..];
        let close_idx = rest.find('}').ok_or_else(|| {
            crate::ormer_error!("Unclosed table route placeholder in {}", table_name)
        })?;
        let key = &rest[..close_idx];
        let value = route.get(key).ok_or_else(|| {
            crate::ormer_error!("Missing table route value {} for table {}", key, table_name)
        })?;
        validate_table_route_segment(key, value)?;
        rendered.push_str(value);
        rest = &rest[close_idx + 1..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

pub fn routed_table_name_for_db(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
    route: &TableRoute,
) -> crate::Result<String> {
    if table_name.contains('{') {
        let rendered = render_table_name_template(table_name, route)?;
        Ok(normalize_table_name_for_db(db_type, &rendered).to_string())
    } else {
        Ok(normalize_table_name_for_db(db_type, table_name).to_string())
    }
}

pub fn routed_model_table_name_for_db<T: Model>(
    db_type: crate::abstract_layer::DbType,
    route: &TableRoute,
) -> crate::Result<String> {
    if route.is_empty() {
        return Ok(normalize_table_name_for_db(db_type, T::TABLE_NAME).to_string());
    }

    if T::TABLE_NAME.contains('{') {
        return routed_table_name_for_db(db_type, T::TABLE_NAME, route);
    }

    #[cfg(feature = "postgresql")]
    if matches!(db_type, crate::abstract_layer::DbType::PostgreSQL) {
        if let Some(route_key) = T::hypertable_route_key() {
            let table_name = format!("{}_{{{}}}", T::TABLE_NAME, route_key);
            return routed_table_name_for_db(db_type, &table_name, route);
        }
    }

    routed_table_name_for_db(db_type, T::TABLE_NAME, route)
}

/// 拆分支持 schema 的数据库表名，未指定 schema 时使用默认 schema。
pub fn split_schema_table_name<'a>(
    table_name: &'a str,
    default_schema: &'a str,
) -> (&'a str, &'a str) {
    table_name
        .rsplit_once('.')
        .unwrap_or((default_schema, table_name))
}

/// FieldType trait - 用于标记可直接作为模型字段的自定义字段类型 (由派生宏自动实现)
pub trait FieldType: FieldTypeProvider + Into<Value> + FromValue {
    /// 获取枚举的所有变体名称  
    const VARIANTS: &'static [&'static str];

    /// 获取当前变体的名称
    fn name(&self) -> &'static str;

    /// 从名称构造枚举值
    fn from_name(name: &str) -> crate::Result<Self>
    where
        Self: Sized;

    /// 获取当前变体的数值表示（用于数值枚举）
    /// 默认返回 0，数值枚举应重写此方法
    fn as_i64(&self) -> i64 {
        0
    }

    /// 从数值构造枚举值（用于数值枚举）
    /// 默认返回错误，数值枚举应重写此方法
    fn from_i64(_value: i64) -> crate::Result<Self>
    where
        Self: Sized,
    {
        Err(crate::ormer_error!(
            "This enum does not support numeric conversion"
        ))
    }

    /// 判断是否为数值枚举
    /// 默认返回 false，数值枚举应重写此方法返回 true
    fn is_numeric_enum() -> bool {
        false
    }

    /// 将字段类型转换为查询参数值。
    fn to_filter_value(value: Self) -> Value {
        value.into()
    }

    /// 判断是否支持数值比较操作。
    fn supports_comparison() -> bool {
        false
    }
}

#[deprecated(note = "renamed to FieldTypeProvider")]
pub use FieldTypeProvider as ModelEnumProvider;

#[deprecated(note = "renamed to FieldType")]
pub use FieldType as ModelEnum;

/// 为 `Option<T>` 实现 FieldTypeProvider，透传内部类型的字段类型信息
impl<T: FieldTypeProvider> FieldTypeProvider for Option<T> {
    const ENUM_VARIANTS: Option<&'static [&'static str]> = T::ENUM_VARIANTS;
    const DB_VALUE_TYPE: Option<fn(crate::abstract_layer::DbType) -> &'static str> =
        T::DB_VALUE_TYPE;
    const RUST_TYPE: Option<&'static str> = T::RUST_TYPE;
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(value) => value.into(),
            None => Value::Null,
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Null => Ok(None),
            value => T::from_value(value).map(Some),
        }
    }
}

// 为常见非自定义字段类型实现 FieldTypeProvider，返回 None
macro_rules! impl_field_type_provider_for_plain_type {
    ($($t:ty),* $(,)?) => {
        $(
            impl FieldTypeProvider for $t {
                const ENUM_VARIANTS: Option<&'static [&'static str]> = None;
            }
        )*
    };
}

impl_field_type_provider_for_plain_type!(
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    isize,
    usize,
    f32,
    f64,
    rust_decimal::Decimal,
    bigdecimal::BigDecimal,
    bool,
    String,
    &str,
    Vec<u8>,
    Vec<i32>,
    Vec<i64>,
    Vec<Option<i64>>,
    Vec<String>,
    std::time::Duration,
    chrono::DateTime<chrono::Utc>,
    chrono::NaiveDateTime,
    chrono::NaiveDate,
    chrono::NaiveTime,
    serde_json::Value,
    uuid::Uuid,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveValue<T> {
    NotSet,
    Set(T),
    Unchanged(T),
}

impl<T> ActiveValue<T> {
    pub fn not_set() -> Self {
        Self::NotSet
    }

    pub fn set(value: T) -> Self {
        Self::Set(value)
    }

    pub fn unchanged(value: T) -> Self {
        Self::Unchanged(value)
    }

    pub fn is_not_set(&self) -> bool {
        matches!(self, Self::NotSet)
    }
}

impl<T> Default for ActiveValue<T> {
    fn default() -> Self {
        Self::NotSet
    }
}

impl<T> From<T> for ActiveValue<T> {
    fn from(value: T) -> Self {
        Self::Set(value)
    }
}

pub trait InsertModel<T: Model> {
    fn insert_table_name(&self) -> &'static str;
    fn insert_assignments(&self) -> Vec<crate::query::insert::InsertAssignment>;
}

impl<T, M> InsertModel<T> for &M
where
    T: Model,
    M: InsertModel<T> + ?Sized,
{
    fn insert_table_name(&self) -> &'static str {
        (*self).insert_table_name()
    }

    fn insert_assignments(&self) -> Vec<crate::query::insert::InsertAssignment> {
        (*self).insert_assignments()
    }
}

/// 用于 insert/insert_or_update 的参数类型 trait
#[async_trait::async_trait]
pub trait Insertable {
    type Model: crate::model::Model;
    fn as_refs(&self) -> Vec<&Self::Model>;
    fn as_refs_mut(&mut self) -> Vec<&mut Self::Model>;

    /// Run insert hooks for mutable hook-aware inputs.
    ///
    /// Immutable inputs intentionally keep the historical no-hook behavior.
    async fn run_before_insert(
        &mut self,
        _ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        Ok(())
    }

    async fn run_after_insert(
        &self,
        _ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        Ok(())
    }
}

impl<T: crate::model::WritableModel> Insertable for &T {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        vec![*self]
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &T 无法提供 &mut T，返回空向量（仅在需要可变引用时会使用其他实现）
        vec![]
    }
}

/// A mutable single model opts into automatic insert hooks.
#[async_trait::async_trait]
impl<T> Insertable for &mut T
where
    T: crate::model::WritableModel
        + crate::hooks::BeforeInsert
        + crate::hooks::AfterInsert
        + Send
        + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        vec![&**self]
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        vec![&mut **self]
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        let mut ctx = ctx;
        (**self).before_insert(&mut ctx).await
    }

    async fn run_after_insert(&self, ctx: crate::hooks::HookContext<'static>) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        let mut ctx = ctx;
        (**self).after_insert(&mut ctx).await
    }
}

impl<T: crate::model::WritableModel> Insertable for Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }
}

/// A mutable batch opts into automatic per-row insert hooks.
#[async_trait::async_trait]
impl<T> Insertable for &mut Vec<T>
where
    T: crate::model::WritableModel
        + crate::hooks::BeforeInsert
        + crate::hooks::AfterInsert
        + Send
        + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(&self, ctx: crate::hooks::HookContext<'static>) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
    }
}

impl<T: crate::model::WritableModel> Insertable for &Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &Vec<T> 无法提供 &mut T，返回空向量
        vec![]
    }
}

#[async_trait::async_trait]
impl<T> Insertable for &mut [T]
where
    T: crate::model::WritableModel
        + crate::hooks::BeforeInsert
        + crate::hooks::AfterInsert
        + Send
        + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(&self, ctx: crate::hooks::HookContext<'static>) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
    }
}

impl<T: crate::model::WritableModel> Insertable for &[T] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &[T] 无法提供 &mut T，返回空向量
        vec![]
    }
}

impl<T: crate::model::WritableModel, const N: usize> Insertable for &[T; N] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &[T; N] 无法提供 &mut T，返回空向量
        vec![]
    }
}

#[async_trait::async_trait]
impl<T, const N: usize> Insertable for &mut [T; N]
where
    T: crate::model::WritableModel
        + crate::hooks::BeforeInsert
        + crate::hooks::AfterInsert
        + Send
        + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(&self, ctx: crate::hooks::HookContext<'static>) -> crate::Result<()> {
        if !ctx.hooks_enabled() {
            return Ok(());
        }
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
    }
}

/// 为具体的 Model 类型生成引用的集合类型的 Insertable 实现
/// 这个宏用于解决 orphan rule 问题
#[macro_export]
macro_rules! impl_insertable_for_ref_collections {
    ($model_type:ty) => {
        impl Insertable for Vec<&$model_type> {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.as_slice().to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl Insertable for &Vec<&$model_type> {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.as_slice().to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl<const N: usize> Insertable for &[&$model_type; N] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl Insertable for &[&$model_type] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }
    };
}

pub fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn duckdb_auto_increment_sequence_name(table_name: &str, column_name: &str) -> String {
    format!(
        "__ormer_seq_{}_{}",
        table_name.replace('.', "_"),
        column_name
    )
}

fn needs_identifier_quote(identifier: &str) -> bool {
    if identifier.is_empty() {
        return true;
    }

    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !(first == '_' || first.is_ascii_lowercase()) {
        return true;
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_lowercase() || ch.is_ascii_digit())) {
        return true;
    }

    matches!(
        identifier,
        "all"
            | "and"
            | "as"
            | "by"
            | "check"
            | "column"
            | "constraint"
            | "create"
            | "default"
            | "delete"
            | "desc"
            | "from"
            | "group"
            | "index"
            | "insert"
            | "into"
            | "key"
            | "not"
            | "null"
            | "order"
            | "primary"
            | "references"
            | "select"
            | "table"
            | "unique"
            | "update"
            | "user"
            | "where"
    )
}

pub fn quote_identifier(db_type: crate::abstract_layer::DbType, identifier: &str) -> String {
    if !needs_identifier_quote(identifier) {
        return identifier.to_string();
    }

    match db_type {
        #[cfg(feature = "mysql")]
        crate::abstract_layer::DbType::MySQL => {
            format!("`{}`", identifier.replace('`', "``"))
        }
        #[cfg(feature = "mssql")]
        crate::abstract_layer::DbType::MSSQL => {
            format!("[{}]", identifier.replace(']', "]]"))
        }
        #[cfg(feature = "sqlite")]
        crate::abstract_layer::DbType::Sqlite => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
        #[cfg(feature = "postgresql")]
        crate::abstract_layer::DbType::PostgreSQL => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
        #[cfg(feature = "duckdb")]
        crate::abstract_layer::DbType::DuckDB => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
        #[cfg(feature = "clickhouse")]
        crate::abstract_layer::DbType::ClickHouse => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
    }
}

pub fn quote_qualified_identifier(db_type: crate::abstract_layer::DbType, name: &str) -> String {
    name.split('.')
        .map(|part| quote_identifier(db_type, part))
        .collect::<Vec<_>>()
        .join(".")
}

pub fn quote_column_reference(db_type: crate::abstract_layer::DbType, column: &str) -> String {
    if column.contains('(') || column.contains(' ') {
        return column.to_string();
    }
    quote_qualified_identifier(db_type, column)
}

/// 运行时动态生成 CREATE TABLE SQL
pub fn generate_create_table_sql<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
) -> crate::Result<String> {
    generate_create_table_sql_with_name::<T>(db_type, None)
}

/// Generate ClickHouse CREATE TABLE SQL with an explicit engine clause.
///
/// `engine` is the expression after `ENGINE =`, for example
/// `MergeTree ORDER BY (id)`. ClickHouse requires an engine for every table,
/// so the generic create-table helper intentionally remains unsupported for
/// this backend unless this function is used.
#[cfg(feature = "clickhouse")]
pub fn generate_clickhouse_create_table_sql<T: WritableModel>(
    engine: &str,
) -> crate::Result<String> {
    generate_clickhouse_create_table_sql_with_name::<T>(engine, None)
}

/// Generate ClickHouse CREATE TABLE SQL with an explicit engine and table name.
#[cfg(feature = "clickhouse")]
pub fn generate_clickhouse_create_table_sql_with_name<T: WritableModel>(
    engine: &str,
    table_name: Option<&str>,
) -> crate::Result<String> {
    generate_create_table_sql_with_engine::<T>(
        crate::abstract_layer::DbType::ClickHouse,
        table_name,
        Some(engine),
    )
}

/// 生成 CREATE TABLE SQL 语句，支持自定义表名
pub fn generate_create_table_sql_with_name<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
    table_name: Option<&str>,
) -> crate::Result<String> {
    generate_create_table_sql_with_engine::<T>(db_type, table_name, None)
}

fn generate_create_table_sql_with_engine<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
    table_name: Option<&str>,
    mut clickhouse_engine: Option<&str>,
) -> crate::Result<String> {
    #[cfg(feature = "clickhouse")]
    let is_clickhouse = matches!(db_type, crate::abstract_layer::DbType::ClickHouse);
    #[cfg(not(feature = "clickhouse"))]
    let is_clickhouse = false;

    let table_options = T::table_options();
    if is_clickhouse {
        let model_engine = table_options
            .as_ref()
            .and_then(TableOptions::clickhouse_engine_clause);
        let Some(engine) = clickhouse_engine
            .map(str::trim)
            .filter(|engine| !engine.is_empty())
            .or(model_engine)
        else {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "CREATE TABLE without explicit ClickHouse engine settings",
            });
        };
        if engine.contains(';') {
            return Err(crate::ormer_error!(
                "ClickHouse engine clause must not contain ';'"
            ));
        }
        clickhouse_engine = Some(engine);
    } else if clickhouse_engine.is_some() {
        return Err(crate::ormer_error!(
            "ClickHouse engine settings can only be used with ClickHouse"
        ));
    }

    let table_name = normalize_table_name_for_db(db_type, table_name.unwrap_or(T::TABLE_NAME));
    let quoted_table_name = quote_qualified_identifier(db_type, table_name);

    let column_schema = T::column_schema();
    let is_duckdb = {
        #[cfg(feature = "duckdb")]
        {
            matches!(db_type, crate::abstract_layer::DbType::DuckDB)
        }
        #[cfg(not(feature = "duckdb"))]
        {
            false
        }
    };
    let auto_increment_column = column_schema
        .iter()
        .find(|column| column.is_primary && column.is_auto_increment);
    let mut sql = if is_duckdb {
        if let Some(column) = auto_increment_column {
            let sequence_name = duckdb_auto_increment_sequence_name(table_name, column.name);
            format!(
                "CREATE SEQUENCE IF NOT EXISTS {}; ",
                quote_identifier(db_type, &sequence_name)
            )
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    sql.push_str(&format!(
        "CREATE TABLE IF NOT EXISTS {} (",
        quoted_table_name
    ));
    for (i, column) in column_schema.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }

        // 检查是否有复合主键（多个主键字段）
        let primary_key_count = column_schema.iter().filter(|c| c.is_primary).count();
        let is_composite_primary = primary_key_count > 1;

        // 对于复合主键，不在列定义中添加 PRIMARY KEY，而是在最后添加表级约束
        let sql_type = if let Some(db_value_type) = column.db_value_type {
            crate::abstract_layer::common::common_helpers::sql_type_with_nullability(
                db_value_type(db_type),
                column.is_nullable,
            )
        } else {
            let effective_rust_type = column.data_type.unwrap_or(column.rust_type);
            if !is_clickhouse && is_composite_primary && column.is_primary {
                db_type.sql_type(
                    effective_rust_type,
                    false, // 不在列级别标记为主键
                    column.is_auto_increment,
                    column.is_nullable,
                    column.enum_variants,
                )
            } else {
                db_type.sql_type(
                    effective_rust_type,
                    column.is_primary && !is_clickhouse,
                    column.is_auto_increment,
                    column.is_nullable,
                    column.enum_variants,
                )
            }
        };

        // 添加列级压缩属性（PostgreSQL 支持，且必须在 NOT NULL 之前）
        let is_postgresql = {
            #[cfg(feature = "postgresql")]
            {
                matches!(db_type, crate::abstract_layer::DbType::PostgreSQL)
            }
            #[cfg(not(feature = "postgresql"))]
            {
                false
            }
        };
        if is_postgresql {
            if let Some(compression) = column_compression_algorithm(column) {
                if !matches!(
                    compression,
                    CompressionAlgorithm::Pglz | CompressionAlgorithm::Lz4
                ) {
                    return Err(crate::ormer_error!(
                        "PostgreSQL does not support compression algorithm {}",
                        compression.as_str()
                    ));
                }
                let method = compression.as_str();
                if sql_type.ends_with(" NOT NULL") {
                    let base = &sql_type[..sql_type.len() - " NOT NULL".len()];
                    sql.push_str(&format!(
                        "{} {base} COMPRESSION {method} NOT NULL",
                        quote_identifier(db_type, column.name)
                    ));
                } else {
                    sql.push_str(&format!(
                        "{} {sql_type} COMPRESSION {method}",
                        quote_identifier(db_type, column.name)
                    ));
                }
            } else {
                sql.push_str(&format!(
                    "{} {sql_type}",
                    quote_identifier(db_type, column.name)
                ));
            }
        } else {
            sql.push_str(&format!(
                "{} {sql_type}",
                quote_identifier(db_type, column.name)
            ));
        }

        if is_duckdb && auto_increment_column.is_some_and(|auto| auto.name == column.name) {
            let sequence_name = duckdb_auto_increment_sequence_name(table_name, column.name);
            sql.push_str(" DEFAULT nextval(");
            sql.push_str(&quote_sql_literal(&sequence_name));
            sql.push(')');
        } else if let Some(default) = column.default {
            sql.push_str(" DEFAULT ");
            sql.push_str(&default.to_sql(db_type));
        }

        // 添加单列 UNIQUE 约束（group 中只有一个字段的情况）
        if !is_clickhouse && column.unique_group.is_some() {
            // 检查这个 group 中是否有多个字段
            let group_count = column_schema
                .iter()
                .filter(|c| c.unique_group == column.unique_group)
                .count();

            if group_count == 1 {
                // 单列唯一约束
                if column.unique_name.is_none() {
                    sql.push_str(" UNIQUE");
                }
            }
        }

        if !is_clickhouse {
            if let Some(check) = column.check {
                sql.push(' ');
                if let Some(name) = check.name {
                    sql.push_str(&format!("CONSTRAINT {} ", quote_identifier(db_type, name)));
                }
                sql.push_str(&format!("CHECK ({})", check.expr));
            }
        }
    }

    if !is_clickhouse {
        // 添加外键约束
        let foreign_key_constraints = generate_foreign_key_constraints::<T>(db_type);
        if !foreign_key_constraints.is_empty() {
            sql.push_str(", ");
            sql.push_str(&foreign_key_constraints.join(", "));
        }

        // 添加复合主键约束（如果有多个主键字段）
        let composite_primary_constraint = generate_composite_primary_key_constraint::<T>(db_type);
        if !composite_primary_constraint.is_empty() {
            sql.push_str(", ");
            sql.push_str(&composite_primary_constraint);
        }

        // 添加联合 UNIQUE 约束
        let unique_constraints = generate_unique_constraints::<T>(db_type);
        if !unique_constraints.is_empty() {
            sql.push_str(", ");
            sql.push_str(&unique_constraints.join(", "));
        }
    }

    sql.push(')');

    if is_clickhouse {
        sql.push_str(" ENGINE = ");
        sql.push_str(clickhouse_engine.expect("validated ClickHouse engine"));
        if let Some(options) = &table_options {
            options.append_clickhouse_options(&mut sql)?;
        }
        return Ok(sql);
    }

    #[cfg(feature = "mssql")]
    if matches!(db_type, crate::abstract_layer::DbType::MSSQL)
        && let Some(options) = &table_options
    {
        options.append_mssql_options(&mut sql)?;
    }

    #[cfg(feature = "postgresql")]
    if matches!(db_type, crate::abstract_layer::DbType::PostgreSQL)
        && let Some(options) = &table_options
    {
        options.append_postgresql_options(&mut sql)?;
    }

    #[cfg(feature = "mysql")]
    if matches!(db_type, crate::abstract_layer::DbType::MySQL) {
        if let Some(options) = &table_options {
            options.append_mysql_options(&mut sql);
        }
        if let Some(compression) = table_compression_algorithm::<T>()? {
            sql.push_str(" COMPRESSION='");
            sql.push_str(compression.as_upper_str());
            sql.push('\'');
        }
    }

    // 添加索引
    let index_sql = generate_indexes_with_name::<T>(db_type, table_name)?;
    if !index_sql.is_empty() {
        sql.push(';');
        sql.push_str(&index_sql);
    }

    Ok(sql)
}

#[cfg(feature = "mysql")]
pub(crate) fn table_compression_algorithm<T: Model>() -> crate::Result<Option<CompressionAlgorithm>>
{
    let mut compression: Option<CompressionAlgorithm> = None;
    for column in T::column_schema() {
        let Some(candidate) = column_compression_algorithm(&column) else {
            continue;
        };
        if !matches!(
            candidate,
            CompressionAlgorithm::Lz4 | CompressionAlgorithm::Zlib
        ) {
            return Err(crate::ormer_error!(
                "MySQL does not support compression algorithm {}",
                candidate.as_str()
            ));
        }
        if let Some(existing) = compression {
            if existing != candidate {
                return Err(crate::ormer_error!(
                    "MySQL table compression cannot use multiple algorithms: {} and {}",
                    existing.as_str(),
                    candidate.as_str()
                ));
            }
        } else {
            compression = Some(candidate);
        }
    }
    Ok(compression)
}

/// 生成 UNIQUE 约束
fn generate_unique_constraints<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
) -> Vec<String> {
    let mut constraints = Vec::new();

    // 收集所有 unique_group
    let mut group_map: std::collections::BTreeMap<i32, Vec<&ColumnSchema>> =
        std::collections::BTreeMap::new();

    let column_schema = T::column_schema();
    for column in column_schema.iter() {
        if let Some(group_id) = column.unique_group {
            group_map.entry(group_id).or_default().push(column);
        }
    }

    // 生成约束
    for (_group_id, columns) in group_map {
        let unique_name = columns.iter().find_map(|column| column.unique_name);
        if columns.len() == 1 && unique_name.is_none() {
            // 单列唯一约束已经在列定义中处理
        } else {
            // 联合唯一约束
            let cols = columns
                .iter()
                .map(|column| quote_identifier(db_type, column.name))
                .collect::<Vec<_>>()
                .join(", ");
            let prefix = unique_name
                .map(|name| format!("CONSTRAINT {} ", quote_identifier(db_type, name)))
                .unwrap_or_default();
            constraints.push(format!("{prefix}UNIQUE ({cols})"));
        }
    }

    constraints
}

/// 生成索引 SQL，支持自定义表名
fn generate_indexes_with_name<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
) -> crate::Result<String> {
    let mut sqls = Vec::new();

    // 检查是否为 MySQL 数据库（通过调试字符串）
    let is_mysql = format!("{:?}", db_type).contains("MySQL");
    let column_schema = T::column_schema();
    #[cfg(feature = "sqlite")]
    if matches!(db_type, crate::abstract_layer::DbType::Sqlite)
        && column_schema
            .iter()
            .any(|column| column.index_method.is_some_and(|method| method == "fulltext"))
    {
        let mut columns = column_schema
            .iter()
            .filter(|column| column.is_indexed)
            .map(|column| column.name)
            .collect::<Vec<_>>();
        if let Some(explicit_columns) = column_schema
            .iter()
            .find_map(|column| column.index_columns)
        {
            columns = explicit_columns
                .trim_start_matches('(')
                .trim_end_matches(')')
                .split(',')
                .map(str::trim)
                .filter(|column| !column.is_empty())
                .collect();
        }
        return render_sqlite_fulltext(table_name, &columns);
    }

    let mut grouped_indexes: std::collections::BTreeMap<i32, Vec<&ColumnSchema>> =
        std::collections::BTreeMap::new();

    for column in column_schema.iter() {
        if !column.is_indexed {
            continue;
        }

        if let Some(group_id) = column.index_group {
            grouped_indexes.entry(group_id).or_default().push(column);
            continue;
        }

        let index_name = column
            .index_name
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("idx_{}_{}", table_name.replace('.', "_"), column.name));
        sqls.push(render_index_sql(
            db_type,
            is_mysql,
            table_name,
            &index_name,
            &[column],
        )?);
    }

    for (group_id, columns) in grouped_indexes {
        if columns.is_empty() {
            continue;
        }

        let index_name = columns
            .iter()
            .find_map(|column| column.index_name)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("idx_{}_{}", table_name.replace('.', "_"), group_id));
        sqls.push(render_index_sql(
            db_type,
            is_mysql,
            table_name,
            &index_name,
            &columns,
        )?);
    }

    Ok(sqls.join(";"))
}

fn render_index_sql(
    db_type: crate::abstract_layer::DbType,
    is_mysql: bool,
    table_name: &str,
    index_name: &str,
    columns: &[&ColumnSchema],
) -> crate::Result<String> {
    #[cfg(feature = "postgresql")]
    let is_postgresql = matches!(db_type, crate::abstract_layer::DbType::PostgreSQL);
    #[cfg(not(feature = "postgresql"))]
    let is_postgresql = false;
    let method = columns.iter().find_map(|column| column.index_method);
    let expression = columns.iter().find_map(|column| column.index_expression);
    let explicit_columns = columns.iter().find_map(|column| column.index_columns);
    let where_clause = columns.iter().find_map(|column| column.index_where);
    if method.is_some() && expression.is_some() {
        return Err(crate::ormer_error!(
            "An index cannot specify both method and expression"
        ));
    }
    if expression.is_some() && columns.len() > 1 {
        return Err(crate::ormer_error!(
            "Expression indexes must be declared on only one member of an index group"
        ));
    }
    if where_clause.is_some() && is_mysql {
        return Err(crate::ormer_error!(
            "MySQL does not support partial index WHERE clauses"
        ));
    }
    if where_clause.is_some() {
        #[cfg(feature = "sqlite")]
        if matches!(db_type, crate::abstract_layer::DbType::Sqlite) {
            return Err(
                crate::abstract_layer::common::common_helpers::unsupported_partial_index_where(
                    db_type,
                ),
            );
        }
        #[cfg(feature = "duckdb")]
        if matches!(db_type, crate::abstract_layer::DbType::DuckDB) {
            return Err(
                crate::abstract_layer::common::common_helpers::unsupported_partial_index_where(
                    db_type,
                ),
            );
        }
    }

    let columns_sql = columns
        .iter()
        .map(|column| {
            let mut col = quote_identifier(db_type, column.name);
            if let Some(order) = column.index_order {
                col.push(' ');
                col.push_str(order);
            }
            col
        })
        .collect::<Vec<_>>()
        .join(", ");
    let columns_sql = expression
        .map(ToString::to_string)
        .or_else(|| {
            explicit_columns.map(|columns| {
                columns
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .to_string()
            })
        })
        .unwrap_or(columns_sql);
    if let Some(method) = method {
        if is_mysql {
            if method != "fulltext" {
                return Err(crate::ormer_error!(
                    "MySQL does not support index method {}",
                    method
                ));
            }
        } else if !is_postgresql {
            return Err(crate::ormer_error!(
                "Index method {} is only supported on PostgreSQL",
                method
            ));
        }
    }
    let sql = if is_mysql {
        let fulltext = method == Some("fulltext");
        format!(
            "CREATE {}INDEX {} ON {} ({})",
            if fulltext { "FULLTEXT " } else { "" },
            quote_identifier(db_type, index_name),
            quote_qualified_identifier(db_type, table_name),
            columns_sql
        )
    } else {
        format!(
            "CREATE INDEX IF NOT EXISTS {} ON {}{}",
            quote_identifier(db_type, index_name),
            quote_qualified_identifier(db_type, table_name),
            if let Some(method) = method {
                format!(" USING {method} ({columns_sql})")
            } else {
                format!(" ({columns_sql})")
            }
        )
    };

    Ok(if let Some(where_clause) = where_clause {
        format!("{sql} WHERE {where_clause}")
    } else {
        sql
    })
}

#[cfg(feature = "sqlite")]
fn render_sqlite_fulltext(table_name: &str, columns: &[&str]) -> crate::Result<String> {
    if columns.is_empty() {
        return Err(crate::ormer_error!(
            "SQLite full-text indexes require at least one column"
        ));
    }
    let fts_table = quote_identifier(
        crate::abstract_layer::DbType::Sqlite,
        &format!("{table_name}_fts"),
    );
    let quoted_table = quote_identifier(crate::abstract_layer::DbType::Sqlite, table_name);
    let quoted_columns = columns
        .iter()
        .map(|column| quote_identifier(crate::abstract_layer::DbType::Sqlite, column))
        .collect::<Vec<_>>()
        .join(", ");
    let new_values = columns
        .iter()
        .map(|column| format!("NEW.{}", quote_identifier(crate::abstract_layer::DbType::Sqlite, column)))
        .collect::<Vec<_>>()
        .join(", ");
    let old_values = columns
        .iter()
        .map(|column| format!("OLD.{}", quote_identifier(crate::abstract_layer::DbType::Sqlite, column)))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {fts_table} USING fts5({quoted_columns}, content={table_literal}, content_rowid='rowid');\
         CREATE TRIGGER IF NOT EXISTS {table_name}_fts_ai AFTER INSERT ON {quoted_table} BEGIN INSERT INTO {fts_table}(rowid, {quoted_columns}) VALUES (NEW.rowid, {new_values}); END;\
         CREATE TRIGGER IF NOT EXISTS {table_name}_fts_ad AFTER DELETE ON {quoted_table} BEGIN INSERT INTO {fts_table}(fts_table, rowid, {quoted_columns}) VALUES('delete', OLD.rowid, {old_values}); END;\
         CREATE TRIGGER IF NOT EXISTS {table_name}_fts_au AFTER UPDATE ON {quoted_table} BEGIN INSERT INTO {fts_table}(fts_table, rowid, {quoted_columns}) VALUES('delete', OLD.rowid, {old_values}); INSERT INTO {fts_table}(rowid, {quoted_columns}) VALUES (NEW.rowid, {new_values}); END;",
        table_literal = quote_sql_literal(table_name),
        table_name = table_name.replace('"', "_"),
    ))
}

/// 生成外键约束 SQL
fn generate_foreign_key_constraints<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
) -> Vec<String> {
    let mut constraints = Vec::new();

    for column in T::column_schema().iter() {
        if let Some(fk) = &column.foreign_key {
            let ref_column = fk.get_ref_column();
            let ref_table = normalize_table_name_for_db(db_type, fk.ref_table);
            let mut constraint = String::new();
            if let Some(name) = fk.name {
                constraint.push_str(&format!("CONSTRAINT {} ", quote_identifier(db_type, name)));
            }
            constraint.push_str(&format!(
                "FOREIGN KEY ({}) REFERENCES {} ({})",
                quote_identifier(db_type, column.name),
                quote_qualified_identifier(db_type, ref_table),
                quote_identifier(db_type, ref_column)
            ));
            if let Some(action) = fk.on_delete {
                constraint.push_str(&format!(" ON DELETE {}", action.as_sql()));
            }
            if let Some(action) = fk.on_update {
                constraint.push_str(&format!(" ON UPDATE {}", action.as_sql()));
            }
            constraints.push(constraint);
        }
    }

    constraints
}

/// 生成复合主键约束 SQL
fn generate_composite_primary_key_constraint<T: WritableModel>(
    db_type: crate::abstract_layer::DbType,
) -> String {
    let primary_keys: Vec<&str> = T::column_schema()
        .iter()
        .filter(|c| c.is_primary)
        .map(|c| c.name)
        .collect();

    if primary_keys.len() > 1 {
        // 复合主键：PRIMARY KEY (col1, col2, ...)
        format!(
            "PRIMARY KEY ({})",
            primary_keys
                .into_iter()
                .map(|column| quote_identifier(db_type, column))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        // 单主键或无主键：不需要表级约束
        String::new()
    }
}

/// 数据库行抽象
#[derive(Debug)]
pub struct Row {
    data: HashMap<String, Value>,
}

impl Row {
    pub fn new(data: HashMap<String, Value>) -> Self {
        Self { data }
    }

    pub fn get<T: FromValue>(&self, column: &str) -> crate::Result<T> {
        self.data
            .get(column)
            .ok_or_else(|| crate::ormer_error!("Column not found: {}", column))
            .and_then(|v| T::from_value(v))
    }
}

pub(crate) fn normalize_string_vec(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn parse_string_vec_text(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    if let Ok(values) = serde_json::from_str::<Vec<String>>(raw) {
        return normalize_string_vec(values);
    }

    vec![raw.to_string()]
}

#[cfg(any(
    feature = "sqlite",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb"
))]
pub(crate) fn stringify_string_vec(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

fn parse_utc_datetime_text(raw: &str) -> crate::Result<chrono::DateTime<chrono::Utc>> {
    let raw = raw.trim();
    if let Ok(value) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(value.with_timezone(&chrono::Utc));
    }

    parse_naive_datetime_text(raw)
        .map(|value| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(value, chrono::Utc))
}

fn parse_naive_datetime_text(raw: &str) -> crate::Result<chrono::NaiveDateTime> {
    let raw = raw.trim();
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f"))
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(raw).map(|value| value.naive_utc()))
        .map_err(|err| crate::ormer_error!("Type mismatch: expected date-time text: {}", err))
}

fn parse_naive_date_text(raw: &str) -> crate::Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|err| crate::ormer_error!("Type mismatch: expected date text: {}", err))
}

fn parse_naive_time_text(raw: &str) -> crate::Result<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M:%S%.f")
        .map_err(|err| crate::ormer_error!("Type mismatch: expected time text: {}", err))
}

/// 值类型
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    BigInt(i128),
    Duration(std::time::Duration),
    Text(String),
    TextArray(Vec<String>),
    Real(f64),
    Decimal(String),
    BigDecimal(String),
    Boolean(bool),
    Bytes(Vec<u8>),
    IntegerArray(Vec<i32>),
    BigIntArray(Vec<i64>),
    NullableBigIntArray(Vec<Option<i64>>),
    DateTime(chrono::DateTime<chrono::Utc>),
    Date(chrono::NaiveDate),
    Time(chrono::NaiveTime),
    Json(serde_json::Value),
    Uuid(uuid::Uuid),
    Null,
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> crate::Result<Self>;
}

impl FromValue for Value {
    fn from_value(value: &Value) -> crate::Result<Self> {
        Ok(value.clone())
    }
}

pub fn downcast_relation_vec_as<Concrete: Model + 'static, Target: Model + 'static>(
    values: Vec<Target>,
) -> crate::Result<Vec<Concrete>> {
    let boxed: Box<dyn Any> = Box::new(values);
    boxed
        .downcast::<Vec<Concrete>>()
        .map(|values| *values)
        .map_err(|_| crate::ormer_error!("Relation target type mismatch"))
}

#[doc(hidden)]
pub struct I32DataTypeEncoder<T>(std::marker::PhantomData<T>);

impl<T> I32DataTypeEncoder<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for I32DataTypeEncoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub trait I32DataTypeEncode<T> {
    fn encode(self, value: T, column_name: &'static str, target_type: &'static str) -> i32;
}

impl<T> I32DataTypeEncode<T> for I32DataTypeEncoder<T>
where
    T: Into<i32>,
{
    fn encode(self, value: T, _column_name: &'static str, _target_type: &'static str) -> i32 {
        value.into()
    }
}

impl<T> I32DataTypeEncode<T> for &I32DataTypeEncoder<T>
where
    T: Copy,
{
    fn encode(self, value: T, column_name: &'static str, target_type: &'static str) -> i32 {
        if std::mem::size_of::<T>() != std::mem::size_of::<i32>() {
            panic!(
                "Failed to convert column '{}' from {}: target type must be i32-sized when it does not implement Into<i32>",
                column_name, target_type
            );
        }

        // Fallback for #[data_type(i32)] C-like enums that intentionally do not
        // implement Into<i32>. Valid discriminants are the caller's contract.
        unsafe { std::mem::transmute_copy::<T, i32>(&value) }
    }
}

#[doc(hidden)]
pub struct I32DataTypeDecoder<T>(std::marker::PhantomData<T>);

impl<T> I32DataTypeDecoder<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for I32DataTypeDecoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub trait I32DataTypeDecode<T> {
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> crate::Result<T>;
}

impl<T> I32DataTypeDecode<T> for I32DataTypeDecoder<T>
where
    T: TryFrom<i32>,
    <T as TryFrom<i32>>::Error: std::fmt::Display,
{
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> crate::Result<T> {
        T::try_from(value).map_err(|err| {
            crate::ormer_error!(
                "Failed to convert column '{}' to {}: {}",
                column_name,
                target_type,
                err
            )
        })
    }
}

impl<T> I32DataTypeDecode<T> for &I32DataTypeDecoder<T>
where
    T: Copy,
{
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> crate::Result<T> {
        if std::mem::size_of::<T>() != std::mem::size_of::<i32>() {
            return Err(crate::ormer_error!(
                "Failed to convert column '{}' to {}: target type must be i32-sized when it does not implement TryFrom<i32>",
                column_name,
                target_type
            ));
        }

        // Fallback for #[data_type(i32)] C-like enums that intentionally do not
        // implement TryFrom<i32>. Valid discriminants are the caller's contract.
        Ok(unsafe { std::mem::transmute_copy::<i32, T>(&value) })
    }
}

/// FromRowValues trait - 用于从一行中的多个值构建类型(如元组、Model)
pub trait FromRowValues: Sized {
    fn row_columns() -> Option<&'static [&'static str]> {
        None
    }

    fn from_row_values(values: &[Value]) -> crate::Result<Self>;
}

fn first_row_value<'a, S>(values: &'a [Value], expected: S) -> crate::Result<&'a Value>
where
    S: AsRef<str>,
{
    values
        .first()
        .ok_or_else(|| crate::ormer_error!("Type mismatch: expected {}", expected.as_ref()))
}

impl<T: ViewModel> FromRowValues for T {
    fn row_columns() -> Option<&'static [&'static str]> {
        Some(Self::COLUMNS)
    }

    fn from_row_values(values: &[Value]) -> crate::Result<Self> {
        T::from_row_values(values)
    }
}

/// FromSingleValue trait - 用于从单个值构建Model(用于map_to后的转换)
/// 当查询单列结果并想转换为Model时使用
pub trait FromSingleValue<V>: Sized {
    fn from_single_value(value: V, column_name: &str) -> crate::Result<Self>;
}

// 为所有可以转换为Value的类型实现FromSingleValue的blanket implementation
impl<T, V> FromSingleValue<V> for T
where
    T: Model,
    V: Into<Value>,
    T: FromValue,
{
    fn from_single_value(value: V, _column_name: &str) -> crate::Result<Self> {
        let ormer_value: Value = value.into();
        Self::from_value(&ormer_value)
    }
}

fn parse_integral_decimal_text<T>(raw: &str, expected: &str) -> crate::Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    let raw = raw.trim();
    let integer = match raw.split_once('.') {
        Some((integer, fraction)) if fraction.chars().all(|ch| ch == '0') => integer,
        _ => raw,
    };
    integer
        .parse::<T>()
        .map_err(|err| crate::ormer_error!("Type mismatch: expected {}: {}", expected, err))
}

// 使用宏生成 FromValue 实现，减少重复代码
macro_rules! impl_from_value_for {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl FromValue for $type {
                fn from_value(value: &Value) -> crate::Result<Self> {
                    match value {
                        Value::$variant(v) => Ok(*v as $type),
                        Value::Decimal(v) | Value::BigDecimal(v) | Value::Text(v) => {
                            parse_integral_decimal_text::<$type>(v, stringify!($type))
                        }
                        _ => Err(crate::ormer_error!("Type mismatch: expected {}", stringify!($type))),
                    }
                }
            }
        )*
    };
}

// 为基本类型生成 FromValue 实现
impl_from_value_for!(
    i8 => Integer,
    i16 => Integer,
    i32 => Integer,
    i64 => Integer,
    u8 => Integer,
    u16 => Integer,
    u32 => Integer,
    u64 => Integer,
    isize => Integer,
    usize => Integer,
);

macro_rules! impl_from_row_values_single {
    ($($type:ty => $expected:expr),* $(,)?) => {
        $(
            impl FromRowValues for $type {
                fn from_row_values(values: &[Value]) -> crate::Result<Self> {
                    Self::from_value(first_row_value(values, $expected)?)
                }
            }
        )*
    };
}

// 为单列类型实现 FromRowValues。
impl_from_row_values_single!(
    i8 => "i8",
    i16 => "i16",
    i32 => "i32",
    i64 => "i64",
    u8 => "u8",
    u16 => "u16",
    u32 => "u32",
    u64 => "u64",
    isize => "isize",
    usize => "usize",
    std::time::Duration => "Duration",
    f64 => "f64",
    rust_decimal::Decimal => "rust_decimal::Decimal",
    bigdecimal::BigDecimal => "bigdecimal::BigDecimal",
    String => "String",
    Vec<String> => "Vec<String>",
    bool => "bool",
    Vec<u8> => "Vec<u8>",
    Vec<i32> => "Vec<i32>",
    Vec<i64> => "Vec<i64>",
    Vec<Option<i64>> => "Vec<Option<i64>>",
    chrono::DateTime<chrono::Utc> => "DateTime<Utc>",
    chrono::NaiveDateTime => "NaiveDateTime",
    chrono::NaiveDate => "NaiveDate",
    chrono::NaiveTime => "NaiveTime",
    uuid::Uuid => "uuid::Uuid",
    serde_json::Value => "serde_json::Value",
);

impl FromValue for std::time::Duration {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Duration(v) => Ok(*v),
            _ => Err(crate::ormer_error!("Type mismatch: expected Duration")),
        }
    }
}

// f64 特殊处理（支持 Integer 和 Real）
impl FromValue for f64 {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Real(v) => Ok(*v),
            Value::Integer(v) => Ok(*v as f64),
            _ => Err(crate::ormer_error!("Type mismatch: expected f64")),
        }
    }
}

macro_rules! impl_decimal_value_traits {
    ($type:ty, $variant:ident, $to_string:ident) => {
        impl From<$type> for Value {
            fn from(v: $type) -> Self {
                Value::$variant(v.$to_string())
            }
        }

        impl FromValue for $type {
            fn from_value(value: &Value) -> crate::Result<Self> {
                match value {
                    Value::Decimal(v) | Value::BigDecimal(v) | Value::Text(v) => {
                        v.parse::<$type>().map_err(|err| {
                            crate::ormer_error!(
                                "Type mismatch: expected {}: {}",
                                stringify!($type),
                                err
                            )
                        })
                    }
                    Value::Integer(v) => Ok(<$type>::from(*v)),
                    _ => Err(crate::ormer_error!(
                        "Type mismatch: expected {}",
                        stringify!($type)
                    )),
                }
            }
        }
    };
}

impl_decimal_value_traits!(rust_decimal::Decimal, Decimal, to_string);
impl_decimal_value_traits!(bigdecimal::BigDecimal, BigDecimal, to_plain_string);

// String 特殊处理（需要 clone）
impl FromValue for String {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            _ => Err(crate::ormer_error!("Type mismatch: expected String")),
        }
    }
}

impl From<Vec<String>> for Value {
    fn from(v: Vec<String>) -> Self {
        Value::TextArray(v)
    }
}

impl FromValue for Vec<String> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::TextArray(v) => Ok(normalize_string_vec(v.clone())),
            Value::Text(v) => Ok(parse_string_vec_text(v)),
            Value::Json(v) => {
                if v.is_null() {
                    return Ok(Vec::new());
                }
                if let Some(user) = v.as_str() {
                    let user = user.trim();
                    return Ok((!user.is_empty())
                        .then(|| user.to_string())
                        .into_iter()
                        .collect());
                }
                let users = serde_json::from_value::<Vec<String>>(v.clone())?;
                Ok(normalize_string_vec(users))
            }
            _ => Err(crate::ormer_error!("Type mismatch: expected Vec<String>")),
        }
    }
}

// bool 特殊处理（从 Boolean 读取）
impl FromValue for bool {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Boolean(v) => Ok(*v),
            Value::Integer(v) => Ok(*v != 0), // 向后兼容
            _ => Err(crate::ormer_error!("Type mismatch: expected bool")),
        }
    }
}

// 为二元组实现 FromValue
impl<T1: FromValue, T2: FromValue> FromValue for (T1, T2) {
    fn from_value(_value: &Value) -> crate::Result<Self> {
        // 元组不能从单个Value构建，这个实现仅用于类型系统完整性
        // 实际上元组应该从多个Value构建
        Err(crate::ormer_error!("Type mismatch: expected tuple"))
    }
}

// 为二元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues> FromRowValues for (T1, T2) {
    fn from_row_values(values: &[Value]) -> crate::Result<Self> {
        if values.len() < 2 {
            return Err(crate::ormer_error!(
                "Type mismatch: expected tuple (T1, T2)"
            ));
        }
        let v1 = T1::from_row_values(&values[0..1])?;
        let v2 = T2::from_row_values(&values[1..2])?;
        Ok((v1, v2))
    }
}

// 为三元组实现 FromValue
impl<T1: FromValue, T2: FromValue, T3: FromValue> FromValue for (T1, T2, T3) {
    fn from_value(_value: &Value) -> crate::Result<Self> {
        Err(crate::ormer_error!("Type mismatch: expected tuple"))
    }
}

// 为三元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues, T3: FromRowValues> FromRowValues for (T1, T2, T3) {
    fn from_row_values(values: &[Value]) -> crate::Result<Self> {
        if values.len() < 3 {
            return Err(crate::ormer_error!(
                "Type mismatch: expected tuple (T1, T2, T3)"
            ));
        }
        let v1 = T1::from_row_values(&values[0..1])?;
        let v2 = T2::from_row_values(&values[1..2])?;
        let v3 = T3::from_row_values(&values[2..3])?;
        Ok((v1, v2, v3))
    }
}

// 为 Option 类型实现 FromRowValues
impl<T: FromValue> FromRowValues for Option<T> {
    fn from_row_values(values: &[Value]) -> crate::Result<Self> {
        let value = first_row_value(values, format!("Option<{}>", std::any::type_name::<T>()))?;
        Option::<T>::from_value(value)
    }
}

// 使用宏生成 From<T> for Value 实现
macro_rules! impl_from_for_value {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$type> for Value {
                fn from(v: $type) -> Self {
                    Value::$variant(v as i64)
                }
            }
        )*
    };
}

// 为整数类型生成 From 实现
impl_from_for_value!(
    i8 => Integer,
    i16 => Integer,
    i32 => Integer,
    i64 => Integer,
    u8 => Integer,
    u16 => Integer,
    u32 => Integer,
    u64 => Integer,
    isize => Integer,
    usize => Integer,
);

// f64 特殊处理
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
    }
}

impl From<std::time::Duration> for Value {
    fn from(v: std::time::Duration) -> Self {
        Value::Duration(v)
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::Null
    }
}

// String 特殊处理
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

// bool 特殊处理（转为 Boolean）
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Boolean(v)
    }
}

// Vec<u8> (Bytes) 特殊处理
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(v)
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            _ => Err(crate::ormer_error!("Type mismatch: expected Vec<u8>")),
        }
    }
}

impl From<Vec<i32>> for Value {
    fn from(v: Vec<i32>) -> Self {
        Value::IntegerArray(v)
    }
}

impl FromValue for Vec<i32> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::IntegerArray(v) => Ok(v.clone()),
            _ => Err(crate::ormer_error!("Type mismatch: expected Vec<i32>")),
        }
    }
}

impl From<Vec<i64>> for Value {
    fn from(v: Vec<i64>) -> Self {
        Value::BigIntArray(v)
    }
}

impl FromValue for Vec<i64> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::BigIntArray(v) => Ok(v.clone()),
            Value::NullableBigIntArray(v) => v
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| crate::ormer_error!("Type mismatch: expected Vec<i64>")),
            _ => Err(crate::ormer_error!("Type mismatch: expected Vec<i64>")),
        }
    }
}

impl From<Vec<Option<i64>>> for Value {
    fn from(v: Vec<Option<i64>>) -> Self {
        Value::NullableBigIntArray(v)
    }
}

impl FromValue for Vec<Option<i64>> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::BigIntArray(v) => Ok(v.iter().copied().map(Some).collect()),
            Value::NullableBigIntArray(v) => Ok(v.clone()),
            _ => Err(crate::ormer_error!(
                "Type mismatch: expected Vec<Option<i64>>"
            )),
        }
    }
}

// chrono::DateTime<Utc> 特殊处理
impl From<chrono::DateTime<chrono::Utc>> for Value {
    fn from(v: chrono::DateTime<chrono::Utc>) -> Self {
        Value::DateTime(v)
    }
}

impl FromValue for chrono::DateTime<chrono::Utc> {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::DateTime(v) => Ok(*v),
            Value::Date(v) => Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                v.and_hms_opt(0, 0, 0)
                    .ok_or_else(|| crate::ormer_error!("Invalid date value"))?,
                chrono::Utc,
            )),
            Value::Text(v) => parse_utc_datetime_text(v),
            _ => Err(crate::ormer_error!("Type mismatch: expected DateTime<Utc>")),
        }
    }
}

// chrono::NaiveDateTime 特殊处理
impl From<chrono::NaiveDateTime> for Value {
    fn from(v: chrono::NaiveDateTime) -> Self {
        Value::DateTime(naive_local_to_utc(v))
    }
}

impl FromValue for chrono::NaiveDateTime {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::DateTime(v) => Ok(utc_to_naive_local(*v)),
            Value::Date(v) => v
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| crate::ormer_error!("Invalid date value")),
            Value::Text(v) => parse_naive_datetime_text(v),
            _ => Err(crate::ormer_error!("Type mismatch: expected NaiveDateTime")),
        }
    }
}

impl From<chrono::NaiveDate> for Value {
    fn from(v: chrono::NaiveDate) -> Self {
        Value::Date(v)
    }
}

impl FromValue for chrono::NaiveDate {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Date(v) => Ok(*v),
            Value::DateTime(v) => Ok(v.date_naive()),
            Value::Text(v) => parse_naive_date_text(v),
            _ => Err(crate::ormer_error!("Type mismatch: expected NaiveDate")),
        }
    }
}

impl From<chrono::NaiveTime> for Value {
    fn from(v: chrono::NaiveTime) -> Self {
        Value::Time(v)
    }
}

impl FromValue for chrono::NaiveTime {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Time(v) => Ok(*v),
            Value::DateTime(v) => Ok(v.time()),
            Value::Text(v) => parse_naive_time_text(v),
            _ => Err(crate::ormer_error!("Type mismatch: expected NaiveTime")),
        }
    }
}

// serde_json::Value 特殊处理
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        Value::Json(v)
    }
}

impl FromValue for serde_json::Value {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Json(v) => Ok(v.clone()),
            _ => Err(crate::ormer_error!(
                "Type mismatch: expected serde_json::Value"
            )),
        }
    }
}

// uuid::Uuid 特殊处理
impl From<uuid::Uuid> for Value {
    fn from(v: uuid::Uuid) -> Self {
        Value::Uuid(v)
    }
}

pub(crate) fn uuid_from_text(raw: &str) -> crate::Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw.trim())
        .map_err(|err| crate::ormer_error!("Type mismatch: expected uuid::Uuid text: {}", err))
}

impl FromValue for uuid::Uuid {
    fn from_value(value: &Value) -> crate::Result<Self> {
        match value {
            Value::Uuid(v) => Ok(*v),
            Value::Text(v) => uuid_from_text(v),
            _ => Err(crate::ormer_error!(
                "Type mismatch: expected uuid::Uuid or UUID text"
            )),
        }
    }
}

// 重新导出钩子 traits 以保持向后兼容
pub use crate::hooks::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate,
};
