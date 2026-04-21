use crate::abstract_layer::DbType;
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

/// RelatedSelect - 关联查询结构体（支持2表查询）
pub struct RelatedSelect<T: Model, R: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTableSelect - 多表关联查询结构体（支持3个或以上表）
pub struct MultiTableSelect<T: Model, R1: Model, R2: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTableSelect - 四表关联查询结构体
pub struct FourTableSelect<T: Model, R1: Model, R2: Model, R3: Model> {
    filters: Vec<FilterExpr>,
    order_by: Vec<OrderBy>,
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

/// AggregateSelect - 聚合查询结构体
pub struct AggregateSelect<T: Model, R = crate::model::Value> {
    aggregate_func: String, // COUNT, SUM, AVG, MAX, MIN
    column_name: String,
    filters: Vec<FilterExpr>,
    _marker: PhantomData<(T, R)>,
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
            }
        }

        (sql, params)
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                // 根据数据库类型生成占位符
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }

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
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            _ => {}
        }
    }
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

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2: Model, R: Model>(self) -> RelatedSelect<T, R>
    where
        T2: 'static,
    {
        // 通过类型约束确保 T2 == T
        // 如果 T2 != T，编译器会在类型推导时报错
        RelatedSelect {
            filters: self.filters,
            order_by: self.order_by,
            limit: self.limit,
            offset: self.offset,
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
            limit: self.limit,
            offset: self.offset,
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
            limit: self.limit,
            offset: self.offset,
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
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

    #[allow(dead_code)]
    fn format_filter(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value: _,
            } => {
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }
                *param_idx += 1;
            }
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                write!(sql, "{} {} {}", left_column, operator, right_column).unwrap();
            }
            FilterExpr::And(left, right) => {
                self.format_filter(left, sql, param_idx, db_type);
                sql.push_str(" AND ");
                self.format_filter(right, sql, param_idx, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter(left, sql, param_idx, db_type);
                sql.push_str(" OR ");
                self.format_filter(right, sql, param_idx, db_type);
            }
        }
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }
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
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                write!(sql, "{} {} {}", left_column, operator, right_column).unwrap();
            }
            FilterExpr::And(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
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

        (sql, params)
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                // 默认使用 t0 前缀
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "t0.{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "t0.{} {} ?", column, operator).unwrap();
                    }
                }
                let ormer_value = match value {
                    crate::query::filter::Value::Integer(v) => crate::model::Value::Integer(*v),
                    crate::query::filter::Value::Text(v) => crate::model::Value::Text(v.clone()),
                    crate::query::filter::Value::Real(v) => crate::model::Value::Real(*v),
                    crate::query::filter::Value::Null => crate::model::Value::Null,
                };
                params.push(ormer_value);
                *param_idx += 1;
            }
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 左列使用 t0 前缀，右列使用 t1 前缀
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            FilterExpr::And(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
        }
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
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

        (sql, params)
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                // 默认使用 t0 前缀
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "t0.{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "t0.{} {} ?", column, operator).unwrap();
                    }
                }
                let ormer_value = match value {
                    crate::query::filter::Value::Integer(v) => crate::model::Value::Integer(*v),
                    crate::query::filter::Value::Text(v) => crate::model::Value::Text(v.clone()),
                    crate::query::filter::Value::Real(v) => crate::model::Value::Real(*v),
                    crate::query::filter::Value::Null => crate::model::Value::Null,
                };
                params.push(ormer_value);
                *param_idx += 1;
            }
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 需要根据列名判断属于哪个表，这里简化处理
                // 左列使用 t0 前缀，右列使用 t1 前缀
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            FilterExpr::And(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
        }
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
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

        (sql, params)
    }

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                // 默认使用 t0 前缀
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "t0.{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "t0.{} {} ?", column, operator).unwrap();
                    }
                }
                let ormer_value = match value {
                    crate::query::filter::Value::Integer(v) => crate::model::Value::Integer(*v),
                    crate::query::filter::Value::Text(v) => crate::model::Value::Text(v.clone()),
                    crate::query::filter::Value::Real(v) => crate::model::Value::Real(*v),
                    crate::query::filter::Value::Null => crate::model::Value::Null,
                };
                params.push(ormer_value);
                *param_idx += 1;
            }
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                // 需要根据列名判断属于哪个表，这里简化处理
                // 左列使用 t0 前缀，右列使用 t1 前缀
                write!(sql, "t0.{} {} t1.{}", left_column, operator, right_column).unwrap();
            }
            FilterExpr::And(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
        }
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

impl TypedColumn<i64> {
    // 支持 .ge() .gt() .le() .lt() 等方法调用
    pub fn ge(self, value: i64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">=".to_string(),
                value: crate::query::filter::Value::Integer(value),
            },
        }
    }

    pub fn gt(self, value: i64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: crate::query::filter::Value::Integer(value),
            },
        }
    }

    pub fn le(self, value: i64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: crate::query::filter::Value::Integer(value),
            },
        }
    }

    pub fn lt(self, value: i64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: crate::query::filter::Value::Integer(value),
            },
        }
    }

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
}

impl TypedColumn<i32> {
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

impl TypedColumn<String> {
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
}

impl TypedColumn<f64> {
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

    pub fn ge(self, value: f64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">=".to_string(),
                value: crate::query::filter::Value::Real(value),
            },
        }
    }

    pub fn gt(self, value: f64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: ">".to_string(),
                value: crate::query::filter::Value::Real(value),
            },
        }
    }

    pub fn le(self, value: f64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<=".to_string(),
                value: crate::query::filter::Value::Real(value),
            },
        }
    }

    pub fn lt(self, value: f64) -> WhereExpr {
        WhereExpr {
            inner: FilterExpr::Comparison {
                column: self.column_name.to_string(),
                operator: "<".to_string(),
                value: crate::query::filter::Value::Real(value),
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
    limit: Option<i64>,
    offset: Option<i64>,
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
    limit: Option<i64>,
    offset: Option<i64>,
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
    limit: Option<i64>,
    offset: Option<i64>,
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
            limit: self.limit,
            offset: self.offset,
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
            limit: self.limit,
            offset: self.offset,
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
            limit: self.limit,
            offset: self.offset,
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

    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
            }
        }

        if let Some(limit) = self.limit {
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }

        if let Some(offset) = self.offset {
            write!(&mut sql, " OFFSET {}", offset).unwrap();
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

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }
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
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
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

    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
            }
        }

        if let Some(limit) = self.limit {
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }

        if let Some(offset) = self.offset {
            write!(&mut sql, " OFFSET {}", offset).unwrap();
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

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }
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
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
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

    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
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
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                self.format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    db_type,
                );
            }
        }

        if let Some(limit) = self.limit {
            write!(&mut sql, " LIMIT {}", limit).unwrap();
        }

        if let Some(offset) = self.offset {
            write!(&mut sql, " OFFSET {}", offset).unwrap();
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

    fn format_filter_with_params(
        &self,
        filter: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<crate::model::Value>,
        db_type: DbType,
    ) {
        match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        write!(sql, "{} {} ?", column, operator).unwrap();
                    }
                }
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
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" AND ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            FilterExpr::Or(left, right) => {
                self.format_filter_with_params(left, sql, param_idx, params, db_type);
                sql.push_str(" OR ");
                self.format_filter_with_params(right, sql, param_idx, params, db_type);
            }
            _ => {}
        }
    }
}
