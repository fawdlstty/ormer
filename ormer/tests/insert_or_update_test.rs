#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_role_simple!(TestRole, "test_insert_or_update_roles_1");

async fn test_insert_or_update_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 连接数据库
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestRole>().execute().await?;

    // 第一次插入
    db.insert_or_update(&TestRole {
        id: 1,
        name: "admin".to_string(),
    })
    .await?;
    println!("第一次插入成功");

    // 查询验证
    let roles = db.select::<TestRole>().collect::<Vec<_>>().await?;
    println!("第一次查询: {:?}", roles);
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "admin");

    // 使用 insert_or_update 更新同一条记录（应该更新而不是报错）
    db.insert_or_update(&TestRole {
        id: 1,
        name: "super_admin".to_string(),
    })
    .await?;
    println!("第二次 insert_or_update 成功（更新操作）");

    // 再次查询验证
    let roles = db.select::<TestRole>().collect::<Vec<_>>().await?;
    println!("第二次查询: {:?}", roles);
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "super_admin"); // 验证名字已更新

    println!("\n测试通过！insert_or_update 方法在遇到重复记录时成功执行了更新操作");

    // 清理测试表
    db.drop_table::<TestRole>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_insert_or_update_impl);
