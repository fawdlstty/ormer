use crate::abstract_layer::DbType;
use crate::model::Model;
use crate::query::filter::{FilterExpr, OrderBy};
use crate::query::filter_formatter::FilterFormatter;
use std::fmt::Write;
use std::marker::PhantomData;

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

/// Select 查询结构体
///
/// 使用方式:Select::<User>().filter(|p| p.age > 12).to_sql()
pub struct Select<T: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    _marker: PhantomData<T>,
}

/// RelatedSelect - 关联查询结构体(支持2表查询)
pub struct RelatedSelect<T: Model, R: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTableSelect - 多表关联查询结构体(支持3个或以上表)
pub struct MultiTableSelect<T: Model, R1: Model, R2: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTableSelect - 四表关联查询结构体
pub struct FourTableSelect<T: Model, R1: Model, R2: Model, R3: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
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
    column_name: String, // 要查询的字段名
    _marker: PhantomData<(T, V)>,
}

impl<T: Model, V> Clone for MappedSelect<T, V> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            column_name: self.column_name.clone(),
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
            self.column_name,
            T::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            let formatter = FilterFormatter::new(db_type);
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        (sql, params)
    }
}

impl<T: Model, V> MappedSelect<T, V> {
    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();

        // SELECT 单个字段
        write!(
            &mut sql,
            "SELECT {} FROM {}",
            self.column_name,
            T::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            let formatter = FilterFormatter::new(db_type);
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self.order_by.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&order_strs.join(", "));
        }

        // RANGE 子句 (LIMIT + OFFSET)
        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }

    /// 生成 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        // 默认使用Turso格式用于调试
        let (sql, _) = self.to_sql_with_params(DbType::Turso);
        sql
    }
}

impl<T: Model> Select<T> {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            order_by: Vec::new(),
            range_start: None,
            range_end: None,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2: Model, R: Model>(self) -> RelatedSelect<T, R>
    where
        T2: 'static,
    {
        // 通过类型约束确保 T2 == T
        // 如果 T2 != T,编译器会在类型推导时报错
        RelatedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2: Model, R1: Model, R2: Model>(self) -> MultiTableSelect<T, R1, R2>
    where
        T2: 'static,
    {
        MultiTableSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2: Model, R1: Model, R2: Model, R3: Model>(self) -> FourTableSelect<T, R1, R2, R3>
    where
        T2: 'static,
    {
        FourTableSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
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
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C>,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("COUNT", column.column_name())
    }

    /// SUM 聚合函数 - 编译期类型推断
    pub fn sum<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("SUM", column.column_name())
    }

    /// AVG 聚合函数 - 总是返回 f64
    pub fn avg<F, C>(self, f: F) -> AggregateSelect<T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("AVG", column.column_name())
    }

    /// MAX 聚合函数 - 编译期类型推断
    pub fn max<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("MAX", column.column_name())
    }

    /// MIN 聚合函数 - 编译期类型推断
    pub fn min<F, C>(self, f: F) -> AggregateSelect<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<C>,
        C: AggregateResultType + 'static,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        self.aggregate_typed("MIN", column.column_name())
    }

    /// 字段投影 - 将查询结果映射到单个字段
    pub fn map_to<F, V>(self, f: F) -> MappedSelect<T, V>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<V>,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);
        MappedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            column_name: column.column_name().to_string(),
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

    /// 生成 SQL
    pub fn to_sql(&self) -> String {
        // 默认使用Turso格式用于调试
        let (sql, _) = self.to_sql_with_params(DbType::Turso);
        sql
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
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
            let formatter = FilterFormatter::new(db_type);
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self.order_by.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&order_strs.join(", "));
        }

        // RANGE 子句 (LIMIT + OFFSET)
        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        // 返回参数
        (sql, params)
    }
}

// 实现 Default trait,支持 Select::<User>() 语法
impl<T: Model> Default for Select<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Model, R: Model> RelatedSelect<T, R> {
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
        write!(
            &mut sql,
            "SELECT {} FROM {} AS t0, {} AS t1",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            R::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self.order_by.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&order_strs.join(", "));
        }

        // RANGE 子句 (LIMIT + OFFSET)
        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }
}

impl<T: Model, R1: Model, R2: Model> MultiTableSelect<T, R1, R2> {
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
        write!(
            &mut sql,
            "SELECT {} FROM {} AS t0, {} AS t1, {} AS t2",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            R1::TABLE_NAME,
            R2::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self.order_by.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&order_strs.join(", "));
        }

        // RANGE 子句 (LIMIT + OFFSET)
        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }
}

impl<T: Model, R1: Model, R2: Model, R3: Model> FourTableSelect<T, R1, R2, R3> {
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
        write!(
            &mut sql,
            "SELECT {} FROM {} AS t0, {} AS t1, {} AS t2, {} AS t3",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            R1::TABLE_NAME,
            R2::TABLE_NAME,
            R3::TABLE_NAME
        )
        .unwrap();

        // WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // ORDER BY 子句
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_strs: Vec<String> = self.order_by.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&order_strs.join(", "));
        }

        // RANGE 子句 (LIMIT + OFFSET)
        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

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
pub struct WhereExpr {
    inner: FilterExpr,
}

impl From<WhereExpr> for FilterExpr {
    fn from(expr: WhereExpr) -> Self {
        expr.inner
    }
}

impl WhereExpr {
    pub fn from_filter(inner: FilterExpr) -> Self {
        Self { inner }
    }

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
        }
    }
}

// 为 MappedSelect 实现 IsInValues（子查询）
impl<T: Model, V: ColumnValueType> IsInValues<V> for MappedSelect<T, V> {
    fn to_in_expr(self, column: String) -> WhereExpr {
        let db_type = DbType::Turso;
        let (sql, params) = self.to_sql_with_params(db_type);
        WhereExpr {
            inner: FilterExpr::InSubquery {
                column,
                subquery_sql: sql,
                subquery_params: params,
            },
        }
    }
}

/// 类型化列代理 - 携带字段类型信息
pub struct TypedColumn<T> {
    column_name: &'static str,
    _marker: PhantomData<T>,
}

impl<T> TypedColumn<T> {
    pub fn new(name: &'static str) -> Self {
        Self {
            column_name: name,
            _marker: PhantomData,
        }
    }

    pub fn column_name(&self) -> &'static str {
        self.column_name
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

impl<T> From<TypedColumn<T>> for OrderBy {
    fn from(col: TypedColumn<T>) -> Self {
        OrderBy::asc(col.column_name.to_string())
    }
}

// 保留 NumericColumn 作为类型别名向后兼容
pub type NumericColumn = TypedColumn<i64>;

/// 列值 - 支持字面量或列引用
pub enum ColumnValue {
    Literal(crate::query::filter::Value),
    ColumnRef(String),
}

impl From<i32> for ColumnValue {
    fn from(v: i32) -> Self {
        ColumnValue::Literal(crate::query::filter::Value::Integer(v as i64))
    }
}

impl From<String> for ColumnValue {
    fn from(v: String) -> Self {
        ColumnValue::Literal(crate::query::filter::Value::Text(v))
    }
}

impl From<&str> for ColumnValue {
    fn from(v: &str) -> Self {
        ColumnValue::Literal(crate::query::filter::Value::Text(v.to_string()))
    }
}

impl<T> From<TypedColumn<T>> for ColumnValue {
    fn from(col: TypedColumn<T>) -> Self {
        ColumnValue::ColumnRef(col.column_name.to_string())
    }
}

// ==================== TypedColumn 泛型实现 ====================
// 为所有实现了 ColumnValueType 的类型提供统一的方法

impl<T: ColumnValueType> TypedColumn<T> {
    /// 等于比较 - 支持字面量或列引用
    pub fn eq(self, value: impl Into<ColumnValue>) -> WhereExpr {
        match value.into() {
            ColumnValue::Literal(v) => WhereExpr {
                inner: FilterExpr::Comparison {
                    column: self.column_name.to_string(),
                    operator: "=".to_string(),
                    value: v,
                },
            },
            ColumnValue::ColumnRef(other_column) => WhereExpr {
                inner: FilterExpr::ColumnComparison {
                    left_column: self.column_name.to_string(),
                    operator: "=".to_string(),
                    right_column: other_column,
                },
            },
        }
    }

    /// IN 语句 - 支持多种集合类型和子查询
    pub fn is_in(self, values: impl IsInValues<T>) -> WhereExpr {
        values.to_in_expr(self.column_name.to_string())
    }
}

// 为支持比较操作的类型（整数和浮点数）实现比较方法
impl<T: ColumnValueType> TypedColumn<T> {
    /// 大于等于
    pub fn ge(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">=".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
        }
    }

    /// 大于
    pub fn gt(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
        }
    }

    /// 小于等于
    pub fn le(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: ColumnValueType::to_filter_value(value),
            },
        }
    }

    /// 小于
    pub fn lt(self, value: T) -> WhereExpr {
        debug_assert!(
            T::supports_comparison(),
            "Type does not support comparison operations"
        );
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: ColumnValueType::to_filter_value(value),
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
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询结构体
#[allow(dead_code)]
pub struct InnerJoinedSelect<T: Model, J: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询结构体
#[allow(dead_code)]
pub struct RightJoinedSelect<T: Model, J: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    range_start: Option<usize>,
    range_end: Option<usize>,
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    _marker: PhantomData<(T, J)>,
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

        LeftJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
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

        InnerJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
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

        RightJoinedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: J::TABLE_NAME.to_string(),
            join_alias: "t1".to_string(),
            on_condition: expr.into(),
            _marker: PhantomData,
        }
    }
}

impl<T: Model, J: Model> LeftJoinedSelect<T, J> {
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
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        write!(
            &mut sql,
            "SELECT {}, {} FROM {} AS t0 LEFT JOIN {} AS {}",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            J::COLUMNS
                .iter()
                .map(|c| format!("t1.{} as j_{}", c, c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            self.join_table,
            self.join_alias
        )
        .unwrap();

        sql.push_str(" ON ");
        self.format_join_condition(&self.on_condition, &mut sql);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }

    fn format_join_condition(&self, filter: &FilterExpr, sql: &mut String) {
        match filter {
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 左列加 t0. 前缀（主表），右列加 t1. 前缀（JOIN表）
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            _ => {}
        }
    }
}

impl<T: Model, J: Model> InnerJoinedSelect<T, J> {
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
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        write!(
            &mut sql,
            "SELECT {}, {} FROM {} AS t0 INNER JOIN {} AS {}",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            J::COLUMNS
                .iter()
                .map(|c| format!("t1.{} as j_{}", c, c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            self.join_table,
            self.join_alias
        )
        .unwrap();

        sql.push_str(" ON ");
        self.format_join_condition(&self.on_condition, &mut sql);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }

    fn format_join_condition(&self, filter: &FilterExpr, sql: &mut String) {
        match filter {
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 左列加 t0. 前缀（主表），右列加 t1. 前缀（JOIN表）
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            _ => {}
        }
    }
}

impl<T: Model, J: Model> RightJoinedSelect<T, J> {
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
        let mut sql = String::new();
        let mut params = Vec::new();
        let mut param_idx = 1;

        write!(
            &mut sql,
            "SELECT {}, {} FROM {} AS t0 RIGHT JOIN {} AS {}",
            T::COLUMNS
                .iter()
                .map(|c| format!("t0.{}", c))
                .collect::<Vec<_>>()
                .join(", "),
            J::COLUMNS
                .iter()
                .map(|c| format!("t1.{} as j_{}", c, c))
                .collect::<Vec<_>>()
                .join(", "),
            T::TABLE_NAME,
            self.join_table,
            self.join_alias
        )
        .unwrap();

        sql.push_str(" ON ");
        self.format_join_condition(&self.on_condition, &mut sql);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type)
                .with_table_prefix("t0")
                .with_right_table_prefix("t1");
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        if let Some(end) = self.range_end {
            let limit = if let Some(start) = self.range_start {
                end - start
            } else {
                end
            };
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }
        if let Some(start) = self.range_start {
            write!(&mut sql, " OFFSET {}", start).unwrap();
        }

        (sql, params)
    }

    fn format_join_condition(&self, filter: &FilterExpr, sql: &mut String) {
        match filter {
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 左列加 t0. 前缀（主表），右列加 t1. 前缀（JOIN表）
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            _ => {}
        }
    }
}
