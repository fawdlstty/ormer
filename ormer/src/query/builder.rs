use crate::abstract_layer::DbType;
use crate::model::{
    ColumnSchema, Model, normalize_table_name_for_db, quote_column_reference,
    quote_qualified_identifier,
};
use crate::query::expr::{AliasedExpr, IntoSqlExpr, SqlExpr, TypedExpr, WindowSpecBuilder};
use crate::query::filter::{FilterExpr, OrderBy};
#[cfg(feature = "postgresql")]
use crate::query::filter::{infer_filter_value_rust_type, infer_model_value_rust_type};
use crate::query::filter_formatter::FilterFormatter;
use std::fmt::Write;
use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Sub};

fn table_name_for<T: Model>(db_type: DbType) -> String {
    quote_qualified_identifier(db_type, T::table_name_for_db(db_type))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(feature = "postgresql")]
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(feature = "postgresql")]
fn postgres_null_expr(column: &ColumnSchema, rust_type: &str) -> String {
    if column.enum_variants.is_some() {
        return format!("NULL::{}", to_snake_case(column.rust_type));
    }

    match rust_type {
        "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "usize" => "NULL::INTEGER",
        "i64" | "u64" => "NULL::BIGINT",
        "f32" | "f64" => "NULL::DOUBLE PRECISION",
        "bool" => "NULL::BOOLEAN",
        "Duration" | "std::time::Duration" => "NULL::INTERVAL",
        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => "NULL::BYTEA",
        "Vec<i32>" | "std::vec::Vec<i32>" | "alloc::vec::Vec<i32>" => "NULL::INTEGER[]",
        "Vec<i64>"
        | "std::vec::Vec<i64>"
        | "alloc::vec::Vec<i64>"
        | "Vec<Option<i64>>"
        | "std::vec::Vec<Option<i64>>"
        | "alloc::vec::Vec<Option<i64>>" => "NULL::BIGINT[]",
        "DateTime" | "chrono::DateTime" | "chrono::DateTime<chrono::Utc>" => "NULL::TIMESTAMPTZ",
        "NaiveDateTime" | "chrono::NaiveDateTime" => "NULL::TIMESTAMPTZ",
        "NaiveDate" | "chrono::NaiveDate" => "NULL::DATE",
        "NaiveTime" | "chrono::NaiveTime" => "NULL::TIME",
        "JsonValue" | "serde_json::Value" => "NULL::JSONB",
        "Uuid" | "uuid::Uuid" => "NULL::UUID",
        _ => "NULL::TEXT",
    }
    .to_string()
}

fn ignored_column_default_expr(column: &ColumnSchema, db_type: DbType) -> String {
    let rust_type = column.data_type.unwrap_or(column.rust_type);

    if column.is_nullable {
        return match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => postgres_null_expr(column, rust_type),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "NULL".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "NULL".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "NULL".to_string(),
        };
    }

    if let Some(variants) = column.enum_variants
        && let Some(first_variant) = variants.first()
    {
        return quote_sql_string(first_variant);
    }

    match rust_type {
        "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "usize" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "0::INTEGER".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "0".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "0".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST(0 AS INT)".to_string(),
        },
        "i64" | "u64" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "0::BIGINT".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "0".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "0".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST(0 AS BIGINT)".to_string(),
        },
        "f32" | "f64" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "0::DOUBLE PRECISION".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "0.0".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "0.0".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST(0 AS FLOAT)".to_string(),
        },
        "bool" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "FALSE".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "0".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "FALSE".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST(0 AS BIT)".to_string(),
        },
        "Duration" | "std::time::Duration" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("0 seconds") + "::INTERVAL",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "0".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "0".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST(0 AS BIGINT)".to_string(),
        },
        "String" | "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>" => {
            quote_sql_string("")
        }
        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("") + "::BYTEA",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "X''".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "X''".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST('' AS VARBINARY(MAX))".to_string(),
        },
        "Vec<i32>" | "std::vec::Vec<i32>" | "alloc::vec::Vec<i32>" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "ARRAY[]::INTEGER[]".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "NULL".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "NULL".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "NULL".to_string(),
        },
        "Vec<i64>"
        | "std::vec::Vec<i64>"
        | "alloc::vec::Vec<i64>"
        | "Vec<Option<i64>>"
        | "std::vec::Vec<Option<i64>>"
        | "alloc::vec::Vec<Option<i64>>" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "ARRAY[]::BIGINT[]".to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "NULL".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "NULL".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "NULL".to_string(),
        },
        "DateTime" | "chrono::DateTime" | "chrono::DateTime<chrono::Utc>" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("1970-01-01 00:00:00+00") + "::TIMESTAMPTZ",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("1970-01-01T00:00:00+00:00"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "CAST('1970-01-01 00:00:00' AS DATETIME)".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST('1970-01-01T00:00:00' AS DATETIME2)".to_string(),
        },
        "NaiveDateTime" | "chrono::NaiveDateTime" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("1970-01-01 00:00:00+00") + "::TIMESTAMPTZ",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("1970-01-01T00:00:00+00:00"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "CAST('1970-01-01 00:00:00' AS DATETIME)".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST('1970-01-01T00:00:00' AS DATETIME2)".to_string(),
        },
        "NaiveDate" | "chrono::NaiveDate" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("1970-01-01") + "::DATE",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("1970-01-01"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "CAST('1970-01-01' AS DATE)".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST('1970-01-01' AS DATE)".to_string(),
        },
        "NaiveTime" | "chrono::NaiveTime" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("00:00:00") + "::TIME",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("00:00:00"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "CAST('00:00:00' AS TIME)".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "CAST('00:00:00' AS TIME)".to_string(),
        },
        "JsonValue" | "serde_json::Value" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => quote_sql_string("null") + "::JSONB",
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("null"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "CAST('null' AS JSON)".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => quote_sql_string("null"),
        },
        "Uuid" | "uuid::Uuid" => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                quote_sql_string("00000000-0000-0000-0000-000000000000") + "::UUID"
            }
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => quote_sql_string("00000000-0000-0000-0000-000000000000"),
            #[cfg(feature = "mysql")]
            DbType::MySQL => quote_sql_string("00000000-0000-0000-0000-000000000000"),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                "CAST(0x00000000000000000000000000000000 AS VARBINARY(16))".to_string()
            }
        },
        _ => quote_sql_string(""),
    }
}

fn select_expr_for_column<T: Model>(
    column: &'static str,
    db_type: DbType,
    ignored_columns: &[String],
    table_prefix: Option<&str>,
) -> String {
    if ignored_columns.iter().any(|ignored| ignored == column) {
        let schema = T::COLUMN_SCHEMA
            .iter()
            .find(|schema| schema.name == column)
            .unwrap_or_else(|| panic!("Column schema not found: {}", column));
        return format!(
            "{} AS {}",
            ignored_column_default_expr(schema, db_type),
            quote_column_reference(db_type, column)
        );
    }

    if let Some(prefix) = table_prefix {
        quote_column_reference(db_type, &format!("{}.{}", prefix, column))
    } else {
        quote_column_reference(db_type, column)
    }
}

fn select_exprs_for_model<T: Model>(
    db_type: DbType,
    ignored_columns: &[String],
    table_prefix: Option<&str>,
) -> String {
    T::COLUMNS
        .iter()
        .map(|column| select_expr_for_column::<T>(column, db_type, ignored_columns, table_prefix))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn default_db_type() -> DbType {
    #[cfg(feature = "sqlite")]
    {
        DbType::Sqlite
    }
    #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
    {
        DbType::PostgreSQL
    }
    #[cfg(all(
        not(feature = "sqlite"),
        not(feature = "postgresql"),
        feature = "mysql"
    ))]
    {
        DbType::MySQL
    }
    #[cfg(all(
        not(feature = "sqlite"),
        not(feature = "postgresql"),
        not(feature = "mysql"),
        feature = "mssql"
    ))]
    {
        DbType::MSSQL
    }
}

fn is_mssql_db(db_type: DbType) -> bool {
    #[cfg(feature = "mssql")]
    {
        db_type == DbType::MSSQL
    }
    #[cfg(not(feature = "mssql"))]
    {
        let _ = db_type;
        false
    }
}

fn append_filter_clause(
    sql: &mut String,
    keyword: &str,
    filters: &[FilterExpr],
    formatter: FilterFormatter,
    param_idx: &mut i32,
    params: &mut Vec<crate::model::Value>,
) {
    if filters.is_empty() {
        return;
    }

    sql.push_str(keyword);
    for (i, filter) in filters.iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        let filter_sql = formatter.format(filter, param_idx, params);
        sql.push_str(&filter_sql);
    }
}

fn append_select_tail(
    sql: &mut String,
    filters: &[FilterExpr],
    filter_keyword: &str,
    formatter: FilterFormatter,
    order_by: &[OrderBy],
    range_start: Option<usize>,
    range_end: Option<usize>,
    lock: Option<RowLock>,
    db_type: DbType,
    param_idx: &mut i32,
    params: &mut Vec<crate::model::Value>,
) {
    append_filter_clause(sql, filter_keyword, filters, formatter, param_idx, params);
    append_order_by_clause(sql, order_by, db_type);
    append_range_clause(sql, range_start, range_end, !order_by.is_empty(), db_type);
    append_lock_clause(sql, lock);
}

fn append_order_by_clause(sql: &mut String, order_by: &[OrderBy], db_type: DbType) {
    if order_by.is_empty() {
        return;
    }

    sql.push_str(" ORDER BY ");
    let order_strs: Vec<String> = order_by.iter().map(|o| o.to_sql_for(db_type)).collect();
    sql.push_str(&order_strs.join(", "));
}

#[derive(Debug, Clone, Copy)]
struct RowLock {
    mode: &'static str,
    skip_locked: bool,
    no_wait: bool,
}

#[derive(Debug, Clone)]
enum GroupingClause {
    GroupingSets(Vec<Vec<SqlExpr>>),
    Rollup(Vec<SqlExpr>),
    Cube(Vec<SqlExpr>),
}

impl RowLock {
    fn for_update() -> Self {
        Self {
            mode: "FOR UPDATE",
            skip_locked: false,
            no_wait: false,
        }
    }

    fn for_share() -> Self {
        Self {
            mode: "FOR SHARE",
            skip_locked: false,
            no_wait: false,
        }
    }
}

fn append_lock_clause(sql: &mut String, lock: Option<RowLock>) {
    if let Some(lock) = lock {
        sql.push(' ');
        sql.push_str(lock.mode);
        if lock.skip_locked {
            sql.push_str(" SKIP LOCKED");
        }
        if lock.no_wait {
            sql.push_str(" NOWAIT");
        }
    }
}

fn select_modifier_sql(
    distinct: bool,
    distinct_on: &[SqlExpr],
    db_type: DbType,
    param_idx: &mut i32,
    params: &mut Vec<crate::model::Value>,
) -> String {
    if !distinct_on.is_empty() {
        let exprs = distinct_on
            .iter()
            .map(|expr| expr.to_sql(db_type, param_idx, params, None))
            .collect::<Vec<_>>()
            .join(", ");
        format!("DISTINCT ON ({exprs}) ")
    } else if distinct {
        "DISTINCT ".to_string()
    } else {
        String::new()
    }
}

fn format_expr_list(
    exprs: &[SqlExpr],
    db_type: DbType,
    param_idx: &mut i32,
    params: &mut Vec<crate::model::Value>,
) -> String {
    exprs
        .iter()
        .map(|expr| expr.to_sql(db_type, param_idx, params, None))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_projection_list(
    exprs: &[SqlExpr],
    aliases: &[Option<String>],
    db_type: DbType,
    param_idx: &mut i32,
    params: &mut Vec<crate::model::Value>,
) -> String {
    exprs
        .iter()
        .enumerate()
        .map(|(index, expr)| {
            let expr_sql = expr.to_sql(db_type, param_idx, params, None);
            if let Some(alias) = aliases.get(index).and_then(|alias| alias.as_ref()) {
                format!("{} AS {}", expr_sql, quote_column_reference(db_type, alias))
            } else {
                expr_sql
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn range_limit(start: Option<usize>, end: usize) -> usize {
    if let Some(start) = start {
        end - start
    } else {
        end
    }
}

fn append_limit_offset_clause(
    sql: &mut String,
    range_start: Option<usize>,
    range_end: Option<usize>,
) {
    if let Some(end) = range_end {
        write!(sql, " LIMIT {}", range_limit(range_start, end))
            .expect("Failed to write LIMIT clause");
    }
    if let Some(start) = range_start {
        write!(sql, " OFFSET {}", start).expect("Failed to write OFFSET clause");
    }
}

fn append_range_clause(
    sql: &mut String,
    range_start: Option<usize>,
    range_end: Option<usize>,
    has_order_by: bool,
    db_type: DbType,
) {
    if is_mssql_db(db_type) {
        if let Some(end) = range_end {
            if !has_order_by {
                sql.push_str(" ORDER BY (SELECT NULL)");
            }
            write!(
                sql,
                " OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
                range_start.unwrap_or(0),
                range_limit(range_start, end)
            )
            .expect("Failed to write OFFSET/FETCH clause");
        }
    } else {
        append_limit_offset_clause(sql, range_start, range_end);
    }
}

fn format_from_table_list(tables: &[String]) -> String {
    tables
        .iter()
        .enumerate()
        .map(|(index, table)| format!("{} AS t{}", table, index))
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_join_condition(db_type: DbType, filter: &FilterExpr, sql: &mut String) {
    if let FilterExpr::ColumnComparison {
        left_column,
        operator,
        right_column,
    } = filter
    {
        write!(
            sql,
            "{} {} {}",
            quote_column_reference(db_type, &format!("t0.{}", left_column)),
            operator,
            quote_column_reference(db_type, &format!("t1.{}", right_column))
        )
        .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));
    }
}

#[derive(Clone, Copy)]
enum JoinKind {
    Left,
    Inner,
    Right,
}

impl JoinKind {
    fn keyword(self) -> &'static str {
        match self {
            JoinKind::Left => "LEFT JOIN",
            JoinKind::Inner => "INNER JOIN",
            JoinKind::Right => "RIGHT JOIN",
        }
    }
}

struct JoinSqlParts<'a> {
    filters: &'a [FilterExpr],
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: &'a [String],
    join_table: &'a str,
    join_alias: &'a str,
    on_condition: &'a FilterExpr,
    join_order_by: &'a [OrderBy],
    join_range_start: Option<usize>,
    join_range_end: Option<usize>,
}

fn joined_select_header<T: Model, J: Model>(
    sql: &mut String,
    db_type: DbType,
    join_kind: JoinKind,
    ignored_columns: &[String],
    join_table: &str,
    join_alias: &str,
    lateral: bool,
) {
    let lateral_sql = if lateral {
        " LATERAL (SELECT * FROM "
    } else {
        " "
    };
    write!(
        sql,
        "SELECT {}, {} FROM {} AS t0 {}{}{}",
        select_exprs_for_model::<T>(db_type, ignored_columns, Some("t0")),
        select_exprs_for_model::<J>(db_type, &[], Some("t1")),
        table_name_for::<T>(db_type),
        join_kind.keyword(),
        lateral_sql,
        quote_qualified_identifier(db_type, normalize_table_name_for_db(db_type, join_table)),
    )
    .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));

    if !lateral {
        write!(sql, " AS {}", join_alias).unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));
    }
}

fn plain_join_sql_with_params<T: Model, J: Model>(
    db_type: DbType,
    join_kind: JoinKind,
    parts: JoinSqlParts<'_>,
) -> (String, Vec<crate::model::Value>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    let mut param_idx = 1;

    joined_select_header::<T, J>(
        &mut sql,
        db_type,
        join_kind,
        parts.ignored_columns,
        parts.join_table,
        parts.join_alias,
        false,
    );

    sql.push_str(" ON ");
    append_join_condition(db_type, parts.on_condition, &mut sql);

    append_filter_clause(
        &mut sql,
        " WHERE ",
        parts.filters,
        FilterFormatter::new(db_type)
            .with_table_prefix("t0")
            .with_right_table_prefix("t1"),
        &mut param_idx,
        &mut params,
    );
    append_range_clause(&mut sql, parts.range_start, parts.range_end, false, db_type);

    (sql, params)
}

fn lateral_join_sql_with_params<T: Model, J: Model>(
    db_type: DbType,
    join_kind: JoinKind,
    parts: JoinSqlParts<'_>,
) -> (String, Vec<crate::model::Value>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    let mut param_idx = 1;

    joined_select_header::<T, J>(
        &mut sql,
        db_type,
        join_kind,
        parts.ignored_columns,
        parts.join_table,
        parts.join_alias,
        true,
    );

    let formatter = FilterFormatter::new(db_type).with_table_prefix("t0");
    let condition_sql = formatter.format(parts.on_condition, &mut param_idx, &mut params);
    write!(&mut sql, " WHERE {}", condition_sql)
        .unwrap_or_else(|e| panic!("Failed to write lateral WHERE clause: {}", e));

    append_order_by_clause(&mut sql, parts.join_order_by, db_type);
    append_limit_offset_clause(&mut sql, parts.join_range_start, parts.join_range_end);

    write!(&mut sql, ") AS {} ON true", parts.join_alias)
        .unwrap_or_else(|e| panic!("Failed to write lateral JOIN closing: {}", e));

    append_filter_clause(
        &mut sql,
        " WHERE ",
        parts.filters,
        FilterFormatter::new(db_type)
            .with_table_prefix("t0")
            .with_right_table_prefix("t1"),
        &mut param_idx,
        &mut params,
    );
    append_limit_offset_clause(&mut sql, parts.range_start, parts.range_end);

    (sql, params)
}

fn join_sql_with_params<T: Model, J: Model>(
    db_type: DbType,
    join_kind: JoinKind,
    lateral: bool,
    parts: JoinSqlParts<'_>,
) -> (String, Vec<crate::model::Value>) {
    if lateral {
        lateral_join_sql_with_params::<T, J>(db_type, join_kind, parts)
    } else {
        plain_join_sql_with_params::<T, J>(db_type, join_kind, parts)
    }
}

/// 范围边界类型,支持多种 range 语法
pub struct RangeBounds {
    pub start: Option<usize>,
    pub end: Option<usize>,
}

impl From<std::ops::Range<usize>> for RangeBounds {
    fn from(range: std::ops::Range<usize>) -> Self {
        RangeBounds {
            start: Some(range.start),
            end: Some(range.end),
        }
    }
}

impl From<std::ops::RangeTo<usize>> for RangeBounds {
    fn from(range: std::ops::RangeTo<usize>) -> Self {
        RangeBounds {
            start: None,
            end: Some(range.end),
        }
    }
}

impl From<std::ops::RangeFrom<usize>> for RangeBounds {
    fn from(range: std::ops::RangeFrom<usize>) -> Self {
        RangeBounds {
            start: Some(range.start),
            end: None,
        }
    }
}

#[cfg(feature = "postgresql")]
fn collect_model_filter_param_rust_types<T: Model>(filters: &[FilterExpr]) -> Vec<&'static str> {
    let mut rust_types = Vec::new();
    for filter in filters {
        collect_filter_param_rust_types::<T>(filter, &mut rust_types);
    }
    rust_types
}

#[cfg(feature = "postgresql")]
fn collect_join_filter_param_rust_types<T: Model>(
    lateral: bool,
    on_condition: &FilterExpr,
    filters: &[FilterExpr],
) -> Vec<&'static str> {
    let mut rust_types = Vec::new();
    if lateral {
        collect_filter_param_rust_types::<T>(on_condition, &mut rust_types);
    }
    for filter in filters {
        collect_filter_param_rust_types::<T>(filter, &mut rust_types);
    }
    rust_types
}

#[cfg(feature = "postgresql")]
fn is_vec_string_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>"
    )
}

#[cfg(feature = "postgresql")]
fn collect_filter_param_rust_types<T: Model>(
    filter: &FilterExpr,
    rust_types: &mut Vec<&'static str>,
) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            let rust_type = model_column_rust_type::<T>(column)
                .unwrap_or_else(|| infer_filter_value_rust_type(value));
            rust_types.push(
                if operator == "@>"
                    && is_vec_string_type(rust_type)
                    && matches!(value, crate::query::filter::Value::Text(_))
                {
                    "String"
                } else {
                    rust_type
                },
            );
        }
        FilterExpr::In { column, values } | FilterExpr::NotIn { column, values } => {
            let rust_type = model_column_rust_type::<T>(column);
            for value in values {
                rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(value)));
            }
        }
        FilterExpr::InSubquery {
            subquery_params, ..
        }
        | FilterExpr::NotInSubquery {
            subquery_params, ..
        } => {
            rust_types.extend(subquery_params.iter().map(infer_model_value_rust_type));
        }
        FilterExpr::And(left, right) | FilterExpr::Or(left, right) => {
            collect_filter_param_rust_types::<T>(left, rust_types);
            collect_filter_param_rust_types::<T>(right, rust_types);
        }
        FilterExpr::Between { column, min, max } => {
            let rust_type = model_column_rust_type::<T>(column);
            rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(min)));
            rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(max)));
        }
        FilterExpr::ColumnComparison { .. }
        | FilterExpr::IsNull { .. }
        | FilterExpr::IsNotNull { .. } => {}
        FilterExpr::Exists {
            subquery_params, ..
        }
        | FilterExpr::NotExists {
            subquery_params, ..
        } => {
            rust_types.extend(subquery_params.iter().map(infer_model_value_rust_type));
        }
        FilterExpr::ExprComparison { left, right, .. } => {
            collect_sql_expr_param_rust_types::<T>(left, rust_types);
            collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        FilterExpr::ExprIn { expr, values } | FilterExpr::ExprNotIn { expr, values } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            for value in values {
                collect_sql_expr_param_rust_types::<T>(value, rust_types);
            }
        }
        FilterExpr::ExprBetween { expr, min, max } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            collect_sql_expr_param_rust_types::<T>(min, rust_types);
            collect_sql_expr_param_rust_types::<T>(max, rust_types);
        }
        FilterExpr::ExprIsNull { expr } | FilterExpr::ExprIsNotNull { expr } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        FilterExpr::TextSearch { expr, .. } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            rust_types.push("String");
        }
    }
}

#[cfg(feature = "postgresql")]
fn collect_sql_expr_param_rust_types<T: Model>(expr: &SqlExpr, rust_types: &mut Vec<&'static str>) {
    match expr {
        SqlExpr::Column(_) | SqlExpr::Raw(_) => {}
        SqlExpr::Value(value) => rust_types.push(infer_model_value_rust_type(value)),
        SqlExpr::Binary { left, right, .. } => {
            collect_sql_expr_param_rust_types::<T>(left, rust_types);
            collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        SqlExpr::Function { args, .. } | SqlExpr::Row(args) => {
            for arg in args {
                collect_sql_expr_param_rust_types::<T>(arg, rust_types);
            }
        }
        SqlExpr::Cast { expr, .. }
        | SqlExpr::Collate { expr, .. }
        | SqlExpr::JsonText { expr, .. } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        SqlExpr::Aggregate {
            expr,
            filter,
            order_by: _,
            over,
            ..
        } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            if let Some(filter) = filter {
                collect_filter_param_rust_types::<T>(filter, rust_types);
            }
            if let Some(over) = over {
                for expr in &over.partition_by {
                    collect_sql_expr_param_rust_types::<T>(expr, rust_types);
                }
            }
        }
        SqlExpr::CaseMatch {
            expr,
            branches,
            else_expr,
        } => {
            collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            for (when, then) in branches {
                collect_sql_expr_param_rust_types::<T>(when, rust_types);
                collect_sql_expr_param_rust_types::<T>(then, rust_types);
            }
            collect_sql_expr_param_rust_types::<T>(else_expr, rust_types);
        }
    }
}

#[cfg(feature = "postgresql")]
fn model_column_rust_type<T: Model>(column: &str) -> Option<&'static str> {
    let column = normalize_filter_column_name(column);
    T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .map(|schema| schema.data_type.unwrap_or(schema.rust_type))
}

#[cfg(feature = "postgresql")]
fn normalize_filter_column_name(column: &str) -> &str {
    let column = column.rsplit('.').next().unwrap_or(column);
    if let Some(open_idx) = column.find('(')
        && let Some(close_idx) = column.rfind(')')
        && close_idx > open_idx + 1
    {
        return &column[open_idx + 1..close_idx];
    }
    column
}

/// Select 查询结构体
///
/// 使用方式:`Select::<User>`().filter(|p| p.age > 12).to_sql()
pub struct Select<T: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    distinct: bool,
    distinct_on: Vec<SqlExpr>,
    lock: Option<RowLock>,
    ignored_columns: Vec<String>,
    _marker: PhantomData<T>,
}

impl<T: Model> Clone for Select<T> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            distinct: self.distinct,
            distinct_on: self.distinct_on.clone(),
            lock: self.lock,
            ignored_columns: self.ignored_columns.clone(),
            _marker: PhantomData,
        }
    }
}

/// RelatedSelect - 关联查询结构体(支持2表查询)
pub struct RelatedSelect<T: Model, R: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTableSelect - 多表关联查询结构体(支持3个或以上表)
pub struct MultiTableSelect<T: Model, R1: Model, R2: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTableSelect - 四表关联查询结构体
pub struct FourTableSelect<T: Model, R1: Model, R2: Model, R3: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

/// AggregateSelect - 聚合查询结构体
pub struct AggregateSelect<T: Model, R = crate::model::Value> {
    aggregate_func: String, // COUNT, SUM, AVG, MAX, MIN
    column_name: String,
    filters: Vec<FilterExpr>,
    _marker: PhantomData<(T, R)>,
}

/// MappedSelect - 字段投影查询结构体
pub struct MappedSelect<T: Model, V> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    column_names: Vec<String>,        // 要查询的字段名列表（支持多字段）
    column_exprs: Vec<SqlExpr>,       // 投影表达式
    alias_names: Vec<Option<String>>, // 别名列表（用于映射到目标Model）
    distinct: bool,
    distinct_on: Vec<SqlExpr>,
    lock: Option<RowLock>,
    _marker: PhantomData<(T, V)>,
}

/// GroupedSelect - 分组聚合查询结构体
pub struct GroupedSelect<T: Model, V> {
    column_names: Vec<String>,            // SELECT 的列（包含聚合函数）
    column_exprs: Vec<SqlExpr>,           // SELECT 表达式
    aggregate_funcs: Vec<Option<String>>, // 聚合函数列表
    alias_names: Vec<Option<String>>,     // 别名列表
    group_by_columns: Vec<String>,        // GROUP BY 的列
    group_by_exprs: Vec<SqlExpr>,         // GROUP BY 表达式
    grouping_clause: Option<GroupingClause>,
    having_filters: Vec<FilterExpr>, // HAVING 条件
    filters: Vec<FilterExpr>,        // WHERE 条件（分组前过滤）
    order_by: Vec<OrderBy>,          // ORDER BY
    range_start: Option<usize>,
    range_end: Option<usize>,
    _marker: PhantomData<(T, V)>,
}

impl<T: Model, V> Clone for MappedSelect<T, V> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            column_names: self.column_names.clone(),
            column_exprs: self.column_exprs.clone(),
            alias_names: self.alias_names.clone(),
            distinct: self.distinct,
            distinct_on: self.distinct_on.clone(),
            lock: self.lock,
            _marker: PhantomData,
        }
    }
}

impl<T: Model, V> Clone for GroupedSelect<T, V> {
    fn clone(&self) -> Self {
        Self {
            column_names: self.column_names.clone(),
            column_exprs: self.column_exprs.clone(),
            aggregate_funcs: self.aggregate_funcs.clone(),
            alias_names: self.alias_names.clone(),
            group_by_columns: self.group_by_columns.clone(),
            group_by_exprs: self.group_by_exprs.clone(),
            grouping_clause: self.grouping_clause.clone(),
            having_filters: self.having_filters.clone(),
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            _marker: PhantomData,
        }
    }
}

impl<T: Model, V> Default for GroupedSelect<T, V> {
    fn default() -> Self {
        Self {
            column_names: Vec::new(),
            column_exprs: Vec::new(),
            aggregate_funcs: Vec::new(),
            alias_names: Vec::new(),
            group_by_columns: Vec::new(),
            group_by_exprs: Vec::new(),
            grouping_clause: None,
            having_filters: Vec::new(),
            filters: Vec::new(),
            order_by: Vec::new(),
            range_start: None,
            range_end: None,
            _marker: PhantomData,
        }
    }
}

impl<T: Model, R> AggregateSelect<T, R> {
    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();

        // SELECT 聚合函数
        write!(
            &mut sql,
            "SELECT {}({}) FROM {}",
            self.aggregate_func,
            quote_column_reference(db_type, &self.column_name),
            table_name_for::<T>(db_type)
        )
        .expect("Failed to write aggregate SELECT clause");

        let mut param_idx = 1;
        append_filter_clause(
            &mut sql,
            " WHERE ",
            &self.filters,
            FilterFormatter::new(db_type),
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }
}

impl<T: Model, V> MappedSelect<T, V> {
    /// 获取列名列表
    pub fn column_names(&self) -> &[String] {
        &self.column_names
    }

    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        let mut rust_types = Vec::new();
        for expr in &self.column_exprs {
            collect_sql_expr_param_rust_types::<T>(expr, &mut rust_types);
        }
        for filter in &self.filters {
            collect_filter_param_rust_types::<T>(filter, &mut rust_types);
        }
        rust_types
    }

    /// 设置别名列表（用于映射到目标Model）
    pub fn with_aliases(mut self, aliases: Vec<String>) -> Self {
        self.alias_names = aliases.into_iter().map(Some).collect();
        self
    }

    pub fn distinct_on<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> G,
        G: GroupByColumns,
    {
        self.distinct_on = f(T::Where::default()).sql_exprs();
        self
    }

    pub fn for_update(mut self) -> Self {
        self.lock = Some(RowLock::for_update());
        self
    }

    pub fn for_share(mut self) -> Self {
        self.lock = Some(RowLock::for_share());
        self
    }

    pub fn skip_locked(mut self) -> Self {
        let mut lock = self.lock.unwrap_or_else(RowLock::for_update);
        lock.skip_locked = true;
        self.lock = Some(lock);
        self
    }

    pub fn nowait(mut self) -> Self {
        let mut lock = self.lock.unwrap_or_else(RowLock::for_update);
        lock.no_wait = true;
        self.lock = Some(lock);
        self
    }

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let expr = f(T::Where::default());
        self.filters.push(expr.into());
        self
    }

    pub fn order_by<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        self.order_by.push(f(T::Where::default()).into());
        self
    }

    pub fn order_by_desc<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        let mut order = f(T::Where::default()).into();
        order.direction = crate::query::filter::OrderDirection::Desc;
        self.order_by.push(order);
        self
    }

    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SELECT 字段（支持单个或多个字段，带别名）
        let column_exprs = if self.column_exprs.is_empty() {
            self.column_names
                .iter()
                .cloned()
                .map(SqlExpr::Column)
                .collect::<Vec<_>>()
        } else {
            self.column_exprs.clone()
        };
        let distinct_str = select_modifier_sql(
            self.distinct,
            &self.distinct_on,
            db_type,
            &mut param_idx,
            &mut params,
        );
        let columns = format_projection_list(
            &column_exprs,
            &self.alias_names,
            db_type,
            &mut param_idx,
            &mut params,
        );
        write!(
            &mut sql,
            "SELECT {}{} FROM {}",
            distinct_str,
            columns,
            table_name_for::<T>(db_type)
        )
        .expect("Failed to write SELECT clause");

        append_select_tail(
            &mut sql,
            &self.filters,
            " WHERE ",
            FilterFormatter::new(db_type),
            &self.order_by,
            self.range_start,
            self.range_end,
            self.lock,
            db_type,
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }

    /// 生成 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        let (sql, _) = self.to_sql_with_params(default_db_type());
        sql
    }
}

impl<T: Model, V> GroupedSelect<T, V> {
    /// 创建新的 GroupedSelect 实例
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加选择的列（支持聚合函数，链式调用）
    pub fn select_column<F, V2>(self, f: F) -> GroupedSelect<T, V2>
    where
        F: FnOnce(<T as Model>::Where) -> V2,
        V2: SelectColumnResult,
    {
        let where_obj = <T as Model>::Where::default();
        let result = f(where_obj);

        // 创建新的 GroupedSelect，保留之前的列信息
        GroupedSelect {
            column_names: self
                .column_names
                .into_iter()
                .chain(result.column_names())
                .collect(),
            column_exprs: self
                .column_exprs
                .into_iter()
                .chain(result.sql_exprs())
                .collect(),
            aggregate_funcs: self
                .aggregate_funcs
                .into_iter()
                .chain(result.aggregate_funcs())
                .collect(),
            alias_names: self
                .alias_names
                .into_iter()
                .chain(result.alias_names())
                .collect(),
            group_by_columns: self.group_by_columns,
            group_by_exprs: self.group_by_exprs,
            grouping_clause: self.grouping_clause,
            having_filters: self.having_filters,
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            _marker: PhantomData,
        }
    }

    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: GroupByColumns,
    {
        let where_obj = <T as Model>::Where::default();
        let group_cols = f(where_obj);
        self.group_by_columns = group_cols.column_names();
        self.group_by_exprs = group_cols.sql_exprs();
        self.grouping_clause = None;
        self
    }

    pub fn grouping_sets<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: IntoGroupingSets,
    {
        let where_obj = <T as Model>::Where::default();
        self.grouping_clause = Some(GroupingClause::GroupingSets(
            f(where_obj).into_grouping_sets(),
        ));
        self.group_by_columns.clear();
        self.group_by_exprs.clear();
        self
    }

    pub fn rollup<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: GroupByColumns,
    {
        let where_obj = <T as Model>::Where::default();
        self.grouping_clause = Some(GroupingClause::Rollup(f(where_obj).sql_exprs()));
        self.group_by_columns.clear();
        self.group_by_exprs.clear();
        self
    }

    pub fn cube<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: GroupByColumns,
    {
        let where_obj = <T as Model>::Where::default();
        self.grouping_clause = Some(GroupingClause::Cube(f(where_obj).sql_exprs()));
        self.group_by_columns.clear();
        self.group_by_exprs.clear();
        self
    }

    /// 添加 HAVING 条件
    pub fn having<F>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> WhereExpr,
    {
        let where_obj = <T as Model>::Where::default();
        let expr = f(where_obj);
        self.having_filters.push(expr.into());
        self
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    /// 添加排序
    pub fn order_by<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        let where_obj = T::Where::default();
        let order = f(where_obj).into();
        self.order_by.push(order);
        self
    }

    /// 添加降序排序
    pub fn order_by_desc<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        let where_obj = T::Where::default();
        let mut order = f(where_obj).into();
        order.direction = crate::query::filter::OrderDirection::Desc;
        self.order_by.push(order);
        self
    }

    /// 设置范围
    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SELECT 子句（处理聚合函数和通用表达式）
        let column_exprs = if self.column_exprs.is_empty() {
            self.column_names
                .iter()
                .zip(self.aggregate_funcs.iter())
                .map(|(col, agg)| match agg {
                    Some(func) => SqlExpr::Aggregate {
                        name: Box::leak(func.clone().into_boxed_str()),
                        expr: Box::new(SqlExpr::Column(col.clone())),
                        filter: None,
                        order_by: Vec::new(),
                        over: None,
                    },
                    None => SqlExpr::Column(col.clone()),
                })
                .collect::<Vec<_>>()
        } else {
            self.column_exprs.clone()
        };
        let columns = format_projection_list(
            &column_exprs,
            &self.alias_names,
            db_type,
            &mut param_idx,
            &mut params,
        );

        write!(
            &mut sql,
            "SELECT {} FROM {}",
            columns,
            table_name_for::<T>(db_type)
        )
        .expect("Failed to write SELECT clause");

        append_filter_clause(
            &mut sql,
            " WHERE ",
            &self.filters,
            FilterFormatter::new(db_type),
            &mut param_idx,
            &mut params,
        );

        // GROUP BY 子句
        let group_by_exprs = if self.group_by_exprs.is_empty() {
            self.group_by_columns
                .iter()
                .cloned()
                .map(SqlExpr::Column)
                .collect::<Vec<_>>()
        } else {
            self.group_by_exprs.clone()
        };
        if let Some(grouping_clause) = &self.grouping_clause {
            sql.push_str(" GROUP BY ");
            match grouping_clause {
                GroupingClause::GroupingSets(sets) => {
                    sql.push_str("GROUPING SETS (");
                    let rendered_sets = sets
                        .iter()
                        .map(|set| {
                            if set.is_empty() {
                                "()".to_string()
                            } else {
                                format!(
                                    "({})",
                                    format_expr_list(set, db_type, &mut param_idx, &mut params)
                                )
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    sql.push_str(&rendered_sets);
                    sql.push(')');
                }
                GroupingClause::Rollup(exprs) => {
                    sql.push_str("ROLLUP (");
                    sql.push_str(&format_expr_list(
                        exprs,
                        db_type,
                        &mut param_idx,
                        &mut params,
                    ));
                    sql.push(')');
                }
                GroupingClause::Cube(exprs) => {
                    sql.push_str("CUBE (");
                    sql.push_str(&format_expr_list(
                        exprs,
                        db_type,
                        &mut param_idx,
                        &mut params,
                    ));
                    sql.push(')');
                }
            }
        } else if !group_by_exprs.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&format_expr_list(
                &group_by_exprs,
                db_type,
                &mut param_idx,
                &mut params,
            ));
        }

        #[cfg(feature = "postgresql")]
        let having_formatter = if matches!(db_type, crate::DbType::PostgreSQL) {
            FilterFormatter::new(db_type).with_postgresql_having_cast(true)
        } else {
            FilterFormatter::new(db_type)
        };
        #[cfg(not(feature = "postgresql"))]
        let having_formatter = FilterFormatter::new(db_type);
        append_select_tail(
            &mut sql,
            &self.having_filters,
            " HAVING ",
            having_formatter,
            &self.order_by,
            self.range_start,
            self.range_end,
            None,
            db_type,
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }

    /// 生成 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        let (sql, _) = self.to_sql_with_params(default_db_type());
        sql
    }

    /// 生成 SQL（公共方法，供执行器使用）
    pub fn build_sql(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        self.to_sql_with_params(db_type)
    }

    /// 获取列数（供执行器使用）
    pub fn column_count(&self) -> usize {
        self.column_names.len()
    }
}

impl<T: Model> Select<T> {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            order_by: Vec::new(),
            range_start: None,
            range_end: None,
            distinct: false,
            distinct_on: Vec::new(),
            lock: None,
            ignored_columns: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// `select::<User>`().from::<User, Role>()
    pub fn from<T2, R: Model>(self) -> RelatedSelect<T, R>
    where
        T2: Model + 'static,
    {
        // 通过类型约束确保 T2 == T
        // 如果 T2 != T,编译器会在类型推导时报错
        RelatedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// `select::<User>`().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelect<T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// `select::<User>`().from4::<User, Role, Permission, Department>()
    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(self) -> FourTableSelect<T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            _marker: PhantomData,
        }
    }

    /// 创建聚合查询
    #[allow(dead_code)]
    fn aggregate(self, func: &str, column: &str) -> AggregateSelect<T> {
        AggregateSelect {
            aggregate_func: func.to_string(),
            column_name: column.to_string(),
            filters: self.filters,
            _marker: PhantomData,
        }
    }

    /// 创建带类型参数的聚合查询
    fn aggregate_typed<R>(self, func: &str, column: &str) -> AggregateSelect<T, R> {
        AggregateSelect {
            aggregate_func: func.to_string(),
            column_name: column.to_string(),
            filters: self.filters,
            _marker: PhantomData,
        }
    }

    /// COUNT 聚合函数 - 返回记录数量（usize类型）
    pub fn count<F, C>(self, f: F) -> AggregateSelect<T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C, T>,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("COUNT", column.column_name())
    }

    /// SUM 聚合函数 - 编译期类型推断
    pub fn sum<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C, T>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("SUM", column.column_name())
    }

    /// AVG 聚合函数 - 总是返回 f64
    pub fn avg<F, C>(self, f: F) -> AggregateSelect<T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C, T>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("AVG", column.column_name())
    }

    /// MAX 聚合函数 - 编译期类型推断
    pub fn max<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C, T>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("MAX", column.column_name())
    }

    /// MIN 聚合函数 - 编译期类型推断
    pub fn min<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C, T>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("MIN", column.column_name())
    }

    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持：
    /// - 单字段：map_to(|r| r.uid) -> MappedSelect<T, i32>
    /// - 元组：map_to(|r| (r.uid, r.id)) -> MappedSelect<T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> MappedSelect<T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: MapToResult,
    {
        let where_obj = <T as Model>::Where::default();
        let result = f(where_obj);
        MappedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            column_names: result.column_names(),
            column_exprs: result.sql_exprs(),
            alias_names: result.alias_names(),
            distinct: self.distinct,
            distinct_on: self.distinct_on,
            lock: self.lock,
            _marker: PhantomData,
        }
    }

    /// 忽略指定字段 - 查询时用默认常量替代真实列值，返回类型仍为完整 Model
    pub fn ignore<F, M>(mut self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: MapToResult,
    {
        let where_obj = <T as Model>::Where::default();
        let result = f(where_obj);
        for column in result.column_names() {
            if !self
                .ignored_columns
                .iter()
                .any(|ignored| ignored == &column)
            {
                self.ignored_columns.push(column);
            }
        }
        self
    }

    /// 字段投影并映射到目标Model - 自动生成别名以匹配目标Model的列名
    /// 例如：map_to_model(|r| r.uid) 会生成 "SELECT uid AS id FROM ..."
    pub fn map_to_model<F, TargetModel>(self, f: F) -> MappedSelect<T, TargetModel>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<<TargetModel as Model>::QueryBuilder, T>,
        TargetModel: Model,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);

        // 使用目标Model的列名作为别名
        let alias_names: Vec<String> = TargetModel::COLUMNS.iter().map(|s| s.to_string()).collect();

        MappedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            column_names: vec![column.column_name.to_string()],
            column_exprs: vec![column.sql_expr()],
            alias_names: alias_names.into_iter().map(Some).collect(),
            distinct: self.distinct,
            distinct_on: self.distinct_on,
            lock: self.lock,
            _marker: PhantomData,
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelect<T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: SelectColumnResult,
    {
        let where_obj = <T as Model>::Where::default();
        let result = f(where_obj);

        GroupedSelect {
            column_names: result.column_names(),
            column_exprs: result.sql_exprs(),
            aggregate_funcs: result.aggregate_funcs(),
            alias_names: result.alias_names(),
            group_by_columns: Vec::new(),
            group_by_exprs: Vec::new(),
            grouping_clause: None,
            having_filters: Vec::new(),
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            _marker: PhantomData,
        }
    }
}

impl<T: Model> Select<T> {
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    /// 添加 WHERE 条件 (使用宏支持 >= 和 > 运算符语法)
    #[doc(hidden)]
    pub fn filter_cmp<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        self.filter(f)
    }

    /// 添加排序
    pub fn order_by<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        let where_obj = T::Where::default();
        let order = f(where_obj).into();
        self.order_by.push(order);
        self
    }

    /// 添加降序排序
    pub fn order_by_desc<F, O>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<OrderBy>,
    {
        let where_obj = T::Where::default();
        let mut order = f(where_obj).into();
        order.direction = crate::query::filter::OrderDirection::Desc;
        self.order_by.push(order);
        self
    }

    /// 设置范围 - 支持完整范围 (start..end)、只有上限 (..end)、只有下限 (start..)
    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 限制只返回第一条记录
    /// 自动设置 range_end = Some(1) 并清除 range_start
    pub fn first(mut self) -> Self {
        self.range_start = None;
        self.range_end = Some(1);
        self
    }

    /// 启用 DISTINCT 去重
    /// 生成的 SQL 将使用 SELECT DISTINCT
    pub fn distinct(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// 启用 PostgreSQL DISTINCT ON
    pub fn distinct_on<F, G>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> G,
        G: GroupByColumns,
    {
        let where_obj = T::Where::default();
        self.distinct_on = f(where_obj).sql_exprs();
        self
    }

    pub fn for_update(mut self) -> Self {
        self.lock = Some(RowLock::for_update());
        self
    }

    pub fn for_share(mut self) -> Self {
        self.lock = Some(RowLock::for_share());
        self
    }

    pub fn skip_locked(mut self) -> Self {
        let mut lock = self.lock.unwrap_or_else(RowLock::for_update);
        lock.skip_locked = true;
        self.lock = Some(lock);
        self
    }

    pub fn nowait(mut self) -> Self {
        let mut lock = self.lock.unwrap_or_else(RowLock::for_update);
        lock.no_wait = true;
        self.lock = Some(lock);
        self
    }

    /// 将此查询转换为 EXISTS 子查询表达式
    ///
    /// 生成 SQL: `EXISTS (SELECT 1 FROM table WHERE ...)`
    ///
    /// # 示例
    /// ```ignore
    /// let users_with_orders = db.select::<User>()
    ///     .filter(|p| {
    ///         Select::<Order>::new()
    ///             .filter(|o| o.user_id.eq(p.id))
    ///             .exists()
    ///     })
    ///     .collect::<Vec<_>>().await?;
    /// ```
    pub fn exists(self) -> WhereExpr {
        let (sql, params) = self.to_exists_sql_with_params();
        WhereExpr {
            inner: FilterExpr::Exists {
                subquery_sql: sql,
                subquery_params: params,
            },
            ..WhereExpr::defaults()
        }
    }

    /// 将此查询转换为 NOT EXISTS 子查询表达式
    ///
    /// 生成 SQL: `NOT EXISTS (SELECT 1 FROM table WHERE ...)`
    pub fn not_exists(self) -> WhereExpr {
        let (sql, params) = self.to_exists_sql_with_params();
        WhereExpr {
            inner: FilterExpr::NotExists {
                subquery_sql: sql,
                subquery_params: params,
            },
            ..WhereExpr::defaults()
        }
    }

    /// 生成 EXISTS 子查询专用 SQL（SELECT 1 FROM ...）
    fn to_exists_sql_with_params(&self) -> (String, Vec<crate::model::Value>) {
        let db_type = default_db_type();

        let mut sql = String::new();
        let mut params = Vec::new();

        write!(&mut sql, "SELECT 1 FROM {}", table_name_for::<T>(db_type))
            .unwrap_or_else(|e| panic!("Failed to write EXISTS subquery SQL: {}", e));

        let mut param_idx = 1;
        append_filter_clause(
            &mut sql,
            " WHERE ",
            &self.filters,
            FilterFormatter::new(db_type),
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }

    /// 生成 SQL
    pub fn to_sql(&self) -> String {
        let (sql, _) = self.to_sql_with_params(default_db_type());
        sql
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();

        // SELECT 子句
        let mut param_idx = 1;
        let mut params = Vec::new();

        let distinct_str = select_modifier_sql(
            self.distinct,
            &self.distinct_on,
            db_type,
            &mut param_idx,
            &mut params,
        );
        write!(
            &mut sql,
            "SELECT {}{} FROM {}",
            distinct_str,
            select_exprs_for_model::<T>(db_type, &self.ignored_columns, None),
            table_name_for::<T>(db_type)
        )
        .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));

        append_select_tail(
            &mut sql,
            &self.filters,
            " WHERE ",
            FilterFormatter::new(db_type),
            &self.order_by,
            self.range_start,
            self.range_end,
            self.lock,
            db_type,
            &mut param_idx,
            &mut params,
        );

        // 返回参数
        (sql, params)
    }

    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_model_filter_param_rust_types::<T>(&self.filters)
    }
}

// 实现 Default trait,支持 Select::<User>() 语法
impl<T: Model> Default for Select<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== UNION / INTERSECT / EXCEPT 功能 ====================

/// 集合操作类型
#[derive(Debug, Clone, Copy)]
pub enum SetOp {
    Union,
    UnionAll,
    Intersect,
    Except,
}

impl SetOp {
    fn as_sql(&self) -> &'static str {
        match self {
            SetOp::Union => "UNION",
            SetOp::UnionAll => "UNION ALL",
            SetOp::Intersect => "INTERSECT",
            SetOp::Except => "EXCEPT",
        }
    }
}

/// 集合操作查询结构体
///
/// 将两个 SELECT 查询通过 UNION/INTERSECT/EXCEPT 组合
///
/// # 示例
/// ```ignore
/// let combined = db.select::<User>()
///     .filter(|p| p.age.gt(30))
///     .union(
///         db.select::<User>().filter(|p| p.name.like("%admin%"))
///     )
///     .collect::<Vec<_>>().await?;
/// // 生成: SELECT ... WHERE age > 30 UNION SELECT ... WHERE name LIKE '%admin%'
/// ```
pub struct UnionSelect<T: Model> {
    left: Select<T>,
    right: Select<T>,
    op: SetOp,
}

impl<T: Model> Clone for UnionSelect<T> {
    fn clone(&self) -> Self {
        Self {
            left: self.left.clone(),
            right: self.right.clone(),
            op: self.op,
        }
    }
}

impl<T: Model> UnionSelect<T> {
    /// 生成 SQL
    pub fn to_sql(&self) -> String {
        let (sql, _) = self.to_sql_with_params(default_db_type());
        sql
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let (left_sql, mut params) = self.left.to_sql_with_params(db_type);
        let (right_sql, right_params) = self.right.to_sql_with_params(db_type);
        params.extend(right_params);

        let sql = format!("{} {} {}", left_sql, self.op.as_sql(), right_sql);
        (sql, params)
    }
}

impl<T: Model> Select<T> {
    /// UNION - 合并两个查询结果（去重）
    pub fn union(self, other: Select<T>) -> UnionSelect<T> {
        UnionSelect {
            left: self,
            right: other,
            op: SetOp::Union,
        }
    }

    /// UNION ALL - 合并两个查询结果（保留重复）
    pub fn union_all(self, other: Select<T>) -> UnionSelect<T> {
        UnionSelect {
            left: self,
            right: other,
            op: SetOp::UnionAll,
        }
    }

    /// INTERSECT - 取两个查询结果的交集
    pub fn intersect(self, other: Select<T>) -> UnionSelect<T> {
        UnionSelect {
            left: self,
            right: other,
            op: SetOp::Intersect,
        }
    }

    /// EXCEPT - 取两个查询结果的差集
    pub fn except(self, other: Select<T>) -> UnionSelect<T> {
        UnionSelect {
            left: self,
            right: other,
            op: SetOp::Except,
        }
    }
}

impl<T: Model, R: Model> RelatedSelect<T, R> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_model_filter_param_rust_types::<T>(&self.filters)
    }

    /// 添加 WHERE 条件（支持两个表的字段比较）
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where, R::Where) -> WhereExpr,
    {
        let t_where = T::Where::default();
        let r_where = R::Where::default();
        let expr = f(t_where, r_where);
        self.filters.push(expr.into());
        self
    }

    /// 添加排序
    pub fn order_by<F>(mut self, f: F) -> Self
    where
        F: FnOnce(WhereColumn<T>) -> OrderBy,
    {
        let column = WhereColumn::new();
        let order = f(column);
        self.order_by.push(order);
        self
    }

    /// 设置范围 - 支持完整范围 (start..end)、只有上限 (..end)、只有下限 (start..)
    pub fn range<RRR: Into<RangeBounds>>(mut self, range: RRR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SELECT 子句 - 只选择主表的列
        let from_tables =
            format_from_table_list(&[table_name_for::<T>(db_type), table_name_for::<R>(db_type)]);
        write!(
            &mut sql,
            "SELECT {} FROM {}",
            select_exprs_for_model::<T>(db_type, &self.ignored_columns, Some("t0")),
            from_tables
        )
        .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));

        append_select_tail(
            &mut sql,
            &self.filters,
            " WHERE ",
            FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1"),
            &self.order_by,
            self.range_start,
            self.range_end,
            None,
            db_type,
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }
}

impl<T: Model, R1: Model, R2: Model> MultiTableSelect<T, R1, R2> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_model_filter_param_rust_types::<T>(&self.filters)
    }

    /// 添加 WHERE 条件（支持三个表的字段比较）
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where) -> WhereExpr,
    {
        let t_where = T::Where::default();
        let r1_where = R1::Where::default();
        let r2_where = R2::Where::default();
        let expr = f(t_where, r1_where, r2_where);
        self.filters.push(expr.into());
        self
    }

    /// 添加排序
    pub fn order_by<F>(mut self, f: F) -> Self
    where
        F: FnOnce(WhereColumn<T>) -> OrderBy,
    {
        let column = WhereColumn::new();
        let order = f(column);
        self.order_by.push(order);
        self
    }

    /// 设置范围 - 支持完整范围 (start..end)、只有上限 (..end)、只有下限 (start..)
    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SELECT 子句 - 只选择主表的列
        let from_tables = format_from_table_list(&[
            table_name_for::<T>(db_type),
            table_name_for::<R1>(db_type),
            table_name_for::<R2>(db_type),
        ]);
        write!(
            &mut sql,
            "SELECT {} FROM {}",
            select_exprs_for_model::<T>(db_type, &self.ignored_columns, Some("t0")),
            from_tables
        )
        .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));

        append_select_tail(
            &mut sql,
            &self.filters,
            " WHERE ",
            FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1"),
            &self.order_by,
            self.range_start,
            self.range_end,
            None,
            db_type,
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }
}

impl<T: Model, R1: Model, R2: Model, R3: Model> FourTableSelect<T, R1, R2, R3> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_model_filter_param_rust_types::<T>(&self.filters)
    }

    /// 添加 WHERE 条件（支持四个表的字段比较）
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where, R3::Where) -> WhereExpr,
    {
        let t_where = T::Where::default();
        let r1_where = R1::Where::default();
        let r2_where = R2::Where::default();
        let r3_where = R3::Where::default();
        let expr = f(t_where, r1_where, r2_where, r3_where);
        self.filters.push(expr.into());
        self
    }

    /// 添加排序
    pub fn order_by<F>(mut self, f: F) -> Self
    where
        F: FnOnce(WhereColumn<T>) -> OrderBy,
    {
        let column = WhereColumn::new();
        let order = f(column);
        self.order_by.push(order);
        self
    }

    /// 设置范围 - 支持完整范围 (start..end)、只有上限 (..end)、只有下限 (start..)
    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SELECT 子句 - 只选择主表的列
        let from_tables = format_from_table_list(&[
            table_name_for::<T>(db_type),
            table_name_for::<R1>(db_type),
            table_name_for::<R2>(db_type),
            table_name_for::<R3>(db_type),
        ]);
        write!(
            &mut sql,
            "SELECT {} FROM {}",
            select_exprs_for_model::<T>(db_type, &self.ignored_columns, Some("t0")),
            from_tables
        )
        .unwrap_or_else(|e| panic!("Failed to write SQL: {}", e));

        append_select_tail(
            &mut sql,
            &self.filters,
            " WHERE ",
            FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1"),
            &self.order_by,
            self.range_start,
            self.range_end,
            None,
            db_type,
            &mut param_idx,
            &mut params,
        );

        (sql, params)
    }
}

/// WhereColumn - WHERE 条件中的列引用
///
/// 这个类型为用户提供字段访问代理，支持比较运算符
pub struct WhereColumn<T: Model> {
    _marker: PhantomData<T>,
}

impl<T: Model> WhereColumn<T> {
    fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// WhereExpr - WHERE 表达式
///
/// 支持链式调用和逻辑组合
#[derive(Clone)]
pub struct WhereExpr {
    inner: FilterExpr,
    /// LATERAL JOIN 子查询的排序条件
    join_order_by: Vec<OrderBy>,
    /// LATERAL JOIN 子查询的范围起始
    join_range_start: Option<usize>,
    /// LATERAL JOIN 子查询的范围结束
    join_range_end: Option<usize>,
}

impl From<WhereExpr> for FilterExpr {
    fn from(expr: WhereExpr) -> Self {
        expr.inner
    }
}

impl WhereExpr {
    fn defaults() -> Self {
        Self {
            inner: FilterExpr::Comparison {
                column: String::new(),
                operator: String::new(),
                value: crate::query::filter::Value::Null,
            },
            join_order_by: Vec::new(),
            join_range_start: None,
            join_range_end: None,
        }
    }

    pub fn from_filter(inner: FilterExpr) -> Self {
        Self {
            inner,
            ..Self::defaults()
        }
    }

    /// 检查是否包含 LATERAL JOIN 信息
    pub fn is_lateral(&self) -> bool {
        !self.join_order_by.is_empty()
            || self.join_range_start.is_some()
            || self.join_range_end.is_some()
    }

    /// 添加升序排序（用于 LATERAL JOIN 子查询）
    pub fn order_by(mut self, col: impl Into<OrderBy>) -> Self {
        self.join_order_by.push(col.into());
        self
    }

    /// 添加降序排序（用于 LATERAL JOIN 子查询）
    pub fn order_by_desc(mut self, col: impl Into<OrderBy>) -> Self {
        let mut order = col.into();
        order.direction = crate::query::filter::OrderDirection::Desc;
        self.join_order_by.push(order);
        self
    }

    /// 设置范围限制（用于 LATERAL JOIN 子查询）
    pub fn range(mut self, range: impl Into<RangeBounds>) -> Self {
        let bounds = range.into();
        self.join_range_start = bounds.start;
        self.join_range_end = bounds.end;
        self
    }

    pub fn and(self, other: WhereExpr) -> Self {
        Self {
            inner: FilterExpr::And(Box::new(self.inner), Box::new(other.inner)),
            join_order_by: self.join_order_by,
            join_range_start: self.join_range_start,
            join_range_end: self.join_range_end,
        }
    }

    pub fn or(self, other: WhereExpr) -> Self {
        Self {
            inner: FilterExpr::Or(Box::new(self.inner), Box::new(other.inner)),
            join_order_by: self.join_order_by,
            join_range_start: self.join_range_start,
            join_range_end: self.join_range_end,
        }
    }
}

/// 整数列代理(示例:针对 i32 类型的字段)
/// 注意:完整实现需要通过过程宏为每个模型的每个字段生成对应的代理类型
pub struct AgeColumn {
    column_name: &'static str,
}

impl AgeColumn {
    pub fn new(name: &'static str) -> Self {
        Self { column_name: name }
    }

    pub fn column_name(&self) -> &'static str {
        self.column_name
    }

    // 支持 .ge() .gt() 等方法调用
    pub fn ge(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">=".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
            ..WhereExpr::defaults()
        }
    }

    pub fn gt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
            ..WhereExpr::defaults()
        }
    }

    pub fn le(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
            ..WhereExpr::defaults()
        }
    }

    pub fn lt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
            ..WhereExpr::defaults()
        }
    }
}

/// 聚合结果类型映射 trait
pub trait AggregateResultType {
    /// 聚合函数返回的 Rust 类型
    type Output;
}

// 为不同字段类型实现 AggregateResultType
impl AggregateResultType for i32 {
    type Output = Option<i32>; // MAX/MIN 可能返回 NULL
}

impl AggregateResultType for i64 {
    type Output = Option<i64>;
}

impl AggregateResultType for f64 {
    type Output = Option<f64>;
}

impl AggregateResultType for String {
    type Output = Option<String>;
}

impl AggregateResultType for usize {
    type Output = usize;
}

// ==================== ColumnValueType Trait ====================
// 用于统一处理不同 Rust 类型到 FilterValue 的转换

/// MapToResult trait - 用于 map_to 方法的返回类型
pub trait MapToResult {
    type Output;
    fn column_names(&self) -> Vec<String>;
    fn alias_names(&self) -> Vec<Option<String>> {
        vec![None; self.column_names().len()]
    }
    fn sql_exprs(&self) -> Vec<SqlExpr> {
        self.column_names()
            .into_iter()
            .map(SqlExpr::Column)
            .collect()
    }
}

/// SelectColumnResult trait - 用于 select_column 方法的返回类型
pub trait SelectColumnResult {
    type Output;
    fn column_names(&self) -> Vec<String>;
    fn aggregate_funcs(&self) -> Vec<Option<String>>;
    fn alias_names(&self) -> Vec<Option<String>> {
        vec![None; self.column_names().len()]
    }
    fn sql_exprs(&self) -> Vec<SqlExpr> {
        self.column_names()
            .into_iter()
            .zip(self.aggregate_funcs())
            .map(|(column, aggregate)| match aggregate {
                Some(func) => SqlExpr::Aggregate {
                    name: Box::leak(func.into_boxed_str()),
                    expr: Box::new(SqlExpr::Column(column)),
                    filter: None,
                    order_by: Vec::new(),
                    over: None,
                },
                None => SqlExpr::Column(column),
            })
            .collect()
    }
}

/// GroupByColumns trait - 用于 group_by 方法的返回类型
pub trait GroupByColumns {
    fn column_names(&self) -> Vec<String>;
    fn sql_exprs(&self) -> Vec<SqlExpr> {
        self.column_names()
            .into_iter()
            .map(SqlExpr::Column)
            .collect()
    }
}

pub trait IntoGroupingSets {
    fn into_grouping_sets(self) -> Vec<Vec<SqlExpr>>;
}

impl IntoGroupingSets for Vec<Vec<SqlExpr>> {
    fn into_grouping_sets(self) -> Vec<Vec<SqlExpr>> {
        self
    }
}

impl<G, const N: usize> IntoGroupingSets for [G; N]
where
    G: GroupByColumns,
{
    fn into_grouping_sets(self) -> Vec<Vec<SqlExpr>> {
        self.into_iter().map(|group| group.sql_exprs()).collect()
    }
}

impl<A, B> IntoGroupingSets for (A, B)
where
    A: GroupByColumns,
    B: GroupByColumns,
{
    fn into_grouping_sets(self) -> Vec<Vec<SqlExpr>> {
        vec![self.0.sql_exprs(), self.1.sql_exprs()]
    }
}

pub trait ProjectionExpr {
    type Output;

    fn column_name(&self) -> String;
    fn sql_expr(&self) -> SqlExpr;
    fn alias_name(&self) -> Option<String> {
        None
    }
}

impl<T, S> ProjectionExpr for TypedColumn<T, S> {
    type Output = T;

    fn column_name(&self) -> String {
        self.column_name.to_string()
    }

    fn sql_expr(&self) -> SqlExpr {
        TypedColumn::sql_expr(self)
    }
}

impl<T, S> ProjectionExpr for TypedExpr<T, S> {
    type Output = T;

    fn column_name(&self) -> String {
        self.sql_expr().to_sql_no_params(default_db_type())
    }

    fn sql_expr(&self) -> SqlExpr {
        self.sql_expr()
    }
}

impl<E> ProjectionExpr for AliasedExpr<E>
where
    E: ProjectionExpr,
{
    type Output = E::Output;

    fn column_name(&self) -> String {
        self.alias.clone()
    }

    fn sql_expr(&self) -> SqlExpr {
        self.expr.sql_expr()
    }

    fn alias_name(&self) -> Option<String> {
        Some(self.alias.clone())
    }
}

impl<E> MapToResult for E
where
    E: ProjectionExpr,
{
    type Output = E::Output;

    fn column_names(&self) -> Vec<String> {
        vec![ProjectionExpr::column_name(self)]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![ProjectionExpr::alias_name(self)]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![ProjectionExpr::sql_expr(self)]
    }
}

impl<E> SelectColumnResult for E
where
    E: ProjectionExpr,
{
    type Output = E::Output;

    fn column_names(&self) -> Vec<String> {
        vec![ProjectionExpr::column_name(self)]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![None]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![ProjectionExpr::alias_name(self)]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![ProjectionExpr::sql_expr(self)]
    }
}

impl<E> GroupByColumns for E
where
    E: ProjectionExpr,
{
    fn column_names(&self) -> Vec<String> {
        vec![ProjectionExpr::column_name(self)]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![ProjectionExpr::sql_expr(self)]
    }
}

impl GroupByColumns for () {
    fn column_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        Vec::new()
    }
}

impl<A> GroupByColumns for (A,)
where
    A: ProjectionExpr,
{
    fn column_names(&self) -> Vec<String> {
        vec![self.0.column_name()]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr()]
    }
}

impl<A, B> MapToResult for (A, B)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
{
    type Output = (A::Output, B::Output);

    fn column_names(&self) -> Vec<String> {
        vec![self.0.column_name(), self.1.column_name()]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![self.0.alias_name(), self.1.alias_name()]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr()]
    }
}

impl<A, B> SelectColumnResult for (A, B)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
{
    type Output = (A::Output, B::Output);

    fn column_names(&self) -> Vec<String> {
        vec![self.0.column_name(), self.1.column_name()]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![None, None]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![self.0.alias_name(), self.1.alias_name()]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr()]
    }
}

impl<A, B> GroupByColumns for (A, B)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
{
    fn column_names(&self) -> Vec<String> {
        vec![self.0.column_name(), self.1.column_name()]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr()]
    }
}

impl<A, B, C> MapToResult for (A, B, C)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
    C: ProjectionExpr,
{
    type Output = (A::Output, B::Output, C::Output);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name(),
            self.1.column_name(),
            self.2.column_name(),
        ]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![
            self.0.alias_name(),
            self.1.alias_name(),
            self.2.alias_name(),
        ]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr(), self.2.sql_expr()]
    }
}

impl<A, B, C> SelectColumnResult for (A, B, C)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
    C: ProjectionExpr,
{
    type Output = (A::Output, B::Output, C::Output);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name(),
            self.1.column_name(),
            self.2.column_name(),
        ]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![None, None, None]
    }

    fn alias_names(&self) -> Vec<Option<String>> {
        vec![
            self.0.alias_name(),
            self.1.alias_name(),
            self.2.alias_name(),
        ]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr(), self.2.sql_expr()]
    }
}

impl<A, B, C> GroupByColumns for (A, B, C)
where
    A: ProjectionExpr,
    B: ProjectionExpr,
    C: ProjectionExpr,
{
    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name(),
            self.1.column_name(),
            self.2.column_name(),
        ]
    }

    fn sql_exprs(&self) -> Vec<SqlExpr> {
        vec![self.0.sql_expr(), self.1.sql_expr(), self.2.sql_expr()]
    }
}

/// 列值类型 trait - 定义 Rust 类型如何转换为 FilterValue
pub trait ColumnValueType {
    /// 将 Rust 值转换为 FilterValue
    fn to_filter_value(value: Self) -> crate::query::filter::Value;

    /// 是否支持数值比较操作（>, >=, <, <=）
    fn supports_comparison() -> bool;
}

// 为所有整数类型实现 ColumnValueType
macro_rules! impl_column_value_type_for_int {
    ($($t:ty),*) => {
        $(
            impl ColumnValueType for $t {
                fn to_filter_value(value: Self) -> crate::query::filter::Value {
                    crate::query::filter::Value::Integer(value as i64)
                }

                fn supports_comparison() -> bool {
                    true
                }
            }
        )*
    };
}

impl_column_value_type_for_int!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);

// 为浮点类型实现 ColumnValueType
macro_rules! impl_column_value_type_for_float {
    ($($t:ty),*) => {
        $(
            impl ColumnValueType for $t {
                fn to_filter_value(value: Self) -> crate::query::filter::Value {
                    crate::query::filter::Value::Real(value as f64)
                }

                fn supports_comparison() -> bool {
                    true
                }
            }
        )*
    };
}

impl_column_value_type_for_float!(f32, f64);

// 为 String 实现 ColumnValueType
impl ColumnValueType for String {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Text(value)
    }

    fn supports_comparison() -> bool {
        false // 字符串不支持数值比较
    }
}

// 为 &str 实现 ColumnValueType
impl ColumnValueType for &str {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Text(value.to_string())
    }

    fn supports_comparison() -> bool {
        false
    }
}

impl ColumnValueType for bool {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Boolean(value)
    }

    fn supports_comparison() -> bool {
        false
    }
}

// 为 chrono::NaiveDateTime 实现 ColumnValueType
impl ColumnValueType for chrono::NaiveDateTime {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::DateTime(crate::time::naive_local_to_utc(value))
    }

    fn supports_comparison() -> bool {
        true // 日期时间支持比较操作
    }
}

impl ColumnValueType for chrono::DateTime<chrono::Utc> {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::DateTime(value)
    }

    fn supports_comparison() -> bool {
        true
    }
}

impl ColumnValueType for chrono::NaiveDate {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Date(value)
    }

    fn supports_comparison() -> bool {
        true
    }
}

impl ColumnValueType for chrono::NaiveTime {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Time(value)
    }

    fn supports_comparison() -> bool {
        true
    }
}

impl ColumnValueType for std::time::Duration {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        crate::query::filter::Value::Duration(value)
    }

    fn supports_comparison() -> bool {
        true
    }
}

impl<T: crate::model::ModelEnum> ColumnValueType for T {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        if T::is_numeric_enum() {
            crate::query::filter::Value::Integer(value.as_i64())
        } else {
            crate::query::filter::Value::Text(value.name().to_string())
        }
    }

    fn supports_comparison() -> bool {
        false
    }
}

impl<T: ColumnValueType> ColumnValueType for Option<T> {
    fn to_filter_value(value: Self) -> crate::query::filter::Value {
        match value {
            Some(value) => T::to_filter_value(value),
            None => crate::query::filter::Value::Null,
        }
    }

    fn supports_comparison() -> bool {
        T::supports_comparison()
    }
}

// ==================== 统一的 IsInValue Trait ====================
// 使用泛型支持所有类型的 IN 语句

/// 用于 is_in 方法的值转换 trait（泛型版本）
pub trait IsInValue<T> {
    fn to_in_value(self) -> T;
}

// 使用统一的宏为所有数值类型实现 IsInValue
macro_rules! impl_is_in_value_for_numeric {
    ($($t:ty),* $(,)?) => {
        $(
            impl IsInValue<$t> for $t {
                fn to_in_value(self) -> $t {
                    self
                }
            }

            impl IsInValue<$t> for &$t {
                fn to_in_value(self) -> $t {
                    *self
                }
            }

            impl IsInValue<$t> for &&$t {
                fn to_in_value(self) -> $t {
                    **self
                }
            }
        )*
    };
}

// 为所有整数和浮点类型实现
impl_is_in_value_for_numeric!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize, f32, f64,);

impl<T> IsInValue<T> for T
where
    T: crate::model::ModelEnum,
{
    fn to_in_value(self) -> T {
        self
    }
}

impl<T> IsInValue<T> for &T
where
    T: crate::model::ModelEnum + Clone,
{
    fn to_in_value(self) -> T {
        self.clone()
    }
}

impl<T> IsInValue<T> for &&T
where
    T: crate::model::ModelEnum + Clone,
{
    fn to_in_value(self) -> T {
        (*self).clone()
    }
}

// 为字符串类型实现 IsInValue
impl IsInValue<String> for String {
    fn to_in_value(self) -> String {
        self
    }
}

impl IsInValue<String> for &String {
    fn to_in_value(self) -> String {
        self.clone()
    }
}

impl IsInValue<String> for &&String {
    fn to_in_value(self) -> String {
        (*self).clone()
    }
}

impl IsInValue<String> for &str {
    fn to_in_value(self) -> String {
        self.to_string()
    }
}

impl IsInValue<String> for &&str {
    fn to_in_value(self) -> String {
        (*self).to_string()
    }
}

/// IsInValues trait - 支持集合和子查询作为 is_in 的参数
pub trait IsInValues<T> {
    fn to_in_expr(self, column: String) -> WhereExpr;
}

/// IsNotInValues trait - 支持集合和子查询作为 is_not_in 的参数
pub trait IsNotInValues<T> {
    fn to_not_in_expr(self, column: String) -> WhereExpr;
}

// 为集合类型实现 IsInValues
impl<T: ColumnValueType, I, V> IsInValues<T> for I
where
    I: IntoIterator<Item = V>,
    V: IsInValue<T>,
{
    fn to_in_expr(self, column: String) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::In {
                column,
                values: self
                    .into_iter()
                    .map(|v| ColumnValueType::to_filter_value(v.to_in_value()))
                    .collect(),
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为集合类型实现 IsNotInValues
impl<T: ColumnValueType, I, V> IsNotInValues<T> for I
where
    I: IntoIterator<Item = V>,
    V: IsInValue<T>,
{
    fn to_not_in_expr(self, column: String) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::NotIn {
                column,
                values: self
                    .into_iter()
                    .map(|v| ColumnValueType::to_filter_value(v.to_in_value()))
                    .collect(),
            },
            ..WhereExpr::defaults()
        }
    }
}

/// SubqueryParam - 子查询参数包装器
pub struct SubqueryParam {
    pub sql: String,
    pub params: Vec<crate::model::Value>,
}

impl<T: ColumnValueType> IsInValues<T> for SubqueryParam {
    fn to_in_expr(self, column: String) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::InSubquery {
                column,
                subquery_sql: self.sql,
                subquery_params: self.params,
            },
            ..WhereExpr::defaults()
        }
    }
}

impl<T: ColumnValueType> IsNotInValues<T> for SubqueryParam {
    fn to_not_in_expr(self, column: String) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::NotInSubquery {
                column,
                subquery_sql: self.sql,
                subquery_params: self.params,
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为 MappedSelect 实现 IsInValues（子查询）
impl<T: Model, V: ColumnValueType> IsInValues<V> for MappedSelect<T, V> {
    fn to_in_expr(self, column: String) -> WhereExpr {
        let (sql, params) = self.to_sql_with_params(default_db_type());
        WhereExpr {
            inner: FilterExpr::InSubquery {
                column,
                subquery_sql: sql,
                subquery_params: params,
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为 MappedSelect 实现 IsNotInValues（子查询）
impl<T: Model, V: ColumnValueType> IsNotInValues<V> for MappedSelect<T, V> {
    fn to_not_in_expr(self, column: String) -> WhereExpr {
        let (sql, params) = self.to_sql_with_params(default_db_type());
        WhereExpr {
            inner: FilterExpr::NotInSubquery {
                column,
                subquery_sql: sql,
                subquery_params: params,
            },
            ..WhereExpr::defaults()
        }
    }
}

/// 类型化列代理 - 携带字段类型信息
pub struct TypedColumn<T, S = ()> {
    column_name: &'static str,
    aggregate_func: Option<&'static str>, // Some("COUNT"), Some("SUM"), etc.
    _marker: PhantomData<(T, S)>,
}

impl<T, S> Copy for TypedColumn<T, S> {}

impl<T, S> Clone for TypedColumn<T, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, S> TypedColumn<T, S> {
    pub fn new(name: &'static str) -> Self {
        Self {
            column_name: name,
            aggregate_func: None,
            _marker: PhantomData,
        }
    }

    pub fn with_aggregate(name: &'static str, func: &'static str) -> Self {
        Self {
            column_name: name,
            aggregate_func: Some(func),
            _marker: PhantomData,
        }
    }

    pub fn column_name(&self) -> &'static str {
        self.column_name
    }

    pub fn aggregate_func(&self) -> Option<&'static str> {
        self.aggregate_func
    }

    pub fn sql_expr(&self) -> SqlExpr {
        if let Some(func) = &self.aggregate_func {
            SqlExpr::Aggregate {
                name: func,
                expr: Box::new(SqlExpr::Column(self.column_name.to_string())),
                filter: None,
                order_by: Vec::new(),
                over: None,
            }
        } else {
            SqlExpr::Column(self.column_name.to_string())
        }
    }

    pub fn alias(self, alias: impl Into<String>) -> AliasedExpr<Self> {
        AliasedExpr {
            expr: self,
            alias: alias.into(),
        }
    }

    /// 创建升序排序
    pub fn asc(self) -> OrderBy {
        OrderBy::asc(self.column_name.to_string())
    }

    /// 创建降序排序
    pub fn desc(self) -> OrderBy {
        OrderBy::desc(self.column_name.to_string())
    }
}

impl<T, S> From<TypedColumn<T, S>> for OrderBy {
    fn from(col: TypedColumn<T, S>) -> Self {
        OrderBy::asc(col.column_name.to_string())
    }
}

impl<T: crate::model::FromValue, S> crate::model::FromRowValues for TypedColumn<T, S> {
    fn from_row_values(values: &[crate::model::Value]) -> crate::Result<Self> {
        if values.is_empty() {
            return Err(crate::ormer_error!(
                "Expected at least 1 value for TypedColumn"
            ));
        }

        // 从第一个值解析出实际的 T 类型
        let _parsed = T::from_value(&values[0])?;

        // 返回一个空的 TypedColumn（实际值已经被解析，这里只是为了满足类型系统）
        // 注意：这个实现主要用于让类型系统通过，实际使用时应该直接使用 T 而不是 TypedColumn<T>
        Ok(TypedColumn {
            column_name: "",
            aggregate_func: None,
            _marker: PhantomData,
        })
    }
}

// 保留 NumericColumn 作为类型别名向后兼容
pub type NumericColumn = TypedColumn<i64>;

/// 列值 - 支持字面量或列引用
pub enum ColumnValue {
    Literal(crate::query::filter::Value),
    ColumnRef(String),
}

impl<T: ColumnValueType> From<T> for ColumnValue {
    fn from(v: T) -> Self {
        ColumnValue::Literal(T::to_filter_value(v))
    }
}

impl<T, S> From<TypedColumn<T, S>> for ColumnValue {
    fn from(col: TypedColumn<T, S>) -> Self {
        ColumnValue::ColumnRef(col.column_name.to_string())
    }
}

impl<T, S> IntoSqlExpr for TypedColumn<T, S> {
    fn into_sql_expr(self) -> SqlExpr {
        self.sql_expr()
    }
}

impl<T, S> crate::query::expr::IntoTypedExpr for TypedColumn<T, S> {
    type Output = T;

    fn into_typed_expr(self) -> TypedExpr<Self::Output> {
        TypedExpr::new(self.sql_expr())
    }
}

pub trait RowValueCompare<Rhs> {
    fn eq(self, rhs: Rhs) -> WhereExpr;
    fn ne(self, rhs: Rhs) -> WhereExpr;
}

impl<A, B, RA, RB> RowValueCompare<(RA, RB)> for (A, B)
where
    A: IntoSqlExpr,
    B: IntoSqlExpr,
    RA: IntoSqlExpr,
    RB: IntoSqlExpr,
{
    fn eq(self, rhs: (RA, RB)) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprComparison {
                left: SqlExpr::Row(vec![self.0.into_sql_expr(), self.1.into_sql_expr()]),
                operator: "=".to_string(),
                right: SqlExpr::Row(vec![rhs.0.into_sql_expr(), rhs.1.into_sql_expr()]),
            },
            ..WhereExpr::defaults()
        }
    }

    fn ne(self, rhs: (RA, RB)) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprComparison {
                left: SqlExpr::Row(vec![self.0.into_sql_expr(), self.1.into_sql_expr()]),
                operator: "!=".to_string(),
                right: SqlExpr::Row(vec![rhs.0.into_sql_expr(), rhs.1.into_sql_expr()]),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl<A, B, C, RA, RB, RC> RowValueCompare<(RA, RB, RC)> for (A, B, C)
where
    A: IntoSqlExpr,
    B: IntoSqlExpr,
    C: IntoSqlExpr,
    RA: IntoSqlExpr,
    RB: IntoSqlExpr,
    RC: IntoSqlExpr,
{
    fn eq(self, rhs: (RA, RB, RC)) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprComparison {
                left: SqlExpr::Row(vec![
                    self.0.into_sql_expr(),
                    self.1.into_sql_expr(),
                    self.2.into_sql_expr(),
                ]),
                operator: "=".to_string(),
                right: SqlExpr::Row(vec![
                    rhs.0.into_sql_expr(),
                    rhs.1.into_sql_expr(),
                    rhs.2.into_sql_expr(),
                ]),
            },
            ..WhereExpr::defaults()
        }
    }

    fn ne(self, rhs: (RA, RB, RC)) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprComparison {
                left: SqlExpr::Row(vec![
                    self.0.into_sql_expr(),
                    self.1.into_sql_expr(),
                    self.2.into_sql_expr(),
                ]),
                operator: "!=".to_string(),
                right: SqlExpr::Row(vec![
                    rhs.0.into_sql_expr(),
                    rhs.1.into_sql_expr(),
                    rhs.2.into_sql_expr(),
                ]),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl<T, S> TypedColumn<T, S> {
    pub fn cast<U>(self) -> TypedExpr<U, S> {
        TypedExpr::<T, S>::new(self.sql_expr()).cast::<U>()
    }

    pub fn collate(self, collation: impl Into<String>) -> TypedExpr<T, S> {
        TypedExpr::new(self.sql_expr()).collate(collation)
    }

    pub fn json_text(self, key: impl Into<String>) -> TypedExpr<String, S> {
        TypedExpr::new(SqlExpr::JsonText {
            expr: Box::new(self.sql_expr()),
            key: key.into(),
        })
    }

    pub fn over<F>(self, f: F) -> TypedExpr<T, S>
    where
        F: FnOnce(WindowSpecBuilder) -> WindowSpecBuilder,
    {
        TypedExpr::new(self.sql_expr()).over(f)
    }
}

impl<T, S> TypedColumn<T, S>
where
    S: Model,
{
    pub fn filter<F>(self, f: F) -> TypedExpr<T, S>
    where
        F: FnOnce(S::Where) -> WhereExpr,
    {
        TypedExpr::new(self.sql_expr()).filter(f)
    }

    pub fn order_by<F, O>(self, f: F) -> TypedExpr<T, S>
    where
        F: FnOnce(S::Where) -> O,
        O: Into<OrderBy>,
    {
        TypedExpr::new(self.sql_expr()).order_by(f)
    }
}

impl<T: ColumnValueType, S> TypedExpr<T, S> {
    fn compare(self, operator: &str, value: impl IntoSqlExpr) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprComparison {
                left: self.expr,
                operator: operator.to_string(),
                right: value.into_sql_expr(),
            },
            ..WhereExpr::defaults()
        }
    }

    pub fn eq(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare("=", value)
    }

    pub fn ne(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare("!=", value)
    }

    pub fn ge(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare(">=", value)
    }

    pub fn gt(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare(">", value)
    }

    pub fn le(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare("<=", value)
    }

    pub fn lt(self, value: impl IntoSqlExpr) -> WhereExpr {
        self.compare("<", value)
    }

    pub fn between(self, min: impl IntoSqlExpr, max: impl IntoSqlExpr) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprBetween {
                expr: self.expr,
                min: min.into_sql_expr(),
                max: max.into_sql_expr(),
            },
            ..WhereExpr::defaults()
        }
    }

    pub fn is_null(self) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprIsNull { expr: self.expr },
            ..WhereExpr::defaults()
        }
    }

    pub fn is_not_null(self) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::ExprIsNotNull { expr: self.expr },
            ..WhereExpr::defaults()
        }
    }
}

impl<T, S> TypedExpr<T, S> {
    pub fn json_text(self, key: impl Into<String>) -> TypedExpr<String, S> {
        TypedExpr::new(SqlExpr::JsonText {
            expr: Box::new(self.sql_expr()),
            key: key.into(),
        })
    }

    pub fn over<F>(self, f: F) -> Self
    where
        F: FnOnce(WindowSpecBuilder) -> WindowSpecBuilder,
    {
        match self.expr {
            SqlExpr::Aggregate {
                name,
                expr,
                filter,
                order_by,
                over: _,
            } => Self::new(SqlExpr::Aggregate {
                name,
                expr,
                filter,
                order_by,
                over: Some(f(WindowSpecBuilder::default()).build()),
            }),
            expr => Self::new(SqlExpr::Raw(format!(
                "{} OVER ()",
                expr.to_sql_no_params(default_db_type())
            ))),
        }
    }
}

impl<T, S> TypedExpr<T, S>
where
    S: Model,
{
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(S::Where) -> WhereExpr,
    {
        let filter_expr = f(S::Where::default()).into();
        match self.expr {
            SqlExpr::Aggregate {
                name,
                expr,
                filter: _,
                order_by,
                over,
            } => Self::new(SqlExpr::Aggregate {
                name,
                expr,
                filter: Some(Box::new(filter_expr)),
                order_by,
                over,
            }),
            expr => Self::new(expr),
        }
    }

    pub fn order_by<F, O>(self, f: F) -> Self
    where
        F: FnOnce(S::Where) -> O,
        O: Into<OrderBy>,
    {
        let order = f(S::Where::default()).into();
        match self.expr {
            SqlExpr::Aggregate {
                name,
                expr,
                filter,
                mut order_by,
                over,
            } => {
                order_by.push(order);
                Self::new(SqlExpr::Aggregate {
                    name,
                    expr,
                    filter,
                    order_by,
                    over,
                })
            }
            expr => Self::new(expr),
        }
    }
}

// ==================== TypedColumn 泛型实现 ====================
// 为所有实现了 ColumnValueType 的类型提供统一的方法

impl<T: ColumnValueType, S> TypedColumn<T, S> {
    /// 等于比较 - 支持字面量或列引用
    pub fn eq(self, value: impl Into<ColumnValue>) -> WhereExpr {
        match value.into() {
            ColumnValue::Literal(v) => WhereExpr {
                inner: FilterExpr::Comparison {
                    column: self.column_name.to_string(),
                    operator: "=".to_string(),
                    value: v,
                },
                ..WhereExpr::defaults()
            },
            ColumnValue::ColumnRef(other_column) => WhereExpr {
                inner: FilterExpr::ColumnComparison {
                    left_column: self.column_name.to_string(),
                    operator: "=".to_string(),
                    right_column: other_column,
                },
                ..WhereExpr::defaults()
            },
        }
    }

    /// IN 语句 - 支持多种集合类型和子查询
    pub fn is_in(self, values: impl IsInValues<T>) -> WhereExpr {
        values.to_in_expr(self.column_name.to_string())
    }

    /// NOT IN 语句 - 支持多种集合类型和子查询
    pub fn is_not_in(self, values: impl IsNotInValues<T>) -> WhereExpr {
        values.to_not_in_expr(self.column_name.to_string())
    }

    /// IS NULL 判断
    pub fn is_null(self) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::IsNull {
                column: self.column_name.to_string(),
            },
            ..WhereExpr::defaults()
        }
    }

    /// IS NOT NULL 判断
    pub fn is_not_null(self) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::IsNotNull {
                column: self.column_name.to_string(),
            },
            ..WhereExpr::defaults()
        }
    }

    /// 不等于比较 - 支持字面量或列引用
    pub fn ne(self, value: impl Into<ColumnValue>) -> WhereExpr {
        match value.into() {
            ColumnValue::Literal(v) => WhereExpr {
                inner: FilterExpr::Comparison {
                    column: self.column_name.to_string(),
                    operator: "!=".to_string(),
                    value: v,
                },
                ..WhereExpr::defaults()
            },
            ColumnValue::ColumnRef(other_column) => WhereExpr {
                inner: FilterExpr::ColumnComparison {
                    left_column: self.column_name.to_string(),
                    operator: "!=".to_string(),
                    right_column: other_column,
                },
                ..WhereExpr::defaults()
            },
        }
    }
}

// 为支持比较操作的类型（整数和浮点数）实现比较方法
impl<T: ColumnValueType, S> TypedColumn<T, S> {
    /// 大于等于
    pub fn ge(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        let column_name = if let Some(ref func) = self.aggregate_func {
            format!("{}({})", func, self.column_name)
        } else {
            self.column_name.to_string()
        };
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: column_name,
                operator: ">=".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
            ..WhereExpr::defaults()
        }
    }

    /// 大于
    pub fn gt(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        let column_name = if let Some(ref func) = self.aggregate_func {
            format!("{}({})", func, self.column_name)
        } else {
            self.column_name.to_string()
        };
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: column_name,
                operator: ">".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
            ..WhereExpr::defaults()
        }
    }

    /// 小于等于
    pub fn le(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        let column_name = if let Some(ref func) = self.aggregate_func {
            format!("{}({})", func, self.column_name)
        } else {
            self.column_name.to_string()
        };
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: column_name,
                operator: "<=".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
            ..WhereExpr::defaults()
        }
    }

    /// 小于
    pub fn lt(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        let column_name = if let Some(ref func) = self.aggregate_func {
            format!("{}({})", func, self.column_name)
        } else {
            self.column_name.to_string()
        };
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: column_name,
                operator: "<".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
            ..WhereExpr::defaults()
        }
    }

    /// BETWEEN 范围查询
    ///
    /// ```text
    /// .filter(|p| p.age.between(18, 30))
    /// ```
    pub fn between(self, min: T, max: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        let column_name = if let Some(ref func) = self.aggregate_func {
            format!("{}({})", func, self.column_name)
        } else {
            self.column_name.to_string()
        };
        WhereExpr {
            inner: FilterExpr::Between {
                column: column_name,
                min: ColumnValueType::to_filter_value(min),
                max: ColumnValueType::to_filter_value(max),
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为 TypedColumn<String> 实现字符串模糊查询方法
impl<S> TypedColumn<String, S> {
    /// LIKE 模糊查询 - 直接使用 SQL LIKE 模式
    ///
    /// ```text
    /// .filter(|p| p.name.like("%alice%"))
    /// ```
    pub fn like(self, pattern: &str) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "LIKE".to_string(),
                value: crate::query::filter::Value::Text(pattern.to_string()),
            },
            ..WhereExpr::defaults()
        }
    }

    /// 包含子串 - 等价于 LIKE '%pattern%'
    ///
    /// ```text
    /// .filter(|p| p.name.contains("alice"))
    /// ```
    pub fn contains(self, pattern: &str) -> WhereExpr {
        self.like(&format!("%{}%", pattern))
    }

    /// 前缀匹配 - 等价于 LIKE 'pattern%'
    ///
    /// ```text
    /// .filter(|p| p.name.starts_with("al"))
    /// ```
    pub fn starts_with(self, pattern: &str) -> WhereExpr {
        self.like(&format!("{}%", pattern))
    }

    /// 后缀匹配 - 等价于 LIKE '%pattern'
    ///
    /// ```text
    /// .filter(|p| p.name.ends_with("ce"))
    /// ```
    pub fn ends_with(self, pattern: &str) -> WhereExpr {
        self.like(&format!("%{}", pattern))
    }

    pub fn to_lower(self) -> TypedExpr<String, S> {
        TypedExpr::new(SqlExpr::Function {
            name: "LOWER",
            args: vec![self.sql_expr()],
        })
    }

    pub fn matches_text(self, query: impl Into<String>) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::TextSearch {
                expr: self.sql_expr(),
                query: query.into(),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl<S> TypedExpr<String, S> {
    pub fn to_lower(self) -> TypedExpr<String, S> {
        TypedExpr::new(SqlExpr::Function {
            name: "LOWER",
            args: vec![self.sql_expr()],
        })
    }

    pub fn like(self, pattern: &str) -> WhereExpr {
        self.compare("LIKE", pattern)
    }

    pub fn contains(self, pattern: &str) -> WhereExpr {
        self.like(&format!("%{}%", pattern))
    }

    pub fn starts_with(self, pattern: &str) -> WhereExpr {
        self.like(&format!("{}%", pattern))
    }

    pub fn ends_with(self, pattern: &str) -> WhereExpr {
        self.like(&format!("%{}", pattern))
    }

    pub fn matches_text(self, query: impl Into<String>) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::TextSearch {
                expr: self.sql_expr(),
                query: query.into(),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl<S> TypedColumn<Vec<String>, S> {
    /// PostgreSQL array membership, generated as `column @> ARRAY[value]`.
    pub fn contains(self, value: impl Into<String>) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "@>".to_string(),
                value: crate::query::filter::Value::Text(value.into()),
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为所有 TypedColumn 实现聚合方法
impl<T: ColumnValueType + 'static, S> TypedColumn<T, S> {
    /// COUNT 聚合 - 返回 usize
    pub fn count(self) -> TypedColumn<usize, S> {
        TypedColumn::with_aggregate(self.column_name, "COUNT")
    }

    /// SUM 聚合 - 返回相同类型
    pub fn sum(self) -> TypedColumn<T, S>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "SUM")
    }

    /// AVG 聚合 - 返回 f64
    pub fn avg(self) -> TypedColumn<f64, S> {
        TypedColumn::with_aggregate(self.column_name, "AVG")
    }

    /// MAX 聚合 - 返回相同类型
    pub fn max(self) -> TypedColumn<T, S>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "MAX")
    }

    /// MIN 聚合 - 返回相同类型
    pub fn min(self) -> TypedColumn<T, S>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "MIN")
    }

    pub fn array_agg(self) -> TypedExpr<Vec<T>, S> {
        TypedExpr::new(SqlExpr::Aggregate {
            name: "ARRAY_AGG",
            expr: Box::new(SqlExpr::Column(self.column_name.to_string())),
            filter: None,
            order_by: Vec::new(),
            over: None,
        })
    }
}

macro_rules! impl_binary_expr_op {
    ($trait:ident, $method:ident, $op:literal) => {
        impl<T, S, R> $trait<R> for TypedColumn<T, S>
        where
            R: IntoSqlExpr,
        {
            type Output = TypedExpr<T, S>;

            fn $method(self, rhs: R) -> Self::Output {
                TypedExpr::new(SqlExpr::Binary {
                    left: Box::new(self.sql_expr()),
                    op: $op,
                    right: Box::new(rhs.into_sql_expr()),
                })
            }
        }

        impl<T, S, R> $trait<R> for TypedExpr<T, S>
        where
            R: IntoSqlExpr,
        {
            type Output = TypedExpr<T, S>;

            fn $method(self, rhs: R) -> Self::Output {
                TypedExpr::new(SqlExpr::Binary {
                    left: Box::new(self.sql_expr()),
                    op: $op,
                    right: Box::new(rhs.into_sql_expr()),
                })
            }
        }
    };
}

impl_binary_expr_op!(Add, add, "+");
impl_binary_expr_op!(Sub, sub, "-");
impl_binary_expr_op!(Mul, mul, "*");
impl_binary_expr_op!(Div, div, "/");

// 关键设计:ColumnProxy 类型
// 当 p.age 被访问时,返回这个代理对象
// 代理对象实现了比较运算符的重载,记录比较操作
pub struct ColumnProxy {
    column_name: String,
}

impl ColumnProxy {
    pub fn new(name: &str) -> Self {
        Self {
            column_name: name.to_string(),
        }
    }
}

// 实现运算符重载 - 这些方法在运算符被使用时调用
// 关键:我们让它们返回 WhereExpr 而不是 bool
impl std::ops::BitOr<i32> for ColumnProxy {
    type Output = WhereExpr;

    fn bitor(self, rhs: i32) -> WhereExpr {
        // 使用 | 运算符表示 >=
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name,
                operator: ">=".to_string(),
                value: crate::query::filter::Value::Integer(rhs as i64),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl std::ops::Shr<i32> for ColumnProxy {
    type Output = WhereExpr;

    fn shr(self, rhs: i32) -> WhereExpr {
        // 使用 >> 运算符表示 >
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name,
                operator: ">".to_string(),
                value: crate::query::filter::Value::Integer(rhs as i64),
            },
            ..WhereExpr::defaults()
        }
    }
}

impl std::ops::Shl<i32> for ColumnProxy {
    type Output = WhereExpr;

    fn shl(self, rhs: i32) -> WhereExpr {
        // 使用 << 运算符表示 <
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name,
                operator: "<".to_string(),
                value: crate::query::filter::Value::Integer(rhs as i64),
            },
            ..WhereExpr::defaults()
        }
    }
}

// 为特定模型实现 WhereColumn 的字段访问
// 注意：在完整实现中，这应该由过程宏自动生成

/// 列构建器，用于构建过滤表达式
pub trait ColumnBuilder {
    type Output;

    fn gt(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn ge(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn lt(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn le(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn eq(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn ne(self, value: impl Into<FilterValue>) -> FilterExpr;
    fn like(self, pattern: &str) -> FilterExpr;
    fn contains(self, pattern: &str) -> FilterExpr;
    fn starts_with(self, pattern: &str) -> FilterExpr;
    fn ends_with(self, pattern: &str) -> FilterExpr;
    fn into_some(self) -> FilterExpr;
    fn into_none(self) -> FilterExpr;
    fn asc(self) -> OrderBy;
    fn desc(self) -> OrderBy;
}

/// 过滤值
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FilterValue {
    inner: crate::query::filter::Value,
}

impl From<i32> for FilterValue {
    fn from(v: i32) -> Self {
        Self {
            inner: crate::query::filter::Value::Integer(v as i64),
        }
    }
}

impl From<i64> for FilterValue {
    fn from(v: i64) -> Self {
        Self {
            inner: crate::query::filter::Value::Integer(v),
        }
    }
}

impl From<String> for FilterValue {
    fn from(v: String) -> Self {
        Self {
            inner: crate::query::filter::Value::Text(v),
        }
    }
}

impl From<&str> for FilterValue {
    fn from(v: &str) -> Self {
        Self {
            inner: crate::query::filter::Value::Text(v.to_string()),
        }
    }
}

// ==================== JOIN 功能 ====================

/// LEFT JOIN 查询结构体
#[allow(dead_code)]
pub struct LeftJoinedSelect<T: Model, J: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    /// 是否为 LATERAL JOIN
    lateral: bool,
    /// LATERAL JOIN 子查询的排序条件
    join_order_by: Vec<OrderBy>,
    /// LATERAL JOIN 子查询的范围起始
    join_range_start: Option<usize>,
    /// LATERAL JOIN 子查询的范围结束
    join_range_end: Option<usize>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for LeftJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns.clone(),
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
            lateral: self.lateral,
            join_order_by: self.join_order_by.clone(),
            join_range_start: self.join_range_start,
            join_range_end: self.join_range_end,
            _marker: PhantomData,
        }
    }
}

/// INNER JOIN 查询结构体
#[allow(dead_code)]
pub struct InnerJoinedSelect<T: Model, J: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    /// 是否为 LATERAL JOIN
    lateral: bool,
    /// LATERAL JOIN 子查询的排序条件
    join_order_by: Vec<OrderBy>,
    /// LATERAL JOIN 子查询的范围起始
    join_range_start: Option<usize>,
    /// LATERAL JOIN 子查询的范围结束
    join_range_end: Option<usize>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for InnerJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns.clone(),
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
            lateral: self.lateral,
            join_order_by: self.join_order_by.clone(),
            join_range_start: self.join_range_start,
            join_range_end: self.join_range_end,
            _marker: PhantomData,
        }
    }
}

/// RIGHT JOIN 查询结构体
#[allow(dead_code)]
pub struct RightJoinedSelect<T: Model, J: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    ignored_columns: Vec<String>,
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    /// 是否为 LATERAL JOIN
    lateral: bool,
    /// LATERAL JOIN 子查询的排序条件
    join_order_by: Vec<OrderBy>,
    /// LATERAL JOIN 子查询的范围起始
    join_range_start: Option<usize>,
    /// LATERAL JOIN 子查询的范围结束
    join_range_end: Option<usize>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for RightJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns.clone(),
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
            lateral: self.lateral,
            join_order_by: self.join_order_by.clone(),
            join_range_start: self.join_range_start,
            join_range_end: self.join_range_end,
            _marker: PhantomData,
        }
    }
}

impl<T: Model> Select<T> {
    /// LEFT JOIN
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelect<T, J> {
        let t_where = T::Where::default();
        let j_where = J::Where::default();
        let expr = f(t_where, j_where);

        let lateral = expr.is_lateral();
        let join_order_by = expr.join_order_by.clone();
        let join_range_start = expr.join_range_start;
        let join_range_end = expr.join_range_end;

        LeftJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
            lateral,
            join_order_by,
            join_range_start,
            join_range_end,
            _marker: PhantomData,
        }
    }

    /// INNER JOIN
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelect<T, J> {
        let t_where = T::Where::default();
        let j_where = J::Where::default();
        let expr = f(t_where, j_where);

        let lateral = expr.is_lateral();
        let join_order_by = expr.join_order_by.clone();
        let join_range_start = expr.join_range_start;
        let join_range_end = expr.join_range_end;

        InnerJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
            lateral,
            join_order_by,
            join_range_start,
            join_range_end,
            _marker: PhantomData,
        }
    }

    /// RIGHT JOIN
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelect<T, J> {
        let t_where = T::Where::default();
        let j_where = J::Where::default();
        let expr = f(t_where, j_where);

        let lateral = expr.is_lateral();
        let join_order_by = expr.join_order_by.clone();
        let join_range_start = expr.join_range_start;
        let join_range_end = expr.join_range_end;

        RightJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            ignored_columns: self.ignored_columns,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
            lateral,
            join_order_by,
            join_range_start,
            join_range_end,
            _marker: PhantomData,
        }
    }
}

impl<T: Model, J: Model> LeftJoinedSelect<T, J> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_join_filter_param_rust_types::<T>(self.lateral, &self.on_condition, &self.filters)
    }

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        join_sql_with_params::<T, J>(
            db_type,
            JoinKind::Left,
            self.lateral,
            JoinSqlParts {
                filters: &self.filters,
                range_start: self.range_start,
                range_end: self.range_end,
                ignored_columns: &self.ignored_columns,
                join_table: &self.join_table,
                join_alias: &self.join_alias,
                on_condition: &self.on_condition,
                join_order_by: &self.join_order_by,
                join_range_start: self.join_range_start,
                join_range_end: self.join_range_end,
            },
        )
    }
}

impl<T: Model, J: Model> InnerJoinedSelect<T, J> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_join_filter_param_rust_types::<T>(self.lateral, &self.on_condition, &self.filters)
    }

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        join_sql_with_params::<T, J>(
            db_type,
            JoinKind::Inner,
            self.lateral,
            JoinSqlParts {
                filters: &self.filters,
                range_start: self.range_start,
                range_end: self.range_end,
                ignored_columns: &self.ignored_columns,
                join_table: &self.join_table,
                join_alias: &self.join_alias,
                on_condition: &self.on_condition,
                join_order_by: &self.join_order_by,
                join_range_start: self.join_range_start,
                join_range_end: self.join_range_end,
            },
        )
    }
}

impl<T: Model, J: Model> RightJoinedSelect<T, J> {
    #[cfg(feature = "postgresql")]
    pub(crate) fn param_rust_types(&self) -> Vec<&'static str> {
        collect_join_filter_param_rust_types::<T>(self.lateral, &self.on_condition, &self.filters)
    }

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    pub fn range<RR: Into<RangeBounds>>(mut self, range: RR) -> Self {
        let bounds = range.into();
        self.range_start = bounds.start;
        self.range_end = bounds.end;
        self
    }

    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        join_sql_with_params::<T, J>(
            db_type,
            JoinKind::Right,
            self.lateral,
            JoinSqlParts {
                filters: &self.filters,
                range_start: self.range_start,
                range_end: self.range_end,
                ignored_columns: &self.ignored_columns,
                join_table: &self.join_table,
                join_alias: &self.join_alias,
                on_condition: &self.on_condition,
                join_order_by: &self.join_order_by,
                join_range_start: self.join_range_start,
                join_range_end: self.join_range_end,
            },
        )
    }
}
