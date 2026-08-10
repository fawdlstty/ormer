#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "partial_insert_users_1"]
struct PartialInsertUser {
    #[primary(auto)]
    id: i32,
    name: String,
    email: Option<String>,
    #[default(7)]
    created_at_epoch: i32,
}

#[derive(ormer::InsertModel)]
#[table = "partial_insert_users_1"]
struct NewPartialInsertUser {
    name: String,
    email: Option<String>,
}

#[derive(ormer::InsertModel)]
#[table = "partial_insert_users_1"]
struct ActivePartialInsertUser {
    name: ormer::ActiveValue<String>,
    email: ormer::ActiveValue<Option<String>>,
    created_at_epoch: ormer::ActiveValue<i32>,
}

async fn test_partial_insert_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<PartialInsertUser>().execute().await;
    db.create_table::<PartialInsertUser>().execute().await?;

    let partial_id: i32 = db
        .insert_partial::<PartialInsertUser>()
        .set(|u| u.name.set("Alice"))
        .set(|u| (u.email, Some("alice@example.com".to_string())))
        .default(|u| u.created_at_epoch)
        .execute()
        .await?;

    assert_ne!(partial_id, 0);

    let new_user = NewPartialInsertUser {
        name: "Bob".to_string(),
        email: None,
    };
    db.insert_model::<PartialInsertUser>(&new_user)
        .execute()
        .await?;

    let active_user = ActivePartialInsertUser {
        name: ormer::ActiveValue::set("Carol".to_string()),
        email: ormer::ActiveValue::unchanged(Some("carol@example.com".to_string())),
        created_at_epoch: ormer::ActiveValue::not_set(),
    };
    db.insert_model::<PartialInsertUser>(&active_user)
        .execute()
        .await?;

    let users: Vec<PartialInsertUser> = db
        .select::<PartialInsertUser>()
        .order_by(|u| u.id.asc())
        .collect()
        .await?;

    assert_eq!(users.len(), 3);
    assert_eq!(users[0].id, partial_id);
    assert!(users.iter().all(|u| u.id != 0));
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].email, Some("alice@example.com".to_string()));
    assert_eq!(users[0].created_at_epoch, 7);
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[1].email, None);
    assert_eq!(users[1].created_at_epoch, 7);
    assert_eq!(users[2].name, "Carol");
    assert_eq!(users[2].email, Some("carol@example.com".to_string()));
    assert_eq!(users[2].created_at_epoch, 7);

    db.drop_table::<PartialInsertUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_partial_insert_impl);
