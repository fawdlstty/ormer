use crate::query::expr::SqlExpr;

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
    /// NOT EXISTS 子查询: NOT EXISTS (SELECT 1 FROM ... WHERE ...)
    NotExists {
        subquery_sql: String,
        subquery_params: Vec<crate::model::Value>,
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
    /// Full text search
    TextSearch { expr: SqlExpr, query: String },
}

/// 值类型（用于过滤）
pub type Value = crate::model::Value;

#[cfg(feature = "postgresql")]
pub(crate) fn infer_filter_value_rust_type(value: &Value) -> &'static str {
    match value {
        Value::Integer(_) => "i32",
        Value::BigInt(_) => "i64",
        Value::Duration(_) => "Duration",
        Value::Text(_) => "String",
        Value::TextArray(_) => "Vec<String>",
        Value::Real(_) => "f64",
        Value::Boolean(_) => "bool",
        Value::Bytes(_) => "Vec<u8>",
        Value::IntegerArray(_) => "Vec<i32>",
        Value::BigIntArray(_) => "Vec<i64>",
        Value::NullableBigIntArray(_) => "Vec<Option<i64>>",
        Value::DateTime(_) => "NaiveDateTime",
        Value::Date(_) => "NaiveDate",
        Value::Time(_) => "NaiveTime",
        Value::Json(_) => "String",
        Value::Uuid(_) => "String",
        Value::Null => "i32",
    }
}

#[cfg(feature = "postgresql")]
pub(crate) fn infer_model_value_rust_type(value: &crate::model::Value) -> &'static str {
    infer_filter_value_rust_type(value)
}

/// 子查询 trait - 用于 is_in 方法
pub trait Subquery {
    /// 获取子查询的 SQL 和参数
    fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>);
}

impl FilterExpr {
    pub fn and(self, other: FilterExpr) -> Self {
        FilterExpr::And(Box::new(self), Box::new(other))
    }

    pub fn or(self, other: FilterExpr) -> Self {
        FilterExpr::Or(Box::new(self), Box::new(other))
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
}

impl OrderBy {
    pub fn asc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Asc,
            expr: None,
        }
    }

    pub fn desc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Desc,
            expr: None,
        }
    }

    pub fn asc_expr(expr: SqlExpr) -> Self {
        Self {
            column: expr.to_sql_no_params(crate::query::builder::default_db_type()),
            direction: OrderDirection::Asc,
            expr: Some(expr),
        }
    }

    pub fn desc_expr(expr: SqlExpr) -> Self {
        Self {
            column: expr.to_sql_no_params(crate::query::builder::default_db_type()),
            direction: OrderDirection::Desc,
            expr: Some(expr),
        }
    }

    pub(crate) fn cloned_expr(&self) -> Option<SqlExpr> {
        self.expr.clone()
    }

    /// 将 OrderBy 转换为 SQL 字符串
    pub fn to_sql(&self) -> String {
        let dir = match self.direction {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };
        let expr_sql = self
            .expr
            .as_ref()
            .map(|expr| expr.to_sql_no_params(crate::query::builder::default_db_type()))
            .unwrap_or_else(|| self.column.clone());
        format!("{} {}", expr_sql, dir)
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
}
