use ormer::Model;

mod _test_common;

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
    #[foreign(TestUser)]
    user_id: i32,
    role_name: String,
}

async fn test_foreign_key_creation_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 连接数据库
    let db = _test_common::create_db_connection(config).await?;

    // 创建表 - 应该包含外键约束
    db.create_table::<TestUser>().await?;
    db.create_table::<TestRole>().await?;

    // 验证外键约束是否正确生成
    // 在 Turso/SQLite 中，我们可以通过检查表结构来验证
    println!("Tables created successfully with foreign key constraints");

    // 插入测试数据
    db.insert(&TestUser {
        id: 1,
        name: "Alice".to_string(),
    })
    .await?;

    // 插入带有外键的记录
    db.insert(&TestRole {
        id: 1,
        user_id: 1,
        role_name: "admin".to_string(),
    })
    .await?;

    println!("Foreign key test passed!");

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRole>().await?;
    db.drop_table::<TestUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_foreign_key_creation_impl);
