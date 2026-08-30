#![cfg(feature = "sqlite")]
#![allow(deprecated)]

use ormer::{
    DbType, FullTextRank, Select, TimePart, generate_create_table_sql,
};
use ormer::query::CteBuilder;

#[derive(ormer::Model)]
#[table = "b6_articles"]
struct Article {
    #[primary(auto)]
    id: i64,
    #[index(method = "fulltext", columns = "(title, body)")]
    title: String,
    body: String,
    status: String,
}

#[derive(ormer::Model)]
#[table = "b7_events"]
struct Event {
    #[primary(auto)]
    id: i64,
    occurred_at: chrono::NaiveDateTime,
    points: i32,
    game_id: i32,
}

#[derive(ormer::Model)]
#[table = "b9_customers"]
struct Customer {
    #[primary(auto)]
    id: i64,
    region: String,
}

#[derive(ormer::Model)]
#[table = "b9_orders"]
struct Order {
    #[primary(auto)]
    id: i64,
    customer_id: i64,
    status: String,
}

#[derive(ormer::Model)]
#[table = "b9_recent_order_rows"]
struct RecentOrderRow {
    #[primary]
    id: i64,
    customer_id: i64,
}

#[test]
fn sqlite_fulltext_uses_shadow_table_and_bm25() {
    let index_sql = generate_create_table_sql::<Article>(DbType::Sqlite).unwrap();
    assert!(index_sql.contains("CREATE VIRTUAL TABLE"), "{index_sql}");
    assert!(index_sql.contains("USING fts5"), "{index_sql}");
    assert!(index_sql.contains("AFTER INSERT ON"), "{index_sql}");

    let (sql, params) = Select::<Article>::new()
        .fields(|a| (a.title, a.body))
        .query("rust")
        .rank(FullTextRank::Relevance)
        .filter(|a| a.status.eq("open"))
        .to_sql_with_params(DbType::Sqlite);
    assert!(sql.contains("CREATE VIRTUAL TABLE") == false);
    assert!(sql.contains("b6_articles_fts MATCH ?"), "{sql}");
    assert!(sql.contains("SELECT t0.title, t0.body FROM"), "{sql}");
    assert!(sql.contains("bm25("), "{sql}");
    assert!(sql.contains("t0.status = ?"), "{sql}");
    assert_eq!(params.len(), 2);
}

#[test]
fn sqlite_date_window_and_cte_sql_is_parameterized() {
    let (date_sql, date_params) = Select::<Event>::new()
        .filter(|e| e.occurred_at.ge(chrono::Utc::now().naive_utc() - chrono::Duration::days(7)))
        .map_to(|e| (e.occurred_at.date_trunc(ormer::TimeUnit::Day).alias("day"), e.points.sum()))
        .to_sql_with_params(DbType::Sqlite);
    assert!(date_sql.contains("strftime("), "{date_sql}");
    assert_eq!(date_params.len(), 1);

    let (interval_sql, interval_params) = Select::<Event>::new()
        .filter(|e| e.occurred_at.le(ormer::now() + ormer::days(7)))
        .map_to(|e| (
            e.occurred_at.until(ormer::now(), TimePart::Hour).alias("age"),
            e.points.sum(),
        ))
        .to_sql_with_params(DbType::Sqlite);
    assert!(interval_sql.contains("datetime('now')"), "{interval_sql}");
    assert!(interval_sql.contains("printf('%+d seconds'"), "{interval_sql}");
    assert!(interval_sql.contains("julianday("), "{interval_sql}");
    assert_eq!(interval_params.len(), 1);

    let (window_sql, _) = Select::<Event>::new()
        .map_to(|e| (
            e.game_id,
            e.points,
            e.points.rank().over(|w| w.partition_by(e.game_id).order_by(e.points.desc())),
        ))
        .to_sql_with_params(DbType::Sqlite);
    assert!(window_sql.contains("RANK() OVER (PARTITION BY game_id ORDER BY points DESC)"));

    let (window_lag_sql, _) = Select::<Event>::new()
        .map_to(|e| e.occurred_at.lag(1).over(|w| w.order_by(e.occurred_at)))
        .to_sql_with_params(DbType::Sqlite);
    assert!(window_lag_sql.contains("LAG(occurred_at, ?) OVER (ORDER BY occurred_at ASC)"));

    let (cte_sql, cte_params) = Select::<Customer>::new()
        .with_cte("recent_orders", |_c| {
            CteBuilder::select::<Order>()
                .filter(|o| o.status.eq("paid"))
                .columns(|o| (o.id, o.customer_id))
        })
        .inner_join_cte::<RecentOrderRow, _>("recent_orders", |customer, row| {
            customer.id.eq(row.customer_id)
        })
        .filter(|c| c.region.eq("eu"))
        .to_sql_with_params(DbType::Sqlite);
    assert!(cte_sql.starts_with("WITH "), "{cte_sql}");
    assert!(cte_sql.contains("recent_orders AS (SELECT id, customer_id FROM "), "{cte_sql}");
    assert!(cte_sql.contains("t0.region = ?"), "{cte_sql}");
    assert_eq!(cte_params.len(), 2);

    let recursive_sql = Select::<Order>::new()
        .descendants(|o| (o.id, o.customer_id), 1)
        .to_sql();
    assert!(recursive_sql.contains("WITH RECURSIVE"));

    #[cfg(feature = "duckdb")]
    assert!(Select::<Order>::new()
        .descendants(|o| (o.id, o.customer_id), 1)
        .to_sql_with_params(DbType::DuckDB)
        .0
        .contains("WITH RECURSIVE"));
    #[cfg(feature = "clickhouse")]
    assert!(Select::<Order>::new()
        .descendants(|o| (o.id, o.customer_id), 1)
        .to_sql_with_params(DbType::ClickHouse)
        .0
        .contains("WITH RECURSIVE"));
}

#[test]
fn table_options_generate_dialect_ddl() {
    let sql = generate_create_table_sql::<B10TenantEvent>(DbType::Sqlite).unwrap();
    assert!(sql.starts_with("CREATE TABLE"));

    #[cfg(feature = "mysql")]
    {
        let sql = generate_create_table_sql::<B10TenantEvent>(DbType::MySQL).unwrap();
        assert!(sql.contains("ENGINE='InnoDB'"), "{sql}");
        assert!(sql.contains("DEFAULT CHARSET='utf8mb4'"), "{sql}");
    }
    #[cfg(feature = "postgresql")]
    {
        let sql = generate_create_table_sql::<B10TenantEvent>(DbType::PostgreSQL).unwrap();
        assert!(sql.contains("WITH (storage = 'main', fillfactor = 80)"), "{sql}");
    }
    #[cfg(feature = "mssql")]
    {
        let sql = generate_create_table_sql::<B10TenantEvent>(DbType::MSSQL).unwrap();
        assert!(sql.contains(") ON PRIMARY"), "{sql}");
    }
    #[cfg(feature = "clickhouse")]
    {
        let sql = generate_create_table_sql::<B10TenantEvent>(DbType::ClickHouse).unwrap();
        assert!(sql.contains("ENGINE = MergeTree"), "{sql}");
        assert!(sql.contains("ORDER BY (tenant_id, occurred_at)"), "{sql}");
    }
}

#[derive(ormer::Model)]
#[table = "b10_tenant_events"]
#[postgresql(storage = "main", fillfactor = 80)]
#[mssql(filegroup = "PRIMARY")]
#[clickhouse(engine = "MergeTree", order_by = "(tenant_id, occurred_at)")]
#[mysql(engine = "InnoDB", charset = "utf8mb4", collation = "utf8mb4_unicode_ci")]
struct B10TenantEvent {
    #[primary]
    id: i64,
    tenant_id: i64,
}
