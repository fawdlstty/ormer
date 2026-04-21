/// 宏定义 - 用于减少重复代码
///
/// 本文件包含用于生成重复代码模式的宏

/// 为 JOIN Executor 生成通用的 filter/limit/offset 方法
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

            pub fn limit(self, limit: i64) -> Self {
                Self {
                    select: self.select.limit(limit),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }

            pub fn offset(self, offset: i64) -> Self {
                Self {
                    select: self.select.offset(offset),
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

/// 为 Executor 生成通用的方法 (filter/order_by/limit/offset)
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

            pub fn order_by<F>(self, f: F) -> Self
            where
                F: FnOnce($crate::WhereColumn<T>) -> $crate::OrderBy,
            {
                Self {
                    select: self.select.order_by(f),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }

            pub fn limit(self, limit: i64) -> Self {
                Self {
                    select: self.select.limit(limit),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }

            pub fn offset(self, offset: i64) -> Self {
                Self {
                    select: self.select.offset(offset),
                    $conn_field: self.$conn_field,
                    _marker: std::marker::PhantomData,
                }
            }
        }
    };
}

/// 示例用法 (在实际代码中使用):
///
/// ```rust
/// // 在 turso_backend.rs 中:
/// impl_join_executor_methods!(LeftJoinedSelectExecutor, conn, Arc<turso::Connection>);
/// impl_join_executor_methods!(InnerJoinedSelectExecutor, conn, Arc<turso::Connection>);
/// impl_join_executor_methods!(RightJoinedSelectExecutor, conn, Arc<turso::Connection>);
///
/// // 在 mysql_backend.rs 中:
/// impl_join_executor_methods!(LeftJoinedSelectExecutor, pool, &'a Pool);
/// impl_join_executor_methods!(InnerJoinedSelectExecutor, pool, &'a Pool);
/// impl_join_executor_methods!(RightJoinedSelectExecutor, pool, &'a Pool);
/// ```
///
pub fn _placeholder() {}
