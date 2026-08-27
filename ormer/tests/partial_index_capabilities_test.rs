#![cfg(any(feature = "sqlite", feature = "duckdb", feature = "postgresql"))]

use ormer::{DbType, OrmerError};

#[derive(Debug, ormer::Model)]
#[table = "partial_index_capability_users"]
struct PartialIndexCapabilityUser {
    #[primary]
    id: i32,
    #[index(where = "active = 1")]
    email: String,
    active: bool,
}

#[derive(Debug, ormer::Model)]
#[table = "partial_index_migration_users"]
struct PartialIndexMigrationUserV1 {
    #[primary]
    id: i32,
    active: bool,
}

#[derive(Debug, ormer::Model)]
#[table = "partial_index_migration_users"]
struct PartialIndexMigrationUserV2 {
    #[primary]
    id: i32,
    active: bool,
    #[index(where = "active = 1")]
    email: Option<String>,
}

fn assert_partial_index_error(error: OrmerError, db_type: DbType) {
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend,
            feature: "partial index WHERE clauses",
        } if backend == db_type
    ));
}

#[test]
#[cfg(feature = "sqlite")]
fn sqlite_partial_index_create_sql_is_capability_gated() {
    let error = ormer::generate_create_table_sql::<PartialIndexCapabilityUser>(DbType::Sqlite)
        .expect_err("SQLite must reject partial index metadata");
    assert_partial_index_error(error, DbType::Sqlite);
}

#[tokio::test]
#[cfg(feature = "sqlite")]
async fn sqlite_partial_index_create_table_is_capability_gated() -> ormer::Result<()> {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:").await?;
    let error = db
        .create_table::<PartialIndexCapabilityUser>()
        .execute()
        .await
        .expect_err("SQLite create_table must reject partial index metadata");
    assert_partial_index_error(error, DbType::Sqlite);
    Ok(())
}

#[tokio::test]
#[cfg(feature = "sqlite")]
async fn sqlite_partial_index_migration_plan_is_capability_gated() -> ormer::Result<()> {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:").await?;
    db.migrate_table::<PartialIndexMigrationUserV1>()
        .execute()
        .await?;

    let error = db
        .migrate_table::<PartialIndexMigrationUserV2>()
        .plan()
        .await
        .expect_err("SQLite migration planning must reject partial index metadata");
    assert_partial_index_error(error, DbType::Sqlite);
    Ok(())
}

#[test]
#[cfg(feature = "duckdb")]
fn duckdb_partial_index_create_sql_is_capability_gated() {
    let error = ormer::generate_create_table_sql::<PartialIndexCapabilityUser>(DbType::DuckDB)
        .expect_err("DuckDB must reject partial index metadata");
    assert_partial_index_error(error, DbType::DuckDB);
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_partial_index_create_table_is_capability_gated() -> ormer::Result<()> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    let error = db
        .create_table::<PartialIndexCapabilityUser>()
        .execute()
        .await
        .expect_err("DuckDB create_table must reject partial index metadata");
    assert_partial_index_error(error, DbType::DuckDB);
    Ok(())
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_partial_index_migration_plan_is_capability_gated() -> ormer::Result<()> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.migrate_table::<PartialIndexMigrationUserV1>()
        .execute()
        .await?;

    let error = db
        .migrate_table::<PartialIndexMigrationUserV2>()
        .plan()
        .await
        .expect_err("DuckDB migration planning must reject partial index metadata");
    assert_partial_index_error(error, DbType::DuckDB);
    Ok(())
}

#[test]
#[cfg(feature = "postgresql")]
fn postgresql_partial_index_create_sql_keeps_where_clause() -> ormer::Result<()> {
    let sql = ormer::generate_create_table_sql::<PartialIndexCapabilityUser>(DbType::PostgreSQL)?;
    assert!(sql.contains("WHERE active = 1"));
    Ok(())
}
