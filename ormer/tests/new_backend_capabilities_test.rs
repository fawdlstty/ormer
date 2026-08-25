use ormer::{DbType, OrmerError};

#[test]
#[cfg(feature = "duckdb")]
fn duckdb_exposes_dialect_type_mapping() {
    assert_eq!(
        DbType::DuckDB.sql_type("i64", false, false, false, None),
        "BIGINT NOT NULL"
    );
    assert_eq!(
        DbType::DuckDB.sql_type("serde_json::Value", false, false, true, None),
        "JSON"
    );
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_exposes_dialect_type_mapping() {
    assert_eq!(
        DbType::ClickHouse.sql_type("i64", false, false, false, None),
        "Int64"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("String", false, false, true, None),
        "Nullable(String)"
    );
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_connects_with_an_in_memory_database() {
    ormer::Database::connect(DbType::DuckDB, ":memory:")
        .await
        .expect("DuckDB in-memory connection should be supported");
}

#[tokio::test]
#[cfg(feature = "clickhouse")]
async fn clickhouse_connect_reports_unsupported_until_async_adapter_is_ready() {
    let error = match ormer::Database::connect(DbType::ClickHouse, "http://localhost:8123").await {
        Ok(_) => panic!("ClickHouse execution is intentionally capability-gated"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "Database::connect",
        }
    ));
}
