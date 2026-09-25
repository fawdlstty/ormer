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
