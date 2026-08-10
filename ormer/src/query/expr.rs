use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, quote_column_reference, quote_identifier};
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
    Row(Vec<SqlExpr>),
    Raw(String),
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

#[derive(Debug, Clone)]
pub struct TypedExpr<T, S = ()> {
    pub(crate) expr: SqlExpr,
    _marker: PhantomData<(T, S)>,
}

#[derive(Debug, Clone)]
pub struct AliasedExpr<E> {
    pub(crate) expr: E,
    pub(crate) alias: String,
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

impl<T, S> IntoTypedExpr for TypedExpr<T, S> {
    type Output = T;

    fn into_typed_expr(self) -> TypedExpr<Self::Output> {
        TypedExpr::new(self.expr)
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
    TypedExpr::new(SqlExpr::Raw(sql.into()))
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
                            .map(|order| order.to_sql_for(db_type))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                sql.push(')');
                if let Some(filter) = filter {
                    let filter_sql = crate::query::filter_formatter::FilterFormatter::new(db_type)
                        .format(filter, param_idx, params);
                    sql.push_str(" FILTER (WHERE ");
                    sql.push_str(&filter_sql);
                    sql.push(')');
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
                                .map(|order| order.to_sql_for(db_type))
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
                }
            }
            SqlExpr::Row(exprs) => {
                let values = exprs
                    .iter()
                    .map(|expr| expr.to_sql(db_type, param_idx, params, table_prefix))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({values})")
            }
            SqlExpr::Raw(sql) => sql.clone(),
        }
    }

    pub(crate) fn to_sql_no_params(&self, db_type: DbType) -> String {
        let mut param_idx = 1;
        let mut params = Vec::new();
        self.to_sql(db_type, &mut param_idx, &mut params, None)
    }
}

#[allow(dead_code)]
fn quote_json_key(key: &str) -> String {
    format!("'{}'", key.replace('\'', "''"))
}

fn quote_json_path(key: &str) -> String {
    format!("'$.{}'", key.replace('\'', "''"))
}
