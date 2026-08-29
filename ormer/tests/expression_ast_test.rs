#![cfg(feature = "sqlite")]

use ormer::{DbType, RowValueCompare, Select};

#[derive(ormer::Model)]
#[table = "expr_ast_users_1"]
struct ExprUser {
    #[primary]
    id: i32,
    org_id: i32,
    email: String,
    status: String,
    score: i32,
    active: bool,
    profile: String,
}

#[test]
fn scalar_expression_sql_is_shared_by_projection_filter_and_order() {
    let (sql, params) = Select::<ExprUser>::new()
        .filter(|u| u.email.to_lower().eq("alice@example.com"))
        .map_to(|u| {
            (
                u.id.cast::<String>(),
                ormer::expr!(match u.status {
                    "paid" => "done",
                    "new" => "open",
                    _ => "other",
                }),
                u.email.collate("nocase"),
            )
        })
        .order_by(|u| u.email.collate("nocase").asc())
        .to_sql_with_params(DbType::Sqlite);

    assert!(sql.contains("CAST(id AS TEXT)"));
    assert!(sql.contains("CASE status WHEN ? THEN ? WHEN ? THEN ? ELSE ? END"));
    assert!(sql.contains("email COLLATE nocase"));
    assert!(sql.contains("LOWER(email) = ?"));
    assert!(sql.contains("ORDER BY email COLLATE nocase ASC"));
    assert_eq!(params.len(), 6);
}

#[test]
fn aliased_projection_sql_is_rendered_per_column() {
    let (sql, params) = Select::<ExprUser>::new()
        .map_to(|u| {
            (
                u.id.alias("user_id"),
                u.email,
                ormer::expr!(match u.status {
                    "paid" => "done",
                    "new" => "open",
                    _ => "other",
                })
                .alias("status_label"),
            )
        })
        .to_sql_with_params(DbType::Sqlite);

    assert!(sql.contains("id AS"));
    assert!(sql.contains("user_id"));
    assert!(sql.contains("CASE status WHEN ? THEN ? WHEN ? THEN ? ELSE ? END AS"));
    assert!(sql.contains("status_label"));
    assert_eq!(params.len(), 5);
}

#[test]
fn aggregate_window_and_grouping_sql_are_expression_nodes() {
    let sql = Select::<ExprUser>::new()
        .select_column(|u| (u.org_id, u.score.sum().filter(|u| u.active.eq(true))))
        .group_by(|u| u.org_id)
        .to_sql();

    assert!(sql.contains("SELECT org_id, SUM(score) FILTER (WHERE active = ?)"));
    assert!(sql.contains("GROUP BY org_id"));

    let window_sql = Select::<ExprUser>::new()
        .map_to(|u| (u.org_id, u.score.sum().over(|w| w.partition_by(u.org_id))))
        .to_sql();

    assert!(window_sql.contains("SUM(score) OVER (PARTITION BY org_id)"));

    let rollup_sql = Select::<ExprUser>::new()
        .select_column(|u| (u.org_id, u.score.sum()))
        .rollup(|u| (u.org_id, u.status))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("SQLite cannot provide ROLLUP");

    assert!(matches!(
        rollup_sql,
        ormer::OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "advanced GROUP BY syntax",
        }
    ));

    let cube_sql = Select::<ExprUser>::new()
        .select_column(|u| (u.org_id, u.score.sum()))
        .cube(|u| (u.org_id, u.status))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("SQLite cannot provide CUBE");

    assert!(matches!(
        cube_sql,
        ormer::OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "advanced GROUP BY syntax",
        }
    ));

    let grouping_sets_sql = Select::<ExprUser>::new()
        .select_column(|u| (u.org_id, u.score.sum()))
        .grouping_sets(|u| ((u.org_id,), ()))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("SQLite cannot provide GROUPING SETS");

    assert!(matches!(
        grouping_sets_sql,
        ormer::OrmerError::UnsupportedFeature {
            backend: DbType::Sqlite,
            feature: "advanced GROUP BY syntax",
        }
    ));
}

#[test]
fn row_value_json_text_search_distinct_and_lock_sql() {
    let (sql, params) = Select::<ExprUser>::new()
        .distinct_on(|u| u.org_id)
        .order_by(|u| u.org_id.asc())
        .filter(|u| (u.org_id, u.email).eq((1, "a@example.com")))
        .filter(|u| u.profile.json_text("role").eq("admin"))
        .filter(|u| u.email.matches_text("rust"))
        .range(..10)
        .to_sql_with_params(DbType::Sqlite);

    assert!(sql.contains("ROW_NUMBER() OVER (PARTITION BY org_id ORDER BY org_id ASC)"));
    assert!(
        sql.contains("__ormer_ranked.__ormer_c0 AS id"),
        "{sql}"
    );
    assert!(sql.contains("__ormer_order_0\" ASC LIMIT 10"), "{sql}");
    assert!(sql.contains("\"__ormer_ranked\".\"__ormer_rank\" = 1"));
    assert!(sql.contains("(org_id, email) = (?, ?)"));
    assert!(sql.contains("json_extract(profile, '$.role') = ?"));
    assert!(sql.contains("email MATCH ?"));
    assert!(sql.contains("LIMIT 10"));
    assert_eq!(params.len(), 4);
}
