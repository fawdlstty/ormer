#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_with_score!(TestSubqueryUser, "test_subquery_users_1");
define_test_user_with_score!(TestSubqueryPost, "test_subquery_posts_1");

/// 测试基本的子查询 - WHERE column IN (SELECT ...)
#[test]
fn test_subquery_basic_sql() {
    use ormer::Select;

    // 测试 SQL 生成 - 查找age在子查询结果中的用户
    let subquery = Select::<TestSubqueryPost>::new().map_to(|p| p.age);

    let sql = Select::<TestSubqueryUser>::new()
        .filter(|u| u.age.is_in(subquery))
        .to_sql();

    println!("BASIC SUBQUERY SQL: {}", sql);
    assert!(sql.contains("SELECT age"));
    assert!(sql.contains("FROM test_subquery_posts_1"));
    assert!(sql.contains("WHERE age IN ("));
}

/// 测试带条件的子查询
#[test]
fn test_subquery_with_filter_sql() {
    use ormer::Select;

    // 子查询: 查找score大于80的用户的age
    let subquery = Select::<TestSubqueryUser>::new()
        .filter(|u| u.score.gt(80))
        .map_to(|u| u.age);

    let sql = Select::<TestSubqueryPost>::new()
        .filter(|p| p.age.is_in(subquery))
        .to_sql();

    println!("FILTERED SUBQUERY SQL: {}", sql);
    assert!(sql.contains("SELECT age"));
    assert!(sql.contains("FROM test_subquery_users_1"));
    assert!(sql.contains("WHERE score >"));
    assert!(sql.contains("WHERE age IN ("));
}

/// 测试嵌套子查询
#[test]
fn test_subquery_nested_sql() {
    use ormer::Select;

    // 子查询: 查找age>20的用户的age
    let inner_subquery = Select::<TestSubqueryUser>::new()
        .filter(|u| u.age.gt(20))
        .map_to(|u| u.age);

    // 外层子查询: 查找age在这些值中的用户的age
    let outer_subquery = Select::<TestSubqueryPost>::new()
        .filter(|p| p.age.is_in(inner_subquery))
        .map_to(|p| p.age);

    // 最外层: 查找age在外层子查询结果中的用户
    let sql = Select::<TestSubqueryUser>::new()
        .filter(|u| u.age.is_in(outer_subquery))
        .to_sql();

    println!("NESTED SUBQUERY SQL: {}", sql);
    assert!(sql.contains("WHERE age IN ("));
    // 应该包含两个IN子句（嵌套）
    let in_count = sql.matches("IN (").count();
    assert_eq!(in_count, 2, "Should have 2 IN clauses for nested subquery");
}

/// 测试子查询带参数
#[test]
fn test_subquery_with_params() {
    use ormer::Select;
    use ormer::abstract_layer::DbType;

    // 子查询带参数
    let subquery = Select::<TestSubqueryUser>::new()
        .filter(|u| u.age.gt(25))
        .map_to(|u| u.age);

    let (sql, params) = Select::<TestSubqueryPost>::new()
        .filter(|p| p.age.is_in(subquery))
        .to_sql_with_params(DbType::Sqlite);

    println!("SUBQUERY WITH PARAMS SQL: {}", sql);
    println!("Params: {:?}", params);

    assert!(sql.contains("WHERE age >"));
    assert!(sql.contains("?")); // Sqlite uses ? for parameters
    assert!(!params.is_empty(), "Should have parameters");
}
