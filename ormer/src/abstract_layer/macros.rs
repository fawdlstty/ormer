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
        impl<T: $crate::Model> $executor_type<T> {
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
    ($executor_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.filter(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                }
            }

            pub fn order_by<F, O>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> O,
                O: Into<$crate::OrderBy>,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.order_by(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.order_by(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.order_by(f)),
                }
            }

            pub fn order_by_desc<F, O>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> O,
                O: Into<$crate::OrderBy>,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.order_by_desc(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.order_by_desc(f))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.order_by_desc(f)),
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.range(range), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                }
            }
        }
    };
}

/// 为统一的 DeleteExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_delete_executor {
    ($executor_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.filter(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                }
            }

            pub async fn execute(self) -> Result<u64, $crate::Error> {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = Result<u64, $crate::Error>;
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
    ($executor_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model> $executor_name<'a, T> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.filter(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                }
            }

            pub fn set<F, V, C>(self, field_fn: F, value: V) -> Self
            where
                F: FnOnce(T::Where) -> $crate::query::builder::TypedColumn<C>,
                V: Into<$crate::model::Value>,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.set(field_fn, value), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.set(field_fn, value))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.set(field_fn, value)),
                }
            }

            pub async fn execute(self) -> Result<u64, $crate::Error> {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => exec.execute().await,
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => exec.execute().await,
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => exec.execute().await,
                }
            }
        }

        impl<'a, T: $crate::Model + 'static> std::future::IntoFuture for $executor_name<'a, T> {
            type Output = Result<u64, $crate::Error>;
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
    ($future_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model + 'static, C: FromIterator<T> + 'static> std::future::IntoFuture
            for $future_name<'a, T, C>
        {
            type Output = Result<C, $crate::Error>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "turso")]
                    $future_name::Turso(future, _) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                }
            }
        }
    };
}

/// 为统一的 AggregateFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_aggregate_future {
    ($future_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model + 'static, R: $crate::model::FromValue + 'static>
            std::future::IntoFuture for $future_name<'a, T, R>
        {
            type Output = Result<R, $crate::Error>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "turso")]
                    $future_name::Turso(future, _) => Box::pin(async move { future.await }),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(async move { future.await }),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(async move { future.await }),
                }
            }
        }
    };
}

/// 为统一的 JOIN Executor 生成 filter/range 方法
#[macro_export]
macro_rules! impl_unified_join_executor {
    ($executor_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model, J: $crate::Model> $executor_name<'a, T, J> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.filter(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.range(range), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                }
            }
        }
    };
}

/// 为统一的 JOIN CollectFuture 生成 IntoFuture 实现
#[macro_export]
macro_rules! impl_unified_join_collect_future {
    ($future_name:ident, $output_type:ty, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model + 'static, J: $crate::Model + 'static> std::future::IntoFuture
            for $future_name<'a, T, J>
        {
            type Output = $output_type;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "turso")]
                    $future_name::Turso(future, _) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                }
            }
        }
    };
}

/// 为统一的 RelatedSelectExecutor 生成方法
#[macro_export]
macro_rules! impl_unified_related_select_executor {
    ($executor_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model, R: $crate::Model> $executor_name<'a, T, R> {
            pub fn filter<F>(self, f: F) -> Self
            where
                F: FnOnce(T::Where, R::Where) -> $crate::WhereExpr,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.filter(f), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => $executor_name::PostgreSQL(exec.filter(f)),
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.filter(f)),
                }
            }

            pub fn range<RR: Into<$crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        $executor_name::Turso(exec.range(range), $turso_phantom)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        $executor_name::PostgreSQL(exec.range(range))
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => $executor_name::MySQL(exec.range(range)),
                }
            }

            pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<'a, T, R>
            where
                T: 'static,
                R: 'static,
            {
                match self {
                    #[cfg(feature = "turso")]
                    $executor_name::Turso(exec, _) => {
                        RelatedCollectFuture::Turso(exec.exec(), std::marker::PhantomData)
                    }
                    #[cfg(feature = "postgresql")]
                    $executor_name::PostgreSQL(exec) => {
                        RelatedCollectFuture::PostgreSQL(exec.exec())
                    }
                    #[cfg(feature = "mysql")]
                    $executor_name::MySQL(exec) => RelatedCollectFuture::MySQL(exec.exec()),
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
    ($future_name:ident, $turso_phantom:expr) => {
        impl<'a, T: $crate::Model + 'static, R: $crate::Model + 'static> std::future::IntoFuture
            for $future_name<'a, T, R>
        {
            type Output = Result<Vec<T>, $crate::Error>;
            type IntoFuture =
                std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                match self {
                    #[cfg(feature = "turso")]
                    $future_name::Turso(future, _) => Box::pin(future.into_future()),
                    #[cfg(feature = "postgresql")]
                    $future_name::PostgreSQL(future) => Box::pin(future.into_future()),
                    #[cfg(feature = "mysql")]
                    $future_name::MySQL(future) => Box::pin(future.into_future()),
                }
            }
        }
    };
}

/// 示例用法 (在实际代码中使用):
///
/// ```rust,ignore
/// // 在 turso_backend.rs 中:
/// ormer::impl_join_executor_methods!(LeftJoinedSelectExecutor, conn, Arc<turso::Connection>);
/// ormer::impl_join_executor_methods!(InnerJoinedSelectExecutor, conn, Arc<turso::Connection>);
/// ormer::impl_join_executor_methods!(RightJoinedSelectExecutor, conn, Arc<turso::Connection>);
///
/// // 在 mysql_backend.rs 中:
/// ormer::impl_join_executor_methods!(LeftJoinedSelectExecutor, pool, &'a Pool);
/// ormer::impl_join_executor_methods!(InnerJoinedSelectExecutor, pool, &'a Pool);
/// ormer::impl_join_executor_methods!(RightJoinedSelectExecutor, pool, &'a Pool);
/// ```
///
pub fn _placeholder() {}
