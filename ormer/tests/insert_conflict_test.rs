#![cfg(feature = "sqlite")]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_insert_conflict_users_1"]
struct ConflictUser {
    #[primary]
    id: i32,
    #[unique]
    email: String,
    name: String,
    active: bool,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_insert_conflict_memberships_1"]
struct ConflictMembership {
    #[primary]
    id: i32,
    #[unique(group = 1)]
    org_id: i32,
    #[unique(group = 1)]
    user_id: i32,
    role: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_insert_conflict_partial_users_1"]
struct PartialConflictUser {
    #[primary]
    id: i32,
    email: String,
    name: String,
    active: Option<bool>,
}

#[tokio::test]
async fn sqlite_insert_conflict_do_nothing_and_update() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<ConflictUser>().execute().await.ok();
    db.create_table::<ConflictUser>().execute().await?;

    db.insert(&ConflictUser {
        id: 1,
        email: "a@example.com".to_string(),
        name: "Alice".to_string(),
        active: true,
    })
    .execute()
    .await?;

    db.insert(&ConflictUser {
        id: 2,
        email: "a@example.com".to_string(),
        name: "Ignored".to_string(),
        active: false,
    })
    .on_conflict(|u| u.email)
    .do_nothing()
    .execute()
    .await?;

    let users = db.select::<ConflictUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");

    let incoming = ConflictUser {
        id: 3,
        email: "a@example.com".to_string(),
        name: "Updated".to_string(),
        active: false,
    };

    let update_insert = db
        .insert(&incoming)
        .on_conflict(|u| u.email)
        .do_update_if(|u| u.active.eq(true))
        .set(|u| u.name = u.name.incoming());
    update_insert.execute().await?;

    let users = db.select::<ConflictUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, 1);
    assert_eq!(users[0].name, "Updated");
    assert!(users[0].active);

    db.upsert(&ConflictUser {
        id: 1,
        email: "a@example.com".to_string(),
        name: "UpsertAlias".to_string(),
        active: true,
    })
    .execute()
    .await?;

    let users = db.select::<ConflictUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "UpsertAlias");

    db.drop_table::<ConflictUser>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_insert_conflict_composite_target() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<ConflictMembership>().execute().await.ok();
    db.create_table::<ConflictMembership>().execute().await?;

    db.insert(&ConflictMembership {
        id: 1,
        org_id: 10,
        user_id: 20,
        role: "member".to_string(),
    })
    .execute()
    .await?;

    db.insert(&ConflictMembership {
        id: 2,
        org_id: 10,
        user_id: 20,
        role: "admin".to_string(),
    })
    .on_conflict(|m| (m.org_id, m.user_id))
    .do_update()
    .set(|m| m.role = m.role.excluded())
    .execute()
    .await?;

    let rows = db
        .select::<ConflictMembership>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[0].role, "admin");

    db.drop_table::<ConflictMembership>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_insert_conflict_partial_target() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    let sql = db
        .insert(&PartialConflictUser {
            id: 1,
            email: "partial@example.com".to_string(),
            name: "Before".to_string(),
            active: Some(true),
        })
        .on_conflict(|u| u.email)
        .conflict_where(|u| u.active.is_not_null())
        .do_update()
        .set(|u| u.name = u.name.incoming())
        .to_sql()?;

    let statement = &sql.statements[0];
    assert!(statement.sql.contains("ON CONFLICT"));
    assert!(statement.sql.contains("WHERE"));
    assert!(statement.sql.contains("IS NOT NULL"));

    Ok(())
}

#[tokio::test]
async fn sqlite_pooled_insert_conflict_update() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let pool = ormer::Database::create_pool(config.0, config.1)
        .range(0..1)
        .build()
        .await?;
    let conn = pool.get().await?;

    conn.drop_table::<ConflictUser>().execute().await.ok();
    conn.create_table::<ConflictUser>().execute().await?;

    conn.insert(&ConflictUser {
        id: 1,
        email: "pool@example.com".to_string(),
        name: "Before".to_string(),
        active: true,
    })
    .execute()
    .await?;

    conn.insert(&ConflictUser {
        id: 2,
        email: "pool@example.com".to_string(),
        name: "After".to_string(),
        active: true,
    })
    .on_conflict(|u| u.email)
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;

    let users = conn.select::<ConflictUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "After");

    conn.drop_table::<ConflictUser>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_transaction_insert_conflict_update() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<ConflictUser>().execute().await.ok();
    db.create_table::<ConflictUser>().execute().await?;

    db.insert(&ConflictUser {
        id: 1,
        email: "txn@example.com".to_string(),
        name: "Before".to_string(),
        active: true,
    })
    .execute()
    .await?;

    let mut txn = db.begin().await?;
    txn.insert(&ConflictUser {
        id: 2,
        email: "txn@example.com".to_string(),
        name: "After".to_string(),
        active: true,
    })
    .on_conflict(|u| u.email)
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;
    txn.commit().await?;

    let users = db.select::<ConflictUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "After");

    db.drop_table::<ConflictUser>().execute().await?;
    Ok(())
}
