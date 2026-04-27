#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型
define_test_user_for_range!(StreamUser, "stream_users_test");

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
        db.insert(&user).await.unwrap();
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
        db.insert(&user).await.unwrap();
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
        db.insert(&user).await.unwrap();
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
        db.insert(&user).await.unwrap();
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

    // 提交事务
    txn.commit().await.unwrap();
    println!("✓ test_stream_in_transaction passed");
}

// 使用宏生成所有数据库的测试
test_on_turso_only!(test_stream_basic_impl);
test_on_turso_only!(test_stream_empty_result_impl);
test_on_turso_only!(test_stream_with_filter_impl);
test_on_turso_only!(test_stream_with_order_and_range_impl);
test_on_turso_only!(test_stream_in_transaction_impl);
