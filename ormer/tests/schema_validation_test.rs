#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

use ormer::Model;

pub mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_simple!(TestUser, "schema_validation_users_1");

#[derive(Debug, Model)]
#[table = "schema_validation_users_1"]
struct TestUserDifferent {
    #[primary]
    id: i32,
    name: String,
    // 不同的字段：用 address 替换了 age
    address: String,
    email: Option<String>,
}

#[derive(Debug, Model)]
#[table = "schema_validation_users_1"]
struct TestUserMissingColumn {
    #[primary]
    id: i32,
    name: String,
    // 缺少 age 和 email 字段
}

async fn test_schema_validation_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 测试表结构验证功能 ===\n");

    // 连接到数据库
    let db = _test_common::create_db_connection(config).await?;

    // 测试 1: 首次创建表（应该成功）
    println!("测试 1: 首次创建表");
    match db.create_table::<TestUser>().execute().await {
        Ok(_) => println!("✓ 表创建成功\n"),
        Err(e) => println!("✗ 表创建失败: {e}\n"),
    }

    // 测试 2: 再次创建相同结构的表（应该成功，因为结构匹配）
    println!("测试 2: 再次创建相同结构的表");
    match db.create_table::<TestUser>().execute().await {
        Ok(_) => println!("✓ 表结构验证通过（表已存在但结构匹配）\n"),
        Err(e) => println!("✗ 表结构验证失败: {e}\n"),
    }

    // 测试 3: 尝试用不同的表结构创建（应该失败）
    println!("测试 3: 尝试用不同的表结构创建");
    let different_result = db.validate_table::<TestUserDifferent>().await;
    assert!(
        different_result.is_err(),
        "different model schema should be rejected"
    );
    println!(
        "✓ 正确检测到表结构不匹配: {:?}\n",
        different_result.as_ref().err()
    );

    // 测试 4: 尝试用缺少列的表结构创建（应该失败）
    println!("测试 4: 尝试用缺少列的表结构创建");
    let missing_result = db.validate_table::<TestUserMissingColumn>().await;
    assert!(
        missing_result.is_err(),
        "missing model columns should be rejected"
    );
    println!(
        "✓ 正确检测到表结构不匹配: {:?}\n",
        missing_result.as_ref().err()
    );

    println!("=== 测试完成 ===");

    // 清理测试表
    db.drop_table::<TestUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_schema_validation_impl);
