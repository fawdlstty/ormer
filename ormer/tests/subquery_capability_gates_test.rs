#![cfg(any(feature = "clickhouse", feature = "questdb"))]

use ormer::{DbType, OrmerError, Select};

#[derive(Debug, ormer::Model)]
#[table = "subquery_gate_users"]
struct SubqueryGateUser {
    #[primary]
    id: i32,
    profile: serde_json::Value,
}

#[test]
fn unsupported_inner_subquery_expression_is_propagated() {
    #[cfg(feature = "clickhouse")]
    let backend = DbType::ClickHouse;
    #[cfg(all(feature = "questdb", not(feature = "clickhouse")))]
    let backend = DbType::QuestDB;

    let error = Select::<SubqueryGateUser>::new()
        .filter(|user| {
            user.id.is_in(
                Select::<SubqueryGateUser>::new()
                    .filter(|user| {
                        user.profile
                            .json_contains(serde_json::json!({ "role": "admin" }))
                    })
                    .map_to(|user| user.id),
            )
        })
        .try_to_sql_with_params(backend)
        .expect_err("inner subquery capabilities must be validated");

    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            feature: "JSON containment predicates",
            ..
        }
    ));
}
