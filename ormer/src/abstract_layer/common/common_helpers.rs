use super::super::DbType;
use crate::model::{
    FromRowValues, Model, Row, TableRoute, Value, VersionSnapshotUpdate, quote_column_reference,
    quote_identifier, quote_qualified_identifier, routed_model_table_name_for_db,
};
use crate::query::filter::FilterExpr;
use crate::query::filter_formatter::FilterFormatter;
#[cfg(any(
    feature = "postgresql",
    feature = "sqlite",
    feature = "mysql",
    feature = "duckdb",
    feature = "mssql"
))]
use crate::query::insert::InsertConflictAction;
#[cfg(any(
    feature = "postgresql",
    feature = "sqlite",
    feature = "duckdb",
    feature = "mssql"
))]
use crate::query::insert::InsertConflictTarget;
use crate::query::insert::{InsertAssignment, InsertConflict, InsertValue};
use crate::query::update::{UpdateAssignment, UpdateExpr, UpdateValue};
use std::collections::{HashMap, HashSet};

pub fn placeholder(db_type: DbType, _param_idx: usize) -> String {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => format!("${_param_idx}"),
        #[cfg(feature = "questdb")]
        DbType::QuestDB => format!("${_param_idx}"),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => format!("@P{_param_idx}"),
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => "?".to_string(),
        #[cfg(feature = "mysql")]
        DbType::MySQL => "?".to_string(),
        #[cfg(any(feature = "duckdb", feature = "clickhouse", feature = "influxdb"))]
        _ => "?".to_string(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    Active,
    Committed,
    RolledBack,
}

impl TransactionState {
    pub fn is_active(self) -> bool {
        self == Self::Active
    }

    pub fn is_closed(self) -> bool {
        !self.is_active()
    }
}

pub fn placeholder_list(db_type: DbType, start_idx: usize, count: usize) -> String {
    (0..count)
        .map(|offset| placeholder(db_type, start_idx + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn rebase_placeholder_sql(sql: &str, db_type: DbType, offset: usize) -> String {
    crate::query::filter_formatter::rebase_subquery_sql(sql, db_type, offset)
}

pub fn quote_table_name<T: Model>(db_type: DbType) -> String {
    quote_qualified_identifier(db_type, T::table_name_for_db(db_type))
}

/// 带默认 schema 省略规则的表名引用（运行时表名字符串入口，如
/// `Database::table_row_count`）。
///
/// 与模型版 [`quote_table_name`] 的差异：模型版对名字中的每个点分段
/// 逐一引用（`"schema"."table"`），本函数按后端语义省略默认 schema——
/// PostgreSQL 省略 `public`、MSSQL 省略 `dbo`、QuestDB 只取末段，
/// 其余后端直接引用归一化后的整名。
pub(crate) fn quote_table_name_with_schema(db_type: DbType, table_name: &str) -> String {
    let normalized = crate::model::normalize_table_name_for_db(db_type, table_name);
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
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
        DbType::MSSQL => {
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
        DbType::QuestDB => {
            let (_, table) = crate::model::split_schema_table_name(normalized, "public");
            crate::model::quote_identifier(db_type, table)
        }
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => crate::model::quote_identifier(db_type, normalized),
        #[cfg(feature = "mysql")]
        DbType::MySQL => crate::model::quote_identifier(db_type, normalized),
        #[cfg(any(
            feature = "duckdb",
            feature = "clickhouse",
            feature = "influxdb"
        ))]
        _ => crate::model::quote_identifier(db_type, normalized),
    }
}

pub fn quote_routed_table_name<T: Model>(
    db_type: DbType,
    route: &TableRoute,
) -> crate::Result<String> {
    let table_name = routed_model_table_name_for_db::<T>(db_type, route)?;
    Ok(quote_qualified_identifier(db_type, &table_name))
}

pub fn quote_column_list(db_type: DbType, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| quote_identifier(db_type, column))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_column_with_prefix(db_type: DbType, prefix: &str, column: &str) -> String {
    format!("{}.{}", prefix, quote_identifier(db_type, column))
}

pub fn quote_assignment(db_type: DbType, column: &str, value_sql: &str) -> String {
    format!("{} = {}", quote_identifier(db_type, column), value_sql)
}

#[derive(Debug, Clone)]
pub struct ModelUpdatePlan {
    pub sets: Vec<(String, Value)>,
    pub filters: Vec<FilterExpr>,
    pub version_update: Option<VersionSnapshotUpdate>,
}

pub type ModelUpdateBatch = Vec<ModelUpdatePlan>;

#[derive(Debug, Clone)]
pub struct ModelSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub versioned: bool,
    pub version_update: Option<VersionSnapshotUpdate>,
    pub param_columns: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct InsertSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub row_count: usize,
}

pub fn model_primary_key_filters<T: Model>(model: &T) -> Vec<FilterExpr> {
    primary_key_filter_exprs(T::primary_key_columns(), model.primary_key_values())
}

pub(crate) fn primary_key_filter_exprs(
    pk_columns: &[&'static str],
    pk_values: Vec<Value>,
) -> Vec<FilterExpr> {
    pk_columns
        .iter()
        .zip(pk_values)
        .map(|(col, val)| FilterExpr::Comparison {
            column: col.to_string(),
            operator: "=".to_string(),
            value: val.clone(),
        })
        .collect()
}

pub(crate) fn and_filter_exprs(filters: Vec<FilterExpr>) -> Option<FilterExpr> {
    filters
        .into_iter()
        .reduce(|a, b| FilterExpr::And(Box::new(a), Box::new(b)))
}

pub fn model_update_plan<T: Model>(
    model: &T,
    fields: Option<&[String]>,
) -> Option<ModelUpdatePlan> {
    let version_info = T::version_info();
    let old_version = version_info.map(|_| crate::model::model_version(model));
    let mut sets = match fields {
        Some(fields) => model.non_pk_field_values_for_columns(fields),
        None => model.non_pk_field_values(),
    }
    .into_iter()
    .filter(|(column, _)| {
        version_info
            .map(|info| *column != info.column)
            .unwrap_or(true)
    })
    .map(|(column, value)| (column.to_string(), value))
    .collect::<Vec<_>>();

    if let Some(info) = version_info {
        let next_version = old_version.unwrap_or(info.initial).saturating_add(1);
        sets.push((info.column.to_string(), Value::from(next_version)));
    }

    if sets.is_empty() {
        return None;
    }

    let mut filters = model_primary_key_filters(model);
    let version_update = if let Some(info) = version_info {
        let old_version = old_version.unwrap_or(info.initial);
        filters.push(FilterExpr::Comparison {
            column: info.column.to_string(),
            operator: "=".to_string(),
            value: Value::from(old_version),
        });
        crate::model::version_snapshot_update(model, old_version)
    } else {
        None
    };

    Some(ModelUpdatePlan {
        sets,
        filters,
        version_update,
    })
}

pub fn model_delete_filters<T: Model>(model: &T) -> Vec<FilterExpr> {
    let mut filters = model_primary_key_filters(model);
    if let Some(info) = T::version_info() {
        let version = crate::model::model_version(model);
        filters.push(FilterExpr::Comparison {
            column: info.column.to_string(),
            operator: "=".to_string(),
            value: Value::from(version),
        });
    }
    filters
}

pub fn push_filters_sql(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    filters: &[FilterExpr],
) -> crate::Result<()> {
    if filters.is_empty() {
        return Ok(());
    }
    sql.push_str(" WHERE ");
    let mut param_idx = params.len() + 1;
    for (i, filter) in filters.iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
    }
    Ok(())
}

pub fn build_delete_sql<T: Model>(
    db_type: DbType,
    filters: &[FilterExpr],
) -> crate::Result<(String, Vec<Value>)> {
    let mut sql = format!("DELETE FROM {}", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    push_filters_sql(db_type, &mut sql, &mut params, filters)?;
    Ok((sql, params))
}

// ===== 时序数据按块删除（Block Delete）公共层 =====

/// 时序数据分块粒度。
///
/// 由 `#[hypertable(Duration)]` 时长按唯一映射推导（QuestDB 分区、ClickHouse
/// 默认 `PARTITION BY` 与按块删除对齐共用同一份映射）：
/// `< 1h` → 小时，`1h ≤ d < 7d` → 天，`7d ≤ d < 30d` → 周，
/// `30d ≤ d < 365d` → 月，`≥ 365d` → 年。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl PartitionUnit {
    /// 「时长 → 分块粒度」的唯一映射实现。
    pub fn from_duration(duration: std::time::Duration) -> Self {
        const HOUR_SECS: f64 = 3600.0;
        const DAY_SECS: f64 = 86_400.0;
        let secs = duration.as_secs_f64();
        if secs < HOUR_SECS {
            Self::Hour
        } else if secs < 7.0 * DAY_SECS {
            Self::Day
        } else if secs < 30.0 * DAY_SECS {
            Self::Week
        } else if secs < 365.0 * DAY_SECS {
            Self::Month
        } else {
            Self::Year
        }
    }

    /// QuestDB `PARTITION BY` 单位名。
    pub fn questdb_unit(self) -> &'static str {
        match self {
            Self::Hour => "HOUR",
            Self::Day => "DAY",
            Self::Week => "WEEK",
            Self::Month => "MONTH",
            Self::Year => "YEAR",
        }
    }

    /// ClickHouse 默认 `PARTITION BY` 时间函数名。
    pub fn clickhouse_function(self) -> &'static str {
        match self {
            Self::Hour => "toStartOfHour",
            Self::Day => "toYYYYMMDD",
            Self::Week => "toMonday",
            Self::Month => "toYYYYMM",
            Self::Year => "toYYYY",
        }
    }

    /// 识别 ClickHouse `partition_by` 表达式（仅支持映射表列出的时间函数形式），
    /// 返回 `(粒度, 列名)`。
    pub fn parse_clickhouse_partition_by(expr: &str) -> Option<(Self, String)> {
        let expr = expr.trim();
        let open = expr.find('(')?;
        let close = expr.rfind(')')?;
        if close != expr.len() - 1 {
            return None;
        }
        let name = expr[..open].trim();
        let column = expr[open + 1..close]
            .trim()
            .trim_matches(|c| c == '\'' || c == '`' || c == '"')
            .trim();
        let unit = match name {
            "toStartOfHour" => Self::Hour,
            "toYYYYMMDD" => Self::Day,
            "toMonday" => Self::Week,
            "toYYYYMM" => Self::Month,
            "toYYYY" => Self::Year,
            _ => return None,
        };
        if column.is_empty() {
            return None;
        }
        Some((unit, column.to_string()))
    }

    /// ClickHouse `system.parts.partition` 的 key 文本解析为分区起始时间（UTC）。
    pub fn parse_clickhouse_partition_key(
        self,
        key: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::TimeZone;

        let naive = match self {
            Self::Hour => chrono::NaiveDateTime::parse_from_str(key.trim(), "%Y-%m-%d %H:%M:%S").ok()?,
            Self::Day => {
                let n: u32 = key.trim().parse().ok()?;
                chrono::NaiveDate::from_ymd_opt((n / 10_000) as i32, n / 100 % 100, n % 100)?
                    .and_hms_opt(0, 0, 0)?
            }
            Self::Week => chrono::NaiveDate::parse_from_str(key.trim(), "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)?,
            Self::Month => {
                let n: u32 = key.trim().parse().ok()?;
                chrono::NaiveDate::from_ymd_opt((n / 100) as i32, n % 100, 1)?.and_hms_opt(0, 0, 0)?
            }
            Self::Year => {
                let n: u32 = key.trim().parse().ok()?;
                chrono::NaiveDate::from_ymd_opt(n as i32, 1, 1)?.and_hms_opt(0, 0, 0)?
            }
        };
        Some(chrono::Utc.from_utc_datetime(&naive))
    }

    /// 把时间向下对齐（floor）到所在块边界（UTC 日历对齐）。
    pub fn align_to_block(self, t: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
        use chrono::{Datelike, TimeZone, Timelike};
        let naive = t.naive_utc();
        let date = naive.date();
        let aligned = match self {
            Self::Hour => date.and_hms_opt(naive.hour(), 0, 0),
            Self::Day => date.and_hms_opt(0, 0, 0),
            Self::Week => {
                let monday =
                    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
                monday.and_hms_opt(0, 0, 0)
            }
            Self::Month => date.with_day(1).and_then(|d| d.and_hms_opt(0, 0, 0)),
            Self::Year => date
                .with_month(1)
                .and_then(|d| d.with_day(1))
                .and_then(|d| d.and_hms_opt(0, 0, 0)),
        };
        chrono::Utc
            .from_utc_datetime(&aligned.expect("block boundary is always a valid datetime"))
    }

    /// 当前块边界的下一个边界（用于 `between` 起点的向上对齐）。
    /// 溢出转为错误，不 panic（与模块其余路径的空区间 no-op / 参数报错约定一致）。
    fn next_block_boundary(
        self,
        t: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<chrono::DateTime<chrono::Utc>> {
        use chrono::{Months, TimeZone};
        let naive = t.naive_utc();
        let naive = match self {
            Self::Hour => naive + chrono::Duration::hours(1),
            Self::Day => naive + chrono::Duration::days(1),
            Self::Week => naive + chrono::Duration::weeks(1),
            Self::Month => naive.checked_add_months(Months::new(1)).ok_or_else(|| {
                crate::OrmerError::invalid_operation(
                    "next block boundary out of range (month overflow)",
                )
            })?,
            Self::Year => naive.checked_add_months(Months::new(12)).ok_or_else(|| {
                crate::OrmerError::invalid_operation(
                    "next block boundary out of range (year overflow)",
                )
            })?,
        };
        Ok(chrono::Utc.from_utc_datetime(&naive))
    }

    /// 向上对齐（ceil）：不在块边界上时进到下一个边界。
    fn align_to_block_ceil(
        self,
        t: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<chrono::DateTime<chrono::Utc>> {
        let floor = self.align_to_block(t);
        if floor == t {
            Ok(floor)
        } else {
            self.next_block_boundary(floor)
        }
    }
}

/// 模型时序分块声明的解析结果：时间列 + 分块粒度。
#[derive(Debug, Clone, PartialEq)]
pub struct BlockKey {
    pub time_column: String,
    pub unit: PartitionUnit,
}

/// 块删除范围选择：`before` / `between` / `retain` 三选一。
#[derive(Debug, Clone, Copy)]
pub enum BlockRange {
    /// 删除所有「块结束时间 ≤ cutoff」的块（cutoff 向下对齐到块边界）。
    Before {
        cutoff: chrono::DateTime<chrono::Utc>,
    },
    /// 删除完整落在 `[start, end)` 内的块（两端对齐到块边界）。
    Between {
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    },
    /// 保留最近 `duration` 的数据；`now` 由调用方在执行期提供（客户端时钟）。
    Retain { duration: std::time::Duration },
}

/// 对齐后的半开块区间 `[start, end)`；`start` 为 `None` 表示无下界（`before`/`retain`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignedBlockRange {
    pub start: Option<chrono::DateTime<chrono::Utc>>,
    pub end: chrono::DateTime<chrono::Utc>,
}

impl AlignedBlockRange {
    /// 按统一语义对齐 `BlockRange`。返回 `None` 表示对齐后为空区间（安全 no-op）；
    /// `between` 的 `start >= end` 报参数错误。
    pub fn align(
        range: BlockRange,
        unit: PartitionUnit,
        now: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<Option<Self>> {
        match range {
            BlockRange::Before { cutoff } => Ok(Some(Self {
                start: None,
                end: unit.align_to_block(cutoff),
            })),
            BlockRange::Between { start, end } => {
                if start >= end {
                    return Err(crate::OrmerError::invalid_operation(
                        "block delete between requires start < end",
                    ));
                }
                let start = unit.align_to_block_ceil(start)?;
                let end = unit.align_to_block(end);
                if start >= end {
                    return Ok(None);
                }
                Ok(Some(Self {
                    start: Some(start),
                    end,
                }))
            }
            BlockRange::Retain { duration } => {
                let step = chrono::Duration::from_std(duration).map_err(|_| {
                    crate::OrmerError::invalid_operation(
                        "block delete retain duration is out of range",
                    )
                })?;
                Ok(Some(Self {
                    start: None,
                    end: unit.align_to_block(now - step),
                }))
            }
        }
    }

    /// 判断以 `block_start` 起始的块是否完整落在区间内。
    pub fn contains_block(&self, block_start: chrono::DateTime<chrono::Utc>) -> bool {
        if let Some(start) = self.start {
            if block_start < start {
                return false;
            }
        }
        block_start < self.end
    }
}

/// 解析模型的时序分块声明；无 `#[hypertable]` 时间列声明时报 `UnsupportedFeature`。
pub fn resolve_block_key<T: Model>(db_type: DbType) -> crate::Result<BlockKey> {
    let missing = || crate::OrmerError::UnsupportedFeature {
        backend: db_type,
        feature: "block delete (declare #[hypertable(Duration)] on the model's time column)",
    };
    let time_column = T::ts_time_key().ok_or_else(missing)?;
    let duration = T::ts_block_interval().ok_or_else(missing)?;
    Ok(BlockKey {
        time_column: time_column.to_string(),
        unit: PartitionUnit::from_duration(duration),
    })
}

/// Resolve the timestamp column for an HTTP time-series backend.
///
/// InfluxDB models use the timestamp declared by `#[hypertable]`, or a single
/// `DateTime` primary key when no chunk interval is needed.
pub fn resolve_influx_time_key<T: Model>(db_type: DbType) -> crate::Result<String> {
    if let Some(column) = T::ts_time_key() {
        return Ok(column.to_string());
    }

    let mut timestamp_columns = T::column_schema()
        .into_iter()
        .filter(|column| {
            column.is_primary
                && (column.rust_type.starts_with("DateTime<")
                    || column.rust_type.starts_with("chrono::DateTime<")
                    || column.rust_type.starts_with("Option<DateTime<")
                    || column.rust_type.starts_with("Option<chrono::DateTime<"))
        })
        .map(|column| column.name.to_string())
        .collect::<Vec<_>>();
    if timestamp_columns.len() == 1 {
        return Ok(timestamp_columns.remove(0));
    }

    Err(crate::OrmerError::UnsupportedFeature {
        backend: db_type,
        feature: "block delete (declare #[hypertable(Duration)] or mark one DateTime field #[primary])",
    })
}

/// Raw time-range bounds for backends whose server owns shard boundaries.
/// Returns `None` for a safe no-op; reversed `between` remains an error.
pub fn time_delete_bounds(
    range: BlockRange,
    now: chrono::DateTime<chrono::Utc>,
) -> crate::Result<
    Option<(
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
    )>,
> {
    match range {
        BlockRange::Before { cutoff } => Ok(Some((None, cutoff))),
        BlockRange::Between { start, end } => {
            if start >= end {
                return Err(crate::OrmerError::invalid_operation(
                    "block delete between requires start < end",
                ));
            }
            Ok(Some((Some(start), end)))
        }
        BlockRange::Retain { duration } => {
            let step = chrono::Duration::from_std(duration).map_err(|_| {
                crate::OrmerError::invalid_operation(
                    "block delete retain duration is out of range",
                )
            })?;
            Ok(Some((None, now - step)))
        }
    }
}

/// 回退路径（行删除）的过滤条件：对齐后的块区间 → 时间列边界。
pub fn block_delete_fallback_filters(
    range: &AlignedBlockRange,
    time_column: &str,
) -> Vec<FilterExpr> {
    let mut filters = Vec::with_capacity(2);
    if let Some(start) = range.start {
        filters.push(FilterExpr::Comparison {
            column: time_column.to_string(),
            operator: ">=".to_string(),
            value: Value::DateTime(start),
        });
    }
    filters.push(FilterExpr::Comparison {
        column: time_column.to_string(),
        operator: "<".to_string(),
        value: Value::DateTime(range.end),
    });
    filters
}

/// OLTP 后端的块删除回退 SQL：`DELETE FROM t WHERE <对齐边界>`。
///
/// 返回 `None` 表示空区间（安全 no-op）；SQL 带 `block_delete_fallback` 注释，
/// 供 sql_trace 识别回退路径。
pub fn build_block_delete_fallback_sql<T: Model>(
    db_type: DbType,
    key: &BlockKey,
    range: &AlignedBlockRange,
) -> crate::Result<Option<(String, Vec<Value>)>> {
    let filters = block_delete_fallback_filters(range, &key.time_column);
    let (sql, params) = build_delete_sql::<T>(db_type, &filters)?;
    Ok(Some((format!("/* block_delete_fallback */ {sql}"), params)))
}

pub fn build_update_sql<T: Model>(
    db_type: DbType,
    sets: &[UpdateAssignment],
    filters: &[FilterExpr],
) -> crate::Result<(String, Vec<Value>)> {
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    for (index, assignment) in sets.iter().enumerate() {
        validate_update_assignment(db_type, assignment)?;
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format_update_assignment(db_type, assignment, &mut params));
    }
    push_filters_sql(db_type, &mut sql, &mut params, filters)?;
    Ok((sql, params))
}

fn validate_update_assignment(db_type: DbType, assignment: &UpdateAssignment) -> crate::Result<()> {
    validate_update_value(db_type, &assignment.value)
}

fn validate_update_value(db_type: DbType, value: &UpdateValue) -> crate::Result<()> {
    match value {
        UpdateValue::Literal(_) => Ok(()),
        UpdateValue::Expr(expr) => validate_update_expr(db_type, expr),
    }
}

fn validate_update_expr(db_type: DbType, expr: &UpdateExpr) -> crate::Result<()> {
    match expr {
        UpdateExpr::Sql(expr) => expr.validate_for_db(db_type),
        UpdateExpr::Binary { left, right, .. } => {
            validate_update_expr(db_type, left)?;
            validate_update_expr(db_type, right)
        }
        _ => Ok(()),
    }
}

/// QuestDB 语义约束：designated timestamp 列不可被 UPDATE。
///
/// QuestDB 的 UPDATE 为 copy-on-write 实现，服务端明确禁止更新
/// `timestamp(col)` 声明的 designated timestamp 列。这里按模型声明
/// （`#[hypertable]` 时间列）统一校验，供各执行路径复用。
#[cfg(feature = "questdb")]
pub fn validate_questdb_update_columns<'a, T: Model>(
    db_type: DbType,
    columns: impl IntoIterator<Item = &'a str>,
) -> crate::Result<()> {
    if !db_type.is_questdb() {
        return Ok(());
    }
    let Some((time_column, _)) = T::hypertable_info() else {
        return Ok(());
    };
    if columns.into_iter().any(|column| column == time_column) {
        return Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "update on the designated timestamp column (QuestDB forbids updating timestamp(...) columns; rewrite history via block delete and re-insert)",
        });
    }
    Ok(())
}

pub fn build_model_update_sql<T: Model>(
    db_type: DbType,
    plan: &ModelUpdatePlan,
) -> crate::Result<ModelSqlStatement> {
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    for (index, (column, value)) in plan.sets.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            db_type,
            column,
            &placeholder(db_type, params.len() + 1),
        ));
        params.push(value.clone());
    }
    push_filters_sql(db_type, &mut sql, &mut params, &plan.filters)?;
    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: plan.version_update.is_some(),
        version_update: plan.version_update.clone(),
        param_columns: None,
    })
}

#[cfg(feature = "duckdb")]
pub fn build_duckdb_graph_update_sql<T: Model>(
    model: &T,
    plan: &ModelUpdatePlan,
) -> crate::Result<ModelSqlStatement> {
    const TARGET: &str = "__ormer_update_target";
    const SOURCE: &str = "__ormer_update_source";
    let db_type = DbType::DuckDB;
    let mut source_columns = Vec::new();
    let mut join_conditions = Vec::new();
    let mut params = Vec::new();

    for (index, column) in T::primary_key_columns().iter().enumerate() {
        let value = model.column_value(column).ok_or_else(|| {
            crate::ormer_error!("Missing graph update primary key value for column {column}")
        })?;
        let alias = format!("__ormer_key_{index}");
        source_columns.push(format!(
            "{} AS {}",
            placeholder(db_type, params.len() + 1),
            quote_identifier(db_type, &alias)
        ));
        params.push(value);
        join_conditions.push(format!(
            "{}.{} = {}.{}",
            TARGET,
            quote_identifier(db_type, column),
            SOURCE,
            quote_identifier(db_type, &alias)
        ));
    }

    if let Some(info) = T::version_info() {
        let old_version = crate::model::model_version(model);
        let alias = "__ormer_old_version";
        source_columns.push(format!(
            "{} AS {}",
            placeholder(db_type, params.len() + 1),
            quote_identifier(db_type, alias)
        ));
        params.push(Value::from(old_version));
        join_conditions.push(format!(
            "{}.{} = {}.{}",
            TARGET,
            quote_identifier(db_type, info.column),
            SOURCE,
            quote_identifier(db_type, alias)
        ));
    }

    let set_assignments = plan
        .sets
        .iter()
        .enumerate()
        .map(|(index, (column, value))| {
            let alias = format!("__ormer_value_{index}");
            source_columns.push(format!(
                "{} AS {}",
                placeholder(db_type, params.len() + 1),
                quote_identifier(db_type, &alias)
            ));
            params.push(value.clone());
            Ok(format!(
                "{} = {}.{}",
                quote_identifier(db_type, column),
                SOURCE,
                quote_identifier(db_type, &alias)
            ))
        })
        .collect::<crate::Result<Vec<_>>>()?;

    let sql = format!(
        "MERGE INTO {} AS {TARGET} USING (SELECT {}) AS {SOURCE} \
         ON {} WHEN MATCHED THEN UPDATE SET {}",
        quote_table_name::<T>(db_type),
        source_columns.join(", "),
        join_conditions.join(" AND "),
        set_assignments.join(", ")
    );
    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: plan.version_update.is_some(),
        version_update: plan.version_update.clone(),
        param_columns: None,
    })
}

pub fn bind_param_limit(db_type: DbType) -> usize {
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => 999,
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => 65_535,
        #[cfg(feature = "questdb")]
        DbType::QuestDB => 65_535,
        #[cfg(feature = "mysql")]
        DbType::MySQL => 65_535,
        #[cfg(feature = "mssql")]
        DbType::MSSQL => 2_100,
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => 65_535,
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => 65_535,
        #[cfg(feature = "influxdb")]
        DbType::InfluxDB => 65_535,
    }
}

pub(crate) fn model_value_key(value: &Value) -> String {
    match value {
        Value::Integer(v) => format!("i:{v}"),
        Value::BigInt(v) => format!("b:{v}"),
        Value::Duration(v) => format!("du:{v:?}"),
        Value::Text(v) => format!("t:{v}"),
        Value::TextArray(v) => format!("ta:{v:?}"),
        Value::Real(v) => format!("r:{v}"),
        Value::Decimal(v) => format!("de:{v}"),
        Value::BigDecimal(v) => format!("bd:{v}"),
        Value::Boolean(v) => format!("bo:{v}"),
        Value::Bytes(v) => format!("x:{v:?}"),
        Value::IntegerArray(v) => format!("ia:{v:?}"),
        Value::BigIntArray(v) => format!("ba:{v:?}"),
        Value::NullableBigIntArray(v) => format!("nba:{v:?}"),
        Value::DateTime(v) => format!("dt:{v}"),
        Value::Date(v) => format!("d:{v}"),
        Value::Time(v) => format!("ti:{v}"),
        Value::Json(v) => format!("j:{v}"),
        Value::Uuid(v) => format!("u:{v}"),
        Value::Null => "n:".to_string(),
    }
}

fn extract_model_update_pk_values<T: Model>(plan: &ModelUpdatePlan) -> Option<Vec<Value>> {
    let pk_columns = T::primary_key_columns();
    if pk_columns.is_empty() || plan.filters.len() != pk_columns.len() {
        return None;
    }

    let mut values = Vec::with_capacity(pk_columns.len());
    for pk in pk_columns {
        let value = plan.filters.iter().find_map(|filter| match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } if column == pk && operator == "=" => Some(value.clone()),
            _ => None,
        })?;
        values.push(value);
    }
    Some(values)
}

fn model_update_set_columns(plans: &[ModelUpdatePlan]) -> Option<Vec<String>> {
    let first = plans.first()?;
    let columns = first
        .sets
        .iter()
        .map(|(column, _)| column.clone())
        .collect::<Vec<_>>();

    if columns.is_empty() {
        return None;
    }

    plans
        .iter()
        .all(|plan| {
            plan.version_update.is_none()
                && plan.sets.len() == columns.len()
                && plan
                    .sets
                    .iter()
                    .zip(&columns)
                    .all(|((column, _), expected)| column == expected)
        })
        .then_some(columns)
}

fn model_update_pk_values<T: Model>(plans: &[ModelUpdatePlan]) -> Option<Vec<Vec<Value>>> {
    let mut seen = HashSet::new();
    let mut values = Vec::with_capacity(plans.len());
    for plan in plans {
        let pk_values = extract_model_update_pk_values::<T>(plan)?;
        let key = pk_values
            .iter()
            .map(model_value_key)
            .collect::<Vec<_>>()
            .join("|");
        if !seen.insert(key) {
            return None;
        }
        values.push(pk_values);
    }
    Some(values)
}

#[cfg(feature = "sqlite")]
fn bulk_update_params_per_row(db_type: DbType, pk_count: usize, set_count: usize) -> usize {
    if matches!(db_type, DbType::Sqlite) {
        set_count * (pk_count + 1) + pk_count
    } else {
        pk_count + set_count
    }
}

#[cfg(not(feature = "sqlite"))]
fn bulk_update_params_per_row(_db_type: DbType, pk_count: usize, set_count: usize) -> usize {
    pk_count + set_count
}

fn bulk_update_rows_per_statement(db_type: DbType, pk_count: usize, set_count: usize) -> usize {
    let params_per_row = bulk_update_params_per_row(db_type, pk_count, set_count).max(1);
    (bind_param_limit(db_type) / params_per_row).max(1)
}

#[cfg(feature = "sqlite")]
fn push_pk_match_sql(
    db_type: DbType,
    sql: &mut String,
    pk_columns: &[&'static str],
    pk_values: &[Value],
    params: &mut Vec<Value>,
) {
    if pk_columns.len() > 1 {
        sql.push('(');
    }
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_identifier(db_type, pk),
            placeholder(db_type, params.len() + 1)
        ));
        params.push(pk_values[index].clone());
    }
    if pk_columns.len() > 1 {
        sql.push(')');
    }
}

fn plan_set_value<'a>(plan: &'a ModelUpdatePlan, column: &str) -> Option<&'a Value> {
    plan.sets
        .iter()
        .find_map(|(set_column, value)| (set_column == column).then_some(value))
}

#[cfg(feature = "sqlite")]
fn build_sqlite_bulk_model_update_sql<T: Model>(
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let db_type = DbType::Sqlite;
    let pk_columns = T::primary_key_columns();
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    let mut param_columns = Vec::new();

    for (set_index, column) in set_columns.iter().enumerate() {
        if set_index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format!("{} = CASE ", quote_identifier(db_type, column)));
        for (plan, pk_values) in plans.iter().zip(pk_values) {
            sql.push_str("WHEN ");
            push_pk_match_sql(db_type, &mut sql, pk_columns, pk_values, &mut params);
            param_columns.extend(pk_columns.iter().map(|column| (*column).to_string()));
            sql.push_str(" THEN ");
            sql.push_str(&placeholder(db_type, params.len() + 1));
            let value = plan_set_value(plan, column).ok_or_else(|| {
                crate::ormer_error!("Missing bulk update value for column {column}")
            })?;
            params.push(value.clone());
            param_columns.push(column.clone());
            sql.push(' ');
        }
        sql.push_str(&format!("ELSE {} END", quote_identifier(db_type, column)));
    }

    sql.push_str(" WHERE ");
    for (index, pk_values) in pk_values.iter().enumerate() {
        if index > 0 {
            sql.push_str(" OR ");
        }
        push_pk_match_sql(db_type, &mut sql, pk_columns, pk_values, &mut params);
        param_columns.extend(pk_columns.iter().map(|column| (*column).to_string()));
    }

    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

#[cfg(any(
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb"
))]
fn bulk_source_columns<'a>(
    pk_columns: &'a [&'static str],
    set_columns: &'a [String],
) -> Vec<&'a str> {
    pk_columns
        .iter()
        .map(|column| *column)
        .chain(set_columns.iter().map(String::as_str))
        .collect()
}

#[cfg(any(
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql",
    feature = "duckdb"
))]
fn push_bulk_source_row_values<T: Model>(
    params: &mut Vec<Value>,
    param_columns: &mut Vec<String>,
    plan: &ModelUpdatePlan,
    pk_values: &[Value],
    set_columns: &[String],
) -> crate::Result<()> {
    for (pk, value) in T::primary_key_columns().iter().zip(pk_values) {
        params.push(value.clone());
        param_columns.push((*pk).to_string());
    }
    for column in set_columns {
        let value = plan_set_value(plan, column)
            .ok_or_else(|| crate::ormer_error!("Missing bulk update value for column {column}"))?;
        params.push(value.clone());
        param_columns.push(column.clone());
    }
    Ok(())
}

#[cfg(feature = "postgresql")]
fn postgres_bulk_source_cast_type<T: Model>(column: &str) -> crate::Result<String> {
    let schema = T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .ok_or_else(|| crate::ormer_error!("Unknown bulk update column {column}"))?;

    if let Some(db_value_type) = schema.db_value_type {
        return Ok(db_value_type(DbType::PostgreSQL).to_string());
    }

    Ok(DbType::PostgreSQL.sql_type(
        schema.data_type.unwrap_or(schema.rust_type),
        false,
        false,
        true,
        schema.enum_variants,
    ))
}

#[cfg(feature = "postgresql")]
fn postgres_casted_placeholder_list<T: Model>(
    start_idx: usize,
    source_columns: &[&str],
) -> crate::Result<String> {
    source_columns
        .iter()
        .enumerate()
        .map(|(offset, column)| {
            let cast_type = postgres_bulk_source_cast_type::<T>(column)?;
            Ok(format!(
                "CAST({} AS {cast_type})",
                placeholder(DbType::PostgreSQL, start_idx + offset)
            ))
        })
        .collect::<crate::Result<Vec<_>>>()
        .map(|parts| parts.join(", "))
}

#[cfg(any(feature = "postgresql", feature = "mssql", feature = "duckdb"))]
fn build_values_source_bulk_model_update_sql<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let pk_columns = T::primary_key_columns();
    let source_columns = bulk_source_columns(pk_columns, set_columns);
    let source_column_list = quote_column_list(db_type, &source_columns);
    let source_width = source_columns.len();
    let mut params = Vec::new();
    let mut param_columns = Vec::new();

    let mut values_sql = String::new();
    for (index, (plan, pk_values)) in plans.iter().zip(pk_values).enumerate() {
        if index > 0 {
            values_sql.push_str(", ");
        }
        values_sql.push('(');
        #[cfg(feature = "postgresql")]
        if matches!(db_type, DbType::PostgreSQL) {
            values_sql.push_str(&postgres_casted_placeholder_list::<T>(
                params.len() + 1,
                &source_columns,
            )?);
        } else {
            values_sql.push_str(&placeholder_list(db_type, params.len() + 1, source_width));
        }
        #[cfg(not(feature = "postgresql"))]
        values_sql.push_str(&placeholder_list(db_type, params.len() + 1, source_width));
        values_sql.push(')');
        push_bulk_source_row_values::<T>(
            &mut params,
            &mut param_columns,
            plan,
            pk_values,
            set_columns,
        )?;
    }

    let table = quote_table_name::<T>(db_type);
    let sql = match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let assignments = source_assignments_sql(db_type, set_columns, None);
            let predicates = source_pk_predicates_sql(db_type, pk_columns);
            format!(
                "UPDATE {table} AS target SET {assignments} FROM (VALUES {values_sql}) AS source ({source_column_list}) WHERE {predicates}"
            )
        }
        #[cfg(feature = "questdb")]
        DbType::QuestDB => String::new(),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            let assignments = source_assignments_sql(db_type, set_columns, Some("target"));
            let predicates = source_pk_predicates_sql(db_type, pk_columns);
            format!(
                "UPDATE target SET {assignments} FROM {table} AS target JOIN (VALUES {values_sql}) AS source ({source_column_list}) ON {predicates}"
            )
        }
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            let assignments = source_assignments_sql(db_type, set_columns, None);
            let predicates = source_pk_predicates_sql(db_type, pk_columns);
            format!(
                "UPDATE {table} AS target SET {assignments} FROM (VALUES {values_sql}) AS source ({source_column_list}) WHERE {predicates}"
            )
        }
        #[cfg(any(feature = "sqlite", feature = "mysql"))]
        _ => {
            return Err(crate::ormer_error!(
                "VALUES source bulk update is not supported for this database"
            ));
        }
    };

    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

#[cfg(any(
    feature = "postgresql",
    feature = "mssql",
    feature = "mysql",
    feature = "duckdb"
))]
fn source_assignments_sql(
    db_type: DbType,
    set_columns: &[String],
    target_prefix: Option<&str>,
) -> String {
    set_columns
        .iter()
        .map(|column| {
            let target = target_prefix
                .map(|prefix| quote_column_with_prefix(db_type, prefix, column))
                .unwrap_or_else(|| quote_identifier(db_type, column));
            format!(
                "{} = {}",
                target,
                quote_column_with_prefix(db_type, "source", column)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(any(
    feature = "postgresql",
    feature = "mssql",
    feature = "mysql",
    feature = "duckdb"
))]
fn source_pk_predicates_sql(db_type: DbType, pk_columns: &[&'static str]) -> String {
    pk_columns
        .iter()
        .map(|pk| {
            format!(
                "{} = {}",
                quote_column_with_prefix(db_type, "target", pk),
                quote_column_with_prefix(db_type, "source", pk)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

#[cfg(feature = "mysql")]
fn build_mysql_bulk_model_update_sql<T: Model>(
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let db_type = DbType::MySQL;
    let pk_columns = T::primary_key_columns();
    let source_columns = bulk_source_columns(pk_columns, set_columns);
    let source_width = source_columns.len();
    let mut params = Vec::new();
    let mut param_columns = Vec::new();
    let mut source_sql = String::new();

    for (index, (plan, pk_values)) in plans.iter().zip(pk_values).enumerate() {
        if index > 0 {
            source_sql.push_str(" UNION ALL ");
        }
        source_sql.push_str("SELECT ");
        for (column_index, column) in source_columns.iter().enumerate() {
            if column_index > 0 {
                source_sql.push_str(", ");
            }
            source_sql.push_str(&placeholder(db_type, params.len() + column_index + 1));
            if index == 0 {
                source_sql.push_str(" AS ");
                source_sql.push_str(&quote_identifier(db_type, column));
            }
        }
        push_bulk_source_row_values::<T>(
            &mut params,
            &mut param_columns,
            plan,
            pk_values,
            set_columns,
        )?;
        debug_assert_eq!(params.len(), (index + 1) * source_width);
    }

    let assignments = source_assignments_sql(db_type, set_columns, Some("target"));
    let predicates = source_pk_predicates_sql(db_type, pk_columns);

    Ok(ModelSqlStatement {
        sql: format!(
            "UPDATE {} AS target JOIN ({source_sql}) AS source ON {predicates} SET {assignments}",
            quote_table_name::<T>(db_type)
        ),
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

// plans 仅由各后端 cfg 分支消费；仅启用无本地写后端的组合时未使用
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
fn build_bulk_model_update_sql<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => build_sqlite_bulk_model_update_sql::<T>(plans, pk_values, set_columns),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            build_values_source_bulk_model_update_sql::<T>(db_type, plans, pk_values, set_columns)
        }
        #[cfg(feature = "questdb")]
        DbType::QuestDB => Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "bulk model updates",
        }),
        #[cfg(feature = "influxdb")]
        DbType::InfluxDB => Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "bulk model updates",
        }),
        #[cfg(feature = "mysql")]
        DbType::MySQL => build_mysql_bulk_model_update_sql::<T>(plans, pk_values, set_columns),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            build_values_source_bulk_model_update_sql::<T>(db_type, plans, pk_values, set_columns)
        }
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            build_values_source_bulk_model_update_sql::<T>(db_type, plans, pk_values, set_columns)
        }
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "bulk model updates",
        }),
    }
}

pub fn build_bulk_model_update_statements<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
) -> crate::Result<Option<Vec<ModelSqlStatement>>> {
    if plans.len() <= 1 {
        return Ok(None);
    }

    let set_columns = match model_update_set_columns(plans) {
        Some(columns) => columns,
        None => return Ok(None),
    };
    let pk_values = match model_update_pk_values::<T>(plans) {
        Some(values) => values,
        None => return Ok(None),
    };
    let pk_count = T::primary_key_columns().len();
    let rows_per_statement =
        bulk_update_rows_per_statement(db_type, pk_count, set_columns.len()).max(1);
    if rows_per_statement <= 1 {
        return Ok(None);
    }

    let mut statements = Vec::new();
    let mut start = 0;
    while start < plans.len() {
        let end = (start + rows_per_statement).min(plans.len());
        statements.push(build_bulk_model_update_sql::<T>(
            db_type,
            &plans[start..end],
            &pk_values[start..end],
            &set_columns,
        )?);
        start = end;
    }

    Ok(Some(statements))
}

pub fn optimistic_lock_conflict<T: Model>() -> crate::OrmerError {
    if let Some(info) = T::version_info() {
        crate::OrmerError::optimistic_lock(T::TABLE_NAME, info.column)
    } else {
        crate::ormer_error!("Optimistic lock conflict on {}", T::TABLE_NAME)
    }
}

/// `update/delete().returning()` 路径的乐观锁冲突检查：版本化语句未返回任何行
/// 视为并发冲突。各后端 returning 实现必须与 `execute()`（affected == 0 报错）
/// 语义一致，同一 API 不得因后端不同而静默返回空结果。
pub fn ensure_optimistic_lock_returned<T: Model>(
    versioned: bool,
    results: &[T],
) -> crate::Result<()> {
    if versioned && results.is_empty() {
        return Err(optimistic_lock_conflict::<T>());
    }
    Ok(())
}

pub fn unsupported_postgresql_array_value(db_type: DbType) -> crate::OrmerError {
    crate::OrmerError::UnsupportedFeature {
        backend: db_type,
        feature: "PostgreSQL array values",
    }
}

pub fn unsupported_partial_index_where(db_type: DbType) -> crate::OrmerError {
    crate::OrmerError::UnsupportedFeature {
        backend: db_type,
        feature: "partial index WHERE clauses",
    }
}

pub fn format_update_assignment(
    db_type: DbType,
    assignment: &UpdateAssignment,
    params: &mut Vec<Value>,
) -> String {
    let value_sql = format_update_value(db_type, &assignment.value, params);
    quote_assignment(db_type, &assignment.column, &value_sql)
}

fn format_update_value(db_type: DbType, value: &UpdateValue, params: &mut Vec<Value>) -> String {
    match value {
        UpdateValue::Literal(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateValue::Expr(expr) => {
            format_update_expr_with_incoming(db_type, expr, params, quote_column_reference)
        }
    }
}

fn format_update_expr_with_incoming(
    db_type: DbType,
    expr: &UpdateExpr,
    params: &mut Vec<Value>,
    incoming_column: fn(DbType, &str) -> String,
) -> String {
    match expr {
        UpdateExpr::Column(column) => quote_column_reference(db_type, column),
        UpdateExpr::IncomingColumn(column) => incoming_column(db_type, column),
        UpdateExpr::Value(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateExpr::Binary { left, op, right } => format!(
            "{} {} {}",
            format_update_expr_with_incoming(db_type, left, params, incoming_column),
            op.sql(),
            format_update_expr_with_incoming(db_type, right, params, incoming_column)
        ),
        UpdateExpr::Sql(expr) => {
            let mut param_idx = params.len() as i32 + 1;
            expr.to_sql(db_type, &mut param_idx, params, None)
        }
    }
}

fn incoming_column_sql(db_type: DbType, column: &str) -> String {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => quote_column_with_prefix(db_type, "EXCLUDED", column),
        #[cfg(feature = "questdb")]
        DbType::QuestDB => quote_column_with_prefix(db_type, "excluded", column),
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => quote_column_with_prefix(db_type, "excluded", column),
        #[cfg(feature = "mysql")]
        DbType::MySQL => format!("VALUES({})", quote_identifier(db_type, column)),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => quote_column_with_prefix(db_type, "source", column),
        #[cfg(any(feature = "duckdb", feature = "clickhouse", feature = "influxdb"))]
        _ => quote_column_with_prefix(db_type, "excluded", column),
    }
}

fn format_upsert_update_value(
    db_type: DbType,
    value: &UpdateValue,
    params: &mut Vec<Value>,
) -> String {
    match value {
        UpdateValue::Literal(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateValue::Expr(expr) => {
            format_update_expr_with_incoming(db_type, expr, params, incoming_column_sql)
        }
    }
}

pub fn format_upsert_update_assignment(
    db_type: DbType,
    assignment: &UpdateAssignment,
    params: &mut Vec<Value>,
) -> String {
    let value_sql = format_upsert_update_value(db_type, &assignment.value, params);
    quote_assignment(db_type, &assignment.column, &value_sql)
}

pub fn quote_mysql_values_assignment(db_type: DbType, column: &str) -> String {
    quote_assignment(
        db_type,
        column,
        &format!("VALUES({})", quote_identifier(db_type, column)),
    )
}

pub fn append_standard_upsert_clause<T: Model>(
    db_type: DbType,
    sql: &mut String,
    columns: &[&str],
) -> crate::Result<()> {
    let primary_key_columns = T::primary_key_columns();
    if primary_key_columns.is_empty() {
        return Err(crate::ormer_error!(
            "insert_or_update requires at least one primary key column"
        ));
    }

    sql.push_str(" ON CONFLICT (");
    sql.push_str(&quote_column_list(db_type, primary_key_columns));
    sql.push_str(") DO UPDATE SET ");

    let mut first = true;
    for column in columns {
        if primary_key_columns.contains(column) {
            continue;
        }
        if !first {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            db_type,
            column,
            &quote_column_with_prefix(db_type, "excluded", column),
        ));
        first = false;
    }

    if first {
        sql.truncate(sql.len() - " DO UPDATE SET ".len());
        sql.push_str(" DO NOTHING");
    }

    Ok(())
}

/// 判断自增主键的值是否"已设置"（非类型默认值）。
fn auto_increment_key_is_set<T: Model>(model: &T, column: &str) -> bool {
    !matches!(
        model.column_value(column),
        Some(Value::Integer(0)) | Some(Value::BigInt(0)) | Some(Value::Null) | None
    )
}

/// 自增感知 upsert 语句：SQL、参数与实际写入的列清单。
pub struct UpsertSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub row_count: usize,
    /// 实际写入的列（含或不含自增主键），供参数类型推导。
    pub columns: Vec<&'static str>,
}

/// 生成自增感知的批量 upsert 语句组（按参数上限分块）。
///
/// 模型带单列自增主键时按"主键是否已设置"分行处理：
/// - 已设置：携带主键列插入并追加 `upsert_clause`，冲突更新按该主键生效
///   （graph 同步、显式 id 的 insert_or_update 场景）；
/// - 未设置：排除自增列由序列生成。若也追加冲突子句，多条新记录会因
///   显式写入的默认主键值互相冲突塌缩成一行。
///
/// 无自增主键的模型退化为单条全列 upsert（行数未超限时，与历史行为一致）。
///
/// 各分组分别按其实际写入列数（全列组含自增列）经由
/// [`build_chunked_insert_statements_for_columns`] 分块，避免单条语句
/// 绑定参数超限（MSSQL 2100 / SQLite 999）；`upsert_clause` 在每块的
/// build 闭包内追加。
pub fn build_auto_increment_aware_upsert_statements<T: Model>(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    models: &[&T],
    upsert_clause: impl Fn(&mut String, &[&str]) -> crate::Result<()>,
) -> crate::Result<Vec<UpsertSqlStatement>> {
    let Some(auto_column) = auto_increment_column::<T>().filter(|_| !models.is_empty()) else {
        let columns = T::columns();
        let statements = build_chunked_insert_statements_for_columns::<T>(
            db_type,
            columns.len(),
            models,
            |chunk| {
                let (mut sql, params) = build_batch_insert_statement::<T>(
                    db_type,
                    insert_prefix,
                    table_name,
                    &columns,
                    chunk,
                    BatchInsertValuesMode::All,
                );
                upsert_clause(&mut sql, &columns)?;
                Ok(InsertSqlStatement {
                    sql,
                    params,
                    row_count: chunk.len(),
                })
            },
        )?;
        return Ok(statements
            .into_iter()
            .map(|statement| UpsertSqlStatement {
                sql: statement.sql,
                params: statement.params,
                row_count: statement.row_count,
                columns: columns.clone(),
            })
            .collect());
    };

    let (set, unset): (Vec<&T>, Vec<&T>) = models
        .iter()
        .copied()
        .partition(|model| auto_increment_key_is_set(*model, auto_column));

    let mut statements = Vec::new();
    if !set.is_empty() {
        let columns = T::columns();
        let chunked = build_chunked_insert_statements_for_columns::<T>(
            db_type,
            columns.len(),
            &set,
            |chunk| {
                let (mut sql, params) = build_batch_insert_statement::<T>(
                    db_type,
                    insert_prefix,
                    table_name,
                    &columns,
                    chunk,
                    BatchInsertValuesMode::All,
                );
                upsert_clause(&mut sql, &columns)?;
                Ok(InsertSqlStatement {
                    sql,
                    params,
                    row_count: chunk.len(),
                })
            },
        )?;
        statements.extend(chunked.into_iter().map(|statement| UpsertSqlStatement {
            sql: statement.sql,
            params: statement.params,
            row_count: statement.row_count,
            columns: columns.clone(),
        }));
    }
    if !unset.is_empty() {
        let columns = T::insert_columns();
        let chunked = build_chunked_insert_statements_for_columns::<T>(
            db_type,
            columns.len(),
            &unset,
            |chunk| {
                let (sql, params) = build_batch_insert_statement::<T>(
                    db_type,
                    insert_prefix,
                    table_name,
                    &columns,
                    chunk,
                    BatchInsertValuesMode::WithoutAutoIncrement,
                );
                Ok(InsertSqlStatement {
                    sql,
                    params,
                    row_count: chunk.len(),
                })
            },
        )?;
        statements.extend(chunked.into_iter().map(|statement| UpsertSqlStatement {
            sql: statement.sql,
            params: statement.params,
            row_count: statement.row_count,
            columns: columns.clone(),
        }));
    }
    Ok(statements)
}

pub fn sql_type_with_nullability(base_type: &str, is_nullable: bool) -> String {
    format!("{base_type}{}", if is_nullable { "" } else { " NOT NULL" })
}

/// 通用过滤器格式化函数并收集参数（用于 UPDATE/SELECT）
pub fn format_filter_with_params(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut usize,
    params: &mut Vec<Value>,
    db_type: DbType,
) -> crate::Result<()> {
    let mut next_param_idx = *param_idx as i32;
    sql.push_str(&FilterFormatter::new(db_type).format(filter, &mut next_param_idx, params));
    *param_idx = next_param_idx as usize;
    Ok(())
}

/// 通用行数据提取函数 - 从数据库行中提取模型数据
pub fn extract_model_from_row<T: Model>(row_data: &HashMap<String, Value>) -> crate::Result<T> {
    let row = Row::new(row_data.clone());
    T::from_row(&row)
}

/// P2-6：把行→Value 解码阶段的无定位失败升级为带列名 + rust 类型的
/// [`OrmerError::Decode`]。已带定位的 Decode 错误与非解码类错误原样透传。
fn with_decode_location<T: Model>(column: &str, error: crate::OrmerError) -> crate::OrmerError {
    let rust_type = T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .map(|schema| schema.rust_type);
    match error {
        crate::OrmerError::Decode {
            column: None,
            rust_type: None,
            message,
        } => crate::OrmerError::Decode {
            column: Some(column.to_string()),
            rust_type,
            message,
        },
        other => other,
    }
}

pub fn decode_model_from_indexed_values<T, F>(offset: usize, mut value_at: F) -> crate::Result<T>
where
    T: Model,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut data = HashMap::new();
    for (i, col_name) in T::columns().iter().enumerate() {
        let value = value_at(offset + i)
            .map_err(|error| with_decode_location::<T>(col_name, error))?;
        data.insert(col_name.to_string(), value);
    }

    T::from_row(&Row::new(data))
}

pub fn decode_optional_model_from_indexed_values<T, F>(
    offset: usize,
    mut value_at: F,
) -> crate::Result<Option<T>>
where
    T: Model,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut data = HashMap::new();
    let mut is_null = true;
    for (i, col_name) in T::columns().iter().enumerate() {
        let value =
            value_at(offset + i).map_err(|error| with_decode_location::<T>(col_name, error))?;
        if !matches!(value, Value::Null) {
            is_null = false;
        }
        data.insert(col_name.to_string(), value);
    }

    if is_null {
        Ok(None)
    } else {
        Ok(Some(T::from_row(&Row::new(data))?))
    }
}

pub fn decode_row_values_from_indexed_values<V, F>(
    column_count: usize,
    mut value_at: F,
) -> crate::Result<V>
where
    V: FromRowValues,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut values = Vec::with_capacity(column_count);
    for i in 0..column_count {
        values.push(value_at(i)?);
    }

    V::from_row_values(&values)
}

/// 通用列值转换助手 - 根据 rust_type 转换数据库值到 ormer Value
#[derive(Clone, Copy)]
enum ColumnValueMode<'a> {
    Default,
    Strict { column_name: &'a str },
}

#[allow(clippy::too_many_arguments)]
fn parse_column_value_options(
    rust_type: &str,
    is_nullable: bool,
    int: Option<i64>,
    string: Option<String>,
    real: Option<f64>,
    boolean: Option<i8>,
    bytes: Option<Vec<u8>>,
    datetime: Option<chrono::DateTime<chrono::Utc>>,
    mode: ColumnValueMode<'_>,
) -> crate::Result<Value> {
    fn decimal_string(
        string: Option<String>,
        int: Option<i64>,
        real: Option<f64>,
    ) -> Option<String> {
        string
            .or_else(|| int.map(|value| value.to_string()))
            .or_else(|| real.map(|value| value.to_string()))
    }

    fn uuid_value(
        string: Option<String>,
        bytes: Option<Vec<u8>>,
    ) -> crate::Result<Option<uuid::Uuid>> {
        let raw = match (string, bytes) {
            (Some(value), _) => Some(value),
            (None, Some(value)) => Some(
                String::from_utf8(value)
                    .map_err(|err| crate::ormer_error!("Failed to decode UUID text: {}", err))?,
            ),
            (None, None) => None,
        };
        match raw {
            Some(raw) => crate::model::uuid_from_text(&raw).map(Some),
            None => Ok(None),
        }
    }

    if is_nullable {
        match rust_type {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => match int {
                Some(val) => Ok(Value::Integer(val)),
                None => Ok(Value::Null),
            },
            "String" => match string {
                Some(val) => Ok(Value::Text(val)),
                None => Ok(Value::Null),
            },
            "f32" | "f64" => match real {
                Some(val) => Ok(Value::Real(val)),
                None => Ok(Value::Null),
            },
            "Decimal" | "rust_decimal::Decimal" => match decimal_string(string, int, real) {
                Some(val) => Ok(Value::Decimal(val)),
                None => Ok(Value::Null),
            },
            "BigDecimal" | "bigdecimal::BigDecimal" => match decimal_string(string, int, real) {
                Some(val) => Ok(Value::BigDecimal(val)),
                None => Ok(Value::Null),
            },
            "bool" => match boolean {
                Some(1) => Ok(Value::Boolean(true)),
                Some(0) => Ok(Value::Boolean(false)),
                _ => Ok(Value::Null),
            },
            "Uuid" | "uuid::Uuid" => match uuid_value(string, bytes)? {
                Some(value) => Ok(Value::Uuid(value)),
                None => Ok(Value::Null),
            },
            "Vec<u8>" | "&[u8]" => match bytes {
                Some(val) => Ok(Value::Bytes(val)),
                None => Ok(Value::Null),
            },
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => match datetime {
                Some(val) => Ok(Value::DateTime(val)),
                None => Ok(Value::Null),
            },
            "NaiveDate" | "chrono::NaiveDate" => match string {
                Some(val) => Ok(Value::Date(chrono::NaiveDate::parse_from_str(
                    &val, "%Y-%m-%d",
                )?)),
                None => Ok(Value::Null),
            },
            "NaiveTime" | "chrono::NaiveTime" => match string {
                Some(val) => Ok(Value::Time(chrono::NaiveTime::parse_from_str(
                    &val,
                    "%H:%M:%S%.f",
                )?)),
                None => Ok(Value::Null),
            },
            _ => Err(crate::ormer_error!(
                "Unsupported nullable column type: {rust_type}"
            )),
        }
    } else {
        match (rust_type, mode) {
            (
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64",
                ColumnValueMode::Default,
            ) => Ok(Value::Integer(int.unwrap_or(0))),
            (
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64",
                ColumnValueMode::Strict { column_name },
            ) => int.map(Value::Integer).ok_or_else(|| {
                crate::ormer_error!(
                    "Failed to parse non-nullable column '{}' (expected integer type)",
                    column_name
                )
            }),
            ("String", ColumnValueMode::Default) => Ok(Value::Text(string.unwrap_or_default())),
            ("String", ColumnValueMode::Strict { column_name }) => {
                string.map(Value::Text).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected String type)",
                        column_name
                    )
                })
            }
            ("f32" | "f64", ColumnValueMode::Default) => Ok(Value::Real(real.unwrap_or(0.0))),
            ("f32" | "f64", ColumnValueMode::Strict { column_name }) => {
                real.map(Value::Real).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected float type)",
                        column_name
                    )
                })
            }
            ("Decimal" | "rust_decimal::Decimal", ColumnValueMode::Default) => Ok(Value::Decimal(
                decimal_string(string, int, real).unwrap_or_else(|| "0".to_string()),
            )),
            ("Decimal" | "rust_decimal::Decimal", ColumnValueMode::Strict { column_name }) => {
                decimal_string(string, int, real)
                    .map(Value::Decimal)
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Failed to parse non-nullable column '{}' (expected Decimal type)",
                            column_name
                        )
                    })
            }
            ("BigDecimal" | "bigdecimal::BigDecimal", ColumnValueMode::Default) => {
                Ok(Value::BigDecimal(
                    decimal_string(string, int, real).unwrap_or_else(|| "0".to_string()),
                ))
            }
            ("BigDecimal" | "bigdecimal::BigDecimal", ColumnValueMode::Strict { column_name }) => {
                decimal_string(string, int, real)
                    .map(Value::BigDecimal)
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Failed to parse non-nullable column '{}' (expected BigDecimal type)",
                            column_name
                        )
                    })
            }
            ("bool", ColumnValueMode::Default) => Ok(Value::Boolean(boolean.unwrap_or(0) == 1)),
            ("bool", ColumnValueMode::Strict { column_name }) => boolean
                .map(|value| Value::Boolean(value == 1))
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected bool type)",
                        column_name
                    )
                }),
            ("Uuid" | "uuid::Uuid", ColumnValueMode::Default) => Ok(Value::Uuid(
                uuid_value(string, bytes)?.unwrap_or_else(uuid::Uuid::nil),
            )),
            ("Uuid" | "uuid::Uuid", ColumnValueMode::Strict { column_name }) => {
                uuid_value(string, bytes)?
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Failed to parse non-nullable column '{}' (expected uuid::Uuid type)",
                            column_name
                        )
                    })
                    .map(Value::Uuid)
            }
            ("Vec<u8>" | "&[u8]", ColumnValueMode::Default) => {
                Ok(Value::Bytes(bytes.unwrap_or_default()))
            }
            ("Vec<u8>" | "&[u8]", ColumnValueMode::Strict { column_name }) => {
                bytes.map(Value::Bytes).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected Vec<u8> type)",
                        column_name
                    )
                })
            }
            ("Duration" | "std::time::Duration", ColumnValueMode::Default) => Ok(Value::Duration(
                std::time::Duration::from_micros(int.unwrap_or(0).max(0) as u64),
            )),
            ("Duration" | "std::time::Duration", ColumnValueMode::Strict { .. }) => {
                Err(crate::ormer_error!("Unsupported column type: {rust_type}"))
            }
            (
                "DateTime"
                | "chrono::DateTime"
                | "chrono::DateTime<chrono::Utc>"
                | "NaiveDateTime"
                | "chrono::NaiveDateTime",
                ColumnValueMode::Default,
            ) => Ok(Value::DateTime(
                datetime.unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
            )),
            (
                "DateTime"
                | "chrono::DateTime"
                | "chrono::DateTime<chrono::Utc>"
                | "NaiveDateTime"
                | "chrono::NaiveDateTime",
                ColumnValueMode::Strict { column_name },
            ) => datetime.map(Value::DateTime).ok_or_else(|| {
                crate::ormer_error!(
                    "Failed to parse non-nullable column '{}' (expected DateTime type)",
                    column_name
                )
            }),
            ("NaiveDate" | "chrono::NaiveDate", ColumnValueMode::Default) => Ok(Value::Date(
                string
                    .and_then(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok())
                    .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
            )),
            ("NaiveDate" | "chrono::NaiveDate", ColumnValueMode::Strict { column_name }) => {
                let value = string.ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected date type)",
                        column_name
                    )
                })?;
                Ok(Value::Date(chrono::NaiveDate::parse_from_str(
                    &value, "%Y-%m-%d",
                )?))
            }
            ("NaiveTime" | "chrono::NaiveTime", ColumnValueMode::Default) => Ok(Value::Time(
                string
                    .and_then(|value| chrono::NaiveTime::parse_from_str(&value, "%H:%M:%S%.f").ok())
                    .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
            )),
            ("NaiveTime" | "chrono::NaiveTime", ColumnValueMode::Strict { column_name }) => {
                let value = string.ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected time type)",
                        column_name
                    )
                })?;
                Ok(Value::Time(chrono::NaiveTime::parse_from_str(
                    &value,
                    "%H:%M:%S%.f",
                )?))
            }
            _ => Err(crate::ormer_error!("Unsupported column type: {rust_type}")),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn convert_column_value(
    rust_type: &str,
    is_nullable: bool,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> crate::Result<Value> {
    parse_column_value_options(
        rust_type,
        is_nullable,
        get_int(),
        get_string(),
        get_real(),
        get_bool(),
        get_bytes(),
        get_datetime(),
        ColumnValueMode::Default,
    )
}

fn downcast_auto_increment_key<K: 'static, T: 'static>(value: T) -> K {
    let boxed: Box<dyn std::any::Any> = Box::new(value);
    match boxed.downcast::<K>() {
        Ok(value) => *value,
        Err(_) => unreachable!("auto-increment key type checked before downcast"),
    }
}

/// 将数据库返回的自增 ID 转换为模型指定的 AutoIncrementKeyType。
pub fn convert_auto_increment_key<K: Default + 'static>(
    last_id: impl Into<i128>,
) -> crate::Result<K> {
    let last_id = last_id.into();
    let key_type = std::any::TypeId::of::<K>();
    if key_type == std::any::TypeId::of::<()>() {
        Ok(downcast_auto_increment_key(()))
    } else if key_type == std::any::TypeId::of::<i32>() {
        Ok(downcast_auto_increment_key(last_id as i32))
    } else if key_type == std::any::TypeId::of::<i64>() {
        Ok(downcast_auto_increment_key(last_id as i64))
    } else if key_type == std::any::TypeId::of::<u32>() {
        Ok(downcast_auto_increment_key(last_id as u32))
    } else if key_type == std::any::TypeId::of::<u64>() {
        Ok(downcast_auto_increment_key(last_id as u64))
    } else if key_type == std::any::TypeId::of::<usize>() {
        Ok(downcast_auto_increment_key(last_id as usize))
    } else if key_type == std::any::TypeId::of::<Option<i64>>() {
        Ok(downcast_auto_increment_key(Some(last_id as i64)))
    } else {
        Err(crate::ormer_error!(
            "Unsupported auto-increment key type. Only i32, i64, u32, u64, usize, Option<i64> and () are supported."
        ))
    }
}

fn build_batch_insert_sql_with_prefix(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let table_name = quote_qualified_identifier(db_type, table_name);
    let columns_str = quote_column_list(db_type, columns);
    let col_count = columns.len();

    let mut sql = format!("{insert_prefix} {table_name} ({columns_str}) VALUES ");

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let start_idx = idx * col_count + 1;
        sql.push_str(&format!(
            "({})",
            placeholder_list(db_type, start_idx, col_count)
        ));
    }

    (sql, col_count)
}

/// 构建批量插入 SQL 的公共函数（使用自定义列名列表，排除自增主键）
pub fn build_batch_insert_sql_with_columns(
    db_type: DbType,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    build_batch_insert_sql_with_prefix(db_type, "INSERT INTO", table_name, columns, models_count)
}

#[derive(Debug, Clone, Copy)]
pub enum BatchInsertValuesMode {
    All,
    WithoutAutoIncrement,
}

/// 构建批量 INSERT 主体并收集对应参数，冲突子句由各后端追加。
pub fn build_batch_insert_statement<T: Model>(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models: &[&T],
    values_mode: BatchInsertValuesMode,
) -> (String, Vec<Value>) {
    let (sql, _) = build_batch_insert_sql_with_prefix(
        db_type,
        insert_prefix,
        table_name,
        columns,
        models.len(),
    );
    let values = match values_mode {
        BatchInsertValuesMode::All => collect_batch_insert_values(models),
        BatchInsertValuesMode::WithoutAutoIncrement => {
            collect_batch_insert_values_with_auto_increment(models)
        }
    };
    (sql, values)
}

pub fn auto_increment_column<T: Model>() -> Option<&'static str> {
    T::column_schema()
        .iter()
        .find(|column| column.is_auto_increment)
        .map(|column| column.name)
}

pub fn append_auto_increment_returning<T: Model>(db_type: DbType, sql: String) -> String {
    let Some(_pk_col) = auto_increment_column::<T>() else {
        return sql;
    };

    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => format!("{sql} RETURNING rowid"),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let pk_col = quote_column_list(DbType::PostgreSQL, &[_pk_col]);
            format!("{sql} RETURNING {pk_col}")
        }
        #[cfg(feature = "questdb")]
        DbType::QuestDB => sql,
        #[cfg(feature = "influxdb")]
        DbType::InfluxDB => sql,
        #[cfg(feature = "mysql")]
        DbType::MySQL => sql,
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            let pk_col = quote_column_list(DbType::DuckDB, &[_pk_col]);
            format!("{sql} RETURNING {pk_col}")
        }
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            let output = format!(
                " OUTPUT {}",
                quote_column_with_prefix(DbType::MSSQL, "inserted", _pk_col)
            );
            if let Some(pos) = sql.rfind(" DEFAULT VALUES") {
                let mut sql = sql;
                sql.insert_str(pos, &output);
                sql
            } else if let Some(pos) = sql.rfind(" VALUES ") {
                let mut sql = sql;
                sql.insert_str(pos, &output);
                sql
            } else {
                format!("{sql}{output}")
            }
        }
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => sql,
    }
}

#[cfg(feature = "mssql")]
fn mssql_output_projection<T: Model>(source: &str) -> String {
    T::COLUMNS
        .iter()
        .map(|column| quote_column_with_prefix(DbType::MSSQL, source, column))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(feature = "mssql")]
fn first_marker_pos(sql: &str, markers: &[&str]) -> Option<usize> {
    let lower = sql.to_ascii_lowercase();
    markers
        .iter()
        .filter_map(|marker| lower.find(&marker.to_ascii_lowercase()))
        .min()
}

#[cfg(feature = "mssql")]
fn replace_mssql_output_clause(
    sql: &str,
    projection: &str,
    tail_markers: &[&str],
) -> Option<String> {
    let lower = sql.to_ascii_lowercase();
    let output_pos = lower.find(" output ")?;
    let tail_pos = tail_markers
        .iter()
        .filter_map(|marker| {
            lower[output_pos + " output ".len()..].find(&marker.to_ascii_lowercase())
        })
        .map(|pos| output_pos + " output ".len() + pos)
        .min()?;
    Some(format!(
        "{} OUTPUT {}{}",
        &sql[..output_pos],
        projection,
        &sql[tail_pos..]
    ))
}

#[cfg(feature = "mssql")]
fn insert_mssql_output_clause(sql: &str, projection: &str, tail_markers: &[&str]) -> String {
    if let Some(rewritten) = replace_mssql_output_clause(sql, projection, tail_markers) {
        return rewritten;
    }

    let Some(tail_pos) = first_marker_pos(sql, tail_markers) else {
        return format!("{sql} OUTPUT {projection}");
    };

    format!(
        "{} OUTPUT {}{}",
        &sql[..tail_pos],
        projection,
        &sql[tail_pos..]
    )
}

#[cfg(feature = "mssql")]
pub fn mssql_insert_returning_sql<T: Model>(sql: &str) -> String {
    if sql.trim_start().to_ascii_lowercase().starts_with("merge ") {
        return insert_mssql_terminal_output_clause(sql, &mssql_output_projection::<T>("inserted"));
    }

    insert_mssql_output_clause(
        sql,
        &mssql_output_projection::<T>("inserted"),
        &[" default values", " values "],
    )
}

#[cfg(feature = "mssql")]
pub fn mssql_update_returning_sql<T: Model>(sql: &str) -> String {
    insert_mssql_output_clause(
        sql,
        &mssql_output_projection::<T>("inserted"),
        &[" from ", " where "],
    )
}

#[cfg(feature = "mssql")]
pub fn mssql_delete_returning_sql<T: Model>(sql: &str) -> String {
    insert_mssql_output_clause(sql, &mssql_output_projection::<T>("deleted"), &[" where "])
}

#[cfg(feature = "mssql")]
fn insert_mssql_terminal_output_clause(sql: &str, projection: &str) -> String {
    if let Some(rewritten) = replace_mssql_output_clause(sql, projection, &[";"]) {
        return rewritten;
    }

    let trimmed_len = sql.trim_end().len();
    if sql[..trimmed_len].ends_with(';') {
        let semicolon_pos = sql[..trimmed_len].len() - 1;
        return format!(
            "{} OUTPUT {}{}",
            &sql[..semicolon_pos],
            projection,
            &sql[semicolon_pos..]
        );
    }

    format!("{sql} OUTPUT {projection}")
}

pub fn build_insert_statement<T: Model>(db_type: DbType, models: &[&T]) -> (String, Vec<Value>) {
    let columns = T::insert_columns();
    build_batch_insert_statement::<T>(
        db_type,
        "INSERT INTO",
        T::table_name_for_db(db_type),
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    )
}

fn routed_table_name_for_models<T: Model>(db_type: DbType, models: &[&T]) -> crate::Result<String> {
    let Some(first) = models.first() else {
        return Ok(T::table_name_for_db(db_type).to_string());
    };
    let first_route = first.table_route()?;
    let table_name = routed_model_table_name_for_db::<T>(db_type, &first_route)?;

    for model in models.iter().skip(1) {
        let route = model.table_route()?;
        let model_table = routed_model_table_name_for_db::<T>(db_type, &route)?;
        if model_table != table_name {
            return Err(crate::ormer_error!(
                "Batch insert cannot target multiple routed tables: {} and {}",
                table_name,
                model_table
            ));
        }
    }

    Ok(table_name)
}

pub fn routed_insert_table_name<T: Model>(db_type: DbType, models: &[&T]) -> crate::Result<String> {
    routed_table_name_for_models(db_type, models)
}

pub fn build_routed_insert_statement<T: Model>(
    db_type: DbType,
    models: &[&T],
) -> crate::Result<(String, Vec<Value>)> {
    let columns = T::insert_columns();
    let table_name = routed_table_name_for_models(db_type, models)?;
    Ok(build_batch_insert_statement::<T>(
        db_type,
        "INSERT INTO",
        &table_name,
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    ))
}

#[derive(Debug, Clone)]
pub struct PartialInsertStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub param_rust_types: Vec<&'static str>,
}

pub fn build_partial_insert_statement<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
) -> crate::Result<PartialInsertStatement> {
    build_partial_insert_statement_for_table::<T>(
        db_type,
        assignments,
        T::table_name_for_db(db_type),
    )
}

pub fn build_partial_insert_statement_for_table<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
    table_name: &str,
) -> crate::Result<PartialInsertStatement> {
    let mut values_by_column = HashMap::<&'static str, InsertValue>::new();
    for assignment in assignments {
        let column_name = T::column_name_for_field(&assignment.column).ok_or_else(|| {
            crate::ormer_error!(
                "Column {} not found on model {}",
                assignment.column,
                T::TABLE_NAME
            )
        })?;
        values_by_column.insert(column_name, assignment.value.clone());
    }

    let mut columns = Vec::new();
    let mut params = Vec::new();
    let mut param_rust_types = Vec::new();
    for schema in T::column_schema() {
        match values_by_column.get(schema.name) {
            Some(InsertValue::Literal(value)) => {
                columns.push(schema.name);
                params.push(value.clone());
                param_rust_types.push(schema.data_type.unwrap_or(schema.rust_type));
            }
            Some(InsertValue::Default) | None => {}
        }
    }

    let table_name = quote_qualified_identifier(db_type, table_name);
    let sql = if columns.is_empty() {
        match db_type {
            #[cfg(feature = "mysql")]
            DbType::MySQL => format!("INSERT INTO {table_name} () VALUES ()"),
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: db_type,
                    feature: "partial insert without columns",
                });
            }
            #[cfg(any(feature = "sqlite", feature = "postgresql", feature = "mssql"))]
            _ => format!("INSERT INTO {table_name} DEFAULT VALUES"),
        }
    } else {
        let columns_str = quote_column_list(db_type, &columns);
        let placeholders = placeholder_list(db_type, 1, columns.len());
        format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders})")
    };

    Ok(PartialInsertStatement {
        sql,
        params,
        param_rust_types,
    })
}

pub fn build_partial_insert_statement_with_auto_increment_returning<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
) -> crate::Result<PartialInsertStatement> {
    let mut statement = build_partial_insert_statement::<T>(db_type, assignments)?;
    statement.sql = append_auto_increment_returning::<T>(db_type, statement.sql);
    Ok(statement)
}

pub fn build_partial_insert_statement_with_auto_increment_returning_for_table<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
    table_name: &str,
) -> crate::Result<PartialInsertStatement> {
    let mut statement =
        build_partial_insert_statement_for_table::<T>(db_type, assignments, table_name)?;
    statement.sql = append_auto_increment_returning::<T>(db_type, statement.sql);
    Ok(statement)
}

pub fn validate_insert_model_table<T: Model>(
    db_type: DbType,
    source_table: Option<&'static str>,
) -> crate::Result<()> {
    let Some(source_table) = source_table else {
        return Ok(());
    };
    let target_table = T::TABLE_NAME;
    let db_table = T::table_name_for_db(db_type);
    if source_table == target_table || source_table == db_table {
        return Ok(());
    }

    Err(crate::ormer_error!(
        "Insert model targets table {}, but model {} uses table {}",
        source_table,
        std::any::type_name::<T>(),
        target_table
    ))
}

fn insert_prefix_for_conflict(db_type: DbType, _conflict: Option<&InsertConflict>) -> &'static str {
    match db_type {
        #[cfg(feature = "mysql")]
        DbType::MySQL
            if _conflict.and_then(|conflict| conflict.action)
                == Some(InsertConflictAction::DoNothing) =>
        {
            "INSERT IGNORE INTO"
        }
        _ => "INSERT INTO",
    }
}

pub fn build_insert_statement_with_conflict<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<(String, Vec<Value>)> {
    let columns = T::insert_columns();
    let table_name = routed_table_name_for_models(db_type, models)?;
    let (sql, values) = build_batch_insert_statement::<T>(
        db_type,
        insert_prefix_for_conflict(db_type, conflict),
        &table_name,
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    );
    #[cfg(any(
        feature = "postgresql",
        feature = "sqlite",
        feature = "mysql",
        feature = "duckdb"
    ))]
    let (mut sql, mut values) = (sql, values);

    if let Some(conflict) = conflict
        && conflict.is_configured()
    {
        #[cfg(feature = "mssql")]
        if matches!(db_type, DbType::MSSQL) {
            let statement = build_mssql_insert_conflict_statement::<T>(models, conflict)?;
            return Ok((statement.sql, statement.params));
        }
        #[cfg(any(
            feature = "postgresql",
            feature = "sqlite",
            feature = "mysql",
            feature = "duckdb"
        ))]
        append_insert_conflict_clause::<T>(db_type, &mut sql, &mut values, conflict)?;
    }

    Ok((sql, values))
}

/// 单条 INSERT 允许的最大行数：按后端绑定参数上限（`bind_param_limit`）与
/// 实际写入列数推导，MSSQL 2100 / SQLite 999 参数上限由此生效。
///
/// 写入列集与 `T::insert_columns()` 不同时（如 upsert 携带自增列的全列
/// 路径、含全部列的 `INSERT IGNORE`）必须经由本入口传入真实列数，
/// 避免按 `insert_columns` 分块导致单条语句参数超限。
pub fn insert_rows_per_statement_for(db_type: DbType, column_count: usize) -> usize {
    (bind_param_limit(db_type) / column_count.max(1)).max(1)
}

/// 单条 INSERT 允许的最大行数（按 `T::insert_columns()` 推导）。
pub fn insert_rows_per_statement<T: Model>(db_type: DbType) -> usize {
    insert_rows_per_statement_for(db_type, T::insert_columns().len())
}

/// 按显式每语句行数把模型行分块，逐块调用 `build` 生成语句并聚合。
fn chunk_model_statements<T, S>(
    models: &[&T],
    rows_per_statement: usize,
    mut build: impl FnMut(&[&T]) -> crate::Result<S>,
) -> crate::Result<Vec<S>> {
    if models.len() <= rows_per_statement {
        let statement = build(models)?;
        return Ok(vec![statement]);
    }
    models.chunks(rows_per_statement).map(build).collect()
}

/// 按参数上限把模型行分块，逐块调用 `build` 生成语句并聚合。
///
/// 自增主键与带 conflict 的插入同样受后端参数上限约束，必须经由本 helper
/// 分块，避免单条语句绑定参数超限（MSSQL 2100 / SQLite 999）导致运行时失败；
/// 行数未超限时仍生成单条语句，语句形态与历史行为一致。
pub fn build_chunked_insert_statements<T: Model>(
    db_type: DbType,
    models: &[&T],
    build: impl FnMut(&[&T]) -> crate::Result<InsertSqlStatement>,
) -> crate::Result<Vec<InsertSqlStatement>> {
    chunk_model_statements(models, insert_rows_per_statement::<T>(db_type), build)
}

/// 按参数上限把模型行分块（显式列数版本）：写入列集与 `T::insert_columns()`
/// 不同时由调用方给出真实列数（见 [`insert_rows_per_statement_for`]）。
pub fn build_chunked_insert_statements_for_columns<T: Model>(
    db_type: DbType,
    column_count: usize,
    models: &[&T],
    build: impl FnMut(&[&T]) -> crate::Result<InsertSqlStatement>,
) -> crate::Result<Vec<InsertSqlStatement>> {
    chunk_model_statements(
        models,
        insert_rows_per_statement_for(db_type, column_count),
        build,
    )
}

/// 构建批量插入语句组（含自增主键 / 带 conflict 路径），统一按参数上限分块。
pub fn build_insert_statements_with_conflict<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<Vec<InsertSqlStatement>> {
    if models.is_empty() {
        return Ok(Vec::new());
    }

    routed_table_name_for_models(db_type, models)?;
    build_chunked_insert_statements::<T>(db_type, models, |chunk| {
        let (sql, params) = build_insert_statement_with_conflict::<T>(db_type, chunk, conflict)?;
        Ok(InsertSqlStatement {
            sql,
            params,
            row_count: chunk.len(),
        })
    })
}

#[cfg(any(
    feature = "postgresql",
    feature = "sqlite",
    feature = "mysql",
    feature = "duckdb"
))]
fn append_insert_conflict_clause<T: Model>(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    // 能力矩阵优先：insert_conflict=false 的后端（QuestDB/InfluxDB/ClickHouse）
    // 统一拒绝；MSSQL 虽为 true（insert_or_update 走 MERGE），可配置 conflict
    // 子句的细粒度限制仍在矩阵之后单独拒绝。
    crate::Capabilities::ensure(
        db_type,
        |caps| caps.insert_conflict,
        "insert conflict handling",
    )?;
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            append_standard_insert_conflict_clause::<T>(DbType::PostgreSQL, sql, params, conflict)
        }
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => {
            append_standard_insert_conflict_clause::<T>(DbType::Sqlite, sql, params, conflict)
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => append_mysql_insert_conflict_clause(sql, params, conflict),
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            append_standard_insert_conflict_clause::<T>(DbType::DuckDB, sql, params, conflict)
        }
        #[cfg(feature = "mssql")]
        DbType::MSSQL => Err(crate::ormer_error!(
            "MSSQL does not support configurable insert conflict handling; use insert_or_update for primary-key MERGE"
        )),
        // 矩阵兜底：正常不可达（insert_conflict=false 已在上面拦截）。
        #[allow(unreachable_patterns)]
        _ => Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "insert conflict handling",
        }),
    }
}

#[cfg(any(feature = "postgresql", feature = "sqlite", feature = "duckdb"))]
fn append_standard_insert_conflict_clause<T: Model>(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    let action = conflict.action.ok_or_else(|| {
        crate::ormer_error!("insert conflict handling requires do_nothing, do_update, or set")
    })?;

    sql.push_str(" ON CONFLICT");
    append_standard_conflict_target(db_type, sql, params, conflict)?;

    match action {
        InsertConflictAction::DoNothing => {
            if conflict.update_filter.is_some() || !conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_nothing cannot be combined with update filter or set assignments"
                ));
            }
            sql.push_str(" DO NOTHING");
        }
        InsertConflictAction::DoUpdate => {
            if conflict.target.is_none() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires on_conflict or on_constraint"
                ));
            }
            if conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires at least one set assignment"
                ));
            }

            sql.push_str(" DO UPDATE SET ");
            for (index, assignment) in conflict.assignments.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format_upsert_update_assignment(
                    db_type, assignment, params,
                ));
            }
            if let Some(filter) = &conflict.update_filter {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
            }
        }
    }

    Ok(())
}

#[cfg(any(feature = "postgresql", feature = "sqlite", feature = "duckdb"))]
fn append_standard_conflict_target(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    match &conflict.target {
        Some(InsertConflictTarget::Columns(columns)) => {
            if columns.is_empty() {
                return Err(crate::ormer_error!(
                    "on_conflict requires at least one conflict target column"
                ));
            }
            sql.push_str(" (");
            sql.push_str(&quote_column_list(db_type, columns));
            sql.push(')');
            if let Some(filter) = &conflict.target_filter {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
            }
        }
        Some(InsertConflictTarget::Constraint(_name)) => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                if conflict.target_filter.is_some() {
                    return Err(crate::ormer_error!(
                        "conflict_where cannot be combined with on_constraint"
                    ));
                }
                sql.push_str(" ON CONSTRAINT ");
                sql.push_str(&quote_identifier(db_type, _name));
            }
            #[cfg(feature = "questdb")]
            DbType::QuestDB => {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: db_type,
                    feature: "insert conflict constraint targets",
                });
            }
            #[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql"))]
            _ => {
                return Err(crate::ormer_error!(
                    "on_constraint is only supported for PostgreSQL insert conflict handling"
                ));
            }
        },
        None => {
            if conflict.target_filter.is_some() {
                return Err(crate::ormer_error!(
                    "conflict_where requires on_conflict column targets"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "mysql")]
fn append_mysql_insert_conflict_clause(
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    if conflict.target.is_some() {
        return Err(crate::OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "MySQL ON DUPLICATE KEY conflict target",
        });
    }
    if conflict.target_filter.is_some() {
        return Err(crate::OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "MySQL partial conflict targets",
        });
    }

    let action = conflict.action.ok_or_else(|| {
        crate::ormer_error!("insert conflict handling requires do_nothing, do_update, or set")
    })?;

    match action {
        InsertConflictAction::DoNothing => {
            if conflict.update_filter.is_some() || !conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_nothing cannot be combined with update filter or set assignments"
                ));
            }
        }
        InsertConflictAction::DoUpdate => {
            if conflict.update_filter.is_some() {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: DbType::MySQL,
                    feature: "MySQL conditional conflict updates",
                });
            }
            if conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires at least one set assignment"
                ));
            }
            sql.push_str(" ON DUPLICATE KEY UPDATE ");
            for (index, assignment) in conflict.assignments.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format_upsert_update_assignment(
                    DbType::MySQL,
                    assignment,
                    params,
                ));
            }
        }
    }

    Ok(())
}

pub fn build_insert_statement_with_auto_increment_returning<T: Model>(
    db_type: DbType,
    models: &[&T],
) -> crate::Result<(String, Vec<Value>)> {
    let (sql, values) = build_routed_insert_statement::<T>(db_type, models)?;
    Ok((append_auto_increment_returning::<T>(db_type, sql), values))
}

pub fn build_insert_statement_with_conflict_and_auto_increment_returning<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<(String, Vec<Value>)> {
    let (sql, values) = build_insert_statement_with_conflict::<T>(db_type, models, conflict)?;
    Ok((append_auto_increment_returning::<T>(db_type, sql), values))
}

#[cfg(feature = "mssql")]
fn mssql_insert_conflict_unsupported(feature: &'static str) -> crate::OrmerError {
    crate::OrmerError::UnsupportedFeature {
        backend: DbType::MSSQL,
        feature,
    }
}

#[cfg(feature = "mssql")]
pub fn build_mssql_insert_conflict_statement<T: Model>(
    models: &[&T],
    conflict: &InsertConflict,
) -> crate::Result<InsertSqlStatement> {
    let action = conflict.action.ok_or_else(|| {
        crate::ormer_error!("insert conflict handling requires do_nothing, do_update, or set")
    })?;

    if conflict.target_filter.is_some() {
        return Err(mssql_insert_conflict_unsupported(
            "partial insert conflict targets",
        ));
    }
    if conflict.update_filter.is_some() {
        return Err(mssql_insert_conflict_unsupported(
            "conditional insert conflict updates",
        ));
    }

    let target_columns = match &conflict.target {
        Some(InsertConflictTarget::Columns(columns)) if !columns.is_empty() => columns,
        Some(InsertConflictTarget::Columns(_)) => {
            return Err(crate::ormer_error!(
                "on_conflict requires at least one conflict target column"
            ));
        }
        Some(InsertConflictTarget::Constraint(_)) => {
            return Err(mssql_insert_conflict_unsupported(
                "named insert conflict constraints",
            ));
        }
        None => {
            return Err(mssql_insert_conflict_unsupported(
                "insert conflict without column targets",
            ));
        }
    };

    let insert_columns = T::insert_columns();
    if insert_columns.is_empty() {
        return Err(mssql_insert_conflict_unsupported(
            "insert conflict for default-only inserts",
        ));
    }

    let insert_column_set = insert_columns.iter().copied().collect::<HashSet<_>>();
    let auto_increment_columns = T::column_schema()
        .iter()
        .filter(|column| column.is_auto_increment)
        .map(|column| column.name)
        .collect::<HashSet<_>>();
    for column in target_columns {
        if !insert_column_set.contains(column) && auto_increment_columns.contains(column) {
            return Err(mssql_insert_conflict_unsupported(
                "insert conflict on auto-increment columns",
            ));
        }
        if !insert_column_set.contains(column) {
            return Err(crate::ormer_error!(
                "Column {} not found on model {}",
                column,
                T::TABLE_NAME
            ));
        }
    }

    if matches!(action, InsertConflictAction::DoNothing) && !conflict.assignments.is_empty() {
        return Err(crate::ormer_error!(
            "do_nothing cannot be combined with set assignments"
        ));
    }
    if matches!(action, InsertConflictAction::DoUpdate) && conflict.assignments.is_empty() {
        return Err(crate::ormer_error!(
            "do_update conflict handling requires at least one set assignment"
        ));
    }
    if conflict.assignments.iter().any(|assignment| {
        auto_increment_columns.contains(
            T::column_name_for_field(&assignment.column).unwrap_or(assignment.column.as_str()),
        )
    }) {
        return Err(mssql_insert_conflict_unsupported(
            "updating auto-increment columns",
        ));
    }

    let table_name = routed_table_name_for_models(DbType::MSSQL, models)?;
    let columns = quote_column_list(DbType::MSSQL, &insert_columns);
    let mut sql = format!(
        "MERGE INTO {} AS target USING (VALUES ",
        quote_qualified_identifier(DbType::MSSQL, &table_name)
    );
    let mut params = Vec::new();

    for (idx, model) in models.iter().enumerate() {
        if idx > 0 {
            sql.push_str(", ");
        }
        let placeholders = placeholder_list(DbType::MSSQL, params.len() + 1, insert_columns.len());
        sql.push_str(&format!("({placeholders})"));
        params.extend(model.insert_values());
    }

    sql.push_str(&format!(") AS source ({columns}) ON "));
    for (idx, column) in target_columns.iter().enumerate() {
        if idx > 0 {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_column_with_prefix(DbType::MSSQL, "target", column),
            quote_column_with_prefix(DbType::MSSQL, "source", column)
        ));
    }

    match action {
        InsertConflictAction::DoNothing => {}
        InsertConflictAction::DoUpdate => {
            sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
            for (index, assignment) in conflict.assignments.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format_upsert_update_assignment(
                    DbType::MSSQL,
                    assignment,
                    &mut params,
                ));
            }
        }
    }

    append_mssql_merge_insert_clause_for_columns(&mut sql, &insert_columns);
    append_mssql_merge_auto_increment_output::<T>(&mut sql);

    Ok(InsertSqlStatement {
        sql,
        params,
        row_count: models.len(),
    })
}

#[cfg(feature = "mssql")]
pub fn build_mssql_merge_source<T: Model>(models: &[&T]) -> (String, Vec<Value>) {
    // 与 conflict 路径（build_mssql_insert_conflict_statement）及 MySQL/PG/SQLite 的
    // upsert 语义保持一致：MERGE 源列使用 insert_columns() 排除自增列，避免向
    // IDENTITY 列显式插入未赋值的 0/NULL（SQL Server 错误 544）。
    let columns = T::insert_columns();
    let columns_sql = quote_column_list(DbType::MSSQL, &columns);
    let col_count = columns.len();
    let mut sql = format!(
        "MERGE INTO {} AS target USING (VALUES ",
        quote_table_name::<T>(DbType::MSSQL)
    );
    let mut all_values = Vec::new();

    for (idx, model) in models.iter().enumerate() {
        if idx > 0 {
            sql.push_str(", ");
        }
        let placeholders = placeholder_list(DbType::MSSQL, all_values.len() + 1, col_count);
        sql.push_str(&format!("({placeholders})"));
        all_values.extend(model.insert_values());
    }

    sql.push_str(&format!(") AS source ({columns_sql}) ON "));
    // 仅对存在于源列中的主键生成等值条件；自增主键已从源中排除
    //（未赋值时原本也不可能匹配到已有行），无可用主键时退化为
    // 永不匹配的 1 = 0，使 MERGE 仅执行插入分支。
    let mut has_match_condition = false;
    for pk in T::primary_key_columns() {
        if !columns.contains(pk) {
            continue;
        }
        if has_match_condition {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_column_with_prefix(DbType::MSSQL, "target", pk),
            quote_column_with_prefix(DbType::MSSQL, "source", pk)
        ));
        has_match_condition = true;
    }
    if !has_match_condition {
        sql.push_str("1 = 0");
    }

    (sql, all_values)
}

/// 生成批量 MERGE 语句组（insert_or_update / insert_or_ignore，按参数上限
/// 2100 分块）：MERGE 源列使用 insert_columns()，分块尺寸与其一致，
/// `with_update` 控制 WHEN MATCHED UPDATE 分支（insert_or_update）。
///
/// mssql 后端与连接池的 insert_or_update / insert_or_ignore 路径统一经此
/// 入口，to_sql 与 execute 共用同一份语句构建。
#[cfg(feature = "mssql")]
pub fn build_mssql_merge_statements<T: Model>(
    models: &[&T],
    with_update: bool,
) -> crate::Result<Vec<InsertSqlStatement>> {
    build_chunked_insert_statements::<T>(DbType::MSSQL, models, |chunk| {
        let (mut sql, params) = build_mssql_merge_source::<T>(chunk);
        if with_update {
            append_mssql_merge_update_clause::<T>(&mut sql);
        }
        append_mssql_merge_insert_clause::<T>(&mut sql);
        Ok(InsertSqlStatement {
            sql,
            params,
            row_count: chunk.len(),
        })
    })
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_update_clause<T: Model>(sql: &mut String) {
    // 更新列同样限定在 MERGE 源列（insert_columns，排除自增列）之内，
    // 避免引用源中不存在的自增列；主键列不参与更新。
    let pks = T::primary_key_columns();
    let updatable: Vec<&'static str> = T::insert_columns()
        .into_iter()
        .filter(|col_name| !pks.contains(col_name))
        .collect();
    if updatable.is_empty() {
        return;
    }
    sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
    for (index, col_name) in updatable.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            DbType::MSSQL,
            col_name,
            &quote_column_with_prefix(DbType::MSSQL, "source", col_name),
        ));
    }
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_insert_clause<T: Model>(sql: &mut String) {
    append_mssql_merge_insert_clause_for_columns(sql, &T::insert_columns());
}

#[cfg(feature = "mssql")]
fn append_mssql_merge_insert_clause_for_columns(sql: &mut String, columns: &[&str]) {
    let columns_sql = quote_column_list(DbType::MSSQL, columns);
    sql.push_str(&format!(
        " WHEN NOT MATCHED THEN INSERT ({columns_sql}) VALUES ("
    ));
    for (i, col_name) in columns.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_column_with_prefix(DbType::MSSQL, "source", col_name));
    }
    sql.push_str(");");
}

#[cfg(feature = "mssql")]
fn append_mssql_merge_auto_increment_output<T: Model>(sql: &mut String) {
    let Some(pk_col) = auto_increment_column::<T>() else {
        return;
    };

    if sql.ends_with(';') {
        sql.pop();
    }
    sql.push_str(&format!(
        " OUTPUT {};",
        quote_column_with_prefix(DbType::MSSQL, "inserted", pk_col)
    ));
}

/// 收集批量插入的所有模型值
pub fn collect_batch_insert_values<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.field_values();
        all_values.extend(values);
    }
    all_values
}

/// 收集批量插入的所有模型值（排除自增主键）
pub fn collect_batch_insert_values_with_auto_increment<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.insert_values();
        all_values.extend(values);
    }
    all_values
}

/// 统一的列值解析函数 - 严格模式
///
/// 用于流式查询中解析列值,非空字段解析失败时返回错误而非默认值
#[allow(clippy::too_many_arguments)]
pub fn parse_column_value_strict(
    rust_type: &str,
    is_nullable: bool,
    column_name: &str,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> crate::Result<Value> {
    parse_column_value_options(
        rust_type,
        is_nullable,
        get_int(),
        get_string(),
        get_real(),
        get_bool(),
        get_bytes(),
        get_datetime(),
        ColumnValueMode::Strict { column_name },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_insert_builder_uses_backend_placeholder_rules() {
        let columns = &["id", "name", "age"];

        #[cfg(feature = "sqlite")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::Sqlite, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "mysql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MySQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "postgresql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::PostgreSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES ($1, $2, $3), ($4, $5, $6)"
            );
        }

        #[cfg(feature = "mssql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MSSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (@P1, @P2, @P3), (@P4, @P5, @P6)"
            );
        }
    }
}


#[cfg(test)]
mod block_delete_tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(text: &str) -> chrono::DateTime<chrono::Utc> {
        let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap();
        chrono::Utc.from_utc_datetime(&naive)
    }

    #[test]
    fn partition_unit_maps_duration_to_granularity() {
        // < 1h → 小时
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(1800)),
            PartitionUnit::Hour
        );
        // 1h ≤ d < 7d → 天
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(3600)),
            PartitionUnit::Day
        );
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(6 * 86_400)),
            PartitionUnit::Day
        );
        // 7d ≤ d < 30d → 周
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(7 * 86_400)),
            PartitionUnit::Week
        );
        // 30d ≤ d < 365d → 月
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(30 * 86_400)),
            PartitionUnit::Month
        );
        // ≥ 365d → 年
        assert_eq!(
            PartitionUnit::from_duration(std::time::Duration::from_secs(365 * 86_400)),
            PartitionUnit::Year
        );
    }

    #[test]
    fn questdb_units_and_clickhouse_functions_match_mapping() {
        assert_eq!(PartitionUnit::Hour.questdb_unit(), "HOUR");
        assert_eq!(PartitionUnit::Day.questdb_unit(), "DAY");
        assert_eq!(PartitionUnit::Week.questdb_unit(), "WEEK");
        assert_eq!(PartitionUnit::Month.questdb_unit(), "MONTH");
        assert_eq!(PartitionUnit::Year.questdb_unit(), "YEAR");
        assert_eq!(PartitionUnit::Hour.clickhouse_function(), "toStartOfHour");
        assert_eq!(PartitionUnit::Day.clickhouse_function(), "toYYYYMMDD");
        assert_eq!(PartitionUnit::Week.clickhouse_function(), "toMonday");
        assert_eq!(PartitionUnit::Month.clickhouse_function(), "toYYYYMM");
        assert_eq!(PartitionUnit::Year.clickhouse_function(), "toYYYY");
    }

    #[test]
    fn align_to_block_floor_supports_calendar_boundaries() {
        // 2024-01-17 是周三
        let wednesday = dt("2024-01-17 12:34:56");
        assert_eq!(
            PartitionUnit::Hour.align_to_block(wednesday),
            dt("2024-01-17 12:00:00")
        );
        assert_eq!(PartitionUnit::Day.align_to_block(wednesday), dt("2024-01-17 00:00:00"));
        // 周界：向下对齐到 UTC 周一
        assert_eq!(PartitionUnit::Week.align_to_block(wednesday), dt("2024-01-15 00:00:00"));
        // 月界与年界
        assert_eq!(PartitionUnit::Month.align_to_block(wednesday), dt("2024-01-01 00:00:00"));
        assert_eq!(PartitionUnit::Year.align_to_block(wednesday), dt("2024-01-01 00:00:00"));
        // 恰在块边界上时保持不变
        let boundary = PartitionUnit::Day.align_to_block(wednesday);
        assert_eq!(PartitionUnit::Day.align_to_block(boundary), boundary);
    }

    #[test]
    fn align_before_floors_cutoff() {
        let cutoff = dt("2024-01-17 13:25:00");
        let range = AlignedBlockRange::align(
            BlockRange::Before { cutoff },
            PartitionUnit::Day,
            dt("1970-01-01 00:00:00"),
        )
        .unwrap()
        .expect("before should always produce a range");
        assert!(range.start.is_none());
        assert_eq!(range.end, dt("2024-01-17 00:00:00"));
    }

    #[test]
    fn align_between_uses_ceil_start_and_floor_end() {
        let start = dt("2024-01-17 13:25:00"); // 周三午后
        let end = dt("2024-01-20 16:13:00");
        let range = AlignedBlockRange::align(
            BlockRange::Between { start, end },
            PartitionUnit::Day,
            dt("1970-01-01 00:00:00"),
        )
        .unwrap()
        .expect("between should produce a range");
        // 起点向上对齐到下一个日界，终点向下对齐到所在日界
        assert_eq!(range.start, Some(dt("2024-01-18 00:00:00")));
        assert_eq!(range.end, dt("2024-01-20 00:00:00"));
    }

    #[test]
    fn align_between_rejects_reversed_range_and_nops_empty() {
        let t = dt("2024-01-17 13:25:00");
        // start >= end 报参数错误
        assert!(AlignedBlockRange::align(
            BlockRange::Between { start: t, end: t },
            PartitionUnit::Day,
            dt("1970-01-01 00:00:00"),
        )
        .is_err());
        // 同一天内的区间对齐后为空 → 安全 no-op
        assert_eq!(
            AlignedBlockRange::align(
                BlockRange::Between {
                    start: t,
                    end: dt("2024-01-17 19:00:00")
                },
                PartitionUnit::Day,
                dt("1970-01-01 00:00:00"),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn align_retain_equals_before_now_minus_duration() {
        let now = dt("2024-01-17 13:25:00");
        let duration = std::time::Duration::from_secs(30 * 86_400);
        let range = AlignedBlockRange::align(BlockRange::Retain { duration }, PartitionUnit::Day, now)
            .unwrap()
            .expect("retain produces a range");
        assert!(range.start.is_none());
        assert_eq!(
            range.end,
            PartitionUnit::Day.align_to_block(now - chrono::Duration::seconds(30 * 86_400))
        );
    }

    #[test]
    fn aligned_range_contains_only_complete_blocks() {
        let range = AlignedBlockRange {
            start: Some(dt("2024-01-17 00:00:00")),
            end: dt("2024-01-18 00:00:00"),
        };
        assert!(!range.contains_block(dt("2024-01-16 00:00:00"))); // 前一天
        assert!(range.contains_block(dt("2024-01-17 00:00:00")));
        assert!(!range.contains_block(dt("2024-01-18 00:00:00"))); // 下一天
    }

    #[test]
    fn parse_clickhouse_partition_by_recognizes_time_functions() {
        assert_eq!(
            PartitionUnit::parse_clickhouse_partition_by("toYYYYMM(ts)"),
            Some((PartitionUnit::Month, "ts".to_string()))
        );
        assert_eq!(
            PartitionUnit::parse_clickhouse_partition_by("toStartOfHour( time )"),
            Some((PartitionUnit::Hour, "time".to_string()))
        );
        assert_eq!(
            PartitionUnit::parse_clickhouse_partition_by("toMonday(date_col)"),
            Some((PartitionUnit::Week, "date_col".to_string()))
        );
        // 非映射表列出的时间函数不可识别
        assert_eq!(
            PartitionUnit::parse_clickhouse_partition_by("intDiv(ts, 86400)"),
            None
        );
        assert_eq!(PartitionUnit::parse_clickhouse_partition_by("ts"), None);
    }

    #[test]
    fn parse_clickhouse_partition_key_parses_all_units() {
        assert_eq!(
            PartitionUnit::Hour.parse_clickhouse_partition_key("2024-01-17 05:00:00"),
            Some(dt("2024-01-17 05:00:00"))
        );
        assert_eq!(
            PartitionUnit::Day.parse_clickhouse_partition_key("20240117"),
            Some(dt("2024-01-17 00:00:00"))
        );
        assert_eq!(
            PartitionUnit::Week.parse_clickhouse_partition_key("2024-01-15"),
            Some(dt("2024-01-15 00:00:00"))
        );
        assert_eq!(
            PartitionUnit::Month.parse_clickhouse_partition_key("202401"),
            Some(dt("2024-01-01 00:00:00"))
        );
        assert_eq!(
            PartitionUnit::Year.parse_clickhouse_partition_key("2024"),
            Some(dt("2024-01-01 00:00:00"))
        );
        assert_eq!(PartitionUnit::Day.parse_clickhouse_partition_key("not-a-key"), None);
    }

}
