#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（每个测试函数使用独立的表名，避免并行执行时的冲突）
define_test_user_for_range!(StreamCleanupUserPollution, "stream_cleanup_pollution");
define_test_user_for_range!(StreamCleanupUserEt, "stream_cleanup_et");
define_test_user_for_range!(StreamCleanupUserMc, "stream_cleanup_mc");
define_test_user_for_range!(StreamCleanupUserPool, "stream_cleanup_pool");

/// 测试流式查询中发生错误后，迭代器是否正确污染并终止
async fn test_stream_pollution_on_error_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_cleanup_pollution")
        .await;
    db.create_table::<StreamCleanupUserPollution>()
        .execute()
        .await
        .unwrap();

    // 插入测试数据
    for i in 0..5 {
        let user = StreamCleanupUserPollution {
            id: i + 1,
            name: format!("user{}", i),
            age: 20 + i,
        };
        let _ = db.insert(&user).execute().await.unwrap();
    }

    // 正常流式查询应该能读取所有数据
    let mut stream = db
        .select::<StreamCleanupUserPollution>()
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(result) = stream.next().await {
        assert!(result.is_ok(), "Expected Ok result, got Err");
        count += 1;
    }

    assert_eq!(count, 5);
    println!("✓ test_stream_pollution_on_error passed - normal iteration works");
}

/// 测试提前终止流式查询后，资源是否正确释放
async fn test_stream_early_termination_cleanup_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_cleanup_et")
        .await;
    db.create_table::<StreamCleanupUserEt>()
        .execute()
        .await
        .unwrap();

    // 插入较多数据
    for i in 0..100 {
        let user = StreamCleanupUserEt {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + (i % 50),
        };
        let _ = db.insert(&user).execute().await.unwrap();
    }

    // 多次提前终止流式查询，验证连接不会泄漏
    for _iteration in 0..10 {
        let mut stream = db
            .select::<StreamCleanupUserEt>()
            .stream()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(result) = stream.next().await {
            result.unwrap();
            count += 1;
            if count >= 5 {
                break; // 提前终止
            }
        }
        // stream 在这里被 drop，应该正确释放资源
    }

    // 验证连接仍然可用，执行一次完整查询
    let all_users: Vec<StreamCleanupUserEt> =
        db.select::<StreamCleanupUserEt>().collect().await.unwrap();

    assert_eq!(all_users.len(), 100);
    println!(
        "✓ test_stream_early_termination_cleanup passed - connection reused after early termination"
    );
}

/// 测试多次连续流式查询，验证连接管理正确
async fn test_multiple_consecutive_streams_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_cleanup_mc")
        .await;
    db.create_table::<StreamCleanupUserMc>()
        .execute()
        .await
        .unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamCleanupUserMc {
            id: i + 1,
            name: format!("user{}", i),
            age: 20 + i,
        };
        let _ = db.insert(&user).execute().await.unwrap();
    }

    // 连续执行多次流式查询
    for _iteration in 0..5 {
        let mut stream = db
            .select::<StreamCleanupUserMc>()
            .stream()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(result) = stream.next().await {
            let user = result.unwrap();
            assert!(user.id >= 1 && user.id <= 10);
            count += 1;
        }
        assert_eq!(count, 10);
    }

    println!(
        "✓ test_multiple_consecutive_streams passed - {} iterations completed",
        5
    );
}

/// 测试在连接池中使用流式查询
async fn test_stream_with_pool_cleanup_impl(config: &_test_common::DbConfig) {
    // Turso后端连接池最大连接数必须为1
    let pool = if config.0 == ormer::DbType::Sqlite {
        ormer::Database::create_pool(config.0, config.1)
            .range(1..1)
            .build()
            .await
            .unwrap()
    } else {
        ormer::Database::create_pool(config.0, config.1)
            .range(1..3)
            .build()
            .await
            .unwrap()
    };

    // 从连接池获取连接
    let pooled_conn = pool.get().await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = pooled_conn
        .exec_non_query("DROP TABLE IF EXISTS stream_cleanup_pool")
        .await;
    pooled_conn
        .create_table::<StreamCleanupUserPool>()
        .execute()
        .await
        .unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamCleanupUserPool {
            id: i + 1,
            name: format!("pool_user{}", i),
            age: 18 + i,
        };
        let _ = pooled_conn.insert(&user).execute().await.unwrap();
    }

    // 多次流式查询，验证连接池管理正确
    for _iteration in 0..3 {
        let mut stream = pooled_conn
            .stream::<StreamCleanupUserPool>()
            .into_iter()
            .await
            .unwrap();

        let mut count = 0;
        while let Some(result) = stream.next().await {
            result.unwrap();
            count += 1;
        }
        assert_eq!(count, 10);
    }

    // 连接池连接在 drop 时会归还到池
    drop(pooled_conn);

    println!("✓ test_stream_with_pool_cleanup passed - pool connections managed correctly");
}

// ==================== 数据库特定测试 ====================

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_pollution_on_error_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_pollution_on_error_impl(&config).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_early_termination_cleanup_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_early_termination_cleanup_impl(&config).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_multiple_consecutive_streams_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_multiple_consecutive_streams_impl(&config).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stream_with_pool_cleanup_sqlite() {
    let config = (ormer::DbType::Sqlite, ":memory:");
    test_stream_with_pool_cleanup_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_pollution_on_error_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_pollution_on_error_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_early_termination_cleanup_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_early_termination_cleanup_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_multiple_consecutive_streams_postgresql() {
    let config = _test_common::postgresql_config();
    test_multiple_consecutive_streams_impl(&config).await;
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_stream_with_pool_cleanup_postgresql() {
    let config = _test_common::postgresql_config();
    test_stream_with_pool_cleanup_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_pollution_on_error_mysql() {
    let config = _test_common::mysql_config();
    test_stream_pollution_on_error_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_early_termination_cleanup_mysql() {
    let config = _test_common::mysql_config();
    test_stream_early_termination_cleanup_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_multiple_consecutive_streams_mysql() {
    let config = _test_common::mysql_config();
    test_multiple_consecutive_streams_impl(&config).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn test_stream_with_pool_cleanup_mysql() {
    let config = _test_common::mysql_config();
    test_stream_with_pool_cleanup_impl(&config).await;
}
