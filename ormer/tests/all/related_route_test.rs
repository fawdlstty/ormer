#![cfg(feature = "postgresql")]

//! related/multi/four 表构建器的"主表路由"渲染层测试：
//! 模型声明 `#[hypertable(route)]` 后，`route_table(key, value)` /
//! `with_table_route(route)` 只路由主表（PG 上渲染 `{基础表名}_{路由值}`），
//! 关联表保持基础表名；不带路由时渲染与改造前完全一致。

/// 主表：声明拆表路由键（String 字段）。
#[derive(Debug, Clone, ormer::Model)]
#[table = "route_related_events"]
struct RouteEvent {
    #[primary]
    id: i64,
    #[hypertable(route)]
    region: String,
    title: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "route_related_regions"]
struct Region {
    #[primary]
    id: i64,
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "route_related_tags"]
struct Tag {
    #[primary]
    id: i64,
    label: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "route_related_orgs"]
struct Org {
    #[primary]
    id: i64,
    name: String,
}

#[test]
fn single_select_with_route_renders_routed_table() {
    let (sql, _) = RouteEvent::query()
        .route_table("region", "val")
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    assert_eq!(
        sql,
        "SELECT id, region, title FROM route_related_events_val"
    );
}

#[test]
fn related_select_with_route_renders_routed_main_table_only() {
    let (sql, _) = RouteEvent::query()
        .route_table("region", "val")
        .from::<Region>()
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    // 主表按路由渲染为 `{基础表名}_{路由值}`，关联表保持基础表名
    assert_eq!(
        sql,
        "SELECT t0.id, t0.region, t0.title \
         FROM route_related_events_val AS t0, route_related_regions AS t1"
    );
}

#[test]
fn multi_table_select_with_route_renders_routed_main_table_only() {
    let (sql, _) = RouteEvent::query()
        .route_table("region", "val")
        .from3::<Region, Tag>()
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    assert_eq!(
        sql,
        "SELECT t0.id, t0.region, t0.title \
         FROM route_related_events_val AS t0, route_related_regions AS t1, \
route_related_tags AS t2"
    );
}

#[test]
fn four_table_select_with_route_renders_routed_main_table_only() {
    let (sql, _) = RouteEvent::query()
        .route_table("region", "val")
        .from4::<Region, Tag, Org>()
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    assert_eq!(
        sql,
        "SELECT t0.id, t0.region, t0.title \
         FROM route_related_events_val AS t0, route_related_regions AS t1, \
route_related_tags AS t2, route_related_orgs AS t3"
    );
}

#[test]
fn with_table_route_merges_and_renders() {
    let mut route = ormer::model::TableRoute::new();
    route.insert("region", "val");

    let (sql, _) = RouteEvent::query()
        .with_table_route(route)
        .from::<Region>()
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    assert_eq!(
        sql,
        "SELECT t0.id, t0.region, t0.title \
         FROM route_related_events_val AS t0, route_related_regions AS t1"
    );
}

#[test]
fn related_select_without_route_renders_base_tables() {
    let (sql, _) = RouteEvent::query()
        .from::<Region>()
        .to_sql_with_params(ormer::DbType::PostgreSQL);

    // 不带路由时与既有行为完全一致：全部渲染基础表名
    assert_eq!(
        sql,
        "SELECT t0.id, t0.region, t0.title \
         FROM route_related_events AS t0, route_related_regions AS t1"
    );
}

#[test]
fn invalid_route_value_panics_like_single_select() {
    // 非法路由值（含特殊字符）在渲染层报错，与单表 Select 同一 helper、同一行为
    let result = std::panic::catch_unwind(|| {
        RouteEvent::query()
            .route_table("region", "bad/value")
            .from::<Region>()
            .to_sql_with_params(ormer::DbType::PostgreSQL)
    });
    let err = result.expect_err("invalid route value must panic");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.starts_with("Failed to render table route: Invalid table route value"),
        "unexpected panic message: {message}"
    );
}

#[test]
fn single_select_invalid_route_value_panics_with_same_message() {
    let result = std::panic::catch_unwind(|| {
        RouteEvent::query()
            .route_table("region", "bad/value")
            .to_sql_with_params(ormer::DbType::PostgreSQL)
    });
    let err = result.expect_err("invalid route value must panic");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.starts_with("Failed to render table route: Invalid table route value"),
        "unexpected panic message: {message}"
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn unified_related_executor_forwards_route_table() {
    // 统一层枚举包装的转发链路（sqlite 内存库即可驱动，不依赖外部服务）。
    // SQLite 上拆表路由仅对模板表名生效，基础表名原样渲染，查询可正常执行。
    let fut = async {
        let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
        db.create_table::<RouteEvent>().execute().await?;
        db.create_table::<Region>().execute().await?;

        db.insert(vec![RouteEvent {
            id: 1,
            region: "val".to_string(),
            title: "hello".to_string(),
        }])
        .execute()
        .await?;
        db.insert(vec![Region {
            id: 1,
            name: "val".to_string(),
        }])
        .execute()
        .await?;

        let rows = db
            .select::<RouteEvent>()
            .route_table("region", "val")
            .from::<Region>()
            .collect::<Vec<RouteEvent>>()
            .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "hello");
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fut)
        .unwrap();
}
