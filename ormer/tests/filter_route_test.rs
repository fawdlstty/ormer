#![cfg(feature = "sqlite")]

use ormer::Model;

#[derive(Debug, Clone, ormer::Model)]
#[table = "filter_route_orders_{tenant_id}"]
#[filter(filter_valid, |o| o.deleted_at.is_null())]
#[filter(filter_tenant, |o, tenant_id: i64| o.tenant_id.eq(tenant_id))]
struct RoutedOrder {
    #[primary]
    id: i64,
    tenant_id: i64,
    deleted_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "filter_route_events_{tenant_id}"]
struct RoutedEvent {
    #[primary]
    id: i64,
    name: String,
    #[ormer_ignore]
    tenant_id: i64,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "filter_route_audit_{tenant_id}"]
struct RoutedAudit {
    #[primary]
    id: i64,
    #[ormer_ignore]
    tenant_id: i64,
    name: String,
}

#[test]
fn model_filter_methods_append_filters() {
    use RoutedOrderFilterExt;

    let (sql, params) = RoutedOrder::query()
        .filter_tenant(7)
        .filter_valid()
        .to_sql_with_params(ormer::DbType::Sqlite);

    assert_eq!(
        sql,
        "SELECT id, tenant_id, deleted_at FROM \"filter_route_orders_{tenant_id}\" WHERE tenant_id = ? AND deleted_at IS NULL"
    );
    assert!(matches!(params.as_slice(), [ormer::Value::Integer(7)]));
}

#[test]
fn select_route_table_renders_template() {
    let (sql, params) = RoutedOrder::query()
        .route_table("tenant_id", 7)
        .filter(|o| o.tenant_id.eq(7))
        .to_sql_with_params(ormer::DbType::Sqlite);

    assert_eq!(
        sql,
        "SELECT id, tenant_id, deleted_at FROM filter_route_orders_7 WHERE tenant_id = ?"
    );
    assert!(matches!(params.as_slice(), [ormer::Value::Integer(7)]));
}

#[test]
fn ignored_field_is_available_for_insert_table_route() {
    assert_eq!(RoutedEvent::COLUMNS, &["id", "name"]);

    let event = RoutedEvent {
        id: 1,
        name: "created".to_string(),
        tenant_id: 9,
    };
    let route = event.table_route().unwrap();
    let rendered = ormer::model::routed_table_name_for_db(
        ormer::DbType::Sqlite,
        RoutedEvent::TABLE_NAME,
        &route,
    )
    .unwrap();

    assert_eq!(rendered, "filter_route_events_9");
}

#[test]
fn ignored_field_does_not_shift_row_value_indexes() {
    let audit = RoutedAudit::from_row_values(&[
        ormer::Value::Integer(1),
        ormer::Value::Text("created".to_string()),
    ])
    .unwrap();

    assert_eq!(audit.id, 1);
    assert_eq!(audit.name, "created");
    assert_eq!(audit.tenant_id, 0);
}

#[test]
fn insert_sql_uses_model_table_route() {
    let event = RoutedEvent {
        id: 1,
        name: "created".to_string(),
        tenant_id: 9,
    };

    let statement =
        ormer::abstract_layer::common::common_helpers::build_insert_statement_with_conflict::<
            RoutedEvent,
        >(ormer::DbType::Sqlite, &[&event], None)
        .unwrap();

    assert_eq!(
        statement.0,
        "INSERT INTO filter_route_events_9 (id, name) VALUES (?, ?)"
    );
    assert_eq!(statement.1.len(), 2);
}
