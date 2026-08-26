/// 钩子系统模块
/// 提供数据操作生命周期中的回调机制
use crate::model::{Insertable, Model};
use std::future::Future;
use std::marker::PhantomData;

tokio::task_local! {
    static HOOKS_DISABLED: bool;
}

pub(crate) fn hooks_enabled() -> bool {
    !HOOKS_DISABLED
        .try_with(|disabled| *disabled)
        .unwrap_or(false)
}

pub(crate) async fn without_hooks_scope<F>(future: F) -> F::Output
where
    F: Future,
{
    HOOKS_DISABLED.scope(true, future).await
}

/// Internal execution trait used by `WithoutHooksExecutor`.
#[doc(hidden)]
#[async_trait::async_trait(?Send)]
pub trait HookExecutable: Sized {
    type Output;

    async fn execute(self) -> crate::Result<Self::Output>;
}

/// An execution chain that skips write hooks for this operation only.
#[doc(hidden)]
pub struct WithoutHooksExecutor<E>(pub(crate) E);

impl<E> WithoutHooksExecutor<E>
where
    E: HookExecutable,
{
    pub async fn execute(self) -> crate::Result<E::Output> {
        without_hooks_scope(self.0.execute()).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::InsertExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::InsertExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::InsertOrUpdateExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::InsertOrUpdateExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::InsertOrIgnoreExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::InsertOrIgnoreExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::TransactionInsertExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::TransactionInsertExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::TransactionInsertOrUpdateExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::TransactionInsertOrUpdateExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable for crate::abstract_layer::TransactionInsertOrIgnoreExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::TransactionInsertOrIgnoreExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, T: Model> HookExecutable for crate::abstract_layer::DeleteExecutor<'a, T> {
    type Output = u64;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::DeleteExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, T: Model> HookExecutable for crate::abstract_layer::UpdateExecutor<'a, T> {
    type Output = u64;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::UpdateExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, T: Model> HookExecutable for crate::abstract_layer::ScopedDeleteExecutor<'a, T> {
    type Output = u64;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::ScopedDeleteExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, T: Model> HookExecutable for crate::abstract_layer::ScopedUpdateExecutor<'a, T> {
    type Output = u64;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::ScopedUpdateExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable
    for crate::abstract_layer::common::connection_pool::PooledInsertExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::common::connection_pool::PooledInsertExecutor::execute(self).await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable
    for crate::abstract_layer::common::connection_pool::PooledInsertOrUpdateExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::common::connection_pool::PooledInsertOrUpdateExecutor::execute(self)
            .await
    }
}

#[async_trait::async_trait(?Send)]
impl<'a, I> HookExecutable
    for crate::abstract_layer::common::connection_pool::PooledInsertOrIgnoreExecutor<'a, I>
where
    I: Insertable + Send + Sync,
{
    type Output = ();

    async fn execute(self) -> crate::Result<Self::Output> {
        crate::abstract_layer::common::connection_pool::PooledInsertOrIgnoreExecutor::execute(self)
            .await
    }
}

/// The operation currently being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOperation {
    Insert,
    Update,
    Delete,
}

/// Context passed to lifecycle hooks.
///
/// The lifetime is reserved for operation-scoped data supplied by future
/// executor integrations. Keeping it on the public type now lets hooks use a
/// stable signature without exposing backend connection types.
#[derive(Debug, Clone, Copy)]
pub struct HookContext<'a> {
    operation: HookOperation,
    batch_index: Option<usize>,
    in_transaction: bool,
    _marker: PhantomData<&'a ()>,
}

impl<'a> HookContext<'a> {
    pub fn new(operation: HookOperation) -> Self {
        Self {
            operation,
            batch_index: None,
            in_transaction: false,
            _marker: PhantomData,
        }
    }

    pub fn operation(&self) -> HookOperation {
        self.operation
    }

    pub fn batch_index(&self) -> Option<usize> {
        self.batch_index
    }

    pub fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    pub fn for_batch(&self, index: usize) -> Self {
        Self {
            operation: self.operation,
            batch_index: Some(index),
            in_transaction: self.in_transaction,
            _marker: PhantomData,
        }
    }

    pub fn transaction(mut self) -> Self {
        self.in_transaction = true;
        self
    }

    #[doc(hidden)]
    pub fn hooks_enabled(&self) -> bool {
        hooks_enabled()
    }
}

macro_rules! define_lifecycle_hook {
    ($(#[$meta:meta])* $public:ident, $method:ident, $hidden:ident, $call:ident, mut, $($bound:tt)+) => {
        $(#[$meta])*
        #[async_trait::async_trait]
        pub trait $public: Model {
            async fn $method(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
        }

        #[doc(hidden)]
        #[async_trait::async_trait]
        pub trait $hidden {
            async fn $call(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
        }

        #[async_trait::async_trait]
        impl<M: $public + $($bound)+> $hidden for M {
            async fn $call(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
                if !ctx.hooks_enabled() {
                    return Ok(());
                }
                self.$method(ctx).await
            }
        }
    };
    ($(#[$meta:meta])* $public:ident, $method:ident, $hidden:ident, $call:ident, ref, $($bound:tt)+) => {
        $(#[$meta])*
        #[async_trait::async_trait]
        pub trait $public: Model {
            async fn $method(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
        }

        #[doc(hidden)]
        #[async_trait::async_trait]
        pub trait $hidden {
            async fn $call(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
        }

        #[async_trait::async_trait]
        impl<M: $public + $($bound)+> $hidden for M {
            async fn $call(&self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
                if !ctx.hooks_enabled() {
                    return Ok(());
                }
                self.$method(ctx).await
            }
        }
    };
}

define_lifecycle_hook!(
    /// 插入前钩子
    BeforeInsert,
    before_insert,
    HookBeforeInsert,
    call_before_insert,
    mut,
    Send
);
define_lifecycle_hook!(
    /// 插入后钩子
    AfterInsert,
    after_insert,
    HookAfterInsert,
    call_after_insert,
    ref,
    Send + Sync
);
define_lifecycle_hook!(
    /// 更新前钩子
    BeforeUpdate,
    before_update,
    HookBeforeUpdate,
    call_before_update,
    mut,
    Send
);
define_lifecycle_hook!(
    /// 更新后钩子
    AfterUpdate,
    after_update,
    HookAfterUpdate,
    call_after_update,
    ref,
    Send + Sync
);
define_lifecycle_hook!(
    /// 删除前钩子
    BeforeDelete,
    before_delete,
    HookBeforeDelete,
    call_before_delete,
    ref,
    Send + Sync
);
define_lifecycle_hook!(
    /// 删除后钩子
    AfterDelete,
    after_delete,
    HookAfterDelete,
    call_after_delete,
    ref,
    Send + Sync
);
