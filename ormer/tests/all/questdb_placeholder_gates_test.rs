#![cfg(feature = "questdb")]

use ormer::{DbType, OrmerError, Select};

#[derive(Debug, ormer::Model)]
#[table = "questdb_placeholder_gate_rows"]
struct PlaceholderGateRow {
    #[primary]
    id: i32,
    title: String,
    tags: Vec<String>,
    metadata: serde_json::Value,
    #[field(risk.score: f64)]
    profile: serde_json::Value,
}

fn assert_rejected(select: Select<PlaceholderGateRow>, feature: &'static str) {
    let error = select
        .try_to_sql_with_params(DbType::QuestDB)
        .expect_err("QuestDB must reject placeholder-backed capabilities");

    match error {
        OrmerError::UnsupportedFeature {
            backend: DbType::QuestDB,
            feature: actual,
        } => assert_eq!(actual, feature),
        other => panic!("unexpected error: {other:?}"),
    }
}

fn select_where<F>(filter: F) -> Select<PlaceholderGateRow>
where
    F: FnOnce(<PlaceholderGateRow as ormer::Model>::Where) -> ormer::WhereExpr,
{
    Select::new().filter(filter)
}

#[test]
fn questdb_rejects_placeholder_backed_expressions() {
    let select = select_where(|row| row.metadata.json_text("role").eq("admin"));
    assert_rejected(select, "JSON text extraction");
    assert_rejected(
        select_where(|row| row.metadata.json_path_text(["account", "role"]).eq("admin")),
        "JSON text extraction",
    );
    assert_rejected(
        select_where(|row| row.profile.risk.score.gt(0.5)),
        "JSON path value extraction",
    );
    assert_rejected(
        select_where(|row| row.profile.risk.score.exists()),
        "JSON path existence",
    );
    assert_rejected(
        select_where(|row| row.tags.contains_all(["read"])),
        "array containment predicates",
    );
    assert_rejected(
        select_where(|row| row.tags.overlaps(["read", "write"])),
        "array overlap predicates",
    );
    assert_rejected(select_where(|row| row.tags.len().gt(1)), "array length");
    assert_rejected(
        select_where(|row| row.title.matches_text("admin")),
        "text search",
    );
    assert_rejected(
        Select::new()
            .fields(|row: <PlaceholderGateRow as ormer::Model>::Where| row.title)
            .query("admin"),
        "full-text search",
    );
}
