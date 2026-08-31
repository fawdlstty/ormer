#![cfg(any(
    feature = "sqlite",
    feature = "duckdb",
    feature = "clickhouse",
    feature = "questdb"
))]

use ormer::DbType;

#[derive(Debug, ormer::Model)]
#[table = "timezone_gate_events"]
struct TimezoneEvent {
    #[primary]
    id: i32,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

fn assert_timezone_conversion_is_rejected(backend: DbType) {
    let error = ormer::Select::<TimezoneEvent>::new()
        .map_to(|event| event.occurred_at.at_time_zone("Asia/Shanghai"))
        .try_to_sql_with_params(backend)
        .expect_err("timezone conversion must be capability gated");

    assert!(
        matches!(
            error,
            ormer::OrmerError::UnsupportedFeature {
                feature: "timezone conversion",
                ..
            }
        ),
        "unexpected error for {backend:?}"
    );
}

#[test]
#[cfg(feature = "sqlite")]
fn sqlite_rejects_timezone_conversion() {
    assert_timezone_conversion_is_rejected(DbType::Sqlite);
}

#[test]
#[cfg(feature = "duckdb")]
fn duckdb_rejects_timezone_conversion() {
    assert_timezone_conversion_is_rejected(DbType::DuckDB);
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_rejects_timezone_conversion() {
    assert_timezone_conversion_is_rejected(DbType::ClickHouse);
}

#[test]
#[cfg(feature = "questdb")]
fn questdb_rejects_timezone_conversion() {
    assert_timezone_conversion_is_rejected(DbType::QuestDB);
}
