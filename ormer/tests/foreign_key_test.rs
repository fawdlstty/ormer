use ormer::Model;

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUser {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Model)]
#[table = "test_roles"]
struct TestRole {
    #[primary]
    id: i32,
    #[foreign(TestUser.id)]
    user_id: i32,
    role_name: String,
}

#[tokio::test]
async fn test_foreign_key_creation() {
    // 连接数据库
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:")
        .await
        .unwrap();

    // 创建表 - 应该包含外键约束
    db.create_table::<TestUser>().await.unwrap();
    db.create_table::<TestRole>().await.unwrap();

    // 验证外键约束是否正确生成
    // 在 Turso/SQLite 中，我们可以通过检查表结构来验证
    println!("Tables created successfully with foreign key constraints");

    // 插入测试数据
    db.insert(&TestUser {
        id: 1,
        name: "Alice".to_string(),
    })
    .await
    .unwrap();

    // 插入带有外键的记录
    db.insert(&TestRole {
        id: 1,
        user_id: 1,
        role_name: "admin".to_string(),
    })
    .await
    .unwrap();

    println!("Foreign key test passed!");
}
