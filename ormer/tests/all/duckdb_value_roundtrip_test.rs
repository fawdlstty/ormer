#![cfg(feature = "duckdb")]

use std::time::Duration;

use ormer::{DbType, Model};

#[derive(Debug, Model)]
#[table = "duckdb_value_roundtrip_users"]
struct DuckDbValueRoundtripUser {
    #[primary]
    id: i32,
    profile: serde_json::Value,
    duration: Duration,
    tags: Vec<i32>,
    labels: Vec<String>,
}

#[derive(Debug, Model)]
#[table = "duckdb_conflict_regression_users"]
struct DuckDbConflictRegressionUser {
    #[primary]
    id: i32,
    #[unique]
    email: String,
    name: String,
}

#[tokio::test]
async fn duckdb_roundtrips_json_duration_and_arrays() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.create_table::<DuckDbValueRoundtripUser>()
        .execute()
        .await?;

    let expected_profile = serde_json::json!({
        "roles": ["admin", "operator"],
        "enabled": true
    });
    let expected_duration = Duration::from_micros(1_234_567);

    db.insert(&DuckDbValueRoundtripUser {
        id: 1,
        profile: expected_profile.clone(),
        duration: expected_duration,
        tags: vec![1, 2, 3],
        labels: vec!["first".to_string(), "second".to_string()],
    })
    .execute()
    .await?;

    let user = db
        .find_by_id::<DuckDbValueRoundtripUser>(1)
        .await?
        .expect("inserted DuckDB row");
    assert_eq!(user.profile, expected_profile);
    assert_eq!(user.duration, expected_duration);
    assert_eq!(user.tags, vec![1, 2, 3]);
    assert_eq!(user.labels, vec!["first", "second"]);
    Ok(())
}

#[tokio::test]
async fn duckdb_conflict_executors_use_their_rendered_sql() -> Result<(), Box<dyn std::error::Error>>
{
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.create_table::<DuckDbConflictRegressionUser>()
        .execute()
        .await?;

    db.insert_or_update(&DuckDbConflictRegressionUser {
        id: 1,
        email: "alice@example.com".to_string(),
        name: "Alice".to_string(),
    })
    .execute()
    .await?;
    db.insert_or_update(&DuckDbConflictRegressionUser {
        id: 1,
        email: "alice@example.com".to_string(),
        name: "Alice updated".to_string(),
    })
    .execute()
    .await?;

    db.insert_or_ignore(&[
        DuckDbConflictRegressionUser {
            id: 1,
            email: "alice@example.com".to_string(),
            name: "Ignored".to_string(),
        },
        DuckDbConflictRegressionUser {
            id: 2,
            email: "bob@example.com".to_string(),
            name: "Bob".to_string(),
        },
    ])
    .execute()
    .await?;

    let rows = db
        .select::<DuckDbConflictRegressionUser>()
        .order_by(|user| user.id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "Alice updated");
    assert_eq!(rows[1].name, "Bob");

    let upsert_sql = db
        .insert_or_update(&DuckDbConflictRegressionUser {
            id: 3,
            email: "carol@example.com".to_string(),
            name: "Carol".to_string(),
        })
        .to_sql()?;
    assert!(
        upsert_sql.statements[0]
            .sql
            .contains("ON CONFLICT (id) DO UPDATE SET")
    );

    let ignore_sql = db
        .insert_or_ignore(&DuckDbConflictRegressionUser {
            id: 4,
            email: "dave@example.com".to_string(),
            name: "Dave".to_string(),
        })
        .to_sql()?;
    assert!(
        ignore_sql.statements[0]
            .sql
            .ends_with("ON CONFLICT DO NOTHING")
    );

    Ok(())
}
