use ormer::Model;

// 定义测试模型
#[derive(Debug, Model, Clone)]
#[table = "test_agg_users"]
struct TestAggUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

/// 测试 COUNT 聚合函数
#[tokio::test]
async fn test_count_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试 COUNT(*)
    let count: usize = db.select::<TestAggUser>().count(|p| p.id).await?;
    println!("COUNT result: {:?}", count);

    // 验证结果
    assert_eq!(count, 3);

    Ok(())
}

/// 测试 SUM 聚合函数
#[tokio::test]
async fn test_sum_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试 SUM(age)
    let sum: Option<i32> = db.select::<TestAggUser>().sum(|p| p.age).await?;
    println!("SUM result: {:?}", sum);

    // 验证结果
    assert_eq!(sum, Some(67)); // 20 + 25 + 22

    Ok(())
}

/// 测试 AVG 聚合函数
#[tokio::test]
async fn test_avg_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试 AVG(score)
    let avg: Option<f64> = db.select::<TestAggUser>().avg(|p| p.score).await?;
    println!("AVG result: {:?}", avg);

    // 验证结果 (85 + 92 + 78) / 3 = 85.0
    assert!((avg.unwrap() - 85.0).abs() < 0.01);

    Ok(())
}

/// 测试 MAX 聚合函数
#[tokio::test]
async fn test_max_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试 MAX(age)
    let max: Option<i32> = db.select::<TestAggUser>().max(|p| p.age).await?;
    println!("MAX result: {:?}", max);

    // 验证结果
    assert_eq!(max, Some(25));

    Ok(())
}

/// 测试 MIN 聚合函数
#[tokio::test]
async fn test_min_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试 MIN(age)
    let min: Option<i32> = db.select::<TestAggUser>().min(|p| p.age).await?;
    println!("MIN result: {:?}", min);

    // 验证结果
    assert_eq!(min, Some(20));

    Ok(())
}

/// 测试带过滤条件的聚合函数
#[tokio::test]
async fn test_aggregate_with_filter() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestAggUser>().await?;

    // 插入测试数据
    db.insert(&TestAggUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92,
    })
    .await?;
    db.insert(&TestAggUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        score: 78,
    })
    .await?;

    // 测试带过滤条件的 COUNT: age >= 22
    let count: usize = db
        .select::<TestAggUser>()
        .filter(|p| p.age.ge(22))
        .count(|p| p.id)
        .await?;
    println!("COUNT with filter result: {:?}", count);

    // 验证结果 (Bob: 25, Charlie: 22)
    assert_eq!(count, 2);

    // 测试带过滤条件的 MAX: age >= 22
    let max: Option<i32> = db
        .select::<TestAggUser>()
        .filter(|p| p.age.ge(22))
        .max(|p| p.score)
        .await?;
    println!("MAX with filter result: {:?}", max);

    // 验证结果 (Bob: 92)
    assert_eq!(max, Some(92));

    Ok(())
}
