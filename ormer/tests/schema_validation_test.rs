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

    // 测试 3: 用不同的表结构诊断（应给出迁移计划：补 address 列，
    // 多余的 age 列在默认 Keep 策略下保留）
    println!("测试 3: 用不同的表结构诊断");
    let different_result = db.plan_table::<TestUserDifferent>().await?;
    assert!(
        matches!(
            different_result,
            ormer::TableDiagnosis::Migratable(_)
        ),
        "different model schema should be Migratable, got {different_result:?}"
    );
    println!("✓ 诊断出可迁移的表结构差异\n");

    // 测试 4: 缺列的模型诊断（默认 Keep 策略下多余的 age 列保留，无其余
    // 差异即收敛为 Ready，不再视为错误）
    println!("测试 4: 用缺列的模型诊断");
    let missing_result = db.plan_table::<TestUserMissingColumn>().await?;
    assert!(
        matches!(missing_result, ormer::TableDiagnosis::Ready),
        "missing columns should converge under Keep policy, got {missing_result:?}"
    );
    println!("✓ 缺列模型在 Keep 策略下收敛为 Ready\n");

    println!("=== 测试完成 ===");

    // 清理测试表
    db.drop_table::<TestUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_schema_validation_impl);
