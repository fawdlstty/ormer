#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

pub mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(SetModelUser, "set_model_users_1");
define_test_user!(BatchSetModelUser, "set_model_users_batch_1");
define_test_user!(TrackedSaveUser, "tracked_save_users_1");

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

    // 字段白名单更新只修改指定列
    let partial_user = SetModelUser {
        id: 1,
        name: "Carol".to_string(),
        age: 99,
        email: Some("carol@test.com".to_string()),
    };
    db.update::<SetModelUser>()
        .set_model_fields(&partial_user, |p| (p.name, p.email))
        .execute()
        .await?;

    let partial_users: Vec<SetModelUser> = db
        .select::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(partial_users.len(), 1);
    assert_eq!(partial_users[0].name, "Carol");
    assert_eq!(partial_users[0].age, 25);
    assert_eq!(partial_users[0].email, Some("carol@test.com".to_string()));

    let increment_sql = db
        .update::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .set(|p| p.age += 1)
        .to_sql()?;
    assert!(increment_sql.statements[0].sql.contains("age = age +"));

    db.update::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .set(|p| p.age += 1)
        .execute()
        .await?;

    let incremented_users: Vec<SetModelUser> = db
        .select::<SetModelUser>()
        .filter(|p| p.id.eq(1))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(incremented_users.len(), 1);
    assert_eq!(incremented_users[0].age, 26);

    // 清理
    db.drop_table::<SetModelUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_set_model_impl);

async fn test_tracked_save_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    let _ = db.drop_table::<TrackedSaveUser>().execute().await;
    db.create_table::<TrackedSaveUser>().execute().await?;

    db.insert(&TrackedSaveUser {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: Some("alice@test.com".to_string()),
    })
    .execute()
    .await?;

    let mut user = db.find_by_id::<TrackedSaveUser>(1).await?.unwrap().track();
    assert!(!user.is_dirty());
    assert!(db.save(&mut user).to_sql()?.statements.is_empty());

    user.name = "Alice Updated".to_string();
    assert_eq!(user.dirty_columns(), vec!["name".to_string()]);

    let save_sql = db.save(&mut user).to_sql()?;
    assert_eq!(save_sql.statements.len(), 1);
    let sql = &save_sql.statements[0].sql;
    assert!(sql.contains("name"));
    assert!(!sql.contains("age"));
    assert!(!sql.contains("email"));

    let affected = db.save(&mut user).execute().await?;
    assert!(affected > 0);
    assert!(!user.is_dirty());
    assert_eq!(db.save(&mut user).execute().await?, 0);

    user.email = Some("alice-updated@test.com".to_string());
    assert!(db.save(&mut user).execute().await? > 0);

    let saved = db.find_by_id::<TrackedSaveUser>(1).await?.unwrap();
    assert_eq!(saved.name, "Alice Updated");
    assert_eq!(saved.age, 18);
    assert_eq!(saved.email, Some("alice-updated@test.com".to_string()));

    db.drop_table::<TrackedSaveUser>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_tracked_save_impl);

async fn test_batch_set_model_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    let _ = db.drop_table::<BatchSetModelUser>().execute().await;
    db.create_table::<BatchSetModelUser>().execute().await?;

    db.insert(&vec![
        BatchSetModelUser {
            id: 1,
            name: "Batch Alice".to_string(),
            age: 18,
            email: None,
        },
        BatchSetModelUser {
            id: 2,
            name: "Batch Bob".to_string(),
            age: 20,
            email: None,
        },
        BatchSetModelUser {
            id: 3,
            name: "Batch Carol".to_string(),
            age: 22,
            email: None,
        },
    ])
    .execute()
    .await?;

    let updates = vec![
        BatchSetModelUser {
            id: 1,
            name: "Batch Alice Updated".to_string(),
            age: 31,
            email: Some("alice-batch@test.com".to_string()),
        },
        BatchSetModelUser {
            id: 2,
            name: "Batch Bob Updated".to_string(),
            age: 32,
            email: Some("bob-batch@test.com".to_string()),
        },
    ];
    let sql = db
        .update::<BatchSetModelUser>()
        .set_model(&updates)
        .to_sql()?;
    assert_eq!(sql.statements.len(), 1);

    let affected = db
        .update::<BatchSetModelUser>()
        .set_model(&updates)
        .execute()
        .await?;
    assert!(affected > 0);

    let user1 = db
        .select::<BatchSetModelUser>()
        .filter(|p| p.id.eq(1))
        .first()
        .await?
        .unwrap();
    assert_eq!(user1.name, "Batch Alice Updated");
    assert_eq!(user1.age, 31);
    assert_eq!(user1.email, Some("alice-batch@test.com".to_string()));

    let partial_updates = vec![
        BatchSetModelUser {
            id: 1,
            name: "Batch Alice Partial".to_string(),
            age: 99,
            email: Some("alice-partial@test.com".to_string()),
        },
        BatchSetModelUser {
            id: 2,
            name: "Batch Bob Partial".to_string(),
            age: 98,
            email: Some("bob-partial@test.com".to_string()),
        },
    ];
    let partial_sql = db
        .update::<BatchSetModelUser>()
        .set_model_fields(&partial_updates, |p| (p.name, p.email))
        .to_sql()?;
    assert_eq!(partial_sql.statements.len(), 1);

    let affected = db
        .update::<BatchSetModelUser>()
        .set_model_fields(&partial_updates, |p| (p.name, p.email))
        .execute()
        .await?;
    assert!(affected > 0);

    let user1 = db
        .select::<BatchSetModelUser>()
        .filter(|p| p.id.eq(1))
        .first()
        .await?
        .unwrap();
    assert_eq!(user1.name, "Batch Alice Partial");
    assert_eq!(user1.age, 31);
    assert_eq!(user1.email, Some("alice-partial@test.com".to_string()));

    let untouched = db
        .select::<BatchSetModelUser>()
        .filter(|p| p.id.eq(3))
        .first()
        .await?
        .unwrap();
    assert_eq!(untouched.name, "Batch Carol");
    assert_eq!(untouched.age, 22);

    db.drop_table::<BatchSetModelUser>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_batch_set_model_impl);
