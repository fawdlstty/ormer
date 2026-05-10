#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型
define_test_user_for_range!(StreamLeakUser, "stream_leak_test");
define_test_user_for_range!(StreamEarlyTermUser, "stream_early_term_test");
define_test_user_for_range!(StreamTxnUser, "stream_txn_test");

/// 测试流式查询后连接是否正确释放
async fn test_stream_connection_release_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_leak_test")
        .await;
    db.create_table::<StreamLeakUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamLeakUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 20 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 执行多次流式查询，检查连接是否正确释放
    for iteration in 0..5 {
        println!("Iteration {}", iteration + 1);
        let mut stream = db
            .select::<StreamLeakUser>()
            .stream()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(user_result) = stream.next().await {
            user_result.unwrap();
            count += 1;
        }
        assert_eq!(count, 10);
        // stream 在这里 Drop，连接应该被释放
        println!("  Streamed {} users, connection should be released", count);
    }

    println!("✓ test_stream_connection_release passed");
}

/// 测试提前终止流时连接是否正确释放
async fn test_stream_early_termination_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_early_term_test")
        .await;
    db.create_table::<StreamEarlyTermUser>()
        .execute()
        .await
        .unwrap();

    // 插入较多数据用于测试提前终止
    for i in 0..100 {
        let user = StreamEarlyTermUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 20 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 提前终止流，测试连接是否正确释放
    for iteration in 0..5 {
        println!("Iteration {}", iteration + 1);
        let mut stream = db
            .select::<StreamEarlyTermUser>()
            .stream()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(user_result) = stream.next().await {
            user_result.unwrap();
            count += 1;
            if count >= 10 {
                break; // 提前终止
            }
        }
        println!(
            "  Streamed {} users (early termination), connection should be released",
            count
        );
        // stream 在这里 Drop，连接应该被释放
    }

    println!("✓ test_stream_early_termination passed");
}

/// 测试在事务中流式查询后连接是否正确释放
async fn test_stream_in_transaction_release_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_txn_test")
        .await;
    db.create_table::<StreamTxnUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamTxnUser {
            id: i + 1,
            name: format!("txn_user{}", i),
            age: 25 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 在事务中执行流式查询
    for iteration in 0..3 {
        println!("Transaction iteration {}", iteration + 1);
        let txn = db.begin().await.unwrap();

        let mut stream = txn
            .select::<StreamTxnUser>()
            .filter(|u| u.age.ge(27))
            .stream()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(user_result) = stream.next().await {
            let user = user_result.unwrap();
            assert!(user.age >= 27);
            count += 1;
        }
        println!("  Streamed {} users in transaction", count);

        // 提交事务前必须先 drop stream，释放对 txn 的借用
        drop(stream);
        txn.commit().await.unwrap();
    }

    println!("✓ test_stream_in_transaction_release passed");
}

// ==================== 数据库特定测试 ====================

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_connection_release_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_connection_release_impl(&config).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_early_termination_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_early_termination_impl(&config).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_in_transaction_release_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_in_transaction_release_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_connection_release_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_connection_release_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_early_termination_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_early_termination_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_in_transaction_release_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_in_transaction_release_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_connection_release_mysql() {
    let config = _test_common::mysql_config();
    test_stream_connection_release_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_early_termination_mysql() {
    let config = _test_common::mysql_config();
    test_stream_early_termination_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_in_transaction_release_mysql() {
    let config = _test_common::mysql_config();
    test_stream_in_transaction_release_impl(&config).await;
}
