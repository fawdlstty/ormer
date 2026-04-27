#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_with_score!(TestGroupByCountUser, "test_groupby_count_users_1");
define_test_user_with_score!(TestGroupBySumUser, "test_groupby_sum_users_1");
define_test_user_with_score!(TestGroupByHavingUser, "test_groupby_having_users_1");
define_test_user_with_score!(TestGroupByMultiUser, "test_groupby_multi_users_1");
define_test_user_with_score!(TestGroupByWhereUser, "test_groupby_where_users_1");

/// 测试单字段分组 + COUNT
#[test]
fn test_group_by_count_sql() {
    use ormer::Select;

    // 测试 SQL 生成
    let sql = Select::<TestGroupByCountUser>::new()
        .select_column(|u| u.id.count())
        .group_by(|u| u.age)
        .to_sql();

    println!("GROUP BY COUNT SQL: {}", sql);
    assert!(sql.contains("SELECT COUNT(id)"));
    assert!(sql.contains("FROM test_groupby_count_users_1"));
    assert!(sql.contains("GROUP BY age"));
}

/// 测试单字段分组 + SUM
#[test]
fn test_group_by_sum_sql() {
    use ormer::Select;

    // 测试 SQL 生成
    let sql = Select::<TestGroupBySumUser>::new()
        .select_column(|u| u.score.sum())
        .group_by(|u| u.age)
        .to_sql();

    println!("GROUP BY SUM SQL: {}", sql);
    assert!(sql.contains("SELECT SUM(score)"));
    assert!(sql.contains("FROM test_groupby_sum_users_1"));
    assert!(sql.contains("GROUP BY age"));
}

/// 测试 HAVING 条件
#[test]
fn test_group_by_having_sql() {
    use ormer::Select;

    // 测试 SQL 生成
    let sql = Select::<TestGroupByHavingUser>::new()
        .select_column(|u| (u.age, u.id.count()))
        .group_by(|u| u.age)
        .having(|u| u.id.count().gt(1))
        .to_sql();

    println!("GROUP BY HAVING SQL: {}", sql);
    assert!(sql.contains("SELECT age, COUNT(id)"));
    assert!(sql.contains("FROM test_groupby_having_users_1"));
    assert!(sql.contains("GROUP BY age"));
    assert!(sql.contains("HAVING COUNT(id) >"));
}

/// 测试多字段分组
#[test]
fn test_group_by_multi_sql() {
    use ormer::Select;

    // 测试 SQL 生成 - 多字段分组
    let sql = Select::<TestGroupByMultiUser>::new()
        .select_column(|u| (u.age, u.name, u.id.count()))
        .group_by(|u| (u.age, u.name))
        .to_sql();

    println!("GROUP BY MULTI SQL: {}", sql);
    assert!(sql.contains("SELECT age, name, COUNT(id)"));
    assert!(sql.contains("FROM test_groupby_multi_users_1"));
    assert!(sql.contains("GROUP BY age, name"));
}

/// 测试 WHERE + GROUP BY + HAVING 组合
#[test]
fn test_group_by_where_having_sql() {
    use ormer::Select;

    // 测试 SQL 生成 - 完整查询
    let sql = Select::<TestGroupByWhereUser>::new()
        .filter(|u| u.age.ge(20))
        .select_column(|u| (u.age, u.id.count(), u.score.avg()))
        .group_by(|u| u.age)
        .having(|u| u.id.count().gt(0))
        .order_by(|u| u.age)
        .range(0..10)
        .to_sql();

    println!("GROUP BY COMPLETE SQL: {}", sql);
    assert!(sql.contains("SELECT age, COUNT(id), AVG(score)"));
    assert!(sql.contains("FROM test_groupby_where_users_1"));
    assert!(sql.contains("WHERE age >="));
    assert!(sql.contains("GROUP BY age"));
    assert!(sql.contains("HAVING COUNT(id) >"));
    assert!(sql.contains("ORDER BY age ASC"));
    assert!(sql.contains("LIMIT 10"));
}
