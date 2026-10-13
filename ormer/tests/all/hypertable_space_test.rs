#![cfg(feature = "sqlite")]

use ormer::Model;
use std::time::Duration;

#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_space_events_1"]
struct HypertableSpaceEvent {
    #[hypertable]
    #[primary]
    project_id: String,
    #[hypertable(Duration::from_secs(86400))]
    #[primary]
    update_time: i64,
    payload: String,
}

/// 多个 `#[hypertable]` 空间分区字段：TimescaleDB 允许时间维度外挂
/// 任意数量的附加分区维度（add_dimension），不再限制为单字段。
#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_space_events_2"]
struct MultiHypertableSpaceEvent {
    #[hypertable]
    project_id: String,
    #[hypertable]
    zone_id: String,
    #[hypertable(Duration::from_secs(86400))]
    #[primary]
    update_time: i64,
    #[primary]
    agvid: i32,
    payload: String,
}

#[test]
fn bare_hypertable_generates_space_dimension() {
    assert_eq!(
        HypertableSpaceEvent::hypertable_info(),
        Some(("update_time", Duration::from_secs(86400)))
    );
    assert_eq!(
        HypertableSpaceEvent::hypertable_space_info(),
        Some(("project_id", 4))
    );
}

#[test]
fn multiple_space_fields_are_collected_in_declaration_order() {
    assert_eq!(
        MultiHypertableSpaceEvent::hypertable_info(),
        Some(("update_time", Duration::from_secs(86400)))
    );
    // 旧单列方法仍取第一个，保持兼容
    assert_eq!(
        MultiHypertableSpaceEvent::hypertable_space_info(),
        Some(("project_id", 4))
    );
    assert_eq!(
        MultiHypertableSpaceEvent::hypertable_space_infos(),
        vec![("project_id", 4), ("zone_id", 4)]
    );
    // TimescaleDB 唯一约束必须包含全部分区列，主键自动补齐两个 space 列
    let mut primary_columns = MultiHypertableSpaceEvent::primary_field_names();
    primary_columns.sort();
    assert_eq!(primary_columns, vec!["agvid", "project_id", "update_time", "zone_id"]);
}
