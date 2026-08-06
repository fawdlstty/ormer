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
}

/// 值类型（用于过滤）
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    BigInt(i128),
    Duration(std::time::Duration),
    Text(String),
    TextArray(Vec<String>),
    Real(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
    IntegerArray(Vec<i32>),
    BigIntArray(Vec<i64>),
    NullableBigIntArray(Vec<Option<i64>>),
    DateTime(chrono::DateTime<chrono::Utc>),
    Json(serde_json::Value),
    Uuid(uuid::Uuid),
    Null,
}

impl From<crate::model::Value> for Value {
    fn from(value: crate::model::Value) -> Self {
        match value {
            crate::model::Value::Integer(v) => Value::Integer(v),
            crate::model::Value::BigInt(v) => Value::BigInt(v),
            crate::model::Value::Duration(v) => Value::Duration(v),
            crate::model::Value::Text(v) => Value::Text(v),
            crate::model::Value::TextArray(v) => Value::TextArray(v),
            crate::model::Value::Real(v) => Value::Real(v),
            crate::model::Value::Boolean(v) => Value::Boolean(v),
            crate::model::Value::Bytes(v) => Value::Bytes(v),
            crate::model::Value::IntegerArray(v) => Value::IntegerArray(v),
            crate::model::Value::BigIntArray(v) => Value::BigIntArray(v),
            crate::model::Value::NullableBigIntArray(v) => Value::NullableBigIntArray(v),
            crate::model::Value::DateTime(v) => Value::DateTime(v),
            crate::model::Value::Json(v) => Value::Json(v),
            crate::model::Value::Uuid(v) => Value::Uuid(v),
            crate::model::Value::Null => Value::Null,
        }
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
        Value::Boolean(_) => "bool",
        Value::Bytes(_) => "Vec<u8>",
        Value::IntegerArray(_) => "Vec<i32>",
        Value::BigIntArray(_) => "Vec<i64>",
        Value::NullableBigIntArray(_) => "Vec<Option<i64>>",
        Value::DateTime(_) => "NaiveDateTime",
        Value::Json(_) => "String",
        Value::Uuid(_) => "String",
        Value::Null => "i32",
    }
}

#[cfg(feature = "postgresql")]
pub(crate) fn infer_model_value_rust_type(value: &crate::model::Value) -> &'static str {
    match value {
        crate::model::Value::Integer(_) => "i32",
        crate::model::Value::BigInt(_) => "i64",
        crate::model::Value::Duration(_) => "Duration",
        crate::model::Value::Text(_) => "String",
        crate::model::Value::TextArray(_) => "Vec<String>",
        crate::model::Value::Real(_) => "f64",
        crate::model::Value::Boolean(_) => "bool",
        crate::model::Value::Bytes(_) => "Vec<u8>",
        crate::model::Value::IntegerArray(_) => "Vec<i32>",
        crate::model::Value::BigIntArray(_) => "Vec<i64>",
        crate::model::Value::NullableBigIntArray(_) => "Vec<Option<i64>>",
        crate::model::Value::DateTime(_) => "NaiveDateTime",
        crate::model::Value::Json(_) => "String",
        crate::model::Value::Uuid(_) => "String",
        crate::model::Value::Null => "i32",
    }
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
#[derive(Debug, Clone)]
pub enum OrderDirection {
    Asc,
    Desc,
}

/// 排序表达式
#[derive(Debug, Clone)]
pub struct OrderBy {
    pub column: String,
    pub direction: OrderDirection,
}

impl OrderBy {
    pub fn asc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Asc,
        }
    }

    pub fn desc(column: String) -> Self {
        Self {
            column,
            direction: OrderDirection::Desc,
        }
    }

    /// 将 OrderBy 转换为 SQL 字符串
    pub fn to_sql(&self) -> String {
        let dir = match self.direction {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };
        format!("{} {}", self.column, dir)
    }

    /// 将 OrderBy 转换为指定后端的 SQL 字符串
    pub fn to_sql_for(&self, db_type: crate::abstract_layer::DbType) -> String {
        let dir = match self.direction {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };
        format!(
            "{} {}",
            crate::model::quote_column_reference(db_type, &self.column),
            dir
        )
    }
}
