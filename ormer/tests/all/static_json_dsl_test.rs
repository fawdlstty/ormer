#![cfg(feature = "sqlite")]

use ormer::DbType;
use ormer::abstract_layer::common::common_helpers::build_update_sql;

#[derive(Debug, ormer::Model)]
#[table = "static_json_dsl_users"]
struct StaticJsonUser {
    #[primary]
    id: i32,
    #[field(settings.active: bool)]
    #[field(settings.retry_count: i64)]
    #[field(risk.score: f64)]
    #[field(security.mfa)]
    #[field(cache.token)]
    #[field(cache.version: i64)]
    #[field(tags[]: String)]
    #[field(scopes[]: String)]
    profile: serde_json::Value,
}

#[test]
fn static_json_where_dsl_renders_values_paths_and_arrays() {
    let (sql, params) = ormer::Select::<StaticJsonUser>::new()
        .filter(|u| u.profile.settings.active.eq(true))
        .filter(|u| u.profile.settings.retry_count.gt(3))
        .filter(|u| u.profile.risk.score.lt(0.5))
        .filter(|u| u.profile.security.mfa.exists())
        .filter(|u| u.profile.scopes.contains_all(["read", "write"]))
        .filter(|u| u.profile.tags.contains_any(["new", "clearance"]))
        .try_to_sql_with_params(DbType::Sqlite)
        .expect("static JSON predicates are supported");

    assert!(
        sql.contains("json_extract(profile, '$.settings.active') = ?"),
        "{sql}"
    );
    assert!(
        sql.contains("json_extract(profile, '$.settings.retry_count') > ?"),
        "{sql}"
    );
    assert!(
        sql.contains("json_extract(profile, '$.risk.score') < ?"),
        "{sql}"
    );
    assert!(
        sql.contains("json_type(profile, '$.security.mfa') IS NOT NULL"),
        "{sql}"
    );
    assert!(sql.contains("json_extract(profile, '$.scopes')"), "{sql}");
    assert!(sql.contains("json_extract(profile, '$.tags')"), "{sql}");
    assert_eq!(params.len(), 5);
    assert!(matches!(params[0], ormer::Value::Boolean(true)));
    assert!(matches!(params[1], ormer::Value::Integer(3)));
    assert!(matches!(params[2], ormer::Value::Real(value) if value == 0.5));
    assert!(
        matches!(&params[3], ormer::Value::Json(value) if value == &serde_json::json!(["read", "write"]))
    );
}

#[test]
fn static_json_update_dsl_renders_nested_set_and_remove() {
    let mut update = <StaticJsonUser as ormer::Model>::Update::default();
    update.profile.cache.token.remove();
    update.profile.cache.version.set(2);
    let mut assignments = Vec::new();
    update.profile.cache.collect_assignments(&mut assignments);

    let (sql, params) = build_update_sql::<StaticJsonUser>(DbType::Sqlite, &assignments, &[])
        .expect("static JSON update SQL renders");

    assert!(
        sql.contains("profile = json_remove(profile, '$.cache.token')"),
        "SQL: {sql}"
    );
    assert!(
        sql.contains("profile = json_set(profile, '$.cache.version', ?)"),
        "SQL: {sql}"
    );
    assert!(matches!(params.as_slice(), [ormer::Value::Integer(2)]));
}
