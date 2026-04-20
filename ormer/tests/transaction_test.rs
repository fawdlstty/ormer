/// 事务管理测试
use ormer::Model;
use ormer::abstract_layer::Database;

#[derive(Model, Debug, Clone)]
#[table = "test_users"]
struct TestUser {
    #[primary]
    id: Option<i64>,
    name: String,
    email: String,
}

/// 测试事务提交功能
#[tokio::test]
async fn test_transaction_commit() {
    // 使用 Turso 数据库进行测试
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect to database");

    // 创建表
    db.create_table::<TestUser>()
        .await
        .expect("Failed to create table");

    // 开始事务
    let txn = db.begin().await.expect("Failed to begin transaction");

    // 在事务中插入数据
    let user1 = TestUser {
        id: None,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };

    txn.insert(&user1).await.expect("Failed to insert user1");

    // 提交事务
    txn.commit().await.expect("Failed to commit transaction");

    // 验证数据已插入
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query users");

    assert_eq!(users.len(), 1, "Should have 1 user after commit");
    assert_eq!(users[0].name, "Alice");
}

/// 测试事务回滚功能
#[tokio::test]
async fn test_transaction_rollback() {
    // 使用 Turso 数据库进行测试
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect to database");

    // 创建表
    db.create_table::<TestUser>()
        .await
        .expect("Failed to create table");

    // 先插入一条数据
    let initial_user = TestUser {
        id: None,
        name: "Initial".to_string(),
        email: "initial@example.com".to_string(),
    };
    db.insert(&initial_user)
        .await
        .expect("Failed to insert initial user");

    // 开始事务
    let txn = db.begin().await.expect("Failed to begin transaction");

    // 在事务中插入数据
    let user1 = TestUser {
        id: None,
        name: "Should Rollback".to_string(),
        email: "rollback@example.com".to_string(),
    };

    txn.insert(&user1)
        .await
        .expect("Failed to insert user in transaction");

    // 回滚事务
    txn.rollback()
        .await
        .expect("Failed to rollback transaction");

    // 验证事务中的数据未插入
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query users");

    assert_eq!(users.len(), 1, "Should have only 1 user after rollback");
    assert_eq!(users[0].name, "Initial");
}

/// 测试事务中的查询功能
#[tokio::test]
async fn test_transaction_with_query() {
    // 使用 Turso 数据库进行测试
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect to database");

    // 创建表
    db.create_table::<TestUser>()
        .await
        .expect("Failed to create table");

    // 开始事务
    let txn = db.begin().await.expect("Failed to begin transaction");

    // 在事务中插入数据
    let user = TestUser {
        id: None,
        name: "Query Test".to_string(),
        email: "query@example.com".to_string(),
    };

    txn.insert(&user).await.expect("Failed to insert user");

    // 在事务中查询（应该能看到未提交的数据）
    let users: Vec<TestUser> = txn
        .select::<TestUser>()
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query users in transaction");

    assert_eq!(users.len(), 1, "Should see 1 user in transaction");
    assert_eq!(users[0].name, "Query Test");

    // 提交事务
    txn.commit().await.expect("Failed to commit transaction");

    // 验证提交后数据仍然存在
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query users after commit");

    assert_eq!(users.len(), 1, "Should have 1 user after commit");
}

/// 测试事务中的更新操作
#[tokio::test]
async fn test_transaction_with_update() {
    // 使用 Turso 数据库进行测试
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect to database");

    // 创建表
    db.create_table::<TestUser>()
        .await
        .expect("Failed to create table");

    // 先插入一条数据
    let user = TestUser {
        id: None,
        name: "Original".to_string(),
        email: "original@example.com".to_string(),
    };
    db.insert(&user).await.expect("Failed to insert user");

    // 开始事务
    let txn = db.begin().await.expect("Failed to begin transaction");

    // 在事务中更新数据
    use ormer::WhereColumn;
    txn.update::<TestUser>()
        .filter(|w| w.name.eq("Original"))
        .set(|w| w.name, "Updated".to_string())
        .execute()
        .await
        .expect("Failed to update user");

    // 提交事务
    txn.commit().await.expect("Failed to commit transaction");

    // 验证提交后更新生效
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .filter(|w| w.name.eq("Updated"))
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query updated user");

    assert_eq!(users.len(), 1, "Should have 1 updated user after commit");
    assert_eq!(users[0].name, "Updated");
}

/// 测试事务中的删除操作
#[tokio::test]
async fn test_transaction_with_delete() {
    // 使用 Turso 数据库进行测试
    let db = Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .expect("Failed to connect to database");

    // 创建表
    db.create_table::<TestUser>()
        .await
        .expect("Failed to create table");

    // 插入两条数据
    let user1 = TestUser {
        id: None,
        name: "ToDelete".to_string(),
        email: "delete@example.com".to_string(),
    };
    let user2 = TestUser {
        id: None,
        name: "ToKeep".to_string(),
        email: "keep@example.com".to_string(),
    };
    db.insert(&user1).await.expect("Failed to insert user1");
    db.insert(&user2).await.expect("Failed to insert user2");

    // 开始事务
    let txn = db.begin().await.expect("Failed to begin transaction");

    // 在事务中删除数据
    txn.delete::<TestUser>()
        .filter(|w| w.name.eq("ToDelete"))
        .execute()
        .await
        .expect("Failed to delete user");

    // 提交事务
    txn.commit().await.expect("Failed to commit transaction");

    // 验证提交后删除生效
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .collect::<Vec<TestUser>>()
        .await
        .expect("Failed to query users");

    assert_eq!(users.len(), 1, "Should have 1 user after delete and commit");
    assert_eq!(users[0].name, "ToKeep");
}
