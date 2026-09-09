#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

pub mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_join!(TestUser, "test_union_users");

// ==================== UNION SQL 生成测试 ====================

#[test]
fn test_union_basic() {
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(30))
        .union(ormer::Select::<TestUser>::new().filter(|u| u.name.like("%admin%")))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("UNION"));
    assert!(sql.contains("WHERE age >"));
    assert!(sql.contains("WHERE name LIKE"));
    // 操作数括号包装：每个括号内各有一条完整 SELECT
    assert!(sql.starts_with("(SELECT"));
    assert!(sql.contains(") UNION (SELECT"));
    assert!(sql.ends_with(')'));
    let select_count = sql.matches("SELECT").count();
    assert_eq!(select_count, 2);
}

#[test]
fn test_union_all() {
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(30))
        .union_all(ormer::Select::<TestUser>::new().filter(|u| u.age.lt(18)))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("UNION ALL"));
    assert!(sql.contains("WHERE age >"));
    assert!(sql.contains("WHERE age <"));
}

#[test]
fn test_intersect() {
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(18))
        .intersect(ormer::Select::<TestUser>::new().filter(|u| u.age.lt(65)))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("INTERSECT"));
    assert!(sql.contains("WHERE age >"));
    assert!(sql.contains("WHERE age <"));
}

#[test]
fn test_except() {
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(18))
        .except(ormer::Select::<TestUser>::new().filter(|u| u.name.eq("admin")))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("EXCEPT"));
    assert!(sql.contains("WHERE age >"));
    assert!(sql.contains("WHERE name ="));
}

#[test]
fn test_union_with_order_and_range() {
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(30))
        .order_by(|u| u.name)
        .range(..10)
        .union(
            ormer::Select::<TestUser>::new()
                .filter(|u| u.age.lt(18))
                .order_by_desc(|u| u.age)
                .range(..5),
        )
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("UNION"));
    assert!(sql.contains("ORDER BY name ASC"));
    assert!(sql.contains("ORDER BY age DESC"));
    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("LIMIT 5"));
    // 集合操作数必须括号包装：`(SELECT ...) UNION (SELECT ...)`，
    // 否则操作数自带的 ORDER BY/LIMIT 会生成语法错误的 SQL。
    assert!(sql.starts_with("(SELECT"));
    assert!(sql.contains(") UNION (SELECT"));
    // 左操作数的 ORDER BY/LIMIT 必须保留在左括号内、右操作数在右括号内
    let left = &sql[..sql.find(") UNION (SELECT").unwrap()];
    assert!(left.contains("ORDER BY name ASC"));
    assert!(left.contains("LIMIT 10"));
    assert!(!left.contains("ORDER BY age DESC"));
}

#[test]
fn test_union_without_filters() {
    let sql = ormer::Select::<TestUser>::new()
        .union(ormer::Select::<TestUser>::new())
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("UNION"));
    assert!(!sql.contains("WHERE"));
    assert!(sql.starts_with("(SELECT"));
    assert!(sql.contains(") UNION (SELECT"));
    let select_count = sql.matches("SELECT").count();
    assert_eq!(select_count, 2);
}

// ==================== UNION 参数化测试 ====================

#[test]
fn test_union_with_params() {
    use ormer::DbType;

    #[cfg(feature = "sqlite")]
    let db_type = DbType::Sqlite;
    #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
    let db_type = DbType::PostgreSQL;
    #[cfg(all(
        not(feature = "sqlite"),
        not(feature = "postgresql"),
        feature = "mysql"
    ))]
    let db_type = DbType::MySQL;

    let (sql, params) = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(30))
        .union(ormer::Select::<TestUser>::new().filter(|u| u.name.eq("admin")))
        .to_sql_with_params(db_type);

    println!("SQL: {}", sql);
    println!("Params: {:?}", params);
    // 应包含两个参数: 30 和 "admin"
    assert_eq!(params.len(), 2);
}

// ==================== Clone 测试 ====================

#[test]
fn test_union_clone() {
    let union_select = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.gt(30))
        .union(ormer::Select::<TestUser>::new().filter(|u| u.name.eq("admin")));

    let cloned = union_select.clone();

    assert_eq!(union_select.to_sql(), cloned.to_sql());
}
