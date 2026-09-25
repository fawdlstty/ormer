#![cfg(feature = "sqlite")]

use ormer::{DbType, OrmerError};

#[tokio::test]
async fn sqlite_rejects_postgresql_array_values_without_panic()
-> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:").await?;

    let error = db
        .execute_sql(ormer::sql("SELECT {}").bind(vec![1_i32, 2_i32]))
        .await
        .expect_err("SQLite must reject PostgreSQL array values");

    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "PostgreSQL array values",
        }
    ));

    Ok(())
}
