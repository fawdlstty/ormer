#![cfg(feature = "clickhouse")]

//! 直接断言显式 engine 入口生成的建表 SQL，无需真实 ClickHouse 服务。

#[derive(Debug, ormer::Model)]
#[table = "ormer_ch_engine_events"]
struct EngineEvent {
    #[primary]
    id: i32,
    name: String,
}

#[test]
fn explicit_engine_renders_engine_clause() {
    let sql = ormer::generate_clickhouse_create_table_sql::<EngineEvent>("MergeTree ORDER BY (id)")
        .expect("explicit engine should render");
    // 普通小写标识符按 needs_identifier_quote 策略不加引号，保留字/特殊字符才引号化。
    assert!(sql.starts_with("CREATE TABLE IF NOT EXISTS ormer_ch_engine_events ("));
    assert!(sql.contains("ENGINE = MergeTree ORDER BY (id)"));
}

#[test]
fn with_name_overrides_table_name() {
    let sql = ormer::generate_clickhouse_create_table_sql_with_name::<EngineEvent>(
        "MergeTree ORDER BY (id)",
        Some("ormer_ch_engine_renamed"),
    )
    .expect("custom table name should render");
    assert!(sql.starts_with("CREATE TABLE IF NOT EXISTS ormer_ch_engine_renamed ("));
    assert!(!sql.contains("ormer_ch_engine_events"));
    assert!(sql.contains("ENGINE = MergeTree ORDER BY (id)"));
}

#[test]
fn empty_engine_is_rejected() {
    assert!(ormer::generate_clickhouse_create_table_sql::<EngineEvent>("").is_err());
    assert!(ormer::generate_clickhouse_create_table_sql::<EngineEvent>("   ").is_err());
    assert!(ormer::generate_clickhouse_create_table_sql_with_name::<EngineEvent>(
        "",
        Some("ormer_ch_engine_renamed")
    )
    .is_err());
}

#[test]
fn engine_with_semicolon_is_rejected() {
    let error = ormer::generate_clickhouse_create_table_sql::<EngineEvent>(
        "MergeTree ORDER BY (id); DROP TABLE ormer_ch_engine_events",
    )
    .expect_err("engine clause must not contain ';'");
    assert!(error.to_string().contains("engine"));
}
