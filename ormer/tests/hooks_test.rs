#[cfg(feature = "sqlite")]
use ormer::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate, Database,
    DbType, HookContext, HookOperation, Model,
};
#[cfg(not(feature = "sqlite"))]
use ormer::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate, HookContext,
    HookOperation, Model,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;

// 全局计数器用于验证钩子是否被调用
static BEFORE_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
static BEFORE_UPDATE_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_UPDATE_COUNT: AtomicUsize = AtomicUsize::new(0);
static BEFORE_DELETE_COUNT: AtomicUsize = AtomicUsize::new(0);
static AFTER_DELETE_COUNT: AtomicUsize = AtomicUsize::new(0);
static TRANSACTION_INSERT_COUNT: AtomicUsize = AtomicUsize::new(0);
static UPDATE_BATCH_INDEX_SUM: AtomicUsize = AtomicUsize::new(0);
static DELETE_BATCH_INDEX_SUM: AtomicUsize = AtomicUsize::new(0);

// 互斥锁确保使用全局计数器的测试串行执行，避免并行时 reset_counters 互相干扰
static HOOKS_TEST_MUTEX: Mutex<()> = Mutex::const_new(());

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
    async fn before_insert(&mut self, ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        BEFORE_INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
        if ctx.in_transaction() {
            TRANSACTION_INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
        }
        if self.email == "reject@example.com" {
            return Err(ormer::ormer_error!("email rejected by hook"));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterInsert for HookTestUser {
    async fn after_insert(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        AFTER_INSERT_COUNT.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for HookTestUser {
    async fn before_update(&mut self, ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        BEFORE_UPDATE_COUNT.fetch_add(1, Ordering::SeqCst);
        if let Some(index) = ctx.batch_index() {
            UPDATE_BATCH_INDEX_SUM.fetch_add(index, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterUpdate for HookTestUser {
    async fn after_update(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        AFTER_UPDATE_COUNT.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl BeforeDelete for HookTestUser {
    async fn before_delete(&self, ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        BEFORE_DELETE_COUNT.fetch_add(1, Ordering::SeqCst);
        if let Some(index) = ctx.batch_index() {
            DELETE_BATCH_INDEX_SUM.fetch_add(index, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterDelete for HookTestUser {
    async fn after_delete(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        AFTER_DELETE_COUNT.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn reset_counters() {
    BEFORE_INSERT_COUNT.store(0, Ordering::SeqCst);
    AFTER_INSERT_COUNT.store(0, Ordering::SeqCst);
    BEFORE_UPDATE_COUNT.store(0, Ordering::SeqCst);
    AFTER_UPDATE_COUNT.store(0, Ordering::SeqCst);
    BEFORE_DELETE_COUNT.store(0, Ordering::SeqCst);
    AFTER_DELETE_COUNT.store(0, Ordering::SeqCst);
    TRANSACTION_INSERT_COUNT.store(0, Ordering::SeqCst);
    UPDATE_BATCH_INDEX_SUM.store(0, Ordering::SeqCst);
    DELETE_BATCH_INDEX_SUM.store(0, Ordering::SeqCst);
}

#[tokio::test]
async fn test_hooks_trait_definition() -> ormer::Result<()> {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let mut user = HookTestUser {
        id: 0,
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
    };

    let mut ctx = HookContext::new(HookOperation::Insert);
    user.before_insert(&mut ctx).await?;
    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);

    user.after_insert(&mut ctx).await?;
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 1);

    let mut ctx = HookContext::new(HookOperation::Update);
    user.before_update(&mut ctx).await?;
    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 1);

    user.after_update(&mut ctx).await?;
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 1);

    let mut ctx = HookContext::new(HookOperation::Delete);
    user.before_delete(&mut ctx).await?;
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 1);

    user.after_delete(&mut ctx).await?;
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 1);
    Ok(())
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_database_insert() {
    // 测试使用数据库时的钩子调用
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    #[cfg(feature = "sqlite")]
    {
        let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();

        // 创建表
        db.create_table::<HookTestUser>().execute().await.unwrap();

        // 插入数据
        let mut user = HookTestUser {
            id: 0,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        };

        db.insert(&mut user).execute().await.unwrap();
        assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 1);
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_without_hooks_skips_write_hooks() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut user = HookTestUser {
        id: 0,
        name: "No Hooks".to_string(),
        email: "no-hooks@example.com".to_string(),
    };

    user.id = db
        .insert(&mut user)
        .without_hooks()
        .execute()
        .await
        .unwrap();

    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(
        db.select::<HookTestUser>()
            .collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        1
    );

    user.name = "No Hooks Updated".to_string();
    db.update::<HookTestUser>()
        .set_model(&user)
        .without_hooks()
        .execute()
        .await
        .unwrap();

    db.delete::<HookTestUser>()
        .filter(|fields| fields.id.eq(user.id))
        .without_hooks()
        .execute()
        .await
        .unwrap();

    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_database_batch_insert() {
    // 测试批量插入时的钩子
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    #[cfg(feature = "sqlite")]
    {
        let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
        db.create_table::<HookTestUser>().execute().await.unwrap();

        let mut users = vec![
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

        db.insert(&mut users).execute().await.unwrap();
        assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 2);
        assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 2);
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_update_and_delete_executors() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut user = HookTestUser {
        id: 0,
        name: "Dana".to_string(),
        email: "dana@example.com".to_string(),
    };
    user.id = db.insert(&mut user).execute().await.unwrap();

    user.name = "Dana Updated".to_string();
    let updated = db
        .update::<HookTestUser>()
        .set_model(&user)
        .execute_with_hooks(&mut user)
        .await
        .unwrap();
    assert_eq!(updated, 1);
    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 1);

    let deleted = db
        .delete::<HookTestUser>()
        .filter(|fields| fields.id.eq(user.id))
        .execute_with_hooks(&user)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_without_hooks_skips_current_write_chain() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut user = HookTestUser {
        id: 0,
        name: "Without Hooks".to_string(),
        email: "without-hooks@example.com".to_string(),
    };

    db.insert(&mut user).execute().await.unwrap();
    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 1);

    db.update::<HookTestUser>()
        .set_model(&user)
        .without_hooks()
        .execute()
        .await
        .unwrap();
    db.delete::<HookTestUser>()
        .model(&user)
        .without_hooks()
        .execute()
        .await
        .unwrap();

    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_batch_update_and_delete_executors() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut users = vec![
        HookTestUser {
            id: 0,
            name: "Batch A".to_string(),
            email: "batch-a@example.com".to_string(),
        },
        HookTestUser {
            id: 0,
            name: "Batch B".to_string(),
            email: "batch-b@example.com".to_string(),
        },
    ];
    db.insert(&mut users).execute().await.unwrap();

    let updated = db
        .update::<HookTestUser>()
        .filter(|fields| fields.id.gt(0))
        .set(|fields| fields.name = fields.name.set("Batch Updated".to_string()))
        .execute_models_with_hooks(&mut users)
        .await
        .unwrap();
    assert_eq!(updated, 2);
    assert_eq!(BEFORE_UPDATE_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(AFTER_UPDATE_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(UPDATE_BATCH_INDEX_SUM.load(Ordering::SeqCst), 1);

    let deleted = db
        .delete::<HookTestUser>()
        .filter(|fields| fields.id.gt(0))
        .execute_models_with_hooks(&users)
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(BEFORE_DELETE_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(AFTER_DELETE_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(DELETE_BATCH_INDEX_SUM.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hooks_with_transaction_insert() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut user = HookTestUser {
        id: 0,
        name: "Eve".to_string(),
        email: "eve@example.com".to_string(),
    };
    let mut txn = db.begin().await.unwrap();
    txn.insert(&mut user).execute().await.unwrap();
    txn.commit().await.unwrap();

    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(TRANSACTION_INSERT_COUNT.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_before_insert_error_prevents_sql_execution() {
    let _guard = HOOKS_TEST_MUTEX.lock().await;
    reset_counters();

    let db = Database::connect(DbType::Sqlite, ":memory:").await.unwrap();
    db.create_table::<HookTestUser>().execute().await.unwrap();

    let mut user = HookTestUser {
        id: 0,
        name: "Rejected".to_string(),
        email: "reject@example.com".to_string(),
    };
    let error = db.insert(&mut user).execute().await.unwrap_err();
    assert!(error.to_string().contains("email rejected by hook"));
    assert_eq!(BEFORE_INSERT_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_INSERT_COUNT.load(Ordering::SeqCst), 0);

    let users: Vec<HookTestUser> = db.select::<HookTestUser>().collect().await.unwrap();
    assert!(users.is_empty());
}
