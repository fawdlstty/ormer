use ormer::Model;

mod _test_common;

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model)]
#[table = "test_roles"]
struct TestRole {
    #[primary]
    id: i32,
    user_id: i32,
    role_name: String,
}

// 单字段Model，用于测试collect_with
#[derive(Debug, Model)]
#[table = "user_ids"]
struct UserId {
    #[primary]
    id: i32,
}

async fn test_collect_with_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 创建内存数据库
    let db = _test_common::create_db_connection(config).await?;
    println!("[TEST] Created database connection");

    // 先删除表（如果存在）
    let _ = db.drop_table::<TestRole>().await;
    let _ = db.drop_table::<TestUser>().await;

    // 创建表
    db.create_table::<TestUser>().await?;
    println!("[TEST] Created TestUser table");
    db.create_table::<TestRole>().await?;
    println!("[TEST] Created TestRole table");

    // 插入测试数据
    db.insert(&TestUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .await?;
    println!("[TEST] Inserted TestUser 1");
    db.insert(&TestUser {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
    })
    .await?;
    println!("[TEST] Inserted TestUser 2");
    db.insert(&TestUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
    })
    .await?;
    println!("[TEST] Inserted TestUser 3");

    db.insert(&TestRole {
        id: 1,
        user_id: 1,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&TestRole {
        id: 2,
        user_id: 2,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&TestRole {
        id: 3,
        user_id: 3,
        role_name: "user".to_string(),
    })
    .await?;

    // 测试1: 普通的collect为基本类型
    let admin_user_ids: Vec<i32> = db
        .select::<TestRole>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.user_id)
        .collect::<Vec<i32>>()
        .await?;

    println!("Admin user IDs (as i32): {:?}", admin_user_ids);
    assert_eq!(admin_user_ids, vec![1, 2]);

    // 测试2: 使用collect_with转换为Model
    let admin_user_ids_as_model: Vec<UserId> = db
        .select::<TestRole>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.user_id)
        .collect_with(|id| UserId { id })
        .await?;

    println!(
        "Admin user IDs (as UserId Model): {:?}",
        admin_user_ids_as_model
    );
    assert_eq!(admin_user_ids_as_model.len(), 2);
    assert_eq!(admin_user_ids_as_model[0].id, 1);
    assert_eq!(admin_user_ids_as_model[1].id, 2);

    // 测试3: 元组投影
    let role_data: Vec<(i32, String)> = db
        .select::<TestRole>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| (r.user_id, r.role_name))
        .collect::<Vec<(i32, String)>>()
        .await?;

    println!("Role data (as tuple): {:?}", role_data);
    assert_eq!(role_data.len(), 2);
    assert_eq!(role_data[0], (1, "admin".to_string()));

    println!("\n✅ All tests passed!");

    // 清理测试表
    db.drop_table::<TestRole>().await?;
    db.drop_table::<TestUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_collect_with_impl);
