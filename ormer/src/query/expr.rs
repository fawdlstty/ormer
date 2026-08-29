use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, quote_column_reference, quote_identifier, quote_sql_literal};
use std::marker::PhantomData;

#[derive(Debug, Clone)]
pub enum SqlExpr {
    Column(String),
    Value(Value),
    Binary {
        left: Box<SqlExpr>,
        op: &'static str,
        right: Box<SqlExpr>,
    },
    Function {
        name: &'static str,
        args: Vec<SqlExpr>,
    },
    Cast {
        expr: Box<SqlExpr>,
        sql_type: &'static str,
    },
    Collate {
        expr: Box<SqlExpr>,
        collation: String,
    },
    Aggregate {
        name: &'static str,
        expr: Box<SqlExpr>,
        filter: Option<Box<crate::query::filter::FilterExpr>>,
        order_by: Vec<crate::query::filter::OrderBy>,
        over: Option<WindowSpec>,
    },
    CaseMatch {
        expr: Box<SqlExpr>,
        branches: Vec<(SqlExpr, SqlExpr)>,
        else_expr: Box<SqlExpr>,
    },
    JsonText {
        expr: Box<SqlExpr>,
        key: String,
    },
    JsonPathText {
        expr: Box<SqlExpr>,
        path: Vec<String>,
    },
    JsonPathValue {
        expr: Box<SqlExpr>,
        path: Vec<String>,
        value_type: JsonScalarKind,
    },
    JsonPathExists {
        expr: Box<SqlExpr>,
        path: Vec<String>,
    },
    JsonContains {
        left: Box<SqlExpr>,
        right: Box<SqlExpr>,
    },
    JsonSet {
        expr: Box<SqlExpr>,
        path: Vec<String>,
        value: Box<SqlExpr>,
    },
    JsonRemove {
        expr: Box<SqlExpr>,
        path: Vec<String>,
    },
    ArrayContains {
        left: Box<SqlExpr>,
        right: Box<SqlExpr>,
    },
    ArrayOverlaps {
        left: Box<SqlExpr>,
        right: Box<SqlExpr>,
    },
    ArrayLen {
        expr: Box<SqlExpr>,
    },
    WindowFunction {
        function: &'static str,
        args: Vec<SqlExpr>,
        over: WindowSpec,
    },
    DateTrunc {
        expr: Box<SqlExpr>,
        unit: TimeUnit,
    },
    DatePart {
        expr: Box<SqlExpr>,
        part: TimePart,
    },
    AtTimeZone {
        expr: Box<SqlExpr>,
        timezone: String,
    },
    DateAdd {
        expr: Box<SqlExpr>,
        unit: TimeUnit,
        amount: Box<SqlExpr>,
        negative: bool,
    },
    DateDiff {
        left: Box<SqlExpr>,
        right: Box<SqlExpr>,
        part: TimePart,
    },
    Now,
    Row(Vec<SqlExpr>),
    Raw(RawSqlExpr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePart {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl TimeUnit {
    fn pg_name(self) -> &'static str {
        match self {
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    fn clickhouse_interval(self) -> &'static str {
        match self {
            Self::Second => "toIntervalSecond",
            Self::Minute => "toIntervalMinute",
            Self::Hour => "toIntervalHour",
            Self::Day => "toIntervalDay",
            Self::Week => "toIntervalWeek",
            Self::Month => "toIntervalMonth",
            Self::Year => "toIntervalYear",
        }
    }

    fn sqlite_format(self) -> &'static str {
        match self {
            Self::Second => "%Y-%m-%d %H:%M:%S",
            Self::Minute => "%Y-%m-%d %H:%M:00",
            Self::Hour => "%Y-%m-%d %H:00:00",
            Self::Day => "%Y-%m-%d 00:00:00",
            Self::Week => "%Y-%m-%d 00:00:00",
            Self::Month => "%Y-%m-01 00:00:00",
            Self::Year => "%Y-01-01 00:00:00",
        }
    }
}

impl TimePart {
    fn name(self) -> &'static str {
        match self {
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    fn name_upper(self) -> &'static str {
        match self {
            Self::Second => "SECOND",
            Self::Minute => "MINUTE",
            Self::Hour => "HOUR",
            Self::Day => "DAY",
            Self::Week => "WEEK",
            Self::Month => "MONTH",
            Self::Year => "YEAR",
        }
    }

    fn sqlite_format(self) -> &'static str {
        match self {
            Self::Second => "%S",
            Self::Minute => "%M",
            Self::Hour => "%H",
            Self::Day => "%d",
            Self::Week => "%W",
            Self::Month => "%m",
            Self::Year => "%Y",
        }
    }

    fn epoch_divisor(self) -> f64 {
        match self {
            Self::Second => 1.0,
            Self::Minute => 60.0,
            Self::Hour => 3600.0,
            Self::Day => 86400.0,
            Self::Week => 604800.0,
            Self::Month => 2629746.0,
            Self::Year => 31556952.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RawSqlExpr {
    segments: Vec<RawExprSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonScalarKind {
    Boolean,
    Integer,
    Real,
    String,
    Json,
}

impl JsonScalarKind {
    fn sql_type(self) -> &'static str {
        match self {
            Self::Boolean => "BOOLEAN",
            Self::Integer => "BIGINT",
            Self::Real => "DOUBLE PRECISION",
            Self::String => "TEXT",
            Self::Json => "JSONB",
        }
    }

}

#[derive(Debug, Clone)]
pub enum RawExprSegment {
    Text(String),
    Expr(SqlExpr),
}

#[derive(Debug, Clone, Default)]
pub struct WindowSpec {
    pub partition_by: Vec<SqlExpr>,
    pub order_by: Vec<crate::query::filter::OrderBy>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowSpecBuilder {
    spec: WindowSpec,
}

impl WindowSpecBuilder {
    pub fn partition_by<E>(mut self, expr: E) -> Self
    where
        E: IntoSqlExpr,
    {
        self.spec.partition_by.push(expr.into_sql_expr());
        self
    }

    pub fn order_by<O>(mut self, order: O) -> Self
    where
        O: Into<crate::query::filter::OrderBy>,
    {
        self.spec.order_by.push(order.into());
        self
    }

    pub fn build(self) -> WindowSpec {
        self.spec
    }
}

pub trait IntoSqlExpr {
    fn into_sql_expr(self) -> SqlExpr;
}

pub trait IntoTypedExpr {
    type Output;

    fn into_typed_expr(self) -> TypedExpr<Self::Output>;
}

#[derive(Debug)]
pub struct TypedExpr<T, S = ()> {
    pub(crate) expr: SqlExpr,
    _marker: PhantomData<(T, S)>,
}

#[derive(Debug, Clone)]
pub struct NowExpr;

#[derive(Debug, Clone, Copy)]
pub struct IntervalExpr {
    amount: i64,
    unit: TimeUnit,
}

impl std::ops::Add<IntervalExpr> for NowExpr {
    type Output = TypedExpr<chrono::NaiveDateTime>;

    fn add(self, rhs: IntervalExpr) -> Self::Output {
        TypedExpr::new(SqlExpr::DateAdd {
            expr: Box::new(SqlExpr::Now),
            unit: rhs.unit,
            amount: Box::new(SqlExpr::Value(Value::BigInt(rhs.amount.into()))),
            negative: false,
        })
    }
}

impl std::ops::Sub<IntervalExpr> for NowExpr {
    type Output = TypedExpr<chrono::NaiveDateTime>;

    fn sub(self, rhs: IntervalExpr) -> Self::Output {
        TypedExpr::new(SqlExpr::DateAdd {
            expr: Box::new(SqlExpr::Now),
            unit: rhs.unit,
            amount: Box::new(SqlExpr::Value(Value::BigInt(rhs.amount.into()))),
            negative: true,
        })
    }
}

pub fn now() -> NowExpr {
    NowExpr
}

pub fn days(amount: i64) -> IntervalExpr {
    IntervalExpr { amount, unit: TimeUnit::Day }
}

pub fn hours(amount: i64) -> IntervalExpr {
    IntervalExpr { amount, unit: TimeUnit::Hour }
}

pub fn minutes(amount: i64) -> IntervalExpr {
    IntervalExpr { amount, unit: TimeUnit::Minute }
}

pub fn seconds(amount: i64) -> IntervalExpr {
    IntervalExpr { amount, unit: TimeUnit::Second }
}

impl<T, S> Clone for TypedExpr<T, S> {
    fn clone(&self) -> Self {
        Self {
            expr: self.expr.clone(),
            _marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AliasedExpr<E> {
    pub(crate) expr: E,
    pub(crate) alias: String,
}

#[derive(Debug, Clone)]
pub struct RawExpr<T = ()> {
    expr: RawSqlExpr,
    _marker: PhantomData<T>,
}

impl RawSqlExpr {
    pub fn new(segments: Vec<RawExprSegment>) -> Self {
        Self { segments }
    }

    pub fn plain(sql: impl Into<String>) -> Self {
        Self {
            segments: vec![RawExprSegment::Text(sql.into())],
        }
    }

    #[cfg(feature = "postgresql")]
    pub(crate) fn segments(&self) -> &[RawExprSegment] {
        &self.segments
    }

    fn to_sql(
        &self,
        db_type: DbType,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
        table_prefix: Option<&str>,
    ) -> String {
        let mut sql = String::new();
        for segment in &self.segments {
            match segment {
                RawExprSegment::Text(text) => sql.push_str(text),
                RawExprSegment::Expr(expr) => {
                    sql.push_str(&expr.to_sql(db_type, param_idx, params, table_prefix));
                }
            }
        }
        sql
    }
}

impl RawExprSegment {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn expr(expr: impl IntoSqlExpr) -> Self {
        Self::Expr(expr.into_sql_expr())
    }
}

impl<T> RawExpr<T> {
    pub fn new(expr: RawSqlExpr) -> Self {
        Self {
            expr,
            _marker: PhantomData,
        }
    }

    pub fn sql_expr(&self) -> SqlExpr {
        SqlExpr::Raw(self.expr.clone())
    }

    pub fn typed<U>(self) -> RawExpr<U> {
        RawExpr {
            expr: self.expr,
            _marker: PhantomData,
        }
    }

    pub fn alias(self, alias: impl Into<String>) -> AliasedExpr<Self> {
        AliasedExpr {
            expr: self,
            alias: alias.into(),
        }
    }
}

impl<T, S> TypedExpr<T, S> {
    pub fn new(expr: SqlExpr) -> Self {
        Self {
            expr,
            _marker: PhantomData,
        }
    }

    pub fn sql_expr(&self) -> SqlExpr {
        self.expr.clone()
    }

    pub fn asc(self) -> crate::query::filter::OrderBy {
        crate::query::filter::OrderBy::asc_expr(self.expr)
    }

    pub fn desc(self) -> crate::query::filter::OrderBy {
        crate::query::filter::OrderBy::desc_expr(self.expr)
    }

    pub fn cast<U>(self) -> TypedExpr<U, S> {
        TypedExpr::new(SqlExpr::Cast {
            expr: Box::new(self.expr),
            sql_type: rust_type_to_sql::<U>(),
        })
    }

    pub fn collate(self, collation: impl Into<String>) -> Self {
        Self::new(SqlExpr::Collate {
            expr: Box::new(self.expr),
            collation: collation.into(),
        })
    }

    pub fn alias(self, alias: impl Into<String>) -> AliasedExpr<Self> {
        AliasedExpr {
            expr: self,
            alias: alias.into(),
        }
    }
}

impl<T, S> IntoSqlExpr for TypedExpr<T, S> {
    fn into_sql_expr(self) -> SqlExpr {
        self.expr
    }
}

impl IntoSqlExpr for SqlExpr {
    fn into_sql_expr(self) -> SqlExpr {
        self
    }
}

impl IntoSqlExpr for NowExpr {
    fn into_sql_expr(self) -> SqlExpr {
        SqlExpr::Now
    }
}

impl IntoSqlExpr for IntervalExpr {
    fn into_sql_expr(self) -> SqlExpr {
        SqlExpr::Value(Value::BigInt(self.amount.into()))
    }
}

impl<T> IntoSqlExpr for &T
where
    T: IntoSqlExpr + Clone,
{
    fn into_sql_expr(self) -> SqlExpr {
        self.clone().into_sql_expr()
    }
}

impl<T> IntoSqlExpr for RawExpr<T> {
    fn into_sql_expr(self) -> SqlExpr {
        SqlExpr::Raw(self.expr)
    }
}

impl<T, S> IntoTypedExpr for TypedExpr<T, S> {
    type Output = T;

    fn into_typed_expr(self) -> TypedExpr<Self::Output> {
        TypedExpr::new(self.expr)
    }
}

impl<T> IntoTypedExpr for RawExpr<T> {
    type Output = T;

    fn into_typed_expr(self) -> TypedExpr<Self::Output> {
        TypedExpr::new(self.into_sql_expr())
    }
}

macro_rules! impl_integer_expr {
    ($ty:ty) => {
        impl IntoSqlExpr for $ty {
            fn into_sql_expr(self) -> SqlExpr {
                SqlExpr::Value(Value::Integer(self as i64))
            }
        }

        impl IntoTypedExpr for $ty {
            type Output = $ty;

            fn into_typed_expr(self) -> TypedExpr<Self::Output> {
                TypedExpr::new(SqlExpr::Value(Value::Integer(self as i64)))
            }
        }
    };
}

impl_integer_expr!(i8);
impl_integer_expr!(i16);
impl_integer_expr!(i32);
impl_integer_expr!(i64);
impl_integer_expr!(u8);
impl_integer_expr!(u16);
impl_integer_expr!(u32);
impl_integer_expr!(u64);
impl_integer_expr!(isize);
impl_integer_expr!(usize);

macro_rules! impl_float_expr {
    ($ty:ty) => {
        impl IntoSqlExpr for $ty {
            fn into_sql_expr(self) -> SqlExpr {
                SqlExpr::Value(Value::Real(self as f64))
            }
        }

        impl IntoTypedExpr for $ty {
            type Output = $ty;

            fn into_typed_expr(self) -> TypedExpr<Self::Output> {
                TypedExpr::new(SqlExpr::Value(Value::Real(self as f64)))
            }
        }
    };
}

impl_float_expr!(f32);
impl_float_expr!(f64);

macro_rules! impl_model_value_expr {
    ($ty:ty, $out:ty) => {
        impl IntoSqlExpr for $ty {
            fn into_sql_expr(self) -> SqlExpr {
                SqlExpr::Value(Value::from(self))
            }
        }

        impl IntoTypedExpr for $ty {
            type Output = $out;

            fn into_typed_expr(self) -> TypedExpr<Self::Output> {
                TypedExpr::new(SqlExpr::Value(Value::from(self)))
            }
        }
    };
}

impl_model_value_expr!(bool, bool);
impl_model_value_expr!(String, String);
impl_model_value_expr!(&str, String);
impl_model_value_expr!(std::time::Duration, std::time::Duration);
impl_model_value_expr!(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>);
impl_model_value_expr!(chrono::NaiveDateTime, chrono::NaiveDateTime);
impl_model_value_expr!(chrono::NaiveDate, chrono::NaiveDate);
impl_model_value_expr!(chrono::NaiveTime, chrono::NaiveTime);
impl_model_value_expr!(serde_json::Value, serde_json::Value);
impl_model_value_expr!(uuid::Uuid, uuid::Uuid);

pub fn value<T>(value: T) -> TypedExpr<<T as IntoTypedExpr>::Output>
where
    T: IntoTypedExpr,
{
    value.into_typed_expr()
}

pub fn raw<T>(sql: impl Into<String>) -> TypedExpr<T> {
    TypedExpr::new(SqlExpr::Raw(RawSqlExpr::plain(sql)))
}

pub fn row<E>(exprs: E) -> SqlExpr
where
    E: IntoRowExpr,
{
    SqlExpr::Row(exprs.into_row_expr())
}

pub trait IntoRowExpr {
    fn into_row_expr(self) -> Vec<SqlExpr>;
}

impl<A, B> IntoRowExpr for (A, B)
where
    A: IntoSqlExpr,
    B: IntoSqlExpr,
{
    fn into_row_expr(self) -> Vec<SqlExpr> {
        vec![self.0.into_sql_expr(), self.1.into_sql_expr()]
    }
}

impl<A, B, C> IntoRowExpr for (A, B, C)
where
    A: IntoSqlExpr,
    B: IntoSqlExpr,
    C: IntoSqlExpr,
{
    fn into_row_expr(self) -> Vec<SqlExpr> {
        vec![
            self.0.into_sql_expr(),
            self.1.into_sql_expr(),
            self.2.into_sql_expr(),
        ]
    }
}

pub struct CaseMatchBuilder {
    expr: SqlExpr,
    branches: Vec<(SqlExpr, SqlExpr)>,
}

impl CaseMatchBuilder {
    pub fn when<M, R>(mut self, match_value: M, result: R) -> Self
    where
        M: IntoSqlExpr,
        R: IntoSqlExpr,
    {
        self.branches
            .push((match_value.into_sql_expr(), result.into_sql_expr()));
        self
    }

    pub fn otherwise<R>(self, result: R) -> TypedExpr<<R as IntoTypedExpr>::Output>
    where
        R: IntoTypedExpr,
    {
        let else_expr = result.into_typed_expr();
        TypedExpr::new(SqlExpr::CaseMatch {
            expr: Box::new(self.expr),
            branches: self.branches,
            else_expr: Box::new(else_expr.expr),
        })
    }
}

pub fn case_match<E>(expr: E) -> CaseMatchBuilder
where
    E: IntoSqlExpr,
{
    CaseMatchBuilder {
        expr: expr.into_sql_expr(),
        branches: Vec::new(),
    }
}

fn rust_type_to_sql<T>() -> &'static str {
    match std::any::type_name::<T>() {
        "alloc::string::String" | "std::string::String" | "&str" => "TEXT",
        "bool" => "BOOLEAN",
        "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "usize" => "INTEGER",
        "i64" | "u64" | "isize" => "BIGINT",
        "f32" | "f64" => "DOUBLE PRECISION",
        "chrono::datetime::DateTime<chrono::offset::utc::Utc>" => "TIMESTAMPTZ",
        "chrono::naive::datetime::NaiveDateTime" => "TIMESTAMPTZ",
        "chrono::naive::date::NaiveDate" => "DATE",
        "chrono::naive::time::NaiveTime" => "TIME",
        "serde_json::value::Value" => "JSON",
        "uuid::Uuid" => "UUID",
        _ => "TEXT",
    }
}

fn quote_collation(db_type: DbType, collation: &str) -> String {
    quote_identifier(db_type, collation)
}

impl SqlExpr {
    pub fn column(name: impl Into<String>) -> Self {
        SqlExpr::Column(name.into())
    }

    pub fn value(value: impl Into<Value>) -> Self {
        SqlExpr::Value(value.into())
    }

    pub(crate) fn to_sql(
        &self,
        db_type: DbType,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
        table_prefix: Option<&str>,
    ) -> String {
        match self {
            SqlExpr::Column(column) => {
                let col_name = if table_prefix.is_some()
                    && !column.contains('.')
                    && !column.contains('(')
                    && !column.contains(' ')
                {
                    format!("{}.{}", table_prefix.unwrap(), column)
                } else {
                    column.clone()
                };
                quote_column_reference(db_type, &col_name)
            }
            SqlExpr::Value(value) => {
                params.push(value.clone());
                let placeholder = placeholder(db_type, *param_idx as usize);
                *param_idx += 1;
                placeholder
            }
            SqlExpr::Binary { left, op, right } => format!(
                "{} {} {}",
                left.to_sql(db_type, param_idx, params, table_prefix),
                op,
                right.to_sql(db_type, param_idx, params, table_prefix)
            ),
            SqlExpr::Function { name, args } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_sql(db_type, param_idx, params, table_prefix))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}({args})")
            }
            SqlExpr::Cast { expr, sql_type } => {
                format!(
                    "CAST({} AS {})",
                    expr.to_sql(db_type, param_idx, params, table_prefix),
                    sql_type
                )
            }
            SqlExpr::Collate { expr, collation } => {
                format!(
                    "{} COLLATE {}",
                    expr.to_sql(db_type, param_idx, params, table_prefix),
                    quote_collation(db_type, collation)
                )
            }
            SqlExpr::Aggregate {
                name,
                expr,
                filter,
                order_by,
                over,
            } => {
                let mut sql = format!(
                    "{}({}",
                    name,
                    expr.to_sql(db_type, param_idx, params, table_prefix)
                );
                if !order_by.is_empty() {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(
                        &order_by
                            .iter()
                            .map(|order| {
                                order.to_sql_with_params(db_type, param_idx, params, table_prefix)
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                sql.push(')');
                if let Some(filter) = filter {
                    if aggregate_filter_native(db_type) {
                        let filter_sql =
                            crate::query::filter_formatter::FilterFormatter::new(db_type)
                                .format(filter, param_idx, params);
                        sql.push_str(" FILTER (WHERE ");
                        sql.push_str(&filter_sql);
                        sql.push(')');
                    } else {
                        sql.clear();
                        sql.push_str(name);
                        sql.push('(');
                        let condition =
                            crate::query::filter_formatter::FilterFormatter::new(db_type)
                                .format(filter, param_idx, params);
                        let argument = expr.to_sql(db_type, param_idx, params, table_prefix);
                        if *name == "COUNT" {
                            sql.clear();
                            sql.push_str("COUNT(CASE WHEN ");
                            sql.push_str(&condition);
                            sql.push_str(" THEN 1 END)");
                        } else {
                            sql.push_str("CASE WHEN ");
                            sql.push_str(&condition);
                            sql.push_str(" THEN ");
                            sql.push_str(&argument);
                            sql.push_str(" END");
                            sql.push(')');
                        }
                    }
                }
                if let Some(over) = over {
                    sql.push_str(" OVER (");
                    let mut parts = Vec::new();
                    if !over.partition_by.is_empty() {
                        parts.push(format!(
                            "PARTITION BY {}",
                            over.partition_by
                                .iter()
                                .map(|expr| expr.to_sql(db_type, param_idx, params, table_prefix))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if !over.order_by.is_empty() {
                        parts.push(format!(
                            "ORDER BY {}",
                            over.order_by
                                .iter()
                                .map(|order| {
                                    order.to_sql_with_params(
                                        db_type,
                                        param_idx,
                                        params,
                                        table_prefix,
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    sql.push_str(&parts.join(" "));
                    sql.push(')');
                }
                sql
            }
            SqlExpr::CaseMatch {
                expr,
                branches,
                else_expr,
            } => {
                let mut sql = format!(
                    "CASE {}",
                    expr.to_sql(db_type, param_idx, params, table_prefix)
                );
                for (match_value, result) in branches {
                    sql.push_str(" WHEN ");
                    sql.push_str(&match_value.to_sql(db_type, param_idx, params, table_prefix));
                    sql.push_str(" THEN ");
                    sql.push_str(&result.to_sql(db_type, param_idx, params, table_prefix));
                }
                sql.push_str(" ELSE ");
                sql.push_str(&else_expr.to_sql(db_type, param_idx, params, table_prefix));
                sql.push_str(" END");
                sql
            }
            SqlExpr::JsonText { expr, key } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{} ->> {}", expr_sql, quote_json_key(key)),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        format!(
                            "JSON_UNQUOTE(JSON_EXTRACT({}, {}))",
                            expr_sql,
                            quote_json_path(key)
                        )
                    }
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        format!("json_extract({}, {})", expr_sql, quote_json_path(key))
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("JSON_VALUE({}, {})", expr_sql, quote_json_path(key)),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => {
                        format!(
                            "JSONExtractString({}, '{}')",
                            expr_sql,
                            key.replace('\'', "''")
                        )
                    }
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!(
                        "json_extract_string({}, {})",
                        expr_sql,
                        quote_json_path(key)
                    ),
                }
            }
            SqlExpr::JsonPathText { expr, path } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{} #>> {}", expr_sql, quote_pg_text_path(path)),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!(
                        "JSON_UNQUOTE(JSON_EXTRACT({}, {}))",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        format!(
                            "json_extract({}, {})",
                            expr_sql,
                            quote_json_path_parts(path)
                        )
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        format!("JSON_VALUE({}, {})", expr_sql, quote_json_path_parts(path))
                    }
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!(
                        "JSONExtractString({}{})",
                        expr_sql,
                        path.iter().fold(String::new(), |mut sql, key| {
                            sql.push_str(", '");
                            sql.push_str(&key.replace('\'', "''"));
                            sql.push('\'');
                            sql
                        })
                    ),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!(
                        "json_extract({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                }
            }
            SqlExpr::JsonPathValue {
                expr,
                path,
                value_type,
            } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        if *value_type == JsonScalarKind::String {
                            format!("{} #>> {}", expr_sql, quote_pg_text_path(path))
                        } else {
                            format!(
                                "({} #> {})::{}",
                                expr_sql,
                                quote_pg_text_path(path),
                                value_type.sql_type()
                            )
                        }
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => match value_type {
                        JsonScalarKind::String => format!(
                            "JSON_UNQUOTE(JSON_EXTRACT({}, {}))",
                            expr_sql,
                            quote_json_path_parts(path)
                        ),
                        JsonScalarKind::Integer => format!(
                            "CAST(JSON_EXTRACT({}, {}) AS SIGNED)",
                            expr_sql,
                            quote_json_path_parts(path)
                        ),
                        JsonScalarKind::Real => format!(
                            "CAST(JSON_EXTRACT({}, {}) AS DOUBLE)",
                            expr_sql,
                            quote_json_path_parts(path)
                        ),
                        JsonScalarKind::Boolean | JsonScalarKind::Json => {
                            format!("JSON_EXTRACT({}, {})", expr_sql, quote_json_path_parts(path))
                        }
                    },
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        format!(
                            "json_extract({}, {})",
                            expr_sql,
                            quote_json_path_parts(path)
                        )
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => match value_type {
                        JsonScalarKind::Boolean => format!("JSON_QUERY({}, {})", expr_sql, quote_json_path_parts(path)),
                        JsonScalarKind::String => {
                            format!("JSON_VALUE({}, {})", expr_sql, quote_json_path_parts(path))
                        }
                        _ => format!(
                            "TRY_CAST(JSON_VALUE({}, {}) AS {})",
                            expr_sql,
                            quote_json_path_parts(path),
                            value_type.sql_type()
                        ),
                    },
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => {
                        let kind = match value_type {
                            JsonScalarKind::Boolean => "Bool",
                            JsonScalarKind::Integer => "Int64",
                            JsonScalarKind::Real => "Float64",
                            JsonScalarKind::String => "String",
                            JsonScalarKind::Json => "String",
                        };
                        format!(
                            "JSONExtract({}{}, '{}')",
                            expr_sql,
                            path.iter().fold(String::new(), |mut sql, key| {
                                sql.push_str(", '");
                                sql.push_str(&key.replace('\'', "''"));
                                sql.push('\'');
                                sql
                            }),
                            kind
                        )
                    }
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => match value_type {
                        JsonScalarKind::String => format!(
                            "json_extract_string({}, {})",
                            expr_sql,
                            quote_json_path_parts(path)
                        ),
                        _ => format!(
                            "CAST(json_extract({}, {}) AS {})",
                            expr_sql,
                            quote_json_path_parts(path),
                            value_type.sql_type()
                        ),
                    },
                }
            }
            SqlExpr::JsonPathExists { expr, path } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        format!("({} #> {}) IS NOT NULL", expr_sql, quote_pg_text_path(path))
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!(
                        "JSON_CONTAINS_PATH({}, 'one', {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!(
                        "json_type({}, {}) IS NOT NULL",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!(
                        "JSON_PATH_EXISTS({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!(
                        "JSONHas({}{})",
                        expr_sql,
                        path.iter().fold(String::new(), |mut sql, key| {
                            sql.push_str(", '");
                            sql.push_str(&key.replace('\'', "''"));
                            sql.push('\'');
                            sql
                        })
                    ),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!(
                        "json_exists({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                }
            }
            SqlExpr::JsonContains { left, right } => match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => {
                    let left_sql = left.to_sql(db_type, param_idx, params, table_prefix);
                    let right_sql = right.to_sql(db_type, param_idx, params, table_prefix);
                    format!("{}::jsonb @> {}::jsonb", left_sql, right_sql)
                }
                #[cfg(feature = "mysql")]
                DbType::MySQL => {
                    let left_sql = left.to_sql(db_type, param_idx, params, table_prefix);
                    let right_sql = right.to_sql(db_type, param_idx, params, table_prefix);
                    format!("JSON_CONTAINS({}, {})", left_sql, right_sql)
                }
                _ => {
                    let _ = (left, right);
                    "0 = 1 /* OrmerError::UnsupportedFeature: JSON containment predicates */"
                        .to_string()
                }
            },
            SqlExpr::JsonSet { expr, path, value } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                let value_sql = value.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        let value_sql = match value.as_ref() {
                            SqlExpr::Value(Value::Json(_)) => format!("{value_sql}::jsonb"),
                            _ => format!("to_jsonb({value_sql})"),
                        };
                        format!(
                            "jsonb_set({}::jsonb, {}, {}, true)",
                            expr_sql,
                            quote_pg_text_path(path),
                            value_sql
                        )
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        format!(
                            "JSON_SET({}, {}, {})",
                            expr_sql,
                            quote_json_path_parts(path),
                            value_sql
                        )
                    }
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        format!(
                            "json_set({}, {}, {})",
                            expr_sql,
                            quote_json_path_parts(path),
                            value_sql
                        )
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        format!(
                            "JSON_MODIFY({}, {}, {})",
                            expr_sql,
                            quote_json_path_parts(path),
                            value_sql
                        )
                    }
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => {
                        "0 = 1 /* OrmerError::UnsupportedFeature: JSON updates */".to_string()
                    }
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!(
                        "json_merge_patch({}, json_object({}, {}))",
                        expr_sql,
                        quote_json_path_parts(path),
                        value_sql
                    ),
                }
            }
            SqlExpr::ArrayContains { left, right } => {
                let left_sql = left.to_sql(db_type, param_idx, params, table_prefix);
                let right_sql = right.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{} @> {}", left_sql, right_sql),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("JSON_CONTAINS({}, {})", left_sql, right_sql),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!(
                        "EXISTS (SELECT 1 FROM json_each({}) AS l INNER JOIN json_each({}) AS r ON l.value = r.value)",
                        left_sql, right_sql
                    ),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!(
                        "EXISTS (SELECT 1 FROM OPENJSON({}) l INNER JOIN OPENJSON({}) r ON l.value = r.value)",
                        left_sql, right_sql
                    ),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("hasAll({}, {})", left_sql, right_sql),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("list_has_all({}, {})", left_sql, right_sql),
                }
            }
            SqlExpr::JsonRemove { expr, path } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        format!("{} #- {}", expr_sql, quote_pg_text_path(path))
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!(
                        "JSON_REMOVE({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!(
                        "json_remove({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!(
                        "JSON_MODIFY({}, {}, NULL)",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => {
                        "0 = 1 /* OrmerError::UnsupportedFeature: JSON updates */".to_string()
                    }
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!(
                        "json_remove({}, {})",
                        expr_sql,
                        quote_json_path_parts(path)
                    ),
                }
            }
            SqlExpr::ArrayOverlaps { left, right } => {
                let left_sql = left.to_sql(db_type, param_idx, params, table_prefix);
                let right_sql = right.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{} && {}", left_sql, right_sql),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("JSON_OVERLAPS({}, {})", left_sql, right_sql),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!(
                        "EXISTS (SELECT 1 FROM json_each({}) AS l INNER JOIN json_each({}) AS r ON l.value = r.value)",
                        left_sql, right_sql
                    ),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!(
                        "EXISTS (SELECT 1 FROM OPENJSON({}) l INNER JOIN OPENJSON({}) r ON l.value = r.value)",
                        left_sql, right_sql
                    ),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("hasAny({}, {})", left_sql, right_sql),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("list_has_any({}, {})", left_sql, right_sql),
                }
            }
            SqlExpr::ArrayLen { expr } => {
                let expr_sql = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("cardinality({})", expr_sql),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("JSON_LENGTH({})", expr_sql),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!("json_array_length({})", expr_sql),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("(SELECT COUNT(*) FROM OPENJSON({}))", expr_sql),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("length({})", expr_sql),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("length({})", expr_sql),
                }
            }
            SqlExpr::WindowFunction {
                function,
                args,
                over,
            } => {
                let arguments = args
                    .iter()
                    .map(|arg| arg.to_sql(db_type, param_idx, params, table_prefix))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut sql = format!("{function}({arguments}) OVER (");
                let mut parts = Vec::new();
                if !over.partition_by.is_empty() {
                    parts.push(format!(
                        "PARTITION BY {}",
                        over.partition_by
                            .iter()
                            .map(|expr| expr.to_sql(db_type, param_idx, params, table_prefix))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !over.order_by.is_empty() {
                    parts.push(format!(
                        "ORDER BY {}",
                        over.order_by
                            .iter()
                            .map(|order| {
                                order.to_sql_with_params(
                                    db_type,
                                    param_idx,
                                    params,
                                    table_prefix,
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                sql.push_str(&parts.join(" "));
                sql.push(')');
                sql
            }
            SqlExpr::DateTrunc { expr, unit } => {
                let value = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("date_trunc('{}', {})", unit.pg_name(), value),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => match unit {
                        TimeUnit::Second => format!("DATE_FORMAT({value}, '%Y-%m-%d %H:%i:%s')"),
                        TimeUnit::Minute => format!("DATE_FORMAT({value}, '%Y-%m-%d %H:%i:00')"),
                        TimeUnit::Hour => format!("DATE_FORMAT({value}, '%Y-%m-%d %H:00:00')"),
                        TimeUnit::Day => format!("DATE({value})"),
                        TimeUnit::Week => format!("DATE_SUB(DATE({value}), INTERVAL WEEKDAY({value}) DAY)"),
                        TimeUnit::Month => format!("DATE_FORMAT({value}, '%Y-%m-01')"),
                        TimeUnit::Year => format!("DATE_FORMAT({value}, '%Y-01-01')"),
                    },
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!("strftime({}, {})", quote_sql_literal(unit.sqlite_format()), value),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("DATETRUNC({}, {})", unit.pg_name(), value),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("dateTrunc('{}', {})", unit.pg_name(), value),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("date_trunc('{}', {})", unit.pg_name(), value),
                }
            }
            SqlExpr::DatePart { expr, part } => {
                let value = expr.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("date_part('{}', {})", part.name(), value),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("EXTRACT({} FROM {})", part.name_upper(), value),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!("CAST(strftime({}, {}) AS INTEGER)", quote_sql_literal(part.sqlite_format()), value),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("DATEPART({}, {})", part.name_upper(), value),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("to{}({})", part.name_upper(), value),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("date_part('{}', {})", part.name(), value),
                }
            }
            SqlExpr::AtTimeZone { expr, timezone } => {
                let value = expr.to_sql(db_type, param_idx, params, table_prefix);
                let zone = timezone.replace('\'', "''");
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{value} AT TIME ZONE '{zone}'"),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("{value} AT TIME ZONE '{zone}'"),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("CONVERT_TZ({value}, 'UTC', '{zone}')"),
                    #[cfg(any(feature = "sqlite", feature = "duckdb", feature = "clickhouse"))]
                    _ => "CAST(NULL AS TEXT) /* OrmerError::UnsupportedFeature: timezone conversion */".to_string(),
                }
            }
            SqlExpr::DateAdd {
                expr,
                unit,
                amount,
                negative,
            } => {
                let value = expr.to_sql(db_type, param_idx, params, table_prefix);
                let mut delta = amount.to_sql(db_type, param_idx, params, table_prefix);
                if *negative {
                    delta = format!("-({delta})");
                }
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => format!("{} + ({}) * INTERVAL '1 {}'", value, delta, unit.pg_name()),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("DATE_ADD({}, INTERVAL {} {})", value, delta, unit.pg_name()),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => format!("datetime({}, printf('%+d seconds', {}))", value, delta),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("DATEADD({}, {}, {})", unit.pg_name(), delta, value),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!(
                        "dateAdd('{}', {}({}), {})",
                        unit.pg_name(),
                        unit.clickhouse_interval(),
                        delta,
                        value
                    ),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("{} + ({}) * INTERVAL '1 {}'", value, delta, unit.pg_name()),
                }
            }
            SqlExpr::DateDiff { left, right, part } => {
                let left_sql = left.to_sql(db_type, param_idx, params, table_prefix);
                let right_sql = right.to_sql(db_type, param_idx, params, table_prefix);
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        let divisor = part.epoch_divisor();
                        format!("date_part('epoch', {left_sql} - {right_sql}) / {divisor}")
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => format!("TIMESTAMPDIFF({}, {}, {})", part.name_upper(), left_sql, right_sql),
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        let divisor = part.epoch_divisor() * 86400.0;
                        format!("CAST((julianday({left_sql}) - julianday({right_sql})) * {divisor} AS INTEGER)")
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => format!("DATEDIFF({}, {}, {})", part.name_upper(), left_sql, right_sql),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => format!("dateDiff('{}', {}, {})", part.name(), left_sql, right_sql),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => format!("date_diff('{}', {}, {})", part.name(), left_sql, right_sql),
                }
            }
            SqlExpr::Now => match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => "NOW()".to_string(),
                #[cfg(feature = "mysql")]
                DbType::MySQL => "CURRENT_TIMESTAMP(6)".to_string(),
                #[cfg(feature = "sqlite")]
                DbType::Sqlite => "datetime('now')".to_string(),
                #[cfg(feature = "mssql")]
                DbType::MSSQL => "SYSDATETIMEOFFSET()".to_string(),
                #[cfg(feature = "clickhouse")]
                DbType::ClickHouse => "now()".to_string(),
                #[cfg(feature = "duckdb")]
                DbType::DuckDB => "now()".to_string(),
            },
            SqlExpr::Row(exprs) => {
                let values = exprs
                    .iter()
                    .map(|expr| expr.to_sql(db_type, param_idx, params, table_prefix))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({values})")
            }
            SqlExpr::Raw(expr) => expr.to_sql(db_type, param_idx, params, table_prefix),
        }
    }

    pub(crate) fn to_sql_no_params(&self, db_type: DbType) -> String {
        let mut param_idx = 1;
        let mut params = Vec::new();
        self.to_sql(db_type, &mut param_idx, &mut params, None)
    }

    pub(crate) fn validate_for_db(&self, db_type: DbType) -> crate::Result<()> {
        match self {
            SqlExpr::Column(_) | SqlExpr::Value(_) => Ok(()),
            SqlExpr::Binary { left, right, .. }
            | SqlExpr::ArrayContains { left, right }
            | SqlExpr::ArrayOverlaps { left, right } => {
                left.validate_for_db(db_type)?;
                right.validate_for_db(db_type)?;
                Ok(())
            }
            SqlExpr::JsonContains { left, right } => {
                left.validate_for_db(db_type)?;
                right.validate_for_db(db_type)?;
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => Ok(()),
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => Ok(()),
                    _ => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "JSON containment predicates",
                    }),
                }
            }
            SqlExpr::Function { args, .. } | SqlExpr::Row(args) => {
                for arg in args {
                    arg.validate_for_db(db_type)?;
                }
                Ok(())
            }
            SqlExpr::WindowFunction { args, over, .. } => {
                for arg in args {
                    arg.validate_for_db(db_type)?;
                }
                for expr in &over.partition_by {
                    expr.validate_for_db(db_type)?;
                }
                for order in &over.order_by {
                    if let Some(expr) = order.cloned_expr() {
                        expr.validate_for_db(db_type)?;
                    }
                }
                Ok(())
            }
            SqlExpr::DateTrunc { expr, .. }
            | SqlExpr::DatePart { expr, .. }
            | SqlExpr::AtTimeZone { expr, .. } => expr.validate_for_db(db_type),
            SqlExpr::DateAdd { amount, .. } => amount.validate_for_db(db_type),
            SqlExpr::DateDiff { left, right, .. } => {
                left.validate_for_db(db_type)?;
                right.validate_for_db(db_type)
            }
            SqlExpr::Now => Ok(()),
            #[cfg(feature = "clickhouse")]
            SqlExpr::JsonSet { .. } | SqlExpr::JsonRemove { .. }
                if matches!(db_type, DbType::ClickHouse) =>
            {
                Err(crate::OrmerError::UnsupportedFeature {
                    backend: db_type,
                    feature: "JSON updates",
                })
            }
            SqlExpr::Cast { expr, .. }
            | SqlExpr::Collate { expr, .. }
            | SqlExpr::JsonText { expr, .. }
            | SqlExpr::JsonPathText { expr, .. }
            | SqlExpr::JsonPathValue { expr, .. }
            | SqlExpr::JsonPathExists { expr, .. }
            | SqlExpr::ArrayLen { expr } => expr.validate_for_db(db_type),
            SqlExpr::Aggregate {
                expr,
                filter,
                order_by,
                over,
                ..
            } => {
                expr.validate_for_db(db_type)?;
                if let Some(filter) = filter {
                    validate_filter_for_db(filter, db_type)?;
                }
                for order in order_by {
                    if let Some(expr) = order.cloned_expr() {
                        expr.validate_for_db(db_type)?;
                    }
                }
                if let Some(over) = over {
                    for expr in &over.partition_by {
                        expr.validate_for_db(db_type)?;
                    }
                    for order in &over.order_by {
                        if let Some(expr) = order.cloned_expr() {
                            expr.validate_for_db(db_type)?;
                        }
                    }
                }
                Ok(())
            }
            SqlExpr::CaseMatch {
                expr,
                branches,
                else_expr,
            } => {
                expr.validate_for_db(db_type)?;
                for (when, then) in branches {
                    when.validate_for_db(db_type)?;
                    then.validate_for_db(db_type)?;
                }
                else_expr.validate_for_db(db_type)
            }
            SqlExpr::JsonSet { expr, value, .. } => {
                expr.validate_for_db(db_type)?;
                value.validate_for_db(db_type)
            }
            SqlExpr::JsonRemove { expr, .. } => expr.validate_for_db(db_type),
            SqlExpr::Raw(raw) => {
                for segment in &raw.segments {
                    if let RawExprSegment::Expr(expr) = segment {
                        expr.validate_for_db(db_type)?;
                    }
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn validate_filter_for_db(
    filter: &crate::query::filter::FilterExpr,
    db_type: DbType,
) -> crate::Result<()> {
    use crate::query::filter::FilterExpr;

    match filter {
        FilterExpr::InvalidDynamicField { model, field } => Err(crate::ormer_error!(
            "Field '{}' does not exist on model {}",
            field,
            model
        )),
        FilterExpr::And(left, right) | FilterExpr::Or(left, right) => {
            validate_filter_for_db(left, db_type)?;
            validate_filter_for_db(right, db_type)
        }
        FilterExpr::RelationExists { filter, .. }
        | FilterExpr::ThroughRelationExists { filter, .. } => {
            if let Some(filter) = filter {
                validate_filter_for_db(filter, db_type)?;
            }
            Ok(())
        }
        FilterExpr::ExprComparison { left, right, .. } => {
            left.validate_for_db(db_type)?;
            right.validate_for_db(db_type)
        }
        FilterExpr::ExprIn { expr, values } | FilterExpr::ExprNotIn { expr, values } => {
            expr.validate_for_db(db_type)?;
            for value in values {
                value.validate_for_db(db_type)?;
            }
            Ok(())
        }
        FilterExpr::ExprBetween { expr, min, max } => {
            expr.validate_for_db(db_type)?;
            min.validate_for_db(db_type)?;
            max.validate_for_db(db_type)
        }
        FilterExpr::ExprIsNull { expr }
        | FilterExpr::ExprIsNotNull { expr }
        | FilterExpr::ExprPredicate { expr } => expr.validate_for_db(db_type),
        FilterExpr::TextSearch { expr, .. } => expr.validate_for_db(db_type),
        FilterExpr::FullTextSearch(search) => {
            for expr in &search.exprs {
                expr.validate_for_db(db_type)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn aggregate_filter_native(db_type: DbType) -> bool {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => true,
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => true,
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => true,
        #[cfg(any(
            feature = "mysql",
            feature = "mssql",
            feature = "clickhouse",
            feature = "postgresql",
            feature = "sqlite",
            feature = "duckdb"
        ))]
        _ => false,
    }
}

#[cfg(feature = "postgresql")]
fn quote_json_key(key: &str) -> String {
    format!("'{}'", key.replace('\'', "''"))
}

#[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql"))]
fn quote_json_path(key: &str) -> String {
    format!("'$.{}'", key.replace('\'', "''"))
}

#[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql"))]
fn quote_json_path_parts(path: &[String]) -> String {
    if path.is_empty() {
        return "'$'".to_string();
    }
    format!(
        "'$.{}'",
        path.iter()
            .map(|part| part.replace('\'', "''"))
            .collect::<Vec<_>>()
            .join(".")
    )
}

#[cfg(feature = "postgresql")]
fn quote_pg_text_path(path: &[String]) -> String {
    format!(
        "'{{{}}}'",
        path.iter()
            .map(|part| part
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\'', "''"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
