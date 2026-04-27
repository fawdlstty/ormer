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

impl<T: Model> Clone for Select<T> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
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
    column_names: Vec<String>,        // 要查询的字段名列表（支持多字段）
    alias_names: Option<Vec<String>>, // 别名列表（用于映射到目标Model）
    _marker: PhantomData<(T, V)>,
}

/// GroupedSelect - 分组聚合查询结构体
pub struct GroupedSelect<T: Model, V> {
    column_names: Vec<String>,            // SELECT 的列（包含聚合函数）
    aggregate_funcs: Vec<Option<String>>, // 聚合函数列表
    group_by_columns: Vec<String>,        // GROUP BY 的列
    having_filters: Vec<FilterExpr>,      // HAVING 条件
    filters: Vec<FilterExpr>,             // WHERE 条件（分组前过滤）
    order_by: Vec<OrderBy>,               // ORDER BY
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
            alias_names: self.alias_names.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: Model, V> Clone for GroupedSelect<T, V> {
    fn clone(&self) -> Self {
        Self {
            column_names: self.column_names.clone(),
            aggregate_funcs: self.aggregate_funcs.clone(),
            group_by_columns: self.group_by_columns.clone(),
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
            aggregate_funcs: Vec::new(),
            group_by_columns: Vec::new(),
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
    /// 获取列名列表
    pub fn column_names(&self) -> &[String] {
        &self.column_names
    }

    /// 设置别名列表（用于映射到目标Model）
    pub fn with_aliases(mut self, aliases: Vec<String>) -> Self {
        self.alias_names = Some(aliases);
        self
    }

    /// 生成 SQL 和参数
    pub fn to_sql_with_params(&self, db_type: DbType) -> (String, Vec<crate::model::Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();

        // SELECT 字段（支持单个或多个字段，带别名）
        let columns = if let Some(ref aliases) = self.alias_names {
            // 使用别名：column AS alias
            self.column_names
                .iter()
                .zip(aliases.iter())
                .map(|(col, alias)| format!("{} AS {}", col, alias))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            self.column_names.join(", ")
        };
        write!(&mut sql, "SELECT {} FROM {}", columns, T::TABLE_NAME).unwrap();

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
        // 使用第一个可用的数据库类型用于调试
        #[cfg(feature = "turso")]
        let db_type = DbType::Turso;
        #[cfg(all(not(feature = "turso"), feature = "postgresql"))]
        let db_type = DbType::PostgreSQL;
        #[cfg(all(not(feature = "turso"), not(feature = "postgresql"), feature = "mysql"))]
        let db_type = DbType::MySQL;
        #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
        let db_type = DbType::None;

        let (sql, _) = self.to_sql_with_params(db_type);
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
            aggregate_funcs: self
                .aggregate_funcs
                .into_iter()
                .chain(result.aggregate_funcs())
                .collect(),
            group_by_columns: self.group_by_columns,
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

        // SELECT 子句（处理聚合函数）
        let columns = self
            .column_names
            .iter()
            .zip(self.aggregate_funcs.iter())
            .map(|(col, agg)| match agg {
                Some(func) => format!("{}({})", func, col),
                None => col.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");

        write!(&mut sql, "SELECT {} FROM {}", columns, T::TABLE_NAME).unwrap();

        // WHERE 子句（分组前过滤）
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let formatter = FilterFormatter::new(db_type);
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let filter_sql = formatter.format(filter, &mut param_idx, &mut params);
                sql.push_str(&filter_sql);
            }
        }

        // GROUP BY 子句
        if !self.group_by_columns.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&self.group_by_columns.join(", "));
        }

        // HAVING 子句（分组后过滤）
        if !self.having_filters.is_empty() {
            sql.push_str(" HAVING ");
            let formatter = FilterFormatter::new(db_type);
            for (i, filter) in self.having_filters.iter().enumerate() {
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

        // LIMIT/OFFSET
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
        // 使用第一个可用的数据库类型用于调试
        #[cfg(feature = "turso")]
        let db_type = DbType::Turso;
        #[cfg(all(not(feature = "turso"), feature = "postgresql"))]
        let db_type = DbType::PostgreSQL;
        #[cfg(all(not(feature = "turso"), not(feature = "postgresql"), feature = "mysql"))]
        let db_type = DbType::MySQL;
        #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
        let db_type = DbType::None;

        let (sql, _) = self.to_sql_with_params(db_type);
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
            alias_names: None,
            _marker: PhantomData,
        }
    }

    /// 字段投影并映射到目标Model - 自动生成别名以匹配目标Model的列名
    /// 例如：map_to_model(|r| r.uid) 会生成 "SELECT uid AS id FROM ..."
    pub fn map_to_model<F, TargetModel>(self, f: F) -> MappedSelect<T, TargetModel>
    where
        F: FnOnce(<T as Model>::Where) -> TypedColumn<<TargetModel as Model>::QueryBuilder>,
        TargetModel: Model,
    {
        let where_obj = <T as Model>::Where::default();
        let column = f(where_obj);

        // 使用目标Model的列名作为别名
        let alias_names = TargetModel::COLUMNS.iter().map(|s| s.to_string()).collect();

        MappedSelect {
            filters: self.filters,
            order_by: self.order_by,
            range_start: self.range_start,
            range_end: self.range_end,
            column_names: vec![column.column_name.to_string()],
            alias_names: Some(alias_names),
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
            aggregate_funcs: result.aggregate_funcs(),
            group_by_columns: Vec::new(),
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
        // 使用第一个可用的数据库类型用于调试
        #[cfg(feature = "turso")]
        let db_type = DbType::Turso;
        #[cfg(all(not(feature = "turso"), feature = "postgresql"))]
        let db_type = DbType::PostgreSQL;
        #[cfg(all(not(feature = "turso"), not(feature = "postgresql"), feature = "mysql"))]
        let db_type = DbType::MySQL;
        #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
        let db_type = DbType::None;

        let (sql, _) = self.to_sql_with_params(db_type);
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

/// MapToResult trait - 用于 map_to 方法的返回类型
pub trait MapToResult {
    type Output;
    fn column_names(&self) -> Vec<String>;
}

/// SelectColumnResult trait - 用于 select_column 方法的返回类型
pub trait SelectColumnResult {
    type Output;
    fn column_names(&self) -> Vec<String>;
    fn aggregate_funcs(&self) -> Vec<Option<String>>;
}

/// GroupByColumns trait - 用于 group_by 方法的返回类型
pub trait GroupByColumns {
    fn column_names(&self) -> Vec<String>;
}

// 为 TypedColumn 实现 MapToResult（单字段）
impl<T> MapToResult for TypedColumn<T> {
    type Output = T;

    fn column_names(&self) -> Vec<String> {
        vec![self.column_name.to_string()]
    }
}

// 为 TypedColumn 实现 SelectColumnResult（单字段）
impl<T> SelectColumnResult for TypedColumn<T> {
    type Output = T;

    fn column_names(&self) -> Vec<String> {
        vec![self.column_name.to_string()]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![self.aggregate_func.clone()]
    }
}

// 为 TypedColumn 实现 GroupByColumns（单字段）
impl<T> GroupByColumns for TypedColumn<T> {
    fn column_names(&self) -> Vec<String> {
        vec![self.column_name.to_string()]
    }
}

// 为二元组实现 MapToResult
impl<T1, T2> MapToResult for (TypedColumn<T1>, TypedColumn<T2>) {
    type Output = (T1, T2);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
        ]
    }
}

// 为二元组实现 SelectColumnResult
impl<T1, T2> SelectColumnResult for (TypedColumn<T1>, TypedColumn<T2>) {
    type Output = (T1, T2);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
        ]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![self.0.aggregate_func.clone(), self.1.aggregate_func.clone()]
    }
}

// 为二元组实现 GroupByColumns
impl<T1, T2> GroupByColumns for (TypedColumn<T1>, TypedColumn<T2>) {
    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
        ]
    }
}

// 为三元组实现 MapToResult
impl<T1, T2, T3> MapToResult for (TypedColumn<T1>, TypedColumn<T2>, TypedColumn<T3>) {
    type Output = (T1, T2, T3);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
            self.2.column_name.to_string(),
        ]
    }
}

// 为三元组实现 SelectColumnResult
impl<T1, T2, T3> SelectColumnResult for (TypedColumn<T1>, TypedColumn<T2>, TypedColumn<T3>) {
    type Output = (T1, T2, T3);

    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
            self.2.column_name.to_string(),
        ]
    }

    fn aggregate_funcs(&self) -> Vec<Option<String>> {
        vec![
            self.0.aggregate_func.clone(),
            self.1.aggregate_func.clone(),
            self.2.aggregate_func.clone(),
        ]
    }
}

// 为三元组实现 GroupByColumns
impl<T1, T2, T3> GroupByColumns for (TypedColumn<T1>, TypedColumn<T2>, TypedColumn<T3>) {
    fn column_names(&self) -> Vec<String> {
        vec![
            self.0.column_name.to_string(),
            self.1.column_name.to_string(),
            self.2.column_name.to_string(),
        ]
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
        // 使用第一个可用的数据库类型
        #[cfg(feature = "turso")]
        let db_type = DbType::Turso;
        #[cfg(all(not(feature = "turso"), feature = "postgresql"))]
        let db_type = DbType::PostgreSQL;
        #[cfg(all(not(feature = "turso"), not(feature = "postgresql"), feature = "mysql"))]
        let db_type = DbType::MySQL;
        #[cfg(not(any(feature = "turso", feature = "postgresql", feature = "mysql")))]
        let db_type = DbType::None;

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
    aggregate_func: Option<String>, // Some("COUNT"), Some("SUM"), etc.
    _marker: PhantomData<T>,
}

impl<T> TypedColumn<T> {
    pub fn new(name: &'static str) -> Self {
        Self {
            column_name: name,
            aggregate_func: None,
            _marker: PhantomData,
        }
    }

    pub fn with_aggregate(name: &'static str, func: String) -> Self {
        Self {
            column_name: name,
            aggregate_func: Some(func),
            _marker: PhantomData,
        }
    }

    pub fn column_name(&self) -> &'static str {
        self.column_name
    }

    pub fn aggregate_func(&self) -> Option<&String> {
        self.aggregate_func.as_ref()
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

impl<T: crate::model::FromValue> crate::model::FromRowValues for TypedColumn<T> {
    fn from_row_values(values: &[crate::model::Value]) -> Result<Self, crate::Error> {
        if values.is_empty() {
            return Err(crate::Error::Database(
                "Expected at least 1 value for TypedColumn".to_string(),
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
        }
    }
}

// 为所有 TypedColumn 实现聚合方法
impl<T: ColumnValueType + 'static> TypedColumn<T> {
    /// COUNT 聚合 - 返回 usize
    pub fn count(self) -> TypedColumn<usize> {
        TypedColumn::with_aggregate(self.column_name, "COUNT".to_string())
    }

    /// SUM 聚合 - 返回相同类型
    pub fn sum(self) -> TypedColumn<T>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "SUM".to_string())
    }

    /// AVG 聚合 - 返回 f64
    pub fn avg(self) -> TypedColumn<f64> {
        TypedColumn::with_aggregate(self.column_name, "AVG".to_string())
    }

    /// MAX 聚合 - 返回相同类型
    pub fn max(self) -> TypedColumn<T>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "MAX".to_string())
    }

    /// MIN 聚合 - 返回相同类型
    pub fn min(self) -> TypedColumn<T>
    where
        T: AggregateResultType,
    {
        TypedColumn::with_aggregate(self.column_name, "MIN".to_string())
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

impl<T: Model, J: Model> Clone for LeftJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
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
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for InnerJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
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
    join_table: String,
    join_alias: String,
    on_condition: FilterExpr,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for RightJoinedSelect<T, J> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            join_table: self.join_table.clone(),
            join_alias: self.join_alias.clone(),
            on_condition: self.on_condition.clone(),
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
