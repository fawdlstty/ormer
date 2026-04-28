/// 钩子系统模块
/// 提供数据操作生命周期中的回调机制
use crate::model::Model;

/// 插入前钩子
#[async_trait::async_trait]
pub trait BeforeInsert: Model {
    async fn before_insert(&mut self);
}

/// 插入后钩子
#[async_trait::async_trait]
pub trait AfterInsert: Model {
    async fn after_insert(&self);
}

/// 更新前钩子
#[async_trait::async_trait]
pub trait BeforeUpdate: Model {
    async fn before_update(&mut self);
}

/// 更新后钩子
#[async_trait::async_trait]
pub trait AfterUpdate: Model {
    async fn after_update(&self);
}

/// 删除前钩子
#[async_trait::async_trait]
pub trait BeforeDelete: Model {
    async fn before_delete(&self);
}

/// 删除后钩子
#[async_trait::async_trait]
pub trait AfterDelete: Model {
    async fn after_delete(&self);
}

/// 钩子执行辅助 trait
/// 用于在执行器中自动调用钩子
/// 内部 trait：用于自动调用 BeforeInsert 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeInsert {
    async fn call_before_insert(&mut self);
}

/// 内部 trait：用于自动调用 AfterInsert 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterInsert {
    async fn call_after_insert(&self);
}

/// 内部 trait：用于自动调用 BeforeUpdate 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeUpdate {
    async fn call_before_update(&mut self);
}

/// 内部 trait：用于自动调用 AfterUpdate 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterUpdate {
    async fn call_after_update(&self);
}

/// 内部 trait：用于自动调用 BeforeDelete 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookBeforeDelete {
    async fn call_before_delete(&self);
}

/// 内部 trait：用于自动调用 AfterDelete 钩子
#[doc(hidden)]
#[async_trait::async_trait]
pub trait HookAfterDelete {
    async fn call_after_delete(&self);
}

// 为实现了 BeforeInsert 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeInsert + Send> HookBeforeInsert for M {
    async fn call_before_insert(&mut self) {
        self.before_insert().await;
    }
}

// 为实现了 AfterInsert 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterInsert + Send + Sync> HookAfterInsert for M {
    async fn call_after_insert(&self) {
        self.after_insert().await;
    }
}

// 为实现了 BeforeUpdate 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeUpdate + Send> HookBeforeUpdate for M {
    async fn call_before_update(&mut self) {
        self.before_update().await;
    }
}

// 为实现了 AfterUpdate 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterUpdate + Send + Sync> HookAfterUpdate for M {
    async fn call_after_update(&self) {
        self.after_update().await;
    }
}

// 为实现了 BeforeDelete 的模型生成特化实现
#[async_trait::async_trait]
impl<M: BeforeDelete + Send + Sync> HookBeforeDelete for M {
    async fn call_before_delete(&self) {
        self.before_delete().await;
    }
}

// 为实现了 AfterDelete 的模型生成特化实现
#[async_trait::async_trait]
impl<M: AfterDelete + Send + Sync> HookAfterDelete for M {
    async fn call_after_delete(&self) {
        self.after_delete().await;
    }
}
