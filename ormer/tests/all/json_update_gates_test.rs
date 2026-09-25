#![cfg(any(feature = "clickhouse", feature = "questdb"))]

use ormer::abstract_layer::common::common_helpers::build_update_sql;
use ormer::{DbType, OrmerError, StaticJsonUpdate, UpdateField};

fn assert_update_is_rejected(backend: DbType, assignment: ormer::UpdateAssignment) {
    let error = build_update_sql::<JsonUpdateUser>(backend, &[assignment], &[])
        .expect_err("JSON updates must be capability gated");

    assert!(
        matches!(
            error,
            OrmerError::UnsupportedFeature {
                feature: "JSON updates",
                ..
            }
        ),
        "unexpected error for {backend:?}"
    );
}

#[derive(Debug, ormer::Model)]
#[table = "json_update_gate_users"]
struct JsonUpdateUser {
    #[primary]
    id: i32,
    profile: serde_json::Value,
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_rejects_json_updates() {
    let set = UpdateField::<serde_json::Value>::new("profile")
        .json_set("role", serde_json::json!("admin"))
        .assignment()
        .expect("assigned JSON set");
    assert_update_is_rejected(DbType::ClickHouse, set);

    let mut remove = StaticJsonUpdate::new("profile", vec!["role".to_string()]);
    remove.remove();
    assert_update_is_rejected(DbType::ClickHouse, remove.assignment().unwrap());
}

#[test]
#[cfg(feature = "questdb")]
fn questdb_rejects_json_updates() {
    let set = UpdateField::<serde_json::Value>::new("profile")
        .json_set("role", serde_json::json!("admin"))
        .assignment()
        .expect("assigned JSON set");
    assert_update_is_rejected(DbType::QuestDB, set);

    let mut remove = StaticJsonUpdate::new("profile", vec!["role".to_string()]);
    remove.remove();
    assert_update_is_rejected(DbType::QuestDB, remove.assignment().unwrap());
}
