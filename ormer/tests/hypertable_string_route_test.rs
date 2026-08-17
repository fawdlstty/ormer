#![cfg(any(feature = "sqlite", feature = "postgresql"))]

use ormer::Model;

#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_route_events_1"]
struct HypertableRouteEvent {
    #[primary]
    id: i64,
    payload: String,
    #[hypertable]
    #[ormer_ignore]
    tenant: String,
}

fn route_event(tenant: &str) -> HypertableRouteEvent {
    HypertableRouteEvent {
        id: 1,
        payload: "created".to_string(),
        tenant: tenant.to_string(),
    }
}

#[test]
fn string_hypertable_route_value_is_extracted_from_ignored_field() {
    assert_eq!(HypertableRouteEvent::COLUMNS, &["id", "payload"]);

    let route = route_event("acme").table_route().unwrap();
    assert_eq!(route.get("tenant"), Some("acme"));
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_insert_ignores_string_hypertable_route() {
    let event = route_event("acme");
    let statement =
        ormer::abstract_layer::common::common_helpers::build_insert_statement_with_conflict::<
            HypertableRouteEvent,
        >(ormer::DbType::Sqlite, &[&event], None)
        .unwrap();

    assert_eq!(
        statement.0,
        "INSERT INTO hypertable_route_events_1 (id, payload) VALUES (?, ?)"
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_select_route_table_keeps_base_table() {
    let (sql, params) = HypertableRouteEvent::query()
        .route_table("tenant", "acme")
        .to_sql_with_params(ormer::DbType::Sqlite);

    assert_eq!(sql, "SELECT id, payload FROM hypertable_route_events_1");
    assert!(params.is_empty());
}

#[cfg(feature = "postgresql")]
#[test]
fn postgresql_insert_routes_string_hypertable_table() {
    let event = route_event("acme");
    let statement =
        ormer::abstract_layer::common::common_helpers::build_insert_statement_with_conflict::<
            HypertableRouteEvent,
        >(ormer::DbType::PostgreSQL, &[&event], None)
        .unwrap();

    assert_eq!(
        statement.0,
        "INSERT INTO hypertable_route_events_1_acme (id, payload) VALUES ($1, $2)"
    );
}

#[cfg(feature = "postgresql")]
#[test]
fn postgresql_select_routes_string_hypertable_table() {
    let (sql, params) = HypertableRouteEvent::query()
        .route_table("tenant", "acme")
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    assert_eq!(
        sql,
        "SELECT id, payload FROM hypertable_route_events_1_acme"
    );
    assert!(params.is_empty());
}
