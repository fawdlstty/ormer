#![cfg(feature = "duckdb")]

use ormer::Model;

#[derive(Debug, Model)]
#[table = "duckdb_auto_increment_users"]
struct DuckDbAutoIncrementUser {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Model)]
#[table = "duckdb_i64_auto_increment_users"]
struct DuckDbI64AutoIncrementUser {
    #[primary(auto)]
    id: i64,
    name: String,
}

#[derive(Debug, Model)]
#[table = "duckdb_text_primary_users"]
struct DuckDbTextPrimaryUser {
    #[primary]
    id: String,
    name: String,
}

#[tokio::test]
async fn duckdb_auto_increment_insert_returns_generated_key()
-> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::DuckDB, ":memory:").await?;
    db.create_table::<DuckDbAutoIncrementUser>()
        .execute()
        .await?;

    let first_id = db
        .insert(&DuckDbAutoIncrementUser {
            id: 0,
            name: "Alice".to_string(),
        })
        .execute()
        .await?;
    let second_id = db
        .insert(&DuckDbAutoIncrementUser {
            id: 0,
            name: "Bob".to_string(),
        })
        .execute()
        .await?;

    assert_eq!(first_id, 1);
    assert_eq!(second_id, 2);
    Ok(())
}

#[tokio::test]
async fn duckdb_preserves_primary_key_types() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::DuckDB, ":memory:").await?;

    db.create_table::<DuckDbI64AutoIncrementUser>()
        .execute()
        .await?;
    db.validate_table::<DuckDbI64AutoIncrementUser>().await?;
    let entities = db.generate_entities(None).await?;
    assert!(
        entities.contains("#[primary(auto)]\n    pub id: i64"),
        "{entities}"
    );
    let generated_id = db
        .insert(&DuckDbI64AutoIncrementUser {
            id: 0,
            name: "Alice".to_string(),
        })
        .execute()
        .await?;
    assert_eq!(generated_id, 1);

    db.create_table::<DuckDbTextPrimaryUser>().execute().await?;
    db.validate_table::<DuckDbTextPrimaryUser>().await?;
    db.insert(&DuckDbTextPrimaryUser {
        id: "alice".to_string(),
        name: "Alice".to_string(),
    })
    .execute()
    .await?;
    let user = db
        .find_by_id::<DuckDbTextPrimaryUser>("alice".to_string())
        .await?
        .expect("inserted text-primary row");
    assert_eq!(user.name, "Alice");

    Ok(())
}
