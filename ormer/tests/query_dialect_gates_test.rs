#![cfg(feature = "sqlite")]

use ormer::DbType;

#[derive(Debug, ormer::Model)]
#[table = "query_dialect_gate_users"]
struct GateUser {
    #[primary]
    id: i32,
    org_id: i32,
    amount: i64,
    active: bool,
}

#[derive(Debug, ormer::Model)]
#[table = "capability_dialect_rows"]
struct CapabilityRow {
    #[primary]
    id: i32,
    tenant_id: i32,
    sku: String,
    country: String,
    region: String,
    updated_at: String,
    amount: i64,
    active: bool,
}

#[test]
fn distinct_on_supports_composite_keys_and_rejects_bad_order() {
    let select = ormer::Select::<CapabilityRow>::new()
        .order_by(|r| r.tenant_id.asc())
        .order_by(|r| r.sku.asc())
        .order_by(|r| r.updated_at.desc())
        .order_by(|r| r.id.desc())
        .distinct_on(|r| (r.tenant_id, r.sku));

    #[cfg(feature = "postgresql")]
    {
        let (sql, _) = select
            .clone()
            .try_to_sql_with_params(ormer::DbType::PostgreSQL)
            .expect("PostgreSQL supports DISTINCT ON");
        assert!(sql.starts_with("SELECT DISTINCT ON (tenant_id, sku) "));
    }

    #[cfg(feature = "duckdb")]
    {
        let (sql, _) = select
            .clone()
            .try_to_sql_with_params(ormer::DbType::DuckDB)
            .expect("DuckDB supports DISTINCT ON");
        assert!(sql.starts_with("SELECT DISTINCT ON (tenant_id, sku) "));
    }

    #[cfg(feature = "sqlite")]
    {
        let (sql, _) = select
            .clone()
            .try_to_sql_with_params(ormer::DbType::Sqlite)
            .expect("SQLite uses the ROW_NUMBER fallback");
        assert!(
            sql.contains("ROW_NUMBER() OVER (PARTITION BY tenant_id, sku ORDER BY tenant_id ASC, sku ASC, updated_at DESC, id DESC)"),
            "SQL: {sql}"
        );
        assert!(sql.contains(r#""__ormer_rank" = 1"#));
        assert!(
            sql.contains(
                "ORDER BY \"__ormer_order_0\" ASC, \"__ormer_order_1\" ASC, \"__ormer_order_2\" DESC, \"__ormer_order_3\" DESC",
            ),
            "SQL: {sql}"
        );
    }

    let missing_order = ormer::Select::<CapabilityRow>::new()
        .order_by(|r| r.tenant_id.asc())
        .distinct_on(|r| (r.tenant_id, r.sku))
        .try_to_sql_with_params(ormer::DbType::Sqlite);
    assert!(
        missing_order
            .expect_err("every composite partition key must lead ORDER BY")
            .to_string()
            .contains("must start with")
    );
}

#[test]
fn conditional_aggregates_and_mysql_rollup_use_dialect_sql() {
    #[cfg(feature = "mysql")]
    {
        let (sql, params) = ormer::Select::<CapabilityRow>::new()
            .select_column(|r| r.amount.sum().filter(|r| r.active.eq(true)))
            .to_sql_with_params(ormer::DbType::MySQL);
        assert!(
            sql.contains("SUM(CASE WHEN active = ? THEN amount END)"),
            "{sql}"
        );
        assert_eq!(params.len(), 1);

        let rollup = ormer::Select::<CapabilityRow>::new()
            .select_column(|r| (r.country, r.region, r.amount.sum()))
            .rollup(|r| (r.country, r.region))
            .try_to_sql_with_params(ormer::DbType::MySQL)
            .expect("MySQL supports WITH ROLLUP");
        assert!(
            rollup.0.contains("GROUP BY country, region WITH ROLLUP"),
            "SQL: {}",
            rollup.0
        );

        let cube = ormer::Select::<CapabilityRow>::new()
            .select_column(|r| (r.country, r.region, r.amount.sum()))
            .cube(|r| (r.country, r.region))
            .try_to_sql_with_params(ormer::DbType::MySQL);
        assert!(matches!(
            cube,
            Err(ormer::OrmerError::UnsupportedFeature {
                backend: ormer::DbType::MySQL,
                feature: "advanced GROUP BY syntax",
            })
        ));
    }

    #[cfg(feature = "clickhouse")]
    {
        let rollup = ormer::Select::<CapabilityRow>::new()
            .select_column(|r| (r.country, r.amount.sum()))
            .rollup(|r| (r.country,))
            .try_to_sql_with_params(ormer::DbType::ClickHouse);
        assert!(matches!(
            rollup,
            Err(ormer::OrmerError::UnsupportedFeature {
                backend: ormer::DbType::ClickHouse,
                feature: "advanced GROUP BY syntax",
            })
        ));
    }
}

#[test]
fn sqlite_rejects_row_locks_and_rewrites_distinct_on() {
    let row_lock = ormer::Select::<GateUser>::new()
        .for_update()
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("SQLite cannot provide row locks");
    assert!(matches!(
        row_lock,
        ormer::OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "row locking",
        }
    ));

    let missing_order = ormer::Select::<GateUser>::new()
        .distinct_on(|u| u.org_id)
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("DISTINCT ON requires partition keys to lead ORDER BY");
    assert!(
        missing_order
            .to_string()
            .contains("ORDER BY must start with")
    );

    let (sql, _) = ormer::Select::<GateUser>::new()
        .distinct_on(|u| u.org_id)
        .order_by(|u| u.org_id.asc())
        .order_by(|u| u.id.desc())
        .to_sql_with_params(DbType::Sqlite);
    assert!(sql.contains("ROW_NUMBER() OVER (PARTITION BY org_id"));
    assert!(sql.contains("ORDER BY org_id ASC, id DESC"));
}

#[test]
fn sqlite_rejects_advanced_grouping_but_allows_filtered_aggregates() {
    let rollup = ormer::Select::<GateUser>::new()
        .select_column(|u| u.id.count())
        .rollup(|u| (u.org_id,))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("SQLite cannot provide advanced GROUP BY syntax");
    assert!(matches!(
        rollup,
        ormer::OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "advanced GROUP BY syntax",
        }
    ));

    ormer::Select::<GateUser>::new()
        .select_column(|u| u.amount.sum().filter(|u| u.active.eq(true)))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect("SQLite supports aggregate FILTER");
}
