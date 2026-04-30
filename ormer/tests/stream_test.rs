#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型
define_test_user_for_range!(StreamUser, "stream_users_test");

// 定义连接池测试用的模型
define_test_user_for_pool!(StreamPoolUser, "stream_pool_users_test");

/// 测试基础流式查询
async fn test_stream_basic_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 流式查询所有用户
    let mut stream = db
        .select::<StreamUser>()
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        println!("Streamed user: {:?}", user);
        count += 1;
    }

    assert_eq!(count, 10);
    println!("✓ test_stream_basic passed");
}

/// 测试带过滤条件的流式查询
async fn test_stream_with_filter_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 流式查询年龄 >= 23 的用户
    let mut stream = db
        .select::<StreamUser>()
        .filter(|u| u.age.ge(23))
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        assert!(user.age >= 23);
        println!("Streamed user (age >= 23): {:?}", user);
        count += 1;
    }

    // 年龄 >= 23 的用户应该有 5 个 (23, 24, 25, 26, 27)
    assert_eq!(count, 5);
    println!("✓ test_stream_with_filter passed");
}

/// 测试带排序和范围限制的流式查询
async fn test_stream_with_order_and_range_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 流式查询，按年龄降序，取前 3 个
    let mut stream = db
        .select::<StreamUser>()
        .order_by_desc(|u| u.age)
        .range(0..3)
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    let mut prev_age = i32::MAX;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        assert!(user.age <= prev_age); // 验证降序
        prev_age = user.age;
        println!("Streamed user (ordered): {:?}", user);
        count += 1;
    }

    assert_eq!(count, 3);
    println!("✓ test_stream_with_order_and_range passed");
}

/// 测试空结果的流式查询
async fn test_stream_empty_result_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 流式查询（表为空）
    let mut stream = db
        .select::<StreamUser>()
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(_user_result) = stream.next().await {
        count += 1;
    }

    assert_eq!(count, 0);
    println!("✓ test_stream_empty_result passed");
}

/// 测试在事务中使用流式查询
async fn test_stream_in_transaction_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入测试数据
    for i in 0..5 {
        let user = StreamUser {
            id: i + 1,
            name: format!("txn_user{}", i),
            age: 20 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 在事务中流式查询
    let txn = db.begin().await.unwrap();
    let mut stream = txn
        .select::<StreamUser>()
        .filter(|u| u.age.ge(22))
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        assert!(user.age >= 22);
        println!("Streamed user in txn: {:?}", user);
        count += 1;
    }

    // 年龄 >= 22 的用户应该有 3 个 (22, 23, 24)
    assert_eq!(count, 3);

    // stream 必须先 drop，释放对 txn 的借用
    drop(stream);

    // 提交事务
    txn.commit().await.unwrap();
    println!("✓ test_stream_in_transaction passed");
}

// ==================== 原有测试 ====================

// 使用宏生成所有数据库的测试
// 基础测试目前只在Turso上运行(PostgreSQL/MySQL需要数据库服务)
test_on_sqlite_only!(test_stream_basic_impl);
test_on_sqlite_only!(test_stream_with_filter_impl);
test_on_sqlite_only!(test_stream_with_order_and_range_impl);
test_on_sqlite_only!(test_stream_empty_result_impl);
test_on_sqlite_only!(test_stream_in_transaction_impl);

// ==================== 新增测试用例 ====================

/// 测试大数据量流式查询
async fn test_stream_large_dataset_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入1000条测试数据
    const DATA_SIZE: i32 = 1000;
    for i in 0..DATA_SIZE {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + (i % 50),
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 流式查询所有用户
    let mut stream = db
        .select::<StreamUser>()
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        // 验证数据完整性
        assert!(user.id >= 1 && user.id <= DATA_SIZE);
        count += 1;
    }

    assert_eq!(count, DATA_SIZE as usize);
    println!(
        "✓ test_stream_large_dataset passed ({} records streamed)",
        count
    );
}

/// 测试流式查询错误处理
async fn test_stream_error_handling_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入正常数据
    for i in 0..5 {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 20 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 正常流式查询，验证不会panic
    let mut stream = db
        .select::<StreamUser>()
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        // 每个结果都应该是Ok
        assert!(user_result.is_ok());
        count += 1;
    }

    assert_eq!(count, 5);
    println!("✓ test_stream_error_handling passed");
}

/// 测试连接池中的流式查询
async fn test_stream_with_connection_pool_impl(config: &_test_common::DbConfig) {
    // Turso后端连接池最大连接数必须为1
    let pool = if config.0 == ormer::DbType::Sqlite {
        ormer::Database::create_pool(config.0, config.1)
            .range(0..1)
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
        .exec_non_query("DROP TABLE IF EXISTS stream_pool_users_test")
        .await;
    pooled_conn
        .create_table::<StreamPoolUser>()
        .execute()
        .await
        .unwrap();

    // 插入测试数据
    for i in 0..10 {
        let user = StreamPoolUser {
            id: i + 1,
            name: format!("pool_user{}", i),
            age: 18 + i,
            email: Some(format!("user{}@test.com", i)),
        };
        pooled_conn.insert(&user).execute().await.unwrap();
    }

    // 使用连接池的stream方法
    let mut stream = pooled_conn
        .stream::<StreamPoolUser>()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let user = user_result.unwrap();
        println!("Streamed pool user: {:?}", user);
        count += 1;
    }

    assert_eq!(count, 10);
    println!("✓ test_stream_with_connection_pool passed");
}

/// 测试带LIMIT的流式查询
async fn test_stream_with_limit_impl(config: &_test_common::DbConfig) {
    let db = _test_common::create_db_connection(config).await.unwrap();

    // 删除表（如果存在）并重新创建
    let _ = db
        .exec_non_query("DROP TABLE IF EXISTS stream_users_test")
        .await;
    db.create_table::<StreamUser>().execute().await.unwrap();

    // 插入10条数据
    for i in 0..10 {
        let user = StreamUser {
            id: i + 1,
            name: format!("user{}", i),
            age: 18 + i,
        };
        db.insert(&user).execute().await.unwrap();
    }

    // 流式查询，限制返回5条
    let mut stream = db
        .select::<StreamUser>()
        .range(0..5)
        .stream()
        .into_iter()
        .await
        .unwrap();

    let mut count = 0;
    while let Some(user_result) = stream.next().await {
        let _user = user_result.unwrap();
        count += 1;
    }

    assert_eq!(count, 5);
    println!("✓ test_stream_with_limit passed");
}

// ==================== 新增测试注册 ====================

// 大数据量测试 - 仅Sqlite(避免其他数据库需要准备环境)
test_on_sqlite_only!(test_stream_large_dataset_impl);

// 错误处理测试 - 仅Sqlite
test_on_sqlite_only!(test_stream_error_handling_impl);

// 连接池流式测试 - 仅Sqlite
test_on_sqlite_only!(test_stream_with_connection_pool_impl);

// LIMIT测试 - 仅Sqlite
test_on_sqlite_only!(test_stream_with_limit_impl);
