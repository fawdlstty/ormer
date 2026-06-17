/// 宏定义 - 用于减少重复代码
///
/// 本文件包含用于生成重复代码模式的宏
/// 为 JOIN Executor 生成通用的 filter/range 方法
#[macro_export]
macro_rules! impl_join_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty
    ) => {
        impl<T: $crate::Model, J: $crate::Model> $executor_type<T, J> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                Self {
                    select: self.select.filter(f),
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

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                Self {
                    select: self.select.range(range),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }
        }
    };
}

/// 为统一的 SelectExecutor 生成方法（filter/order_by/range）
#[macro_export]
macro_rules! impl_unified_select_executor_methods {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
                }
            }

            pub fn to_sql(&self) -> anyhow::Result<$crate::SqlStatement> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.to_sql(),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.to_sql(),
                }
            }

            pub async fn execute(self) -> anyhow::Result<u64> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.execute().await,
                }
            }

            pub async fn returning(self) -> anyhow::Result<Vec<T>> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.returning().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.returning().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.returning().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.returning().await,
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = anyhow::Result<u64>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move { self.execute().await })
            }
        }
    };
}

/// 为统一的 UpdateExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_update_executor {
    ($executor_name:ident) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
                }
            }

            pub fn set<F, V, C>(self, field_fn: F, value: V) -> Self
            where
                F: FnOnce(T::Where) -> $crate::query::builder::TypedColumn<C>,
                V: Into<$crate::model::Value>,
            {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, phantom) => {
                        $executor_name::Sqlite(exec.set(field_fn, value), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.set(field_fn, value))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.set(field_fn, value)),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => $executor_name::MSSQL(exec.set(field_fn, value)),
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
                    }
                }
                result
            }

            pub fn to_sql(&self) -> anyhow::Result<$crate::SqlStatement> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.to_sql(),
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.to_sql(),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.to_sql(),
                }
            }

            pub async fn execute(self) -> anyhow::Result<u64> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.execute().await,
                }
            }

            pub async fn returning(self) -> anyhow::Result<Vec<T>> {
                match self {
                    #[cfg(feature = "sqlite")]
                    $executor_name::Sqlite(exec, _) => exec.returning().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.returning().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.returning().await,
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => exec.returning().await,
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = anyhow::Result<u64>;
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
            type Output = anyhow::Result<C>;
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
            type Output = anyhow::Result<R>;
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
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where, R::Where) -> $crate::WhereExpr,
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
                        RelatedCollectFuture::Sqlite(exec.exec(), phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        RelatedCollectFuture::PostgreSQL(exec.exec())
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => RelatedCollectFuture::MySQL(exec.exec()),
                    #[cfg(feature = "mssql")]
                    $executor_name::MSSQL(exec) => RelatedCollectFuture::MSSQL(exec.exec()),
                }
            }

            pub fn exec(self) -> RelatedCollectFuture<'a, T, R>
            where
                T: 'static,
                R: 'static,
            {
                self.collect::<Vec<T>>()
            }

            pub fn execute(self) -> RelatedCollectFuture<'a, T, R>
            where
                T: 'static,
                R: 'static,
            {
                self.collect::<Vec<T>>()
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
            type Output = anyhow::Result<Vec<T>>;
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
                }
            }
        }
    };
}

/// 为数据库后端的 Executor 生成通用的 filter/order_by/range 方法
/// 这个宏用于消除三个后端中重复的 Executor 方法实现
#[macro_export]
macro_rules! impl_backend_executor_methods {
    (
        $executor_type:ident,
        $conn_field:ident,
        $conn_type:ty,
        $select_type:ident
    ) => {
        impl<'a, T: $crate::Model> $executor_type<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                Self {
                    select: self.select.filter(f),
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

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                Self {
                    select: self.select.range(range),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }

            /// 启用 DISTINCT 去重
            pub fn distinct(self) -> Self {
                Self {
                    select: self.select.distinct(),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }
        }
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
        impl<T: $crate::Model, J: $crate::Model> $executor_type<T, J> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
        }
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
        impl<T: $crate::Model, R: $crate::Model> $executor_type<T, R> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where, R::Where) -> $crate::WhereExpr,
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
        }
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
        impl<'a, T: $crate::Model> $executor_type<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                Self {
                    select: self.select.filter(f),
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

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                Self {
                    select: self.select.range(range),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }
        }
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
        impl<'a, T: $crate::Model, J: $crate::Model> $executor_type<'a, T, J> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
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
        }
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
        impl<'a, T: $crate::Model, R: $crate::Model> $executor_type<'a, T, R> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where, R::Where) -> $crate::WhereExpr,
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
