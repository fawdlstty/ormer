#![cfg(feature = "turso")]

use ormer::Model;

#[derive(Debug, Model, Clone)]
#[table = "test_auto_increment_users"]
struct TestUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[tokio::test]
async fn test_auto_increment_insert() -> Result<(), Box<dyn std::error::Error>> {
    // 创建数据库连接
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;

    // 创建表
    db.create_table::<TestUser>().execute().await?;

    // 插入测试数据 - id 字段会被忽略
    let user1 = TestUser {
        id: 0, // 这个值应该被忽略
        name: "Alice".to_string(),
        age: 25,
    };

    let user2 = TestUser {
        id: 0, // 这个值应该被忽略
        name: "Bob".to_string(),
        age: 30,
    };

    // 插入用户
    db.insert(&user1).await?;
    db.insert(&user2).await?;

    // 查询所有用户
    let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    println!("Inserted users: {:?}", users);

    // 验证
    assert_eq!(users.len(), 2, "Should have 2 users");

    // 验证 id 是自动生成的（不为0）
    assert_ne!(users[0].id, 0, "First user id should be auto-generated");
    assert_ne!(users[1].id, 0, "Second user id should be auto-generated");
    assert_ne!(users[0].id, users[1].id, "User ids should be different");

    // 验证其他字段
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].age, 25);
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[1].age, 30);

    println!("Auto increment test passed!");

    Ok(())
}
