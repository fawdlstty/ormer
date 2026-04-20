use ormer::Model;

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUserDifferent {
    #[primary]
    id: i32,
    name: String,
    // 不同的字段：用 address 替换了 age
    address: String,
    email: Option<String>,
}

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUserMissingColumn {
    #[primary]
    id: i32,
    name: String,
    // 缺少 age 和 email 字段
}

#[tokio::test]
async fn test_schema_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 测试表结构验证功能 ===\n");

    // 连接到数据库
    let db = ormer::Database::connect(ormer::DbType::Turso, "data.db").await?;

    // 测试 1: 首次创建表（应该成功）
    println!("测试 1: 首次创建表");
    match db.create_table::<TestUser>().await {
        Ok(_) => println!("✓ 表创建成功\n"),
        Err(e) => println!("✗ 表创建失败: {e}\n"),
    }

    // 测试 2: 再次创建相同结构的表（应该成功，因为结构匹配）
    println!("测试 2: 再次创建相同结构的表");
    match db.create_table::<TestUser>().await {
        Ok(_) => println!("✓ 表结构验证通过（表已存在但结构匹配）\n"),
        Err(e) => println!("✗ 表结构验证失败: {e}\n"),
    }

    // 测试 3: 尝试用不同的表结构创建（应该失败）
    println!("测试 3: 尝试用不同的表结构创建");
    match db.create_table::<TestUserDifferent>().await {
        Ok(_) => println!("✗ 应该失败但却成功了\n"),
        Err(e) => println!("✓ 正确检测到表结构不匹配: {e}\n"),
    }

    // 测试 4: 尝试用缺少列的表结构创建（应该失败）
    println!("测试 4: 尝试用缺少列的表结构创建");
    match db.create_table::<TestUserMissingColumn>().await {
        Ok(_) => println!("✗ 应该失败但却成功了\n"),
        Err(e) => println!("✓ 正确检测到表结构不匹配: {e}\n"),
    }

    println!("=== 测试完成 ===");

    Ok(())
}
