#![cfg(feature = "sqlite")]

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
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;

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

    // 插入用户并获取自增ID
    let id1: i32 = db.insert(&user1).execute().await?;
    let id2: i32 = db.insert(&user2).execute().await?;

    println!("Inserted user1 with id: {}, user2 with id: {}", id1, id2);

    // 验证返回的ID是自动生成的（不为0）
    assert_ne!(id1, 0, "First user id should be auto-generated");
    assert_ne!(id2, 0, "Second user id should be auto-generated");
    assert_ne!(id1, id2, "User ids should be different");

    // 查询所有用户
    let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<TestUser>>().await?;

    println!("Inserted users: {:?}", users);

    // 验证
    assert_eq!(users.len(), 2, "Should have 2 users");

    // 验证查询到的ID与返回的ID一致
    assert_eq!(users[0].id, id1, "First user id should match returned id");
    assert_eq!(users[1].id, id2, "Second user id should match returned id");

    // 验证其他字段
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].age, 25);
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[1].age, 30);

    // 测试批量插入返回第一个ID
    let user3 = TestUser {
        id: 0,
        name: "Charlie".to_string(),
        age: 35,
    };
    let user4 = TestUser {
        id: 0,
        name: "David".to_string(),
        age: 40,
    };
    let batch_id: i32 = db.insert(vec![user3, user4]).execute().await?;
    println!("Batch insert first id: {}", batch_id);
    assert!(
        batch_id > id2,
        "Batch insert id should be greater than previous ids"
    );

    println!("Auto increment test passed!");

    Ok(())
}
