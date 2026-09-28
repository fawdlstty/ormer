#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

use ormer::Model;

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
    config: &crate::_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 测试表结构验证功能 ===\n");

    // 连接到数据库
    let db = crate::_test_common::create_db_connection(config).await?;

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

    // 测试 3: 用不同的表结构迁移（应给出迁移计划：补 address 列并删除
    // 模型中没有的多余列，键、列及配置与模型完全一致）
    println!("测试 3: 用不同的表结构迁移");
    let different_outcome = db
        .migrate_table::<TestUserDifferent>()
        .await?;
    assert!(
        matches!(
            different_outcome.diagnosis,
            ormer::TableDiagnosis::Migratable(_)
        ),
        "different model schema should be Migratable, got {:?}",
        different_outcome.diagnosis
    );
    println!("✓ 诊断出可迁移的表结构差异\n");

    // 测试 4: 缺列的模型诊断（多余列固定删除：执行前为 Migratable，
    // 执行后收敛为 Ready，表与模型完全一致）
    println!("测试 4: 用缺列的模型诊断");
    let missing_result = db
        .migrate_table::<TestUserMissingColumn>()
        .await?;
    assert!(
        matches!(
            missing_result.diagnosis,
            ormer::TableDiagnosis::Migratable(_)
        ),
        "extra columns should be planned for removal, got {:?}",
        missing_result.diagnosis
    );
    assert!(
        matches!(
            db.migrate_table::<TestUserMissingColumn>().await?.diagnosis,
            ormer::TableDiagnosis::Ready
        ),
        "missing-column model should converge after dropping extras"
    );
    println!("✓ 缺列模型删除多余列后收敛为 Ready\n");

    println!("=== 测试完成 ===");

    // 清理测试表
    db.drop_table::<TestUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_schema_validation_impl);
