#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(SetModelUser, "set_model_users_1");

async fn test_set_model_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<SetModelUser>().execute().await?;

    // 插入初始用户
    db.insert(&SetModelUser {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .execute()
    .await?;

    // 使用 set_model 更新所有非主键字段
    let updated_user = SetModelUser {
        id: 1,
        name: "Bob".to_string(),
        age: 25,
        email: Some("bob@test.com".to_string()),
    };
    db.update::<SetModelUser>()
        .set_model(&updated_user)
        .execute()
        .await?;

    // 验证更新结果
    let users: Vec<SetModelUser> = db
        .select::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Bob");
    assert_eq!(users[0].age, 25);
    assert_eq!(users[0].email, Some("bob@test.com".to_string()));

    // 测试第二个用户的 set_model 更新
    db.insert(&SetModelUser {
        id: 2,
        name: "Charlie".to_string(),
        age: 20,
        email: None,
    })
    .execute()
    .await?;

    let user2 = SetModelUser {
        id: 2,
        name: "David".to_string(),
        age: 30,
        email: Some("david@test.com".to_string()),
    };
    db.update::<SetModelUser>()
        .set_model(&user2)
        .execute()
        .await?;

    let users2: Vec<SetModelUser> = db
        .select::<SetModelUser>()
        .filter(|p| p.id.eq(2))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users2.len(), 1);
    assert_eq!(users2[0].name, "David");
    assert_eq!(users2[0].age, 30);

    // 验证用户1没有被第二次更新影响
    let users1: Vec<SetModelUser> = db
        .select::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users1.len(), 1);
    assert_eq!(users1[0].name, "Bob");

    // 清理
    db.drop_table::<SetModelUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_set_model_impl);
