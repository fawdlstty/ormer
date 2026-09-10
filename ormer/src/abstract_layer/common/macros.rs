/// 宏定义 - 用于减少重复代码
///
/// 本文件包含用于生成重复代码模式的宏
/// 为 Insert Executor 生成通用的 conflict 配置方法
#[macro_export]
macro_rules! impl_insert_conflict_methods {
    ($executor_type:ident $(, $with_conflict:ident)?) => {
        impl<'a, I: $crate::model::Insertable> $executor_type<'a, I> {
            fn conflict_mut(&mut self) -> &mut $crate::query::insert::InsertConflict {
                self.conflict
                    .get_or_insert_with($crate::query::insert::InsertConflict::default)
            }

            $(
                pub(crate) fn $with_conflict(
                    mut self,
                    conflict: Option<$crate::query::insert::InsertConflict>,
                ) -> Self {
                    self.conflict = conflict;
                    self
                }
            )?

            pub fn on_conflict<F, C>(mut self, f: F) -> Self
            where
                F: FnOnce(<I::Model as $crate::Model>::Where) -> C,
                C: $crate::query::insert::ConflictColumns,
            {
                self.conflict_mut().target = Some(
                    $crate::query::insert::InsertConflictTarget::Columns(
                        f(<I::Model as $crate::Model>::Where::default()).conflict_columns(),
                    ),
                );
                self
            }

            pub fn on_constraint<Target>(mut self, target: Target) -> Self
            where
                Target: $crate::query::insert::IntoInsertConflictTarget<I::Model>,
            {
                self.conflict_mut().target = Some(target.into_insert_conflict_target());
                self
            }

            pub fn conflict_where<F, W>(mut self, f: F) -> Self
            where
                F: FnOnce(<I::Model as $crate::Model>::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                self.conflict_mut().target_filter =
                    Some($crate::query::insert::where_expr_to_filter(f(
                        <I::Model as $crate::Model>::Where::default(),
                    )));
                self
            }

            pub fn do_nothing(mut self) -> Self {
                self.conflict_mut().action =
                    Some($crate::query::insert::InsertConflictAction::DoNothing);
                self
            }

            pub fn do_update(mut self) -> Self {
                self.conflict_mut().action =
                    Some($crate::query::insert::InsertConflictAction::DoUpdate);
                self
            }

            pub fn do_update_if<F, W>(mut self, f: F) -> Self
            where
                F: FnOnce(<I::Model as $crate::Model>::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                let conflict = self.conflict_mut();
                conflict.action = Some($crate::query::insert::InsertConflictAction::DoUpdate);
                conflict.update_filter =
                    Some($crate::query::insert::where_expr_to_filter(f(
                        <I::Model as $crate::Model>::Where::default(),
                    )));
                self
            }

            pub fn set<F>(mut self, f: F) -> Self
            where
                F: FnOnce(&mut <I::Model as $crate::Model>::Update),
            {
                let mut update = <I::Model as $crate::Model>::Update::default();
                f(&mut update);
                let conflict = self.conflict_mut();
                conflict
                    .action
                    .get_or_insert($crate::query::insert::InsertConflictAction::DoUpdate);
                conflict.assignments.extend(
                    <<I::Model as $crate::Model>::Update as $crate::query::update::UpdateFields>::assignments(
                        &update,
                    ),
                );
                self
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_backend_select_methods {
    ($conn_field:ident) => {
        pub fn filter<F, W>(self, f: F) -> Self
        where
            F: FnOnce(T::Where) -> W,
            W: Into<$crate::WhereExpr>,
        {
            Self {
                select: self.select.filter(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn append_filter_expr(self, expr: $crate::WhereExpr) -> Self {
            Self {
                select: $crate::query::builder::FilterQuery::<T>::append_filter_expr(
                    self.select,
                    expr,
                ),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub(crate) fn with_context_filters(
            self,
            filters: Vec<$crate::query::builder::ContextFilter>,
        ) -> Self {
            Self {
                select: self.select.with_context_filters(filters),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn without_filter(self, name: &'static str) -> Self {
            Self {
                select: $crate::query::builder::WithoutFilterQuery::<T>::without_filter(
                    self.select,
                    name,
                ),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn filter_dynamic<F>(self, f: F) -> Self
        where
            F: FnOnce($crate::query::builder::DynamicColumnSet<T>) -> $crate::WhereExpr,
        {
            Self {
                select: self.select.filter_dynamic(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn route_table(
            self,
            key: impl Into<String>,
            value: impl $crate::model::TableRouteValue,
        ) -> Self {
            Self {
                select: self.select.route_table(key, value),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn fields<F, G>(self, f: F) -> Self
        where
            F: FnOnce(T::Where) -> G,
            G: $crate::query::builder::GroupByColumns,
        {
            Self {
                select: self.select.fields(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn query(self, query: impl Into<String>) -> Self {
            Self {
                select: self.select.query(query),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn mode(self, mode: $crate::query::filter::FullTextMode) -> Self {
            Self {
                select: self.select.mode(mode),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn language(self, language: impl Into<String>) -> Self {
            Self {
                select: self.select.language(language),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn rank(self, rank: $crate::query::filter::FullTextRank) -> Self {
            Self {
                select: self.select.rank(rank),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn with_table_route(self, route: $crate::model::TableRoute) -> Self {
            Self {
                select: self.select.with_table_route(route),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn order_by<F, O>(self, f: F) -> Self
        where
            F: FnOnce(<T as $crate::Model>::Where) -> O,
            O: Into<$crate::OrderBy>,
        {
            Self {
                select: self.select.order_by(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn order_by_desc<F, O>(self, f: F) -> Self
        where
            F: FnOnce(<T as $crate::Model>::Where) -> O,
            O: Into<$crate::OrderBy>,
        {
            Self {
                select: self.select.order_by_desc(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn order_by_dynamic<F>(self, f: F) -> Self
        where
            F: FnOnce($crate::query::builder::DynamicColumnSet<T>) -> $crate::OrderBy,
        {
            Self {
                select: self.select.order_by_dynamic(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
            Self {
                select: self.select.range(range),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn cursor_by<F, G>(self, f: F) -> Self
        where
            F: FnOnce(T::Where) -> G,
            G: $crate::query::builder::GroupByColumns,
        {
            Self {
                select: self.select.cursor_by(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn after<C>(self, cursor: C) -> Self
        where
            C: Into<$crate::query::builder::PageCursor>,
        {
            Self {
                select: self.select.after(cursor),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn before<C>(self, cursor: C) -> Self
        where
            C: Into<$crate::query::builder::PageCursor>,
        {
            Self {
                select: self.select.before(cursor),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn limit(self, limit: usize) -> Self {
            Self {
                select: self.select.limit(limit),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn descendants<F, C>(self, f: F, root_id: impl Into<$crate::Value>) -> Self
        where
            F: FnOnce(T::Where) -> C,
            C: $crate::query::builder::RecursiveColumns<T>,
        {
            Self {
                select: self.select.descendants(f, root_id),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn ancestors<F, C>(self, f: F, leaf_id: impl Into<$crate::Value>) -> Self
        where
            F: FnOnce(T::Where) -> C,
            C: $crate::query::builder::RecursiveColumns<T>,
        {
            Self {
                select: self.select.ancestors(f, leaf_id),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };

    ($conn_field:ident, distinct) => {
        $crate::__ormer_backend_select_methods!($conn_field);

        pub fn distinct(self) -> Self {
            Self {
                select: self.select.distinct(),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_backend_join_methods {
    ($conn_field:ident) => {
        pub fn filter<F, W>(self, f: F) -> Self
        where
            F: FnOnce(T::Where) -> W,
            W: Into<$crate::WhereExpr>,
        {
            Self {
                select: self.select.filter(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
            Self {
                select: self.select.range(range),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_backend_related_methods {
    ($conn_field:ident) => {
        pub fn filter<F, W>(self, f: F) -> Self
        where
            F: FnOnce(T::Where, R::Where) -> W,
            W: Into<$crate::WhereExpr>,
        {
            Self {
                select: self.select.filter(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
            Self {
                select: self.select.range(range),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_backend_multi_table_methods {
    ($conn_field:ident) => {
        pub fn filter<F, W>(self, f: F) -> Self
        where
            F: FnOnce(T::Where, R1::Where, R2::Where) -> W,
            W: Into<$crate::WhereExpr>,
        {
            Self {
                select: self.select.filter(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
            Self {
                select: self.select.range(range),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_backend_four_table_methods {
    ($conn_field:ident) => {
        pub fn filter<F, W>(self, f: F) -> Self
        where
            F: FnOnce(T::Where, R1::Where, R2::Where, R3::Where) -> W,
            W: Into<$crate::WhereExpr>,
        {
            Self {
                select: self.select.filter(f),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
            Self {
                select: self.select.range(range),
                $conn_field: self.$conn_field,
                _marker: std::marker::PhantomData,
            }
        }
    };
}

/// 为 JOIN Executor 生成通用的 filter/range 方法
#[macro_export]
macro_rules! impl_join_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty
    ) => {
        impl<T: $crate::Model, J: $crate::Model> $executor_type<T, J> {
            $crate::__ormer_backend_join_methods!($conn_field);
        }
    };
}

/// 为 CollectFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_collect_future {
    (
        $future_type:ident,
        $output_type:ty,
        lifetime: $($lt:lifetime),*
    ) => {
        impl<$($lt,)* T: $crate::Model + 'static, J: $crate::Model + 'static>
            std::future::IntoFuture for $future_type<$($lt,)* T, J>
        {
            type Output = $output_type;
            type IntoFuture = std::pin::Pin<
                Box<dyn std::future::Future<Output = Self::Output> + $($lt +)* 'static>
            >;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move { self.executor.collect_inner().await })
            }
        }
    };
}

/// 为 CollectFuture (单表) 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_single_collect_future {
    (
        $future_type:ident,
        $output_type:ty,
        lifetime: $($lt:lifetime),*
    ) => {
        impl<$($lt,)* T: $crate::Model + 'static, C: FromIterator<T> + 'static>
            std::future::IntoFuture for $future_type<$($lt,)* T, C>
        {
            type Output = $output_type;
            type IntoFuture = std::pin::Pin<
                Box<dyn std::future::Future<Output = Self::Output> + $($lt +)* 'static>
            >;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move { self.executor.collect_inner().await })
            }
        }
    };
}

/// 为 Executor 生成通用的方法 (filter/order_by/range)
#[macro_export]
macro_rules! impl_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty
    ) => {
        impl<'a, T: $crate::Model> $executor_type<'a, T> {
            $crate::__ormer_backend_select_methods!($conn_field);
        }
    };
}

/// 为统一的 SelectExecutor 生成方法（filter/order_by/range）
#[macro_export]
macro_rules! impl_unified_select_executor_methods {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F, W>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.filter(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.filter(f))
                    }
                }
            }

            pub fn append_filter_expr(self, expr: $crate::WhereExpr) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.append_filter_expr(expr))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.append_filter_expr(expr))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.append_filter_expr(expr))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.append_filter_expr(expr))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.append_filter_expr(expr))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.append_filter_expr(expr))
                    }
                }
            }

            pub(crate) fn with_context_filters(
                self,
                filters: Vec<$crate::query::builder::ContextFilter>,
            ) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.with_context_filters(filters))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.with_context_filters(filters))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.with_context_filters(filters))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.with_context_filters(filters))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.with_context_filters(filters))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.with_context_filters(filters))
                    }
                }
            }

            pub fn without_filter(self, name: &'static str) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.without_filter(name))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.without_filter(name))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.without_filter(name)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.without_filter(name)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.without_filter(name))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.without_filter(name))
                    }
                }
            }

            pub fn filter_dynamic<F>(self, f: F) -> Self
            where
                F: FnOnce($crate::query::builder::DynamicColumnSet<T>) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.filter_dynamic(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.filter_dynamic(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter_dynamic(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter_dynamic(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter_dynamic(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.filter_dynamic(f))
                    }
                }
            }

            pub fn route_table(
                self,
                key: impl Into<String>,
                value: impl $crate::model::TableRouteValue,
            ) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.route_table(key, value))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.route_table(key, value))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.route_table(key, value))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.route_table(key, value))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.route_table(key, value))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.route_table(key, value))
                    }
                }
            }

            pub fn with_table_route(self, route: $crate::model::TableRoute) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.with_table_route(route))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.with_table_route(route))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.with_table_route(route))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.with_table_route(route))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.with_table_route(route))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.with_table_route(route))
                    }
                }
            }

            pub fn order_by<F, O>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> O,
                O: Into<$crate::OrderBy>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.order_by(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.order_by(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.order_by(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.order_by(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.order_by(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.order_by(f))
                    }
                }
            }

            pub fn order_by_desc<F, O>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> O,
                O: Into<$crate::OrderBy>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.order_by_desc(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.order_by_desc(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.order_by_desc(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.order_by_desc(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.order_by_desc(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.order_by_desc(f))
                    }
                }
            }

            pub fn order_by_dynamic<F>(self, f: F) -> Self
            where
                F: FnOnce($crate::query::builder::DynamicColumnSet<T>) -> $crate::OrderBy,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.order_by_dynamic(f))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.order_by_dynamic(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.order_by_dynamic(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.order_by_dynamic(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.order_by_dynamic(f))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.order_by_dynamic(f))
                    }
                }
            }

            pub fn cursor_by<F, G>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> G,
                G: $crate::query::builder::GroupByColumns,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.cursor_by(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.cursor_by(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.cursor_by(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.cursor_by(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.cursor_by(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.cursor_by(f))
                    }
                }
            }

            pub fn after<C>(self, cursor: C) -> Self
            where
                C: Into<$crate::query::builder::PageCursor>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.after(cursor)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.after(cursor))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.after(cursor)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.after(cursor)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.after(cursor)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.after(cursor))
                    }
                }
            }

            pub fn before<C>(self, cursor: C) -> Self
            where
                C: Into<$crate::query::builder::PageCursor>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.before(cursor)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.before(cursor))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.before(cursor)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.before(cursor)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.before(cursor)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.before(cursor))
                    }
                }
            }

            pub fn limit(self, limit: usize) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.limit(limit)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.limit(limit))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.limit(limit)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.limit(limit)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.limit(limit)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.limit(limit))
                    }
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.range(range)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.range(range)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.range(range)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.range(range))
                    }
                }
            }

            pub fn descendants<F, C>(self, f: F, root_id: impl Into<$crate::Value>) -> Self
            where
                F: FnOnce(T::Where) -> C,
                C: $crate::query::builder::RecursiveColumns<T>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.descendants(f, root_id))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.descendants(f, root_id))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.descendants(f, root_id))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.descendants(f, root_id))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.descendants(f, root_id))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.descendants(f, root_id))
                    }
                }
            }

            pub fn ancestors<F, C>(self, f: F, leaf_id: impl Into<$crate::Value>) -> Self
            where
                F: FnOnce(T::Where) -> C,
                C: $crate::query::builder::RecursiveColumns<T>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => {
                        $executor_name::Sqlite(exec.ancestors(f, leaf_id))
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.ancestors(f, leaf_id))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        $executor_name::MySQL(exec.ancestors(f, leaf_id))
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        $executor_name::MSSQL(exec.ancestors(f, leaf_id))
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        $executor_name::DuckDB(exec.ancestors(f, leaf_id))
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.ancestors(f, leaf_id))
                    }
                }
            }

            pub async fn fetch_page(self) -> $crate::Result<$crate::query::builder::CursorPage<T>>
            where
                T: 'static + std::marker::Send + std::marker::Sync,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => exec.fetch_page().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.fetch_page().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.fetch_page().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.fetch_page().await,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.fetch_page().await,
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $crate::abstract_layer::common::unified::clickhouse_fetch_page_on_backend(
                            db, select,
                        )
                        .await
                    }
                }
            }

            /// 启用 DISTINCT 去重
            pub fn distinct(self) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.distinct()),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.distinct()),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.distinct()),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.distinct()),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.distinct()),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.distinct())
                    }
                }
            }

            pub fn ignore<F, M>(self, f: F) -> Self
            where
                F: FnOnce(<T as $crate::Model>::Where) -> M,
                M: $crate::query::builder::MapToResult,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec) => $executor_name::Sqlite(exec.ignore(f)),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.ignore(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.ignore(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.ignore(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.ignore(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::ClickHouse(db, select) => {
                        $executor_name::ClickHouse(db, select.ignore(f))
                    }
                }
            }
        }
    };
}

/// 为统一的 DeleteExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_delete_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F, W>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.filter(f), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter(f)),
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            pub fn model(self, model: &T) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.model(model), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.model(model))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.model(model)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.model(model)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.model(model)),
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            /// 运行时后端类型：PostgreSQL 变体经后端 `db_type()` 判定
            /// （QuestDB 复用 PG 连接，不能按变体推断）。
            fn backend_db_type(&self) -> $crate::DbType {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(..) => $crate::DbType::Sqlite,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.db_type(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(..) => $crate::DbType::MySQL,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(..) => $crate::DbType::MSSQL,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(..) => $crate::DbType::DuckDB,
                    $executor_name::Unsupported { backend, .. } => *backend,
                }
            }

            pub fn to_sql(&self) -> crate::Result<$crate::SqlStatement> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.to_sql(),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.to_sql(),
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature {
                        backend: *backend,
                        feature: *feature,
                    }),
                }
            }

            /// 执行删除并返回受影响行数（`IntoFuture` 的默认路径）。
            ///
            /// 删除执行器不持有模型主体，因此默认路径不触发 Before/After
            /// 钩子；钩子通过 [`Self::execute_with_hooks`] /
            /// [`Self::execute_models_with_hooks`] 显式提供模型主体，
            /// 并统一受 `hooks_enabled()` 开关与 `without_hooks()` 控制
            /// （嵌套的 insert 钩子同样会被关闭范围抑制）。
            pub async fn execute(self) -> crate::Result<u64> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.execute().await,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.execute().await,
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature { backend, feature }),
                }
            }

            pub fn without_hooks(self) -> $crate::WithoutHooksExecutor<Self> {
                $crate::WithoutHooksExecutor(self)
            }

            /// Execute the configured delete and run hooks around it.
            ///
            /// `AfterDelete` runs only when the statement affects at least one
            /// row. The supplied model is the hook subject; filters remain
            /// fully controlled by the executor.
            ///
            /// 钩子经隐藏 trait 派发，统一受 `hooks_enabled()` 开关控制：
            /// `without_hooks()`（或上游写入链的关闭范围）会让本方法跳过
            /// 全部 Before/After 钩子。
            pub async fn execute_with_hooks(self, model: &T) -> crate::Result<u64>
            where
                T: $crate::BeforeDelete + $crate::AfterDelete + Send + Sync,
            {
                let mut ctx = $crate::HookContext::new($crate::HookOperation::Delete);
                $crate::hooks::HookBeforeDelete::call_before_delete(model, &mut ctx).await?;
                let affected = self.model(model).execute().await?;
                if affected > 0 {
                    $crate::hooks::HookAfterDelete::call_after_delete(model, &mut ctx).await?;
                }
                Ok(affected)
            }

            /// Execute the configured delete with per-model hooks.
            pub async fn execute_models_with_hooks(self, models: &[T]) -> crate::Result<u64>
            where
                T: $crate::BeforeDelete + $crate::AfterDelete + Send + Sync,
            {
                for (index, model) in models.iter().enumerate() {
                    let mut ctx =
                        $crate::HookContext::new($crate::HookOperation::Delete).for_batch(index);
                    $crate::hooks::HookBeforeDelete::call_before_delete(model, &mut ctx).await?;
                }

                let affected = self.execute().await?;
                if affected > 0 {
                    for (index, model) in models.iter().enumerate() {
                        let mut ctx = $crate::HookContext::new($crate::HookOperation::Delete)
                            .for_batch(index);
                        $crate::hooks::HookAfterDelete::call_after_delete(model, &mut ctx)
                            .await?;
                    }
                }
                Ok(affected)
            }

            pub async fn returning(self) -> crate::Result<Vec<T>> {
                // 能力矩阵优先：dml_returning=false 的后端（MySQL/QuestDB/
                // ClickHouse/InfluxDB）统一以 "DML RETURNING" 拒绝。
                $crate::Capabilities::ensure(
                    self.backend_db_type(),
                    |caps| caps.dml_returning,
                    "DML RETURNING",
                )?;
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.returning().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.returning().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.returning().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.returning().await,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.returning().await,
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature { backend, feature }),
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = crate::Result<u64>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move { self.execute().await })
            }
        }
    };
}

/// 为统一的 BlockDeleteExecutor 生成 before/between/retain/to_sql/execute 链式 API
#[macro_export]
macro_rules! impl_unified_block_delete_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            /// 删除所有「块结束时间 ≤ cutoff」的完整块（cutoff 向下对齐到块边界）。
            pub fn before(self, cutoff: chrono::DateTime<chrono::Utc>) -> Self {
                let range = $crate::abstract_layer::common::common_helpers::BlockRange::Before {
                    cutoff,
                };
                self.with_range(range)
            }

            /// 删除完整落在 `[start, end)` 内的块（两端对齐到块边界）。
            pub fn between(
                self,
                start: chrono::DateTime<chrono::Utc>,
                end: chrono::DateTime<chrono::Utc>,
            ) -> Self {
                let range = $crate::abstract_layer::common::common_helpers::BlockRange::Between {
                    start,
                    end,
                };
                self.with_range(range)
            }

            /// 保留最近 `duration` 的数据，等价于 `before(now - duration)`，
            /// `now` 取客户端执行时刻。
            pub fn retain(self, duration: std::time::Duration) -> Self {
                let range = $crate::abstract_layer::common::common_helpers::BlockRange::Retain {
                    duration,
                };
                self.with_range(range)
            }

            fn with_range(
                self,
                range: $crate::abstract_layer::common::common_helpers::BlockRange,
            ) -> Self {
                match self {
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.with_range(range)),
                    #[cfg(feature = "clickhouse")]
                    $executor_name::ClickHouse(exec) => $executor_name::ClickHouse(exec.with_range(range)),
                    #[cfg(feature = "influxdb")]
                    $executor_name::InfluxDB(exec) => $executor_name::InfluxDB(exec.with_range(range)),
                    #[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql", feature = "duckdb"))]
                    $executor_name::Fallback {
                        db_type,
                        key,
                        range: _,
                        delete,
                    } => $executor_name::Fallback {
                        db_type,
                        key,
                        range: Some(range),
                        delete,
                    },
                }
            }

            pub fn to_sql(&self) -> crate::Result<$crate::SqlStatement> {
                match self {
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "clickhouse")]
                    $executor_name::ClickHouse(exec) => exec.to_sql(),
                    #[cfg(feature = "influxdb")]
                    $executor_name::InfluxDB(exec) => exec.to_sql(),
                    #[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql", feature = "duckdb"))]
                    $executor_name::Fallback {
                        db_type,
                        key,
                        range,
                        ..
                    } => {
                        let key = key.as_ref().ok_or_else(|| $crate::OrmerError::UnsupportedFeature {
                            backend: *db_type,
                            feature: "block delete (declare #[hypertable(Duration)] on the model's time column)",
                        })?;
                        let range = range.ok_or_else(|| $crate::OrmerError::invalid_operation(
                            "block delete requires before(), between() or retain()",
                        ))?;
                        let Some(range) = $crate::abstract_layer::common::common_helpers::AlignedBlockRange::align(range, key.unit, chrono::Utc::now())? else {
                            return Ok($crate::SqlStatement::batch(*db_type, Vec::new()));
                        };
                        match $crate::abstract_layer::common::common_helpers::build_block_delete_fallback_sql::<T>(*db_type, key, &range)? {
                            Some((sql, params)) => Ok($crate::SqlStatement::batch(
                                *db_type,
                                vec![$crate::SingleSqlStatement::new(sql, params)],
                            )),
                            None => Ok($crate::SqlStatement::batch(*db_type, Vec::new())),
                        }
                    }
                }
            }

            pub async fn execute(self) -> crate::Result<$crate::abstract_layer::common::BlockDeleteResult> {
                match self {
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "clickhouse")]
                    $executor_name::ClickHouse(exec) => exec.execute().await,
                    #[cfg(feature = "influxdb")]
                    $executor_name::InfluxDB(exec) => exec.execute().await,
                    #[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql", feature = "duckdb"))]
                    fallback @ $executor_name::Fallback { .. } => {
                        let sql = fallback.to_sql()?;
                        <$executor_name<'a, T> as $crate::SqlExecutor>::execute_with_sql(fallback, sql).await
                    }
                }
            }
        }
    };
}

/// 为统一的 UpdateExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_update_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F, W>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.filter(f), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            pub fn set<F>(self, f: F) -> Self
            where
                F: FnOnce(&mut T::Update),
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.set(f), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.set(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.set(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.set(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.set(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            /// 从模型实例设置所有非主键字段，并自动添加主键作为 WHERE 条件
            ///
            /// 支持传入单个对象或对象数组（任何实现了 `Insertable` trait 的类型）
            ///
            /// ```ignore
            /// // 单个对象
            /// let user = User { id: 1, name: "Bob".into(), age: 25, email: Some("bob@test.com".into()) };
            /// db.update::<User>().set_model(&user).execute().await?;
            ///
            /// // 多个对象
            /// let users = vec![user1, user2, user3];
            /// db.update::<User>().set_model(&users).execute().await?;
            /// ```
            pub fn set_model<I: $crate::model::Insertable<Model = T>>(self, models: I) -> Self {
                let refs = models.as_refs();
                let mut result = self;
                for model_ref in refs {
                    match result {
                        #[cfg(feature = "sqlite")]
                        $executor_name::Sqlite(exec, phantom) => {
                            result = $executor_name::Sqlite(exec.set_model(model_ref), phantom);
                        }
                        #[cfg(feature = "postgresql")]
                        $executor_name::PostgreSQL(exec) => {
                            result = $executor_name::PostgreSQL(exec.set_model(model_ref));
                        }
                        #[cfg(feature = "mysql")]
                        $executor_name::MySQL(exec) => {
                            result = $executor_name::MySQL(exec.set_model(model_ref));
                        }
                        #[cfg(feature = "mssql")]
                        $executor_name::MSSQL(exec) => {
                            result = $executor_name::MSSQL(exec.set_model(model_ref));
                        }
                        #[cfg(feature = "duckdb")]
                        $executor_name::DuckDB(exec) => {
                            result = $executor_name::DuckDB(exec.set_model(model_ref));
                        }
                        #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                        $executor_name::Unsupported { .. } => return result,
                    }
                }
                result
            }

            /// 从模型实例设置指定字段，并自动添加主键作为 WHERE 条件。
            pub fn set_model_fields<I, F, M>(self, models: I, fields_fn: F) -> Self
            where
                I: $crate::model::Insertable<Model = T>,
                F: FnOnce(T::Where) -> M,
                M: $crate::query::builder::MapToResult,
            {
                let fields = fields_fn(T::Where::default()).column_names();
                let refs = models.as_refs();
                let mut result = self;
                for model_ref in refs {
                    match result {
                        #[cfg(feature = "sqlite")]
                        $executor_name::Sqlite(exec, phantom) => {
                            result = $executor_name::Sqlite(
                                exec.set_model_fields(model_ref, &fields),
                                phantom,
                            );
                        }
                        #[cfg(feature = "postgresql")]
                        $executor_name::PostgreSQL(exec) => {
                            result = $executor_name::PostgreSQL(
                                exec.set_model_fields(model_ref, &fields),
                            );
                        }
                        #[cfg(feature = "mysql")]
                        $executor_name::MySQL(exec) => {
                            result =
                                $executor_name::MySQL(exec.set_model_fields(model_ref, &fields));
                        }
                        #[cfg(feature = "mssql")]
                        $executor_name::MSSQL(exec) => {
                            result =
                                $executor_name::MSSQL(exec.set_model_fields(model_ref, &fields));
                        }
                        #[cfg(feature = "duckdb")]
                        $executor_name::DuckDB(exec) => {
                            result =
                                $executor_name::DuckDB(exec.set_model_fields(model_ref, &fields));
                        }
                        #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                        $executor_name::Unsupported { .. } => return result,
                    }
                }
                result
            }

            pub fn to_sql(&self) -> crate::Result<$crate::SqlStatement> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.to_sql(),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.to_sql(),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature {
                        backend: *backend,
                        feature: *feature,
                    }),
                }
            }

            /// 执行更新并返回受影响行数（`IntoFuture` 的默认路径）。
            ///
            /// 更新执行器不持有模型主体（`set_model` 立即物化为 SQL 赋值
            /// 计划），因此默认路径不触发 Before/After 钩子；钩子通过
            /// [`Self::execute_with_hooks`] / [`Self::execute_models_with_hooks`]
            /// 显式提供模型主体，并统一受 `hooks_enabled()` 开关与
            /// `without_hooks()` 控制。
            pub async fn execute(self) -> crate::Result<u64> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.execute().await,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.execute().await,
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature { backend, feature }),
                }
            }

            pub fn without_hooks(self) -> $crate::WithoutHooksExecutor<Self> {
                $crate::WithoutHooksExecutor(self)
            }

            /// Execute the configured update and run hooks around it.
            ///
            /// The model supplied here is also the model observed by hooks;
            /// use `set_model(model)` when the update should be derived from
            /// that model's values.
            ///
            /// 钩子经隐藏 trait 派发，统一受 `hooks_enabled()` 开关控制：
            /// `without_hooks()`（或上游写入链的关闭范围）会让本方法跳过
            /// 全部 Before/After 钩子。
            pub async fn execute_with_hooks(self, model: &mut T) -> crate::Result<u64>
            where
                T: $crate::BeforeUpdate + $crate::AfterUpdate + Send + Sync,
            {
                let mut ctx = $crate::HookContext::new($crate::HookOperation::Update);
                $crate::hooks::HookBeforeUpdate::call_before_update(model, &mut ctx).await?;
                let affected = self.execute().await?;
                if affected > 0 {
                    $crate::hooks::HookAfterUpdate::call_after_update(&*model, &mut ctx).await?;
                }
                Ok(affected)
            }

            /// Execute the configured update with per-model hooks.
            pub async fn execute_models_with_hooks(self, models: &mut [T]) -> crate::Result<u64>
            where
                T: $crate::BeforeUpdate + $crate::AfterUpdate + Send + Sync,
            {
                for (index, model) in models.iter_mut().enumerate() {
                    let mut ctx =
                        $crate::HookContext::new($crate::HookOperation::Update).for_batch(index);
                    $crate::hooks::HookBeforeUpdate::call_before_update(model, &mut ctx).await?;
                }

                let affected = self.execute().await?;
                if affected > 0 {
                    for (index, model) in models.iter().enumerate() {
                        let mut ctx = $crate::HookContext::new($crate::HookOperation::Update)
                            .for_batch(index);
                        $crate::hooks::HookAfterUpdate::call_after_update(model, &mut ctx).await?;
                    }
                }
                Ok(affected)
            }

            /// 运行时后端类型：PostgreSQL 变体经后端 `db_type()` 判定
            /// （QuestDB 复用 PG 连接，不能按变体推断）。
            fn backend_db_type(&self) -> $crate::DbType {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(..) => $crate::DbType::Sqlite,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.db_type(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(..) => $crate::DbType::MySQL,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(..) => $crate::DbType::MSSQL,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(..) => $crate::DbType::DuckDB,
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::Unsupported { backend, .. } => *backend,
                }
            }

            pub async fn returning(self) -> crate::Result<Vec<T>> {
                // 能力矩阵优先：dml_returning=false 的后端（MySQL/QuestDB/
                // ClickHouse/InfluxDB）统一以 "DML RETURNING" 拒绝。
                $crate::Capabilities::ensure(
                    self.backend_db_type(),
                    |caps| caps.dml_returning,
                    "DML RETURNING",
                )?;
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.returning().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.returning().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.returning().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.returning().await,
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => exec.returning().await,
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => Err($crate::OrmerError::UnsupportedFeature { backend, feature }),
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = crate::Result<u64>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move { self.execute().await })
            }
        }
    };
}

/// 为统一的 CollectFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_collect_future {
    ($future_name:ident) => {
        impl<
            'a,
            T: $crate::Model + 'static + std::marker::Send + std::marker::Sync,
            C: FromIterator<T> + 'static,
        > std::future::IntoFuture for $future_name<'a, T, C>
        {
            type Output = crate::Result<C>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $future_name::Sqlite(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mssql")]
                    $future_name::MSSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "duckdb")]
                    $future_name::DuckDB(future) => Box::pin(future.into_future()),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $future_name::ClickHouse(db, select, _) => Box::pin(async move {
                        $crate::abstract_layer::common::unified::clickhouse_select_models_on_backend(
                            db, select,
                        )
                        .await
                    }),
                }
            }
        }
    };
}

/// 为统一的 AggregateFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_aggregate_future {
    ($future_name:ident) => {
        impl<
            'a,
            T: $crate::Model + 'static + std::marker::Send,
            R: $crate::model::FromValue + 'static + std::marker::Send,
        > std::future::IntoFuture for $future_name<'a, T, R>
        {
            type Output = crate::Result<R>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $future_name::Sqlite(future, _) => Box::pin(async move { future.await }),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(async move { future.await }),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(async move { future.await }),
                    #[cfg(feature = "mssql")]
                    $future_name::MSSQL(future) => Box::pin(async move { future.await }),
                    #[cfg(feature = "duckdb")]
                    $future_name::DuckDB(future) => Box::pin(async move { future.await }),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $future_name::ClickHouse(db, aggregate, _) => Box::pin(async move {
                        $crate::abstract_layer::common::unified::clickhouse_aggregate_on_backend(
                            db, aggregate,
                        )
                        .await
                    }),
                }
            }
        }
    };
}

/// 为统一的 JOIN Executor 生成 filter/range 方法
#[macro_export]
macro_rules! impl_unified_join_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model, J: $crate::Model> $executor_name<'a, T, J> {
            pub fn filter<F, W>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.filter(f), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.range(range), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.range(range)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.range(range)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }
        }
    };
}

/// 为统一的 JOIN CollectFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_join_collect_future {
    ($future_name:ident, $output_type:ty) => {
        impl<
            'a,
            T: $crate::Model + 'static + std::marker::Send,
            J: $crate::Model + 'static + std::marker::Send,
        > std::future::IntoFuture for $future_name<'a, T, J>
        {
            type Output = $output_type;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $future_name::Sqlite(future, _) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mssql")]
                    $future_name::MSSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "duckdb")]
                    $future_name::DuckDB(future) => Box::pin(future.into_future()),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $future_name::Unsupported {
                        backend, feature, ..
                    } => Box::pin(async move {
                        Err($crate::OrmerError::UnsupportedFeature { backend, feature })
                    }),
                }
            }
        }
    };
}

/// 为统一的 RelatedSelectExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_related_select_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model + 'static, R: $crate::Model + 'static> $executor_name<'a, T, R> {
            pub fn filter<F, W>(self, f: F) -> Self
            where
                F: FnOnce(T::Where, R::Where) -> W,
                W: Into<$crate::WhereExpr>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.filter(f), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.filter(f)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.filter(f)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.range(range), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.range(range)),
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => $executor_name::DuckDB(exec.range(range)),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    unsupported @ $executor_name::Unsupported { .. } => unsupported,
                }
            }

            pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<'a, T, R>
            where
                T: 'static,
                R: 'static,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        RelatedCollectFuture::Sqlite(exec.into_collect_future(), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        RelatedCollectFuture::PostgreSQL(exec.into_collect_future())
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => {
                        RelatedCollectFuture::MySQL(exec.into_collect_future())
                    }
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => {
                        RelatedCollectFuture::MSSQL(exec.into_collect_future())
                    }
                    #[cfg(feature = "duckdb")]
                    $executor_name::DuckDB(exec) => {
                        RelatedCollectFuture::DuckDB(exec.into_collect_future())
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $executor_name::Unsupported {
                        backend, feature, ..
                    } => RelatedCollectFuture::Unsupported {
                        backend,
                        feature,
                        _marker: std::marker::PhantomData,
                    },
                }
            }
        }
    };
}

/// 为统一的 RelatedCollectFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_related_collect_future {
    ($future_name:ident) => {
        impl<
            'a,
            T: $crate::Model + 'static + std::marker::Send + std::marker::Sync,
            R: $crate::Model + 'static + std::marker::Send + std::marker::Sync,
        > std::future::IntoFuture for $future_name<'a, T, R>
        where
            Self: 'a,
        {
            type Output = crate::Result<Vec<T>>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $future_name::Sqlite(future, _) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mssql")]
                    $future_name::MSSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "duckdb")]
                    $future_name::DuckDB(future) => Box::pin(future.into_future()),
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $future_name::Unsupported {
                        backend, feature, ..
                    } => Box::pin(async move {
                        Err($crate::OrmerError::UnsupportedFeature { backend, feature })
                    }),
                }
            }
        }
    };
}

/// 统一层关联/多表行数统计 Future 的 IntoFuture：委托到各后端执行器的
/// `count()`（page.md 缺失一：列表分页 total_count 场景）。
///
/// 后端执行器的 `count()` 返回 `i64`，统一层在此强转为 `usize`，与
/// `SelectExecutor::count()`（单表 COUNT 聚合，同样输出 `usize`）保持同一
/// 返回类型，分页场景两个入口可混用（L23）。
///
/// `$decl` 携带泛型与约束（用于 `impl<...>`），`$use` 为纯类型实参
/// （用于 `Future<...>` 类型位置），两者由调用点成对给出。
#[macro_export]
macro_rules! impl_unified_related_count_future {
    ($future_name:ident, $call:ident, [$($decl:tt)*], [$($use:tt)*]) => {
        impl<$($decl)*> std::future::IntoFuture for $future_name<$($use)*>
        where
            Self: 'a,
        {
            type Output = crate::Result<usize>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "sqlite")]
                    $future_name::Sqlite(exec, _) => {
                        Box::pin(async move { Ok(exec.$call().await? as usize) })
                    }
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(exec) => {
                        Box::pin(async move { Ok(exec.$call().await? as usize) })
                    }
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(exec) => {
                        Box::pin(async move { Ok(exec.$call().await? as usize) })
                    }
                    #[cfg(feature = "mssql")]
                    $future_name::MSSQL(exec) => {
                        Box::pin(async move { Ok(exec.$call().await? as usize) })
                    }
                    #[cfg(feature = "duckdb")]
                    $future_name::DuckDB(exec, _) => {
                        Box::pin(async move { Ok(exec.$call().await? as usize) })
                    }
                    #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
                    $future_name::Unsupported {
                        backend, feature, ..
                    } => Box::pin(async move {
                        Err($crate::OrmerError::UnsupportedFeature { backend, feature })
                    }),
                }
            }
        }
    };
}

/// 为数据库后端的 Executor 生成通用的 filter/order_by/range 方法
/// 这个宏用于消除三个后端中重复的 Executor 方法实现
///
/// 实际实现收敛在 [`__ormer_impl_backend_select_methods_for!`]：无生命周期
/// 与带生命周期的版本只差 executor 泛型形状，公开宏名保留为兼容别名。
#[macro_export]
macro_rules! impl_backend_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_select_methods_for!(
            $conn_field, distinct; $executor_type<'a, T>; 'a, T: $crate::Model
        );
    };
}

/// 为数据库后端的 JOIN Executor 生成通用的 filter/range 方法
#[macro_export]
macro_rules! impl_backend_join_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_join_methods_for!(
            $conn_field; $executor_type<T, J>; T: $crate::Model, J: $crate::Model
        );
    };
}

/// 为数据库后端的 RelatedSelectExecutor 生成通用方法
#[macro_export]
macro_rules! impl_backend_related_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_related_methods_for!(
            $conn_field; $executor_type<T, R>; T: $crate::Model, R: $crate::Model
        );
    };
}

/// 为带有生命周期参数的数据库后端 Executor 生成通用的 filter/order_by/range 方法
#[macro_export]
macro_rules! impl_backend_executor_methods_with_lifetime {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_select_methods_for!(
            $conn_field; $executor_type<'a, T>; 'a, T: $crate::Model
        );
    };
}

/// 为带有生命周期参数的数据库后端 JOIN Executor 生成通用的 filter/range 方法
#[macro_export]
macro_rules! impl_backend_join_executor_methods_with_lifetime {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_join_methods_for!(
            $conn_field; $executor_type<'a, T, J>; 'a, T: $crate::Model, J: $crate::Model
        );
    };
}

/// 为带有生命周期参数的数据库后端 RelatedSelectExecutor 生成通用方法
#[macro_export]
macro_rules! impl_backend_related_executor_methods_with_lifetime {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_related_methods_for!(
            $conn_field; $executor_type<'a, T, R>; 'a, T: $crate::Model, R: $crate::Model
        );
    };
}

/// 为带有生命周期参数的数据库后端 MultiTableSelectExecutor 生成通用方法
#[macro_export]
macro_rules! impl_backend_multi_table_executor_methods_with_lifetime {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_multi_table_methods_for!(
            $conn_field;
            $executor_type<'a, T, R1, R2>;
            'a, T: $crate::Model, R1: $crate::Model, R2: $crate::Model
        );
    };
}

/// 为带有生命周期参数的数据库后端 FourTableSelectExecutor 生成通用方法
#[macro_export]
macro_rules! impl_backend_four_table_executor_methods_with_lifetime {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        $crate::__ormer_impl_backend_four_table_methods_for!(
            $conn_field;
            $executor_type<'a, T, R1, R2, R3>;
            'a, T: $crate::Model, R1: $crate::Model, R2: $crate::Model, R3: $crate::Model
        );
    };
}

/// 后端 Executor 通用方法的实现核心：`impl<...> Executor<...>` 的泛型形状
/// 由调用方以 `executor 类型; 泛型参数列表` 传入，可选的 `distinct` 标记
/// 决定是否生成 `distinct()`（无生命周期/带生命周期两套公开宏共用此核心）。
#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_impl_backend_select_methods_for {
    ($conn_field:ident $(, $distinct:ident)? ; $executor_type:ty ; $($generics:tt)*) => {
        impl<$($generics)*> $executor_type {
            $crate::__ormer_backend_select_methods!($conn_field $(, $distinct)?);

            pub async fn fetch_page(self) -> $crate::Result<$crate::query::builder::CursorPage<T>>
            where
                T: 'static + std::marker::Send + std::marker::Sync,
            {
                let (select, cursor_columns) = self.select.prepare_cursor_page()?;
                let executor = Self {
                    select: select.clone(),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                };
                let items: Vec<T> = executor.collect().await?;
                select.finish_cursor_page(items, &cursor_columns)
            }
        }
    };
}

/// 后端 JOIN Executor 通用方法的实现核心（泛型形状由调用方传入）。
#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_impl_backend_join_methods_for {
    ($conn_field:ident; $executor_type:ty; $($generics:tt)*) => {
        impl<$($generics)*> $executor_type {
            $crate::__ormer_backend_join_methods!($conn_field);
        }
    };
}

/// 后端 RelatedSelectExecutor 通用方法的实现核心（泛型形状由调用方传入）。
#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_impl_backend_related_methods_for {
    ($conn_field:ident; $executor_type:ty; $($generics:tt)*) => {
        impl<$($generics)*> $executor_type {
            $crate::__ormer_backend_related_methods!($conn_field);
        }
    };
}

/// 后端 MultiTableSelectExecutor 通用方法的实现核心（泛型形状由调用方传入）。
#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_impl_backend_multi_table_methods_for {
    ($conn_field:ident; $executor_type:ty; $($generics:tt)*) => {
        impl<$($generics)*> $executor_type {
            $crate::__ormer_backend_multi_table_methods!($conn_field);
        }
    };
}

/// 后端 FourTableSelectExecutor 通用方法的实现核心（泛型形状由调用方传入）。
#[doc(hidden)]
#[macro_export]
macro_rules! __ormer_impl_backend_four_table_methods_for {
    ($conn_field:ident; $executor_type:ty; $($generics:tt)*) => {
        impl<$($generics)*> $executor_type {
            $crate::__ormer_backend_four_table_methods!($conn_field);
        }
    };
}

/// 示例用法 (在实际代码中使用):
///
/// ```text
/// // 在 sqlite_backend.rs 中:
/// ormer::impl_join_executor_methods!(LeftJoinedSelectExecutor, conn, Arc<Sqlite::Connection>);
/// ormer::impl_join_executor_methods!(InnerJoinedSelectExecutor, conn, Arc<Sqlite::Connection>);
/// ormer::impl_join_executor_methods!(RightJoinedSelectExecutor, conn, Arc<Sqlite::Connection>);
///
/// // 在 mysql_backend.rs 中:
/// ormer::impl_join_executor_methods!(LeftJoinedSelectExecutor, pool, &'a Pool);
/// ormer::impl_join_executor_methods!(InnerJoinedSelectExecutor, pool, &'a Pool);
/// ormer::impl_join_executor_methods!(RightJoinedSelectExecutor, pool, &'a Pool);
/// ```
///
pub fn _placeholder() {}
