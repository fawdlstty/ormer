#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_ignore_select_users_1"]
struct IgnoreSelectUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[test]
fn ignore_select_sql_uses_default_expr_without_annotations() {
    #[cfg(feature = "sqlite")]
    {
        let (sql, _) = ormer::Select::<IgnoreSelectUser>::new()
            .ignore(|p| (p.id, p.email))
            .to_sql_with_params(ormer::DbType::Sqlite);
        assert_eq!(
            sql,
            "SELECT 0 AS id, name, age, NULL AS email FROM test_ignore_select_users_1"
        );
    }

    #[cfg(feature = "postgresql")]
    {
        let (sql, _) = ormer::Select::<IgnoreSelectUser>::new()
            .ignore(|p| (p.id, p.email))
            .to_sql_with_params(ormer::DbType::PostgreSQL);
        assert_eq!(
            sql,
            "SELECT 0::INTEGER AS id, name, age, NULL::TEXT AS email FROM test_ignore_select_users_1"
        );
    }

    #[cfg(feature = "mysql")]
    {
        let (sql, _) = ormer::Select::<IgnoreSelectUser>::new()
            .ignore(|p| (p.id, p.email))
            .to_sql_with_params(ormer::DbType::MySQL);
        assert_eq!(
            sql,
            "SELECT 0 AS id, name, age, NULL AS email FROM test_ignore_select_users_1"
        );
    }
}

async fn test_ignore_select_collect_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    let _ = db.drop_table::<IgnoreSelectUser>().execute().await;
    db.create_table::<IgnoreSelectUser>().execute().await?;

    db.insert(&IgnoreSelectUser {
        id: 7,
        name: "Alice".to_string(),
        age: 31,
        email: Some("alice@example.com".to_string()),
    })
    .execute()
    .await?;

    let users: Vec<IgnoreSelectUser> = db
        .select::<IgnoreSelectUser>()
        .ignore(|p| (p.id, p.email))
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, 0);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].age, 31);
    assert_eq!(users[0].email, None);

    db.drop_table::<IgnoreSelectUser>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_ignore_select_collect_impl);
