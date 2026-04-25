/// 简单的事务测试
use ormer::Model;

mod _test_common;

#[derive(Model, Debug, Clone)]
#[table = "simple_users"]
struct SimpleUser {
    #[primary]
    id: Option<i64>,
    name: String,
}

/// 测试最基本的事务提交
async fn test_simple_transaction_commit_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    db.create_table::<SimpleUser>().await?;

    // 开始事务
    let mut txn = db.begin().await?;

    // 插入数据
    let user = SimpleUser {
        id: None,
        name: "Test User".to_string(),
    };
    txn.insert(&user).await?;

    // 提交
    txn.commit().await?;

    // 验证
    let users: Vec<SimpleUser> = db
        .select::<SimpleUser>()
        .collect::<Vec<SimpleUser>>()
        .await?;

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Test User");

    println!("✓ Transaction commit test passed");

    // 清理测试表
    db.drop_table::<SimpleUser>().await?;

    Ok(())
}

/// 测试事务回滚
async fn test_simple_transaction_rollback_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    db.create_table::<SimpleUser>().await?;

    // 开始事务
    let mut txn = db.begin().await?;

    // 插入数据
    let user = SimpleUser {
        id: None,
        name: "Should Rollback".to_string(),
    };
    txn.insert(&user).await?;

    // 回滚
    txn.rollback().await?;

    // 验证数据未插入
    let users: Vec<SimpleUser> = db
        .select::<SimpleUser>()
        .collect::<Vec<SimpleUser>>()
        .await?;

    assert_eq!(users.len(), 0);

    println!("✓ Transaction rollback test passed");

    // 清理测试表
    db.drop_table::<SimpleUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_simple_transaction_commit_impl);
test_on_all_dbs_result!(test_simple_transaction_rollback_impl);
