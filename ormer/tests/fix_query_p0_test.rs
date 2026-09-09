#![cfg(feature = "sqlite")]

//! P0 修复回归测试：
//! - P0-3 关联查询对关联表列做值过滤时前缀必须限定到关联表别名（t1/t2/t3）。
//! - P0-4 GroupedSelect 聚合名不再 Box::leak（以生成的 SQL 正确性 + 重复渲染稳定断言）。
//! - P0-5 SQLite DateDiff 各时间单元的换算公式。

pub mod _test_common;

use ormer::query::builder::Select;
use ormer::{DbType, TimePart};

// id / name / age：name 与 Role.name 同名，复现条目中的歧义列场景
define_test_user_for_join!(FixP0User, "fix_p0_users");
// id / uid / name
define_test_role!(FixP0Role, "fix_p0_roles");
// id / uid / role_name：role_name 仅存在于该表
define_test_role_for_join!(FixP0JoinRole, "fix_p0_join_roles");

// 三表/四表查询的第三张表：level 仅存在于该表
#[derive(Debug, ormer::Model, Clone)]
#[table = "fix_p0_grades"]
struct FixP0Grade {
    #[primary]
    id: i32,
    uid: i32,
    level: i32,
}

// P0-5：带时间列的事件表
#[derive(Debug, ormer::Model, Clone)]
#[table = "fix_p0_events"]
struct FixP0Event {
    #[primary(auto)]
    id: i32,
    occurred_at: chrono::NaiveDateTime,
}

/// P0-3：关联查询对关联表列做值过滤时，必须限定到 t1（即使列名与主表重复）
#[test]
fn related_select_value_filter_uses_right_table_alias() {
    let sql = Select::<FixP0User>::new()
        .from::<FixP0Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .filter(|_, q| q.name.eq("admin".to_string()))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("RelatedSelect SQL: {sql}");
    assert!(sql.contains("FROM fix_p0_users AS t0, fix_p0_roles AS t1"), "{sql}");
    // 列-列比较保持位置约定：t0.id = t1.uid
    assert!(sql.contains("t0.id = t1.uid"), "{sql}");
    // 值过滤必须命中关联表：t1.name = ?，而不是 t0.name = ?
    assert!(sql.contains("t1.name = ?"), "{sql}");
    assert!(!sql.contains("t0.name = ?"), "{sql}");
}

/// P0-3：主表独有列的值过滤仍限定到 t0
#[test]
fn related_select_main_table_filter_keeps_t0() {
    let sql = Select::<FixP0User>::new()
        .from::<FixP0Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .filter(|p, _| p.age.ge(18))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("RelatedSelect main-table SQL: {sql}");
    assert!(sql.contains("t0.age >= ?"), "{sql}");
    assert!(!sql.contains("t1.age"), "{sql}");
}

/// P0-3：IN / BETWEEN / IS NULL 等值过滤形态同样解析到关联表别名
#[test]
fn related_select_value_filter_variants_use_right_table_alias() {
    let sql = Select::<FixP0User>::new()
        .from::<FixP0Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .filter(|_, q| q.uid.is_in(vec![1, 2]))
        .filter(|_, q| q.name.is_null())
        .filter(|_, q| q.id.between(1, 10))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("RelatedSelect variants SQL: {sql}");
    assert!(sql.contains("t1.uid IN (?, ?)"), "{sql}");
    assert!(sql.contains("t1.name IS NULL"), "{sql}");
    assert!(sql.contains("t1.id BETWEEN ? AND ?"), "{sql}");
}

/// P0-3：单表查询行为完全不变（无别名前缀）
#[test]
fn single_table_filter_stays_unqualified() {
    let sql = Select::<FixP0User>::new()
        .filter(|p| p.name.eq("bob".to_string()))
        .filter(|p| p.age.ge(18))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("Single-table SQL: {sql}");
    assert!(sql.contains("WHERE name = ? AND age >= ?"), "{sql}");
    assert!(!sql.contains("t0."), "{sql}");
}

/// P0-3：三表查询中第二张关联表（t2）的列正确解析
#[test]
fn multi_table_filter_uses_each_related_alias() {
    let sql = Select::<FixP0User>::new()
        .from3::<FixP0Role, FixP0Grade>()
        .filter(|p, q1, _q2| p.id.eq(q1.uid))
        .filter(|_, q1, _q2| q1.name.eq("admin".to_string()))
        .filter(|_, _, q2| q2.level.ge(3))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("MultiTableSelect SQL: {sql}");
    assert!(sql.contains("FROM fix_p0_users AS t0, fix_p0_roles AS t1, fix_p0_grades AS t2"), "{sql}");
    assert!(sql.contains("t1.name = ?"), "{sql}");
    assert!(sql.contains("t2.level >= ?"), "{sql}");
}

/// P0-3：四表查询中第三张关联表（t3）的列正确解析
#[test]
fn four_table_filter_uses_third_related_alias() {
    let sql = Select::<FixP0User>::new()
        .from4::<FixP0Role, FixP0Grade, FixP0JoinRole>()
        .filter(|p, q1, _q2, _q3| p.id.eq(q1.uid))
        .filter(|_, _, _, q3| q3.role_name.eq("admin".to_string()))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("FourTableSelect SQL: {sql}");
    assert!(sql.contains("t3.role_name = ?"), "{sql}");
}

/// P0-3：JOIN 的 ON 条件中对关联表列的值过滤限定到 t1
#[test]
fn join_on_condition_qualifies_right_table_value_filter() {
    let sql = Select::<FixP0User>::new()
        .left_join::<FixP0JoinRole>(|p, q| p.id.eq(q.uid).and(q.role_name.eq("admin".to_string())))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("JOIN ON SQL: {sql}");
    assert!(sql.contains("ON (t0.id = t1.uid AND t1.role_name = ?)"), "{sql}");
    assert!(!sql.contains("t0.role_name"), "{sql}");
}

/// P0-3：普通 JOIN 的 ON 条件（仅列-列比较）渲染保持不变
#[test]
fn join_on_condition_column_comparison_unchanged() {
    let sql = Select::<FixP0User>::new()
        .left_join::<FixP0JoinRole>(|p, q| p.id.eq(q.uid))
        .to_sql_with_params(DbType::Sqlite)
        .0;

    println!("Plain JOIN SQL: {sql}");
    assert!(sql.contains("ON t0.id = t1.uid"), "{sql}");
}

/// P0-4：GroupedSelect 聚合名改为持有 String，移除 Box::leak；
/// 生成的 SQL 保持正确且重复渲染结果一致
#[test]
fn grouped_select_aggregate_sql_stable_across_renders() {
    let build_sql = || {
        Select::<FixP0User>::new()
            .select_column(|u| u.id.count())
            .group_by(|u| u.age)
            .to_sql()
    };
    let first = build_sql();
    let second = build_sql();

    println!("GroupedSelect SQL: {first}");
    assert_eq!(first, second);
    assert!(first.contains("SELECT COUNT(id)"), "{first}");
    assert!(first.contains("GROUP BY age"), "{first}");
}

/// P0-5：SQLite DateDiff 各时间单元的换算公式。
/// julianday 差值单位为"天"：天数 * 86400 秒 / epoch_divisor。
fn date_diff_projection_sql(part: TimePart) -> String {
    Select::<FixP0Event>::new()
        .map_to(|e| {
            (
                e.id,
                e.occurred_at.until(ormer::now(), part).alias("diff"),
            )
        })
        .to_sql_with_params(DbType::Sqlite)
        .0
}

#[test]
fn sqlite_date_diff_formula_per_unit() {
    let cases = [
        (TimePart::Second, "86400 / 1"),
        (TimePart::Minute, "86400 / 60"),
        (TimePart::Hour, "86400 / 3600"),
        (TimePart::Day, "86400 / 86400"),
        (TimePart::Week, "86400 / 604800"),
        (TimePart::Month, "86400 / 2629746"),
        (TimePart::Year, "86400 / 31556952"),
    ];
    for (part, conversion) in cases {
        let sql = date_diff_projection_sql(part);
        let expected = format!(
            "CAST((julianday(occurred_at) - julianday(datetime('now'))) * {conversion} AS INTEGER)"
        );
        assert!(sql.contains(&expected), "part {part:?}: {sql}");
    }
}
