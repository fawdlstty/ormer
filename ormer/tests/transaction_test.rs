/// 事务管理测试
use ormer::Model;

mod _test_common;

#[derive(Model, Debug, Clone)]
#[table = "test_users"]
struct TestUser {
    #[primary]
    id: Option<i64>,
    name: String,
    email: String,
}

/// 测试事务提交功能
async fn test_transaction_commit_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    db.create_table::<TestUser>().await?;

    // 开始事务
    let mut txn = db.begin().await?;

    // 在事务中插入数据
    let user1 = TestUser {
        id: None,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };

    txn.insert(&user1).await?;

    // 提交事务
    txn.commit().await?;

    // 验证数据已插入
    let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    assert_eq!(users.len(), 1, "Should have 1 user after commit");
    assert_eq!(users[0].name, "Alice");

    // 清理测试表
    db.drop_table::<TestUser>().await?;

    Ok(())
}

/// 测试事务回滚功能
async fn test_transaction_rollback_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    db.create_table::<TestUser>().await?;

    // 先插入一条数据
    let initial_user = TestUser {
        id: None,
        name: "Initial".to_string(),
        email: "initial@example.com".to_string(),
    };
    db.insert(&initial_user).await?;

    // 开始事务
    let mut txn = db.begin().await?;

    // 在事务中插入数据
    let user1 = TestUser {
        id: None,
        name: "Should Rollback".to_string(),
        email: "rollback@example.com".to_string(),
    };

    txn.insert(&user1).await?;

    // 回滚事务
    txn.rollback().await?;

    // 验证事务中的数据未插入
    let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    assert_eq!(users.len(), 1, "Should have only 1 user after rollback");
    assert_eq!(users[0].name, "Initial");

    // 清理测试表
    db.drop_table::<TestUser>().await?;

    Ok(())
}

/// 测试事务中的查询功能
async fn test_transaction_with_query_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    db.create_table::<TestUser>().await?;

    // 开始事务
    let mut txn = db.begin().await?;

    // 在事务中插入数据
    let user = TestUser {
        id: None,
        name: "Query Test".to_string(),
        email: "query@example.com".to_string(),
    };

    txn.insert(&user).await?;

    // 在事务中查询（应该能看到未提交的数据）
    let users: Vec<TestUser> = txn.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    assert_eq!(users.len(), 1, "Should see 1 user in transaction");
    assert_eq!(users[0].name, "Query Test");

    // 提交事务
    txn.commit().await?;

    // 验证提交后数据仍然存在
    let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    assert_eq!(users.len(), 1, "Should have 1 user after commit");

    // 清理测试表
    db.drop_table::<TestUser>().await?;

    Ok(())
}

/// 测试事务中的更新操作
async fn test_transaction_with_update_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    db.create_table::<TestUser>().await?;

    // 先插入一条数据
    let user = TestUser {
        id: None,
        name: "Original".to_string(),
        email: "original@example.com".to_string(),
    };
    db.insert(&user).await?;

    // 开始事务
    let txn = db.begin().await?;

    // 在事务中更新数据
    #[allow(unused_imports)]
    use ormer::WhereColumn;
    txn.update::<TestUser>()
        .filter(|w| w.name.eq("Original"))
        .set(|w| w.name, "Updated".to_string())
        .execute()
        .await?;

    // 提交事务
    txn.commit().await?;

    // 验证提交后更新生效
    let users: Vec<TestUser> = db
        .select::<TestUser>()
        .filter(|w| w.name.eq("Updated"))
        .collect::<Vec<TestUser>>()
        .await?;

    assert_eq!(users.len(), 1, "Should have 1 updated user");
    assert_eq!(users[0].email, "original@example.com");

    // 清理测试表
    db.drop_table::<TestUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_transaction_commit_impl);
test_on_all_dbs_result!(test_transaction_rollback_impl);
test_on_all_dbs_result!(test_transaction_with_query_impl);
test_on_all_dbs_result!(test_transaction_with_update_impl);
