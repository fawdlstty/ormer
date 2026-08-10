/// 钩子系统模块
/// 提供数据操作生命周期中的回调机制
use crate::model::Model;
use std::marker::PhantomData;

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
}

/// 插入前钩子
#[async_trait::async_trait]
pub trait BeforeInsert: Model {
    async fn before_insert(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 插入后钩子
#[async_trait::async_trait]
pub trait AfterInsert: Model {
    async fn after_insert(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 更新前钩子
#[async_trait::async_trait]
pub trait BeforeUpdate: Model {
    async fn before_update(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 更新后钩子
#[async_trait::async_trait]
pub trait AfterUpdate: Model {
    async fn after_update(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 删除前钩子
#[async_trait::async_trait]
pub trait BeforeDelete: Model {
    async fn before_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 删除后钩子
#[async_trait::async_trait]
pub trait AfterDelete: Model {
    async fn after_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 钩子执行辅助 trait
/// 用于在执行器中自动调用钩子
/// 内部 trait：用于自动调用 BeforeInsert 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeInsert {
    async fn call_before_insert(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 内部 trait：用于自动调用 AfterInsert 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterInsert {
    async fn call_after_insert(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 内部 trait：用于自动调用 BeforeUpdate 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeUpdate {
    async fn call_before_update(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 内部 trait：用于自动调用 AfterUpdate 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterUpdate {
    async fn call_after_update(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 内部 trait：用于自动调用 BeforeDelete 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeDelete {
    async fn call_before_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

/// 内部 trait：用于自动调用 AfterDelete 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterDelete {
    async fn call_after_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()>;
}

// 为实现了 BeforeInsert 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeInsert + Send> HookBeforeInsert for M {
    async fn call_before_insert(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.before_insert(ctx).await
    }
}

// 为实现了 AfterInsert 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterInsert + Send + Sync> HookAfterInsert for M {
    async fn call_after_insert(&self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.after_insert(ctx).await
    }
}

// 为实现了 BeforeUpdate 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeUpdate + Send> HookBeforeUpdate for M {
    async fn call_before_update(&mut self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.before_update(ctx).await
    }
}

// 为实现了 AfterUpdate 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterUpdate + Send + Sync> HookAfterUpdate for M {
    async fn call_after_update(&self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.after_update(ctx).await
    }
}

// 为实现了 BeforeDelete 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeDelete + Send + Sync> HookBeforeDelete for M {
    async fn call_before_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.before_delete(ctx).await
    }
}

// 为实现了 AfterDelete 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterDelete + Send + Sync> HookAfterDelete for M {
    async fn call_after_delete(&self, ctx: &mut HookContext<'_>) -> crate::Result<()> {
        self.after_delete(ctx).await
    }
}
