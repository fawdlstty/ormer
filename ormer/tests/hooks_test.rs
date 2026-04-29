#[cfg(feature = "sqlite")]
use ormer::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate, Database,
    DbType, Model,
};
#[cfg(not(feature = "sqlite"))]
use ormer::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate, Model,
};
use std::sync::atomic::{AtomicUsize, Ordering};

// 全局计数器用于验证钩子是否被调用
static BEFORE_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
static BEFORE_UPDATE_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_UPDATE_COUNT: AtomicUsize = AtomicUsize::new(0);
static BEFORE_DELETE_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_DELETE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Model)]
#[table = "hook_test_users"]
struct HookTestUser {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

#[async_trait::async_trait]
impl BeforeInsert for HookTestUser {
    async fn before_insert(&mut self) {
        BEFORE_INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl AfterInsert for HookTestUser {
    async fn after_insert(&self) {
        AFTER_INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for HookTestUser {
    async fn before_update(&mut self) {
        BEFORE_UPDATE_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl AfterUpdate for HookTestUser {
    async fn after_update(&self) {
        AFTER_UPDATE_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl BeforeDelete for HookTestUser {
    async fn before_delete(&self) {
        BEFORE_DELETE_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl AfterDelete for HookTestUser {
    async fn after_delete(&self) {
        AFTER_DELETE_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

fn reset_counters() {
    BEFORE_INSERT_COUNT.store(0, Ordering::SeqCst);
    AFTER_INSERT_COUNT.store(0, Ordering::SeqCst);
    BEFORE_UPDATE_COUNT.store(0, Ordering::SeqCst);
    AFTER_UPDATE_COUNT.store(0, Ordering::SeqCst);
    BEFORE_DELETE_COUNT.store(0, Ordering::SeqCst);
    AFTER_DELETE_COUNT.store(0, Ordering::SeqCst);
}

#[tokio::test]
async fn test_hooks_trait_definition() {
    // 这个测试验证钩子 traits 可以正确实现
    // 注意：由于 Rust 类型系统限制，自动触发机制需要更复杂的实现
    // 当前版本提供了钩子 traits 定义，用户可以手动调用

    reset_counters();

    let mut user = HookTestUser {
        id: 0,
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
    };

    // 手动调用钩子验证实现正确性
    user.before_insert().await;
    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);

    user.after_insert().await;
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 1);

    user.before_update().await;
    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 1);

    user.after_update().await;
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 1);

    user.before_delete().await;
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 1);

    user.after_delete().await;
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_database_insert() {
    // 测试使用数据库时的钩子调用
    reset_counters();

    #[cfg(feature = "sqlite")]
    {
        let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();

        // 创建表
        db.create_table::<HookTestUser>().execute().await.unwrap();

        // 插入数据
        let user = HookTestUser {
            id: 0,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        };

        db.insert(&user).execute().await.unwrap();

        // 注意：由于自动触发机制尚未完全实现，这里验证基本功能
        // 未来版本将支持自动调用钩子
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_database_batch_insert() {
    // 测试批量插入时的钩子
    reset_counters();

    #[cfg(feature = "sqlite")]
    {
        let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
        db.create_table::<HookTestUser>().execute().await.unwrap();

        let users = vec![
            HookTestUser {
                id: 0,
                name: "Bob".to_string(),
                email: "bob@example.com".to_string(),
            },
            HookTestUser {
                id: 0,
                name: "Charlie".to_string(),
                email: "charlie@example.com".to_string(),
            },
        ];

        db.insert(&users).execute().await.unwrap();

        // 验证批量插入功能正常
    }
}
