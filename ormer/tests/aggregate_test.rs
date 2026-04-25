use ormer::Model;

mod _test_common;

// 定义测试模型 - 每个测试使用不同的表名避免并发冲突
#[derive(Debug, Model, Clone)]
#[table = "test_agg_count_users"]
struct TestAggCountUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_agg_sum_users"]
struct TestAggSumUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_agg_avg_users"]
struct TestAggAvgUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_agg_max_users"]
struct TestAggMaxUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_agg_min_users"]
struct TestAggMinUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_agg_filter_users"]
struct TestAggFilterUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

/// 测试 COUNT 聚合函数
async fn test_count_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggCountUser>().await;
    db.create_table::<TestAggCountUser>().await?;

    db.insert(&TestAggCountUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggCountUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggCountUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let count: usize = db.select::<TestAggCountUser>().count(|p| p.id).await?;
    println!("COUNT result: {:?}", count);

    assert_eq!(count, 3);

    let _ = db.drop_table::<TestAggCountUser>().await;

    Ok(())
}

/// 测试 SUM 聚合函数
async fn test_sum_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggSumUser>().await;
    db.create_table::<TestAggSumUser>().await?;

    db.insert(&TestAggSumUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggSumUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggSumUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let sum: Option<i32> = db.select::<TestAggSumUser>().sum(|p| p.age).await?;
    println!("SUM result: {:?}", sum);

    assert_eq!(sum, Some(67)); // 20 + 25 + 22

    db.drop_table::<TestAggSumUser>().await?;

    Ok(())
}

/// 测试 AVG 聚合函数
async fn test_avg_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggAvgUser>().await;
    db.create_table::<TestAggAvgUser>().await?;

    db.insert(&TestAggAvgUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggAvgUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggAvgUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let avg: Option<f64> = db.select::<TestAggAvgUser>().avg(|p| p.score).await?;
    println!("AVG result: {:?}", avg);

    assert!((avg.unwrap() - 85.0).abs() < 0.01);

    db.drop_table::<TestAggAvgUser>().await?;

    Ok(())
}

/// 测试 MAX 聚合函数
async fn test_max_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggMaxUser>().await;
    db.create_table::<TestAggMaxUser>().await?;

    db.insert(&TestAggMaxUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggMaxUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggMaxUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let max: Option<i32> = db.select::<TestAggMaxUser>().max(|p| p.age).await?;
    println!("MAX result: {:?}", max);

    assert_eq!(max, Some(25));

    db.drop_table::<TestAggMaxUser>().await?;

    Ok(())
}

/// 测试 MIN 聚合函数
async fn test_min_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggMinUser>().await;
    db.create_table::<TestAggMinUser>().await?;

    db.insert(&TestAggMinUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggMinUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggMinUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let min: Option<i32> = db.select::<TestAggMinUser>().min(|p| p.age).await?;
    println!("MIN result: {:?}", min);

    assert_eq!(min, Some(20));

    db.drop_table::<TestAggMinUser>().await?;

    Ok(())
}

/// 测试带过滤条件的聚合函数
async fn test_aggregate_with_filter_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<TestAggFilterUser>().await;
    db.create_table::<TestAggFilterUser>().await?;

    db.insert(&TestAggFilterUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggFilterUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggFilterUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    let count: usize = db
        .select::<TestAggFilterUser>()
        .filter(|p| p.age.ge(22))
        .count(|p| p.id)
        .await?;
    println!("COUNT with filter result: {:?}", count);

    assert_eq!(count, 2);

    let max: Option<i32> = db
        .select::<TestAggFilterUser>()
        .filter(|p| p.age.ge(22))
        .max(|p| p.score)
        .await?;
    println!("MAX with filter result: {:?}", max);

    assert_eq!(max, Some(92));

    db.drop_table::<TestAggFilterUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_count_aggregate_impl);
test_on_all_dbs_result!(test_sum_aggregate_impl);
test_on_all_dbs_result!(test_avg_aggregate_impl);
test_on_all_dbs_result!(test_max_aggregate_impl);
test_on_all_dbs_result!(test_min_aggregate_impl);
test_on_all_dbs_result!(test_aggregate_with_filter_impl);
