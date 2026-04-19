use crate::model::Model;
use crate::query::filter::{FilterExpr, OrderBy};
use std::fmt::Write;
use std::marker::PhantomData;

/// Select 查询结构体
///
/// 使用方式:Select::<User>().filter(|p| p.age > 12).to_sql()
pub struct Select<T: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<T>,
}

impl<T: Model> Select<T> {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            _marker: PhantomData,
        }
    }

    /// 添加 WHERE 条件
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
    pub fn filter_cmp<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
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

    /// 限制结果数量
    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 设置偏移量
    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// 生成 SQL
    pub fn to_sql(&self) -> String {
        let (sql, _) = self.to_sql_with_params();
        sql
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();

        // SELECT 子句
        write!(
            &mut sql,
            "SELECT {} FROM {}",
            T::COLUMNS.join(", "),
            T::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(filter, &mut sql, &mut param_idx, &mut params);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self
                .order_by
                .iter()
                .map(|o| {
                    let dir = match o.direction {
                        crate::query::filter::OrderDirection::Asc => "ASC",
                        crate::query::filter::OrderDirection::Desc => "DESC",
                    };
                    format!("{} {}", o.column, dir)
                })
                .collect();
            sql.push_str(&order_strs.join(", "));
        }

        // LIMIT 子句
        if let Some(limit) = self.limit {
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }

        // OFFSET 子句
        if let Some(offset) = self.offset {
            write!(&mut sql, " OFFSET {}", offset).unwrap();
        }

        // 返回参数
        (sql, params)
    }

    fn format_filter(&self, filter: &FilterExpr, sql: &mut String, param_idx: &mut i32) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value: _,
            } => {
                write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                *param_idx += 1;
            }
            FilterExpr::And(left, right) => {
                self.format_filter(left, sql, param_idx);
                sql.push_str(" AND ");
                self.format_filter(right, sql, param_idx);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter(left, sql, param_idx);
                sql.push_str(" OR ");
                self.format_filter(right, sql, param_idx);
            }
        }
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                // 转换 filter Value 到 ormer Value
                let ormer_value = match value {
                    crate::query::filter::Value::Integer(v) => crate::model::Value::Integer(*v),
                    crate::query::filter::Value::Text(v) => crate::model::Value::Text(v.clone()),
                    crate::query::filter::Value::Real(v) => crate::model::Value::Real(*v),
                    crate::query::filter::Value::Null => crate::model::Value::Null,
                };
                params.push(ormer_value);
                *param_idx += 1;
            }
            FilterExpr::And(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params);
            }
        }
    }
}

// 实现 Default trait,支持 Select::<User>() 语法
impl<T: Model> Default for Select<T> {
    fn default() -> Self {
        Self::new()
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
pub struct WhereExpr {
    inner: FilterExpr,
}

impl From<WhereExpr> for FilterExpr {
    fn from(expr: WhereExpr) -> Self {
        expr.inner
    }
}

impl WhereExpr {
    pub fn and(self, other: WhereExpr) -> Self {
        Self {
            inner: FilterExpr::And(Box::new(self.inner), Box::new(other.inner)),
        }
    }

    pub fn or(self, other: WhereExpr) -> Self {
        Self {
            inner: FilterExpr::Or(Box::new(self.inner), Box::new(other.inner)),
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
        }
    }

    pub fn gt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }

    pub fn le(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }

    pub fn lt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }
}

/// 数值列代理 - 支持所有数值类型的比较操作
pub struct NumericColumn {
    column_name: &'static str,
}

impl NumericColumn {
    pub fn new(name: &'static str) -> Self {
        Self { column_name: name }
    }

    pub fn column_name(&self) -> &'static str {
        self.column_name
    }

    // 支持 .ge() .gt() .le() .lt() 等方法调用
    pub fn ge(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">=".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }

    pub fn gt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }

    pub fn le(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }

    pub fn lt(self, value: i32) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: crate::query::filter::Value::Integer(value as i64),
            },
        }
    }
}

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
    fn is_some(self) -> FilterExpr;
    fn is_none(self) -> FilterExpr;
    fn asc(self) -> OrderBy;
    fn desc(self) -> OrderBy;
}

/// 过滤值
#[derive(Debug, Clone)]
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
