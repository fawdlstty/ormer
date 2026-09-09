#![cfg(any(feature = "sqlite", feature = "influxdb"))]

//! P1 修复回归测试：
//! - P1-1 UNION/INTERSECT/EXCEPT 操作数括号包装，操作数内 ORDER BY/LIMIT 合法。
//! - P1-7 查询级命名过滤器保存 name，`unset_xxx()` 可撤销。
//! - P1-8 `exists()` 子查询包含 context filter（如软删除）。
//! - P1-12 JoinedSelect 补齐 order_by/limit/for_update API。
//! - P1-13 `range` 支持 `0..=10` 与 `..`；`start > end` 钳制为 LIMIT 0 而非 panic。
//! - P1-17 InfluxQL 预检：ORDER BY 仅 time、OFFSET 必须伴随 LIMIT、拒绝不可渲染表达式。
//! - P1-18 `array_append`/`array_remove` 等仅 PostgreSQL 的函数在其他后端返回
//!   `UnsupportedFeature`，而不是生成未定义函数调用。

use ormer::query::builder::Select;

pub mod _test_common;

define_test_user_for_join!(FixP1User, "fix_p1_query_users");
define_test_role_for_join!(FixP1Role, "fix_p1_query_roles");

/// 带软删除过滤器的模型（P1-7 / P1-8）
#[derive(Debug, Clone, ormer::Model)]
#[table = "fix_p1_soft_users"]
#[filter(filter_deleted, |m| m.deleted.eq(0))]
struct FixP1SoftUser {
    #[primary(auto)]
    id: i32,
    name: String,
    deleted: i32,
}

/// InfluxDB 时间序列模型：单个 DateTime 主键即时间键（P1-17）
#[cfg(feature = "influxdb")]
#[derive(Debug, Clone, ormer::Model)]
#[table = "fix_p1_metrics"]
struct FixP1Metric {
    #[primary]
    time: chrono::DateTime<chrono::Utc>,
    #[index]
    host: String,
    value: f64,
}

// ==================== P1-1 集合操作括号包装 ====================

#[cfg(feature = "sqlite")]
#[test]
fn union_operands_are_parenthesized() {
    let sql = Select::<FixP1User>::new()
        .filter(|u| u.age.gt(30))
        .order_by(|u| u.name)
        .range(..10)
        .union(
            Select::<FixP1User>::new()
                .filter(|u| u.age.lt(18))
                .order_by_desc(|u| u.age)
                .range(..5),
        )
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;

    println!("Union SQL: {sql}");
    assert!(sql.starts_with("(SELECT"), "{sql}");
    assert!(sql.contains(") UNION (SELECT"), "{sql}");
    assert!(sql.ends_with(')'), "{sql}");
    // 操作数内的 ORDER BY/LIMIT 保留在各自括号内
    let left = &sql[..sql.find(") UNION (SELECT").unwrap()];
    assert!(left.contains("ORDER BY name ASC"), "{sql}");
    assert!(left.contains("LIMIT 10"), "{sql}");
    assert!(!left.contains("ORDER BY age DESC"), "{sql}");
    let right = &sql[sql.find(") UNION (SELECT").unwrap()..];
    assert!(right.contains("ORDER BY age DESC"), "{sql}");
    assert!(right.contains("LIMIT 5"), "{sql}");
}

#[cfg(feature = "sqlite")]
#[test]
fn intersect_and_except_use_parentheses_too() {
    let intersect = Select::<FixP1User>::new()
        .filter(|u| u.age.gt(18))
        .intersect(Select::<FixP1User>::new().filter(|u| u.age.lt(65)))
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    assert!(intersect.contains(") INTERSECT (SELECT"), "{intersect}");

    let except = Select::<FixP1User>::new()
        .filter(|u| u.age.gt(18))
        .except(Select::<FixP1User>::new().filter(|u| u.name.eq("a")))
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    assert!(except.contains(") EXCEPT (SELECT"), "{except}");
}

// ==================== P1-7 命名过滤器可撤销 ====================

#[cfg(feature = "sqlite")]
#[test]
fn named_filter_applies_and_unset_revokes_it() {
    use FixP1SoftUserFilterExt;

    let applied = FixP1SoftUser::query()
        .filter_deleted()
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("named filter SQL: {applied}");
    assert!(applied.contains("WHERE deleted = ?"), "{applied}");

    let revoked = FixP1SoftUser::query()
        .filter_deleted()
        .unset_filter_deleted()
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("unset SQL: {revoked}");
    assert!(!revoked.contains("deleted = ?"), "{revoked}");
    assert!(!revoked.contains("WHERE"), "{revoked}");
}

#[cfg(feature = "sqlite")]
#[test]
fn named_filter_unset_keeps_other_filters_and_unnamed_filters() {
    use FixP1SoftUserFilterExt;

    let sql = FixP1SoftUser::query()
        .filter(|u| u.name.eq("bob".to_string()))
        .filter_deleted()
        .unset_filter_deleted()
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("mixed SQL: {sql}");
    assert!(sql.contains("WHERE name = ?"), "{sql}");
    assert!(!sql.contains("deleted = ?"), "{sql}");
}

#[cfg(feature = "sqlite")]
#[test]
fn named_filter_on_joined_select_can_be_revoked() {
    use ormer::WithoutFilterQuery;

    let sql = ormer::NamedFilterQuery::<FixP1User>::apply_named_filter(
        Select::<FixP1User>::new().left_join::<FixP1Role>(|u, r| u.id.eq(r.uid)),
        "filter_age",
        {
            let u = <FixP1User as ormer::Model>::Where::default();
            u.age.ge(18)
        },
    )
    .without_filter("filter_age")
    .to_sql_with_params(ormer::DbType::Sqlite)
    .0;
    println!("joined named filter revoked SQL: {sql}");
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(!sql.contains("WHERE"), "{sql}");
}

// ==================== P1-8 exists 包含 context filter ====================

#[cfg(feature = "sqlite")]
#[test]
fn exists_subquery_includes_named_context_filter() {
    use FixP1SoftUserFilterExt;

    // 模拟软删除 scope：命名过滤器以 context filter 形式挂在查询上，
    // exists() 子查询不能把它丢掉
    let exists = FixP1SoftUser::query()
        .filter_deleted()
        .filter(|u| u.name.eq("bob".to_string()))
        .exists();

    let outer = Select::<FixP1User>::new()
        .filter(|_| exists)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("EXISTS with context filter: {outer}");
    assert!(outer.contains("EXISTS (SELECT 1 FROM"), "{outer}");
    let exists_part = &outer[outer.find("EXISTS").unwrap()..];
    assert!(exists_part.contains("deleted = ?"), "{outer}");
    assert!(exists_part.contains("name = ?"), "{outer}");
}

// ==================== P1-12 JoinedSelect 排序/limit/行锁 ====================

#[cfg(feature = "sqlite")]
#[test]
fn joined_select_order_by_after_join() {
    let sql = Select::<FixP1User>::new()
        .left_join::<FixP1Role>(|u, r| u.id.eq(r.uid))
        .order_by(|u| u.name)
        .order_by_desc(|u| u.age)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("joined order_by SQL: {sql}");
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(sql.contains("ORDER BY name ASC, age DESC"), "{sql}");
}

#[cfg(feature = "sqlite")]
#[test]
fn joined_select_inner_and_right_order_by_and_limit() {
    let inner = Select::<FixP1User>::new()
        .inner_join::<FixP1Role>(|u, r| u.id.eq(r.uid))
        .order_by(|u| u.id)
        .limit(5)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("inner joined SQL: {inner}");
    assert!(inner.contains("INNER JOIN"), "{inner}");
    assert!(inner.contains("ORDER BY id ASC"), "{inner}");
    assert!(inner.contains("LIMIT 5"), "{inner}");

    let right = Select::<FixP1User>::new()
        .right_join::<FixP1Role>(|u, r| u.id.eq(r.uid))
        .order_by_desc(|u| u.name)
        .range(0..3)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("right joined SQL: {right}");
    assert!(right.contains("RIGHT JOIN"), "{right}");
    assert!(right.contains("ORDER BY name DESC"), "{right}");
    assert!(right.contains("LIMIT 3"), "{right}");
}

#[cfg(feature = "sqlite")]
#[test]
fn joined_select_for_update_appends_lock_clause() {
    let sql = Select::<FixP1User>::new()
        .left_join::<FixP1Role>(|u, r| u.id.eq(r.uid))
        .for_update()
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("joined FOR UPDATE SQL: {sql}");
    assert!(sql.ends_with(" FOR UPDATE"), "{sql}");

    // SQLite 不支持行锁：try 路径应返回 UnsupportedFeature 而不是生成 SQL
    let result = Select::<FixP1User>::new()
        .left_join::<FixP1Role>(|u, r| u.id.eq(r.uid))
        .for_update()
        .try_to_sql_with_params(ormer::DbType::Sqlite);
    assert!(
        result
            .err()
            .is_some_and(|err| matches!(err, ormer::OrmerError::UnsupportedFeature { .. })),
        "expected UnsupportedFeature for SQLite row locking"
    );
}

// ==================== P1-13 range 边界形态 ====================

#[cfg(feature = "sqlite")]
#[test]
fn range_inclusive_and_full_bounds() {
    let inclusive = Select::<FixP1User>::new()
        .range(0..=10)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("inclusive SQL: {inclusive}");
    // 0..=10 与 0..11 相同：LIMIT 11（闭区间右端计入行数）
    let exclusive = Select::<FixP1User>::new()
        .range(0..11)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    assert_eq!(inclusive, exclusive);
    assert!(inclusive.contains("LIMIT 11"), "{inclusive}");

    let full = Select::<FixP1User>::new()
        .range(..)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("full range SQL: {full}");
    assert!(!full.contains("LIMIT"), "{full}");
    assert!(!full.contains("OFFSET"), "{full}");
}

#[cfg(feature = "sqlite")]
#[test]
fn range_start_after_end_clamps_to_limit_zero() {
    // 10..5 → 不再 panic / 回绕，钳制为 LIMIT 0 OFFSET 10
    let sql = Select::<FixP1User>::new()
        .range(10..5)
        .to_sql_with_params(ormer::DbType::Sqlite)
        .0;
    println!("reversed range SQL: {sql}");
    assert!(sql.contains("LIMIT 0"), "{sql}");
    assert!(sql.contains("OFFSET 10"), "{sql}");
}

// ==================== P1-17 InfluxQL 预检 ====================

#[cfg(feature = "influxdb")]
#[test]
fn influx_order_by_allows_only_time_column() {
    let ok = Select::<FixP1Metric>::new()
        .order_by_desc(|m| m.time)
        .range(..10)
        .try_to_sql_with_params(ormer::DbType::InfluxDB);
    let sql = ok.expect("ORDER BY time DESC with LIMIT is valid InfluxQL").0;
    println!("influx ok SQL: {sql}");
    assert!(sql.contains("ORDER BY time DESC"), "{sql}");
    assert!(sql.contains("LIMIT 10"), "{sql}");

    let err = Select::<FixP1Metric>::new()
        .order_by(|m| m.host)
        .try_to_sql_with_params(ormer::DbType::InfluxDB)
        .unwrap_err();
    println!("influx ORDER BY host error: {err}");
    assert!(
        matches!(err, ormer::OrmerError::UnsupportedFeature { .. }),
        "expected UnsupportedFeature for ORDER BY on a tag column"
    );
}

#[cfg(feature = "influxdb")]
#[test]
fn influx_offset_requires_limit() {
    let err = Select::<FixP1Metric>::new()
        .range(5..)
        .try_to_sql_with_params(ormer::DbType::InfluxDB)
        .unwrap_err();
    println!("influx bare OFFSET error: {err}");
    assert!(
        matches!(err, ormer::OrmerError::UnsupportedFeature { .. }),
        "expected UnsupportedFeature for OFFSET without LIMIT"
    );

    // 成对出现（LIMIT + OFFSET）是合法的
    let paired = Select::<FixP1Metric>::new()
        .range(5..15)
        .try_to_sql_with_params(ormer::DbType::InfluxDB)
        .expect("LIMIT/OFFSET pair is valid InfluxQL");
    assert!(paired.0.contains("LIMIT 10"), "{}", paired.0);
    assert!(paired.0.contains("OFFSET 5"), "{}", paired.0);
}

#[cfg(feature = "influxdb")]
#[test]
fn influx_rejects_unrenderable_expressions_before_render() {
    // now() - 1d 会生成 DateAdd 表达式，InfluxQL 不支持，必须在渲染前报错
    let err = Select::<FixP1Metric>::new()
        .filter(|m| m.time.le(ormer::now() - ormer::days(1)))
        .try_to_sql_with_params(ormer::DbType::InfluxDB)
        .unwrap_err();
    println!("influx DateAdd filter error: {err}");
    assert!(
        matches!(err, ormer::OrmerError::UnsupportedFeature { .. }),
        "expected UnsupportedFeature for DateAdd expressions on InfluxDB"
    );

    // 普通列/值比较仍然可用
    let plain = Select::<FixP1Metric>::new()
        .filter(|m| m.host.eq("web-1"))
        .try_to_sql_with_params(ormer::DbType::InfluxDB)
        .expect("plain tag comparison is valid InfluxQL");
    assert!(plain.0.contains("host = ?"), "{}", plain.0);
}

// ==================== P1-18 PG 专有数组函数门控 ====================

#[cfg(feature = "sqlite")]
#[test]
fn array_functions_rejected_outside_postgresql() {
    use ormer::abstract_layer::common::common_helpers::build_update_sql;
    use ormer::{OrmerError, UpdateField};

    let append = UpdateField::<Vec<String>>::new("tags")
        .array_append("rust")
        .assignment()
        .expect("assigned array_append");
    let error =
        build_update_sql::<FixP1User>(ormer::DbType::Sqlite, &[append], &[])
            .expect_err("array_append must be gated on SQLite");
    println!("array_append gate error: {error}");
    assert!(
        matches!(
            error,
            OrmerError::UnsupportedFeature {
                feature: "PostgreSQL-only array functions",
                ..
            }
        ),
        "unexpected error for array_append on SQLite"
    );

    let remove = UpdateField::<Vec<String>>::new("tags")
        .array_remove("rust")
        .assignment()
        .expect("assigned array_remove");
    let error =
        build_update_sql::<FixP1User>(ormer::DbType::Sqlite, &[remove], &[])
            .expect_err("array_remove must be gated on SQLite");
    println!("array_remove gate error: {error}");
    assert!(
        matches!(
            error,
            OrmerError::UnsupportedFeature {
                feature: "PostgreSQL-only array functions",
                ..
            }
        ),
        "unexpected error for array_remove on SQLite"
    );
}
