// 测试聚合函数的编译期类型推断
use ormer::Model;

#[derive(Model, Debug)]
#[table = "test_users"]
struct TestUser {
    #[primary]
    id: i64, // 改为 i64 用于 COUNT 测试
    name: String,
    age: i32,
    score: f64,
}

#[tokio::test]
async fn test_aggregate_type_inference() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestUser>().await?;

    // 插入测试数据
    db.insert(&TestUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
        score: 85.5,
    })
    .await?;

    db.insert(&TestUser {
        id: 2,
        name: "Bob".to_string(),
        age: 25,
        score: 92.3,
    })
    .await?;

    // 测试 MAX(age) - 编译期推断为 Option<i32>
    let max_age: Option<i32> = db.select::<TestUser>().max(|p| p.age).await?;
    println!("MAX(age): {:?}", max_age);
    assert_eq!(max_age, Some(25));

    // 测试 MIN(age) - 编译期推断为 Option<i32>
    let min_age: Option<i32> = db.select::<TestUser>().min(|p| p.age).await?;
    println!("MIN(age): {:?}", min_age);
    assert_eq!(min_age, Some(20));

    // 测试 MAX(score) - 编译期推断为 Option<f64>
    let max_score: Option<f64> = db.select::<TestUser>().max(|p| p.score).await?;
    println!("MAX(score): {:?}", max_score);
    assert!((max_score.unwrap() - 92.3).abs() < 0.01);

    // 测试 AVG(age) - 总是返回 Option<f64>
    let avg_age: Option<f64> = db.select::<TestUser>().avg(|p| p.age).await?;
    println!("AVG(age): {:?}", avg_age);
    assert!((avg_age.unwrap() - 22.5).abs() < 0.01);

    // 测试 COUNT - 返回 usize，使用 id (i64 类型)
    let count: usize = db.select::<TestUser>().count(|p| p.id).await?;
    println!("COUNT: {:?}", count);
    assert_eq!(count, 2);

    println!("All aggregate type inference tests passed!");
    Ok(())
}
