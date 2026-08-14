#![cfg(feature = "sqlite")]

use ormer::{DbType, UpdateFields, Value};

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "json_array_dsl_users"]
struct JsonArrayUser {
    #[primary]
    id: i32,
    profile: serde_json::Value,
    tags: Vec<String>,
}

#[test]
fn json_and_array_filter_dsl_renders_sql_and_params() {
    let (sql, params) = ormer::Select::<JsonArrayUser>::new()
        .filter(|u| {
            u.profile
                .json_contains(serde_json::json!({ "role": "admin" }))
        })
        .filter(|u| u.profile.json_path_text(["account", "role"]).eq("admin"))
        .filter(|u| u.tags.overlaps(["ops", "admin"]))
        .filter(|u| u.tags.contains_all(["ops"]))
        .filter(|u| u.tags.len().gt(1))
        .to_sql_with_params(DbType::Sqlite);

    assert!(sql.contains("json_type(profile) IS NOT NULL"), "SQL: {sql}");
    assert!(
        sql.contains("json_extract(profile, '$.account.role') = ?"),
        "SQL: {sql}"
    );
    assert!(
        sql.contains("json_each(tags)") && sql.contains("json_array_length(tags) > ?"),
        "SQL: {sql}"
    );
    assert_eq!(params.len(), 5);
    assert!(matches!(params[0], Value::Json(_)));
    assert_text_param(&params[1], "admin");
    assert_text_array_param(&params[2], &["ops", "admin"]);
    assert_text_array_param(&params[3], &["ops"]);
    assert_integer_param(&params[4], 1);
}

fn assert_text_param(value: &Value, expected: &str) {
    match value {
        Value::Text(value) => assert_eq!(value, expected),
        other => panic!("expected text parameter, got {other:?}"),
    }
}

fn assert_text_array_param(value: &Value, expected: &[&str]) {
    match value {
        Value::TextArray(values) => {
            assert_eq!(
                values,
                &expected
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            );
        }
        other => panic!("expected text array parameter, got {other:?}"),
    }
}

fn assert_integer_param(value: &Value, expected: i64) {
    match value {
        Value::Integer(value) => assert_eq!(*value, expected),
        other => panic!("expected integer parameter, got {other:?}"),
    }
}

#[test]
fn json_set_update_dsl_renders_sql_and_params() {
    let (sql, params) =
        ormer::abstract_layer::common::common_helpers::build_update_sql::<JsonArrayUser>(
            DbType::Sqlite,
            &{
                let mut update = <JsonArrayUser as ormer::Model>::Update::default();
                update.profile = update
                    .profile
                    .json_set("last_login", serde_json::json!("2026-08-13T00:00:00Z"));
                update.assignments()
            },
            &[],
        )
        .expect("update SQL renders");

    assert!(
        sql.contains(
            "UPDATE json_array_dsl_users SET profile = json_set(profile, '$.last_login', ?)"
        ),
        "SQL: {sql}"
    );
    assert_eq!(params.len(), 1);
    assert!(matches!(params[0], Value::Json(_)));
}
