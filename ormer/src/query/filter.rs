use crate::query::expr::SqlExpr;
use std::fmt;
use std::sync::Arc;

/// 过滤表达式
#[derive(Debug, Clone)]
pub enum FilterExpr {
    /// 简单比较:column operator value
    Comparison {
        column: String,
        operator: String,
        value: Value,
    },
    /// 列-列比较:column1 operator column2
    ColumnComparison {
        left_column: String,
        operator: String,
        right_column: String,
    },
    /// IN 语句:column IN (value1, value2, ...)
    In { column: String, values: Vec<Value> },
    /// NOT IN 语句:column NOT IN (value1, value2, ...)
    NotIn { column: String, values: Vec<Value> },
    /// 子查询 IN: column IN (subquery)
    InSubquery {
        column: String,
        subquery_sql: String,
        subquery_params: Vec<crate::model::Value>,
    },
    /// 子查询 NOT IN: column NOT IN (subquery)
    NotInSubquery {
        column: String,
        subquery_sql: String,
        subquery_params: Vec<crate::model::Value>,
    },
    /// AND 连接
    And(Box<FilterExpr>, Box<FilterExpr>),
    /// OR 连接
    Or(Box<FilterExpr>, Box<FilterExpr>),
    /// IS NULL
    IsNull { column: String },
    /// IS NOT NULL
    IsNotNull { column: String },
    /// BETWEEN min AND max
    Between {
        column: String,
        min: Value,
        max: Value,
    },
    /// EXISTS 子查询: EXISTS (SELECT 1 FROM ... WHERE ...)
    Exists {
        subquery_sql: String,
        subquery_params: Vec<crate::model::Value>,
    },
    /// EXISTS subquery rendered with the final backend dialect.
    ExistsDynamic { subquery: DynamicSubquery },
    /// NOT EXISTS 子查询: NOT EXISTS (SELECT 1 FROM ... WHERE ...)
    NotExists {
        subquery_sql: String,
        subquery_params: Vec<crate::model::Value>,
    },
    /// NOT EXISTS subquery rendered with the final backend dialect.
    NotExistsDynamic { subquery: DynamicSubquery },
    /// IN subquery rendered with the final backend dialect.
    InSubqueryDynamic {
        column: String,
        subquery: DynamicSubquery,
    },
    /// NOT IN subquery rendered with the final backend dialect.
    NotInSubqueryDynamic {
        column: String,
        subquery: DynamicSubquery,
    },
    /// 关系存在性查询: EXISTS (SELECT 1 FROM target WHERE target.fk = owner.pk AND ...)
    RelationExists {
        owner_table: &'static str,
        owner_key: &'static str,
        target_table: &'static str,
        target_key: &'static str,
        filter: Option<Box<FilterExpr>>,
    },
    /// through 关系存在性查询。
    ThroughRelationExists {
        owner_table: &'static str,
        owner_key: &'static str,
        via_table: &'static str,
        via_owner_key: &'static str,
        via_target_key: &'static str,
        target_table: &'static str,
        target_key: &'static str,
        filter: Option<Box<FilterExpr>>,
    },
    /// 表达式比较:left operator right
    ExprComparison {
        left: SqlExpr,
        operator: String,
        right: SqlExpr,
    },
    /// 表达式 IN 语句
    ExprIn { expr: SqlExpr, values: Vec<SqlExpr> },
    /// 表达式 NOT IN 语句
    ExprNotIn { expr: SqlExpr, values: Vec<SqlExpr> },
    /// 表达式 BETWEEN min AND max
    ExprBetween {
        expr: SqlExpr,
        min: SqlExpr,
        max: SqlExpr,
    },
    /// 表达式 IS NULL
    ExprIsNull { expr: SqlExpr },
    /// 表达式 IS NOT NULL
    ExprIsNotNull { expr: SqlExpr },
    /// 布尔表达式谓词
    ExprPredicate { expr: SqlExpr },
    /// Full text search
    TextSearch { expr: SqlExpr, query: String },
    /// Unified full-text search over one or more expressions.
    FullTextSearch(Box<FullTextQuery>),
    /// Runtime dynamic field that could not be resolved against the model.
    InvalidDynamicField { model: &'static str, field: String },
    /// A query part that is known not to be supported by the target backend.
    Unsupported {
        backend: crate::abstract_layer::DbType,
        feature: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullTextMode {
    Natural,
    Boolean,
    WebSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullTextRank {
    None,
    Relevance,
}

#[derive(Debug, Clone)]
pub struct FullTextQuery {
    pub exprs: Vec<SqlExpr>,
    pub query: String,
    pub mode: FullTextMode,
    pub language: Option<String>,
    pub rank: FullTextRank,
}

impl FullTextQuery {
    pub fn new(expr: SqlExpr, query: impl Into<String>) -> Self {
        Self {
            exprs: vec![expr],
            query: query.into(),
            mode: FullTextMode::Natural,
            language: None,
            rank: FullTextRank::None,
        }
    }
}

/// 值类型（用于过滤）
pub type Value = crate::model::Value;

/// A subquery whose SQL is rendered after the target backend is known.
#[derive(Clone)]
#[doc(hidden)]
pub struct DynamicSubquery {
    render: Arc<
        dyn Fn(crate::abstract_layer::DbType) -> crate::Result<(String, Vec<Value>)> + Send + Sync,
    >,
}

impl fmt::Debug for DynamicSubquery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DynamicSubquery(..)")
    }
}

impl DynamicSubquery {
    pub(crate) fn new(
        render: impl Fn(crate::abstract_layer::DbType) -> crate::Result<(String, Vec<Value>)>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            render: Arc::new(render),
        }
    }

    pub(crate) fn render(
        &self,
        db_type: crate::abstract_layer::DbType,
    ) -> crate::Result<(String, Vec<Value>)> {
        (self.render)(db_type)
    }

    /// 渲染并只取参数（PostgreSQL rust 类型收集路径）。
    ///
    /// `Vec<Value>` 签名无法携带错误：渲染失败时返回空列表，保持 rust
    /// 类型清单与实际占位符数量对齐（渲染层对失败子查询生成零占位符的
    /// 错误占位引用，最终以数据库错误而非 panic 的形式抛出）。
    #[cfg(feature = "postgresql")]
    pub(crate) fn params(&self, db_type: crate::abstract_layer::DbType) -> Vec<Value> {
        self.render(db_type)
            .map(|(_, params)| params)
            .unwrap_or_default()
    }
}

#[cfg(feature = "postgresql")]
pub(crate) fn infer_filter_value_rust_type(value: &Value) -> &'static str {
    match value {
        Value::Integer(_) => "i32",
        Value::BigInt(_) => "i64",
        Value::Duration(_) => "Duration",
        Value::Text(_) => "String",
        Value::TextArray(_) => "Vec<String>",
        Value::Real(_) => "f64",
        Value::Decimal(_) => "rust_decimal::Decimal",
        Value::BigDecimal(_) => "bigdecimal::BigDecimal",
        Value::Boolean(_) => "bool",
        Value::Bytes(_) => "Vec<u8>",
        Value::IntegerArray(_) => "Vec<i32>",
        Value::BigIntArray(_) => "Vec<i64>",
        Value::NullableBigIntArray(_) => "Vec<Option<i64>>",
        Value::DateTime(_) => "NaiveDateTime",
        Value::Date(_) => "NaiveDate",
        Value::Time(_) => "NaiveTime",
        Value::Json(_) => "String",
        Value::Uuid(_) => "uuid::Uuid",
        // NULL 没有可推断的列类型：保持与 `pg_value_to_param` 的无类型
        // NULL 分支一致（None::<i32>，整型家族）。等值 NULL 在渲染层已被
        // 改写为 IS NULL / IS NOT NULL（不产生参数），非等值 NULL 由
        // `FilterExpr::validate_null_usage` 在校验阶段拦截；此处仅剩
        // 表达式字面量等罕见路径会真正绑定 NULL 参数，维持现状以免
        // 引入新的绑定失败（其他占位类型只是把失败挪到别的列类型上）。
        Value::Null => "i32",
    }
}

#[cfg(feature = "postgresql")]
pub(crate) fn infer_model_value_rust_type(value: &crate::model::Value) -> &'static str {
    infer_filter_value_rust_type(value)
}

macro_rules! impl_decimal_query_traits {
    ($type:ty) => {
        impl crate::query::builder::AggregateResultType for $type {
            type Output = Option<$type>;
        }

        impl crate::query::builder::ColumnValueType for $type {
            fn to_filter_value(value: Self) -> Value {
                Value::from(value)
            }

            fn supports_comparison() -> bool {
                true
            }
        }

        impl crate::query::builder::IsInValue<$type> for $type {
            fn to_in_value(self) -> $type {
                self
            }
        }

        impl crate::query::builder::IsInValue<$type> for &$type {
            fn to_in_value(self) -> $type {
                (*self).clone()
            }
        }

        impl crate::query::builder::IsInValue<$type> for &&$type {
            fn to_in_value(self) -> $type {
                (**self).clone()
            }
        }

        impl crate::query::expr::IntoSqlExpr for $type {
            fn into_sql_expr(self) -> SqlExpr {
                SqlExpr::Value(Value::from(self))
            }
        }

        impl crate::query::expr::IntoTypedExpr for $type {
            type Output = $type;

            fn into_typed_expr(self) -> crate::query::expr::TypedExpr<Self::Output> {
                crate::query::expr::TypedExpr::new(SqlExpr::Value(Value::from(self)))
            }
        }
    };
}

impl_decimal_query_traits!(rust_decimal::Decimal);
impl_decimal_query_traits!(bigdecimal::BigDecimal);

impl crate::query::builder::ColumnValueType for uuid::Uuid {
    fn to_filter_value(value: Self) -> Value {
        Value::Uuid(value)
    }

    fn supports_comparison() -> bool {
        false
    }
}

impl crate::query::builder::IsInValue<uuid::Uuid> for uuid::Uuid {
    fn to_in_value(self) -> uuid::Uuid {
        self
    }
}

impl crate::query::builder::IsInValue<uuid::Uuid> for &uuid::Uuid {
    fn to_in_value(self) -> uuid::Uuid {
        *self
    }
}

impl crate::query::builder::IsInValue<uuid::Uuid> for &&uuid::Uuid {
    fn to_in_value(self) -> uuid::Uuid {
        **self
    }
}

/// 子查询 trait - 用于 is_in 方法
pub trait Subquery {
    /// 获取子查询的 SQL 和参数
    fn to_subquery_sql(&self) -> crate::Result<(String, Vec<crate::model::Value>)>;
}

impl FilterExpr {
    pub fn and(self, other: FilterExpr) -> Self {
        FilterExpr::And(Box::new(self), Box::new(other))
    }

    pub fn or(self, other: FilterExpr) -> Self {
        FilterExpr::Or(Box::new(self), Box::new(other))
    }

    /// NULL 比较语义校验：三值逻辑下非等值比较（> >= < <= LIKE IN
    /// BETWEEN 等）与 NULL 组合恒为 UNKNOWN，生成恒假 SQL 不会有任何
    /// 警告。此处在校验阶段直接报错，要求改用 IS NULL / IS NOT NULL /
    /// 等值判断（等值/不等值会被渲染层改写为 IS NULL / IS NOT NULL）。
    pub(crate) fn validate_null_usage(&self) -> crate::Result<()> {
        validate_null_usage(self)
    }
}

/// `SqlExpr` 是否为裸 NULL 字面量（区别于嵌套在函数/COALESCE 里的 NULL，
/// 后者是合法 SQL，不拦截）。
fn is_null_literal(expr: &SqlExpr) -> bool {
    matches!(expr, SqlExpr::Value(Value::Null))
}

fn null_comparison_error(subject: &str, operator: &str) -> crate::OrmerError {
    crate::OrmerError::invalid_operation(format!(
        "{} cannot be compared with NULL using '{}': NULL only supports IS NULL / IS NOT NULL / equality checks",
        subject, operator
    ))
}

fn validate_null_usage(expr: &FilterExpr) -> crate::Result<()> {
    match expr {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            if matches!(value, Value::Null) && !matches!(operator.as_str(), "=" | "!=" | "<>") {
                return Err(null_comparison_error(
                    &format!("column '{column}'"),
                    operator,
                ));
            }
            Ok(())
        }
        FilterExpr::Between { column, min, max } => {
            if matches!(min, Value::Null) || matches!(max, Value::Null) {
                return Err(crate::OrmerError::invalid_operation(format!(
                    "column '{column}' BETWEEN bounds cannot be NULL: NULL only supports IS NULL / IS NOT NULL / equality checks"
                )));
            }
            Ok(())
        }
        FilterExpr::In { column, values } | FilterExpr::NotIn { column, values } => {
            if values.iter().any(|value| matches!(value, Value::Null)) {
                return Err(crate::OrmerError::invalid_operation(format!(
                    "column '{column}' IN/NOT IN list cannot contain NULL: NULL never matches, use is_null()/is_not_null() instead"
                )));
            }
            Ok(())
        }
        FilterExpr::ExprComparison {
            left,
            operator,
            right,
        } => {
            let null_operand = is_null_literal(left) || is_null_literal(right);
            if null_operand && !matches!(operator.as_str(), "=" | "!=" | "<>") {
                return Err(null_comparison_error("expression", operator));
            }
            Ok(())
        }
        FilterExpr::ExprBetween { min, max, .. } => {
            if is_null_literal(min) || is_null_literal(max) {
                return Err(crate::OrmerError::invalid_operation(
                    "expression BETWEEN bounds cannot be NULL: NULL only supports IS NULL / IS NOT NULL / equality checks",
                ));
            }
            Ok(())
        }
        FilterExpr::ExprIn { values, .. } | FilterExpr::ExprNotIn { values, .. } => {
            if values.iter().any(is_null_literal) {
                return Err(crate::OrmerError::invalid_operation(
                    "expression IN/NOT IN list cannot contain NULL: NULL never matches, use is_null()/is_not_null() instead",
                ));
            }
            Ok(())
        }
        FilterExpr::And(left, right) | FilterExpr::Or(left, right) => {
            validate_null_usage(left)?;
            validate_null_usage(right)
        }
        FilterExpr::RelationExists { filter, .. }
        | FilterExpr::ThroughRelationExists { filter, .. } => {
            if let Some(filter) = filter {
                return validate_null_usage(filter);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// 排序方向
#[derive(Debug, Clone, Copy)]
pub enum OrderDirection {
    Asc,
    Desc,
}

/// 排序表达式
#[derive(Debug, Clone)]
pub struct OrderBy {
    pub column: String,
    pub direction: OrderDirection,
    expr: Option<SqlExpr>,
    error: Option<String>,
}

impl OrderBy {
    pub fn asc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Asc,
            expr: None,
            error: None,
        }
    }

    pub fn desc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Desc,
            expr: None,
            error: None,
        }
    }

    pub fn asc_expr(expr: SqlExpr) -> Self {
        Self::for_expr(expr, OrderDirection::Asc)
    }

    pub fn desc_expr(expr: SqlExpr) -> Self {
        Self::for_expr(expr, OrderDirection::Desc)
    }

    /// 表达式排序项：`column` 只存规范列名（裸列名，不带方言引用/转换），
    /// 表达式渲染延迟到 `to_sql` 阶段由 `expr` 字段完成。
    ///
    /// `column` 会被 cursor 分页当作列名与主键比较、被 `model.column_value`
    /// 查找；若把 default 方言渲染的 SQL 片段快照进去，真实后端不同时会
    /// 携带另一种方言的片段（见 P2-8）。非列表达式没有规范列名，置空并
    /// 由 `prepare_cursor_page` 强制要求 `cursor_by`。
    fn for_expr(expr: SqlExpr, direction: OrderDirection) -> Self {
        Self {
            column: canonical_expr_column(&expr),
            direction,
            expr: Some(expr),
            error: None,
        }
    }

    pub fn invalid(column: String, error: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Asc,
            expr: None,
            error: Some(error),
        }
    }

    pub(crate) fn cloned_expr(&self) -> Option<SqlExpr> {
        self.expr.clone()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 将 OrderBy 转换为 SQL 字符串（默认方言，含列名引用）
    ///
    /// 与 `to_sql_for(default_db_type())` 完全一致，避免出现同一排序项
    /// 两份拼接逻辑、裸拼列名与实际执行语句不一致的问题。
    pub fn to_sql(&self) -> String {
        self.to_sql_for(crate::query::builder::default_db_type())
    }

    /// 将 OrderBy 转换为指定后端的 SQL 字符串
    pub fn to_sql_for(&self, db_type: crate::abstract_layer::DbType) -> String {
        let dir = match self.direction {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };
        let expr_sql = self
            .expr
            .as_ref()
            .map(|expr| expr.to_sql_no_params(db_type))
            .unwrap_or_else(|| crate::model::quote_column_reference(db_type, &self.column));
        format!("{} {}", expr_sql, dir)
    }

    pub(crate) fn to_sql_with_params(
        &self,
        db_type: crate::abstract_layer::DbType,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        table_prefix: Option<&str>,
    ) -> String {
        let dir = match self.direction {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };
        let expr_sql = self
            .expr
            .as_ref()
            .map(|expr| expr.to_sql(db_type, param_idx, params, table_prefix))
            .unwrap_or_else(|| crate::model::quote_column_reference(db_type, &self.column));
        format!("{} {}", expr_sql, dir)
    }
}

/// 表达式排序项的规范列名：裸列引用直接取列名，其余表达式没有可作列名
/// 使用的规范形式，返回空串（渲染一律走 `expr` 字段）。
fn canonical_expr_column(expr: &SqlExpr) -> String {
    match expr {
        SqlExpr::Column(column) => column.clone(),
        _ => String::new(),
    }
}
