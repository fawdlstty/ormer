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
    /// AND 连接
    And(Box<FilterExpr>, Box<FilterExpr>),
    /// OR 连接
    Or(Box<FilterExpr>, Box<FilterExpr>),
}

/// 值类型（用于过滤）
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    Text(String),
    Real(f64),
    Null,
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
}
