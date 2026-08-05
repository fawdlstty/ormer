// 钩子自动触发示例
// 展示如何实现钩子并在数据库操作中自动触发

use ormer::{AfterInsert, AfterUpdate, BeforeInsert, BeforeUpdate, HookContext, Model};
#[cfg(feature = "sqlite")]
use ormer::{Database, DbType};
#[cfg(feature = "sqlite")]
use std::sync::atomic::{AtomicUsize, Ordering};

// 全局计数器
#[cfg(feature = "sqlite")]
static INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "sqlite")]
static UPDATE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "sqlite")]
#[derive(Debug, Model)]
#[table = "hook_demo_users"]
struct HookDemoUser {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

// 实现插入前钩子
#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl BeforeInsert for HookDemoUser {
    async fn before_insert(&mut self, _ctx: &mut HookContext<'_>) -> anyhow::Result<()> {
        INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
        println!("BeforeInsert: 准备插入用户 {}", self.name);
        Ok(())
    }
}

// 实现插入后钩子
#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl AfterInsert for HookDemoUser {
    async fn after_insert(&self, _ctx: &mut HookContext<'_>) -> anyhow::Result<()> {
        println!("AfterInsert: 用户 {} 已成功插入", self.name);
        Ok(())
    }
}

// 实现更新前钩子
#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl BeforeUpdate for HookDemoUser {
    async fn before_update(&mut self, _ctx: &mut HookContext<'_>) -> anyhow::Result<()> {
        UPDATE_COUNT.fetch_add(1, Ordering::SeqCst);
        println!("BeforeUpdate: 准备更新用户 {}", self.name);
        Ok(())
    }
}

// 实现更新后钩子
#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl AfterUpdate for HookDemoUser {
    async fn after_update(&self, _ctx: &mut HookContext<'_>) -> anyhow::Result<()> {
        println!("AfterUpdate: 用户 {} 已成功更新", self.name);
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_demo() {
    #[cfg(feature = "sqlite")]
    {
        let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();

        // 创建表
        db.create_table::<HookDemoUser>().execute().await.unwrap();

        // 插入数据 - 钩子会自动触发
        let user = HookDemoUser {
            id: 0,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        };

        let mut user = user;
        user.id = db.insert(&mut user).execute().await.unwrap();

        assert_eq!(INSERT_COUNT.load(Ordering::SeqCst), 1);

        // 更新数据
        user.name = "Alice Updated".to_string();
        db.update::<HookDemoUser>()
            .set_model(&user)
            .execute_with_hooks(&mut user)
            .await
            .unwrap();

        assert_eq!(UPDATE_COUNT.load(Ordering::SeqCst), 1);
    }
}
