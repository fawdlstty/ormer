#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

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
    // UNION 前后应各有一个完整的 SELECT 语句
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
}

#[test]
fn test_union_without_filters() {
    let sql = ormer::Select::<TestUser>::new()
        .union(ormer::Select::<TestUser>::new())
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("UNION"));
    assert!(!sql.contains("WHERE"));
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
