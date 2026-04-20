/// 简单的事务测试
use ormer::Model;
use ormer::abstract_layer::Database;

#[derive(Model, Debug, Clone)]
#[table = "simple_users"]
struct SimpleUser {
    #[primary]
    id: Option<i64>,
    name: String,
}

/// 测试最基本的事务提交
#[tokio::test]
async fn test_simple_transaction_commit() {
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect");

    db.create_table::<SimpleUser>()
        .await
        .expect("Failed to create table");

    // 开始事务
    let mut txn = db.begin().await.expect("Failed to begin");

    // 插入数据
    let user = SimpleUser {
        id: None,
        name: "Test User".to_string(),
    };
    txn.insert(&user).await.expect("Failed to insert");

    // 提交
    txn.commit().await.expect("Failed to commit");

    // 验证
    let users: Vec<SimpleUser> = db
        .select::<SimpleUser>()
        .collect::<Vec<SimpleUser>>()
        .await
        .expect("Failed to query");

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Test User");

    println!("✓ Transaction commit test passed");
}

/// 测试事务回滚
#[tokio::test]
async fn test_simple_transaction_rollback() {
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect");

    db.create_table::<SimpleUser>()
        .await
        .expect("Failed to create table");

    // 插入一条初始数据
    let initial = SimpleUser {
        id: None,
        name: "Initial".to_string(),
    };
    db.insert(&initial).await.expect("Failed to insert initial");

    // 开始事务
    let mut txn = db.begin().await.expect("Failed to begin");

    // 插入数据
    let user = SimpleUser {
        id: None,
        name: "Should Rollback".to_string(),
    };
    txn.insert(&user).await.expect("Failed to insert");

    // 回滚
    txn.rollback().await.expect("Failed to rollback");

    // 验证回滚后只有初始数据
    let users: Vec<SimpleUser> = db
        .select::<SimpleUser>()
        .collect::<Vec<SimpleUser>>()
        .await
        .expect("Failed to query");

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Initial");

    println!("✓ Transaction rollback test passed");
}
