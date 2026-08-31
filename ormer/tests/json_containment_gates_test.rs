#![cfg(any(
    feature = "sqlite",
    feature = "mssql",
    feature = "clickhouse",
    feature = "duckdb",
    feature = "questdb"
))]

use ormer::{DbType, OrmerError};

#[derive(Debug, ormer::Model)]
#[table = "json_containment_gate_users"]
struct JsonContainmentUser {
    #[primary]
    id: i32,
    profile: serde_json::Value,
}

fn assert_containment_is_rejected(backend: DbType) {
    let error = ormer::Select::<JsonContainmentUser>::new()
        .filter(|user| {
            user.profile
                .json_contains(serde_json::json!({ "role": "admin" }))
        })
        .try_to_sql_with_params(backend)
        .expect_err("JSON containment must be capability gated");

    assert!(
        matches!(
            error,
            OrmerError::UnsupportedFeature {
                feature: "JSON containment predicates",
                ..
            }
        ),
        "unexpected error for {backend:?}"
    );
}

#[test]
#[cfg(feature = "sqlite")]
fn sqlite_rejects_json_containment() {
    assert_containment_is_rejected(DbType::Sqlite);
}

#[test]
#[cfg(feature = "mssql")]
fn mssql_rejects_json_containment() {
    assert_containment_is_rejected(DbType::MSSQL);
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_rejects_json_containment() {
    assert_containment_is_rejected(DbType::ClickHouse);
}

#[test]
#[cfg(feature = "duckdb")]
fn duckdb_rejects_json_containment() {
    assert_containment_is_rejected(DbType::DuckDB);
}

#[test]
#[cfg(feature = "questdb")]
fn questdb_rejects_json_containment() {
    assert_containment_is_rejected(DbType::QuestDB);
}
