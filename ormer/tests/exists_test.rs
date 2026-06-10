#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_join!(ExistsTestUser, "test_exists_users_1");
define_test_role!(ExistsTestRole, "test_exists_roles_1");

// ==================== EXISTS SQL 生成测试 ====================

#[test]
fn test_exists_basic() {
    // 测试基本 EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.name.eq("admin"));

    let where_expr = subquery.exists();

    // 将 WhereExpr 放入外层查询
    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM test_exists_roles_1"));
    assert!(outer_sql.contains("WHERE name ="));
}

#[test]
fn test_not_exists_basic() {
    // 测试基本 NOT EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.name.eq("admin"));

    let where_expr = subquery.not_exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("NOT EXISTS (SELECT 1 FROM test_exists_roles_1"));
    assert!(outer_sql.contains("WHERE name ="));
}

#[test]
fn test_exists_without_filter() {
    // 测试不带过滤条件的 EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new();
    let where_expr = subquery.exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM test_exists_roles_1)"));
    // 子查询没有 WHERE 子句
    let inner_part = &outer_sql[outer_sql.find("EXISTS (").unwrap()..];
    assert!(!inner_part.contains("WHERE"));
}

#[test]
fn test_exists_with_multiple_filters() {
    // 测试带多个过滤条件的 EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .filter(|r| r.uid.ge(10));

    let where_expr = subquery.exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM test_exists_roles_1"));
    assert!(outer_sql.contains("name ="));
    assert!(outer_sql.contains("uid >="));
    // 子查询中的多个条件用 AND 连接
    let exists_start = outer_sql.find("EXISTS (").unwrap();
    let exists_part = &outer_sql[exists_start..];
    assert!(exists_part.contains(" AND "));
}

#[test]
fn test_exists_combined_with_outer_filter() {
    // 测试外层查询同时有自己的 filter 和 EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.name.eq("admin"));

    let where_expr = subquery.exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|p| p.age.ge(18))
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("age >="));
    assert!(outer_sql.contains(" AND "));
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM test_exists_roles_1"));
}

#[test]
fn test_exists_combined_with_or() {
    // 测试 EXISTS 与普通条件用 OR 连接
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.name.eq("admin"));

    let where_expr = subquery.exists();

    // 使用 or() 方法组合
    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|p| p.age.ge(18).or(where_expr.clone()))
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("age >="));
    assert!(outer_sql.contains(" OR "));
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM test_exists_roles_1"));
}

#[test]
fn test_not_exists_combined_with_outer_filter() {
    // 测试外层查询同时有自己的 filter 和 NOT EXISTS 子查询
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.uid.eq(99));

    let where_expr = subquery.not_exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|p| p.age.ge(18))
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    assert!(outer_sql.contains("age >="));
    assert!(outer_sql.contains("NOT EXISTS (SELECT 1 FROM test_exists_roles_1"));
}

// ==================== EXISTS 与 IN 子查询对比测试 ====================

#[test]
fn test_exists_uses_select_1() {
    // 验证 EXISTS 子查询使用 SELECT 1 而非 SELECT *
    let subquery = ormer::Select::<ExistsTestRole>::new().filter(|r| r.name.eq("admin"));

    let where_expr = subquery.exists();

    let outer_sql = ormer::Select::<ExistsTestUser>::new()
        .filter(|_p| where_expr.clone())
        .to_sql();

    println!("SQL: {}", outer_sql);
    // 确认使用 SELECT 1
    assert!(outer_sql.contains("EXISTS (SELECT 1 FROM"));
    // 确认不使用 SELECT id, ... 或 SELECT *
    assert!(!outer_sql.contains("EXISTS (SELECT id"));
    assert!(!outer_sql.contains("EXISTS (SELECT *"));
}
