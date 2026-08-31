#![allow(deprecated)]

#[cfg(any(
    feature = "mysql",
    feature = "postgresql",
    feature = "mssql",
    feature = "clickhouse"
))]
use ormer::{DbType, generate_create_table_sql};
use ormer::{Model, TableOptions};

#[derive(ormer::Model)]
#[table = "tenant_events"]
#[mysql(
    engine = "InnoDB",
    charset = "utf8mb4",
    collation = "utf8mb4_0900_ai_ci"
)]
#[postgresql(storage = "main", fillfactor = 80)]
#[mssql(filegroup = "PRIMARY")]
#[clickhouse(engine = "MergeTree", order_by = "(tenant_id, occurred_at)")]
#[clickhouse(
    partition_by = "toYYYYMM(occurred_at)",
    ttl = "occurred_at + INTERVAL 12 MONTH",
    settings = "index_granularity=8192"
)]
struct TenantEvent {
    #[primary]
    id: i64,
    tenant_id: i64,
}

fn table_options() -> Option<TableOptions> {
    <TenantEvent as Model>::TABLE_OPTIONS
}

#[test]
fn dialect_attributes_are_gated_by_backend_features() {
    let options = table_options();

    #[cfg(feature = "mysql")]
    assert_eq!(options.unwrap().mysql_engine, Some("InnoDB"));
    #[cfg(all(
        not(feature = "mysql"),
        not(feature = "postgresql"),
        not(feature = "mssql"),
        not(feature = "clickhouse")
    ))]
    assert!(options.is_none());
}

#[cfg(feature = "postgresql")]
#[test]
fn postgresql_table_options_render_as_storage_parameters() {
    let sql = generate_create_table_sql::<TenantEvent>(DbType::PostgreSQL).unwrap();
    assert!(
        sql.contains(" WITH (storage = 'main', fillfactor = 80)"),
        "{sql}"
    );
}

#[cfg(feature = "mysql")]
#[test]
fn mysql_table_options_render_after_columns() {
    let sql = generate_create_table_sql::<TenantEvent>(DbType::MySQL).unwrap();
    assert!(
        sql.contains("ENGINE='InnoDB' DEFAULT CHARSET='utf8mb4' COLLATE='utf8mb4_0900_ai_ci'"),
        "{sql}"
    );
}

#[cfg(feature = "mssql")]
#[test]
fn mssql_table_options_render_as_filegroup() {
    let sql = generate_create_table_sql::<TenantEvent>(DbType::MSSQL).unwrap();
    assert!(sql.contains(") ON PRIMARY"), "{sql}");
}

#[cfg(feature = "clickhouse")]
#[test]
fn clickhouse_table_options_render_engine_clauses() {
    let sql = generate_create_table_sql::<TenantEvent>(DbType::ClickHouse).unwrap();
    for clause in [
        "ENGINE = MergeTree",
        "ORDER BY (tenant_id, occurred_at)",
        "PARTITION BY toYYYYMM(occurred_at)",
        "TTL occurred_at + INTERVAL 12 MONTH",
        "SETTINGS index_granularity=8192",
    ] {
        assert!(sql.contains(clause), "{sql}");
    }
}
