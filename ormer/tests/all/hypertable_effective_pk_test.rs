#![cfg(feature = "postgresql")]

use ormer::Model;
use std::time::Duration;

/// 与 common_rs 模型一致的形态：仅时间列声明 #[primary]，
/// 空间分区列通过 bare #[hypertable] 声明。
#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_effective_pk_events"]
struct HypertableEffectivePkEvent {
    #[hypertable]
    project_id: String,
    #[hypertable(Duration::from_secs(86400))]
    #[primary]
    update_time: i64,
    payload: String,
}

/// 对照组：无空间分区列的普通超表模型。
#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_effective_pk_plain"]
struct HypertableEffectivePkPlain {
    #[hypertable(Duration::from_secs(3600))]
    #[primary]
    update_time: i64,
    payload: String,
}

#[test]
fn effective_primary_keys_include_space_partition_column() {
    assert_eq!(
        HypertableEffectivePkEvent::primary_key_columns(),
        &["update_time", "project_id"]
    );
    assert_eq!(
        ormer::effective_primary_key_columns::<HypertableEffectivePkEvent>(
            ormer::DbType::PostgreSQL
        ),
        vec!["update_time", "project_id"]
    );
}

#[test]
fn create_table_sql_uses_composite_primary_key_for_space_partition() {
    let sql = ormer::generate_create_table_sql::<HypertableEffectivePkEvent>(
        ormer::DbType::PostgreSQL,
    )
    .unwrap();

    // TimescaleDB 要求唯一索引（含主键）包含分区列：
    // 主键必须以表级复合约束输出，且不得在列上内联 PRIMARY KEY。
    assert!(
        sql.contains("PRIMARY KEY (update_time, project_id)"),
        "composite primary key missing: {sql}"
    );
    let constraint_count = sql.matches("PRIMARY KEY").count();
    assert_eq!(constraint_count, 1, "unexpected inline primary key: {sql}");
}

#[test]
fn create_table_sql_keeps_inline_primary_key_without_space_partition() {
    let sql =
        ormer::generate_create_table_sql::<HypertableEffectivePkPlain>(ormer::DbType::PostgreSQL)
            .unwrap();

    assert!(
        sql.contains("PRIMARY KEY"),
        "inline primary key missing: {sql}"
    );
    assert!(
        !sql.contains("PRIMARY KEY ("),
        "unexpected table-level primary key constraint: {sql}"
    );
    assert_eq!(
        ormer::effective_primary_key_columns::<HypertableEffectivePkPlain>(
            ormer::DbType::PostgreSQL
        ),
        vec!["update_time"]
    );
}
