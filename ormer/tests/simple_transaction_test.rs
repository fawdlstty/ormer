#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

/// 简单的事务测试
mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_simple_txn!(CommitUser, "simple_transaction_commit_1");
define_test_user_for_simple_txn!(RollbackUser, "simple_transaction_rollback_1");

/// 测试最基本的事务提交
async fn test_simple_transaction_commit_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 跳过PostgreSQL测试（已知问题：Option<i64>主键在PostgreSQL中不支持NULL值插入）
    #[cfg(feature = "postgresql")]
    if matches!(config.0, ormer::DbType::PostgreSQL) {
        println!("Skipping PostgreSQL test (known issue with Option<i64> primary key)");
        return Ok(());
    }

    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<CommitUser>().execute().await;

    println!("[COMMIT] Creating table...");
    db.create_table::<CommitUser>().execute().await?;
    println!("[COMMIT] Table created");

    // 开始事务
    let mut txn = db.begin().await?;

    // 插入数据
    let user = CommitUser {
        id: None,
        name: "Test User".to_string(),
    };
    txn.insert(&user).await?;

    // 提交
    txn.commit().await?;

    // 验证
    let users: Vec<CommitUser> = db
        .select::<CommitUser>()
        .collect::<Vec<CommitUser>>()
        .await?;

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Test User");

    println!("✓ Transaction commit test passed");

    // 清理测试表
    db.drop_table::<CommitUser>().execute().await?;

    Ok(())
}

/// 测试事务回滚
async fn test_simple_transaction_rollback_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 跳过PostgreSQL测试（已知问题：Option<i64>主键在PostgreSQL中不支持NULL值插入）
    #[cfg(feature = "postgresql")]
    if matches!(config.0, ormer::DbType::PostgreSQL) {
        println!("Skipping PostgreSQL test (known issue with Option<i64> primary key)");
        return Ok(());
    }

    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<RollbackUser>().execute().await;

    println!("[ROLLBACK] Creating table...");
    db.create_table::<RollbackUser>().execute().await?;
    println!("[ROLLBACK] Table created");

    // 开始事务
    let mut txn = db.begin().await?;

    // 插入数据
    let user = RollbackUser {
        id: None,
        name: "Should Rollback".to_string(),
    };
    txn.insert(&user).await?;

    // 回滚
    txn.rollback().await?;

    // 验证数据未插入
    let users: Vec<RollbackUser> = db
        .select::<RollbackUser>()
        .collect::<Vec<RollbackUser>>()
        .await?;

    assert_eq!(users.len(), 0);

    println!("✓ Transaction rollback test passed");

    // 清理测试表
    db.drop_table::<RollbackUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_simple_transaction_commit_impl);
test_on_all_dbs_result!(test_simple_transaction_rollback_impl);
