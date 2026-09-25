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

#[derive(Debug, Clone, ormer::Model)]
#[table = "context_scope_orders"]
#[filter(filter_valid, |o| o.deleted_at.is_null())]
#[filter(filter_tenant, |o, tenant_id: i64| o.tenant_id.eq(tenant_id))]
struct ContextScopeOrder {
    #[primary]
    id: i64,
    tenant_id: i64,
    deleted_at: Option<chrono::NaiveDateTime>,
    status: String,
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

#[tokio::test]
async fn scope_filters_can_be_inherited_and_disabled() -> Result<(), Box<dyn std::error::Error>> {
    use ContextScopeOrderFilterExt;

    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<ContextScopeOrder>().execute().await?;

    let deleted_at = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    db.insert(vec![
        ContextScopeOrder {
            id: 1,
            tenant_id: 7,
            deleted_at: None,
            status: "open".to_string(),
        },
        ContextScopeOrder {
            id: 2,
            tenant_id: 7,
            deleted_at: Some(deleted_at),
            status: "deleted".to_string(),
        },
        ContextScopeOrder {
            id: 3,
            tenant_id: 8,
            deleted_at: None,
            status: "other".to_string(),
        },
    ])
    .execute()
    .await?;

    let scoped = ContextScopeOrderFilterExt::filter_valid(
        ContextScopeOrderFilterExt::filter_tenant(db.scope(), 7),
    );
    let rows = scoped
        .select::<ContextScopeOrder>()
        .order_by(|o| o.id.asc())
        .collect::<Vec<ContextScopeOrder>>()
        .await?;
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), vec![1]);

    let rows = scoped
        .select::<ContextScopeOrder>()
        .unset_filter_valid()
        .order_by(|o| o.id.asc())
        .collect::<Vec<ContextScopeOrder>>()
        .await?;
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1, 2]
    );

    let rows = scoped
        .select::<ContextScopeOrder>()
        .unset_filter_valid()
        .filter(|o| o.deleted_at.is_null())
        .collect::<Vec<ContextScopeOrder>>()
        .await?;
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), vec![1]);

    let updated = scoped
        .update::<ContextScopeOrder>()
        .set(|o| o.status = o.status.set("scoped".to_string()))
        .execute()
        .await?;
    assert_eq!(updated, 1);

    let deleted = scoped
        .delete::<ContextScopeOrder>()
        .unset_filter_valid()
        .filter(|o| o.status.eq("deleted"))
        .execute()
        .await?;
    assert_eq!(deleted, 1);

    let remaining = db
        .select::<ContextScopeOrder>()
        .order_by(|o| o.id.asc())
        .collect::<Vec<ContextScopeOrder>>()
        .await?;
    assert_eq!(
        remaining
            .iter()
            .map(|row| (row.id, row.status.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "scoped"), (3, "other")]
    );

    db.drop_table::<ContextScopeOrder>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn pooled_scope_supports_queries_and_find_by_id() -> Result<(), Box<dyn std::error::Error>> {
    use ContextScopeOrderFilterExt;

    let pool = ormer::Database::create_pool(ormer::DbType::Sqlite, ":memory:")
        .range(0..1)
        .build()
        .await?;
    let conn = pool.get().await?;
    conn.create_table::<ContextScopeOrder>().execute().await?;
    conn.insert(vec![
        ContextScopeOrder {
            id: 1,
            tenant_id: 7,
            deleted_at: None,
            status: "open".to_string(),
        },
        ContextScopeOrder {
            id: 2,
            tenant_id: 8,
            deleted_at: None,
            status: "other".to_string(),
        },
    ])
    .execute()
    .await?;

    let scoped = ContextScopeOrderFilterExt::filter_valid(
        ContextScopeOrderFilterExt::filter_tenant(conn.scope(), 7),
    );
    assert_eq!(
        scoped.find_by_id::<ContextScopeOrder>(1).await?.unwrap().id,
        1
    );
    assert!(scoped.find_by_id::<ContextScopeOrder>(2).await?.is_none());

    conn.drop_table::<ContextScopeOrder>().execute().await?;
    Ok(())
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
