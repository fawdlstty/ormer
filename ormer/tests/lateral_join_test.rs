#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

use ormer::query::builder::Select;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_join!(TestUser, "test_lateral_users");
define_test_role_for_join!(TestRole, "test_lateral_roles");

/// 测试普通 left_join (无 order_by/range) SQL 生成不变
async fn test_plain_left_join_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new().left_join::<TestRole>(|p, q| p.id.eq(q.uid));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Plain LEFT JOIN SQL: {}", sql);
    assert!(sql.contains("LEFT JOIN"));
    assert!(!sql.contains("LATERAL"));
    assert!(sql.contains("ON t0.id = t1.uid"));
}

/// 测试带 order_by_desc + range 的 left_join 生成 LATERAL JOIN SQL
async fn test_lateral_join_with_order_and_range_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new()
        .left_join::<TestRole>(|p, q| p.id.eq(q.uid).order_by_desc(q.role_name).range(..1));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral LEFT JOIN SQL: {}", sql);
    assert!(sql.contains("LEFT JOIN LATERAL"));
    assert!(sql.contains("SELECT * FROM test_lateral_roles"));
    assert!(sql.contains("WHERE t0.id = uid"));
    assert!(sql.contains("ORDER BY role_name DESC"));
    assert!(sql.contains("LIMIT 1"));
    assert!(sql.contains(") AS t1 ON true"));
}

/// 测试只有 order_by 无 range 的情况
async fn test_lateral_join_order_only_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new()
        .left_join::<TestRole>(|p, q| p.id.eq(q.uid).order_by_desc(q.role_name));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral LEFT JOIN (order only) SQL: {}", sql);
    assert!(sql.contains("LEFT JOIN LATERAL"));
    assert!(sql.contains("ORDER BY role_name DESC"));
    assert!(!sql.contains("LIMIT"));
    assert!(sql.contains(") AS t1 ON true"));
}

/// 测试只有 range 无 order_by 的情况
async fn test_lateral_join_range_only_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new().left_join::<TestRole>(|p, q| p.id.eq(q.uid).range(..3));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral LEFT JOIN (range only) SQL: {}", sql);
    assert!(sql.contains("LEFT JOIN LATERAL"));
    assert!(sql.contains("LIMIT 3"));
    assert!(!sql.contains("ORDER BY"));
    assert!(sql.contains(") AS t1 ON true"));
}

/// 测试 LATERAL JOIN 与主查询 filter 组合
async fn test_lateral_join_with_outer_filter_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new()
        .left_join::<TestRole>(|p, q| p.id.eq(q.uid).order_by_desc(q.role_name).range(..1))
        .filter(|p| p.age.ge(18));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral LEFT JOIN with outer filter SQL: {}", sql);
    assert!(sql.contains("LEFT JOIN LATERAL"));
    assert!(sql.contains(") AS t1 ON true"));
    assert!(sql.contains("WHERE t0.age >="));
}

/// 测试普通 inner_join (无 order_by/range) SQL 生成不变
async fn test_plain_inner_join_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new().inner_join::<TestRole>(|p, q| p.id.eq(q.uid));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Plain INNER JOIN SQL: {}", sql);
    assert!(sql.contains("INNER JOIN"));
    assert!(!sql.contains("LATERAL"));
    assert!(sql.contains("ON t0.id = t1.uid"));
}

/// 测试带 order_by_desc + range 的 inner_join 生成 LATERAL JOIN SQL
async fn test_lateral_inner_join_with_order_and_range_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new()
        .inner_join::<TestRole>(|p, q| p.id.eq(q.uid).order_by_desc(q.role_name).range(..1));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral INNER JOIN SQL: {}", sql);
    assert!(sql.contains("INNER JOIN LATERAL"));
    assert!(sql.contains("SELECT * FROM test_lateral_roles"));
    assert!(sql.contains("WHERE t0.id = uid"));
    assert!(sql.contains("ORDER BY role_name DESC"));
    assert!(sql.contains("LIMIT 1"));
    assert!(sql.contains(") AS t1 ON true"));
}

/// 测试普通 right_join (无 order_by/range) SQL 生成不变
async fn test_plain_right_join_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new().right_join::<TestRole>(|p, q| p.id.eq(q.uid));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Plain RIGHT JOIN SQL: {}", sql);
    assert!(sql.contains("RIGHT JOIN"));
    assert!(!sql.contains("LATERAL"));
    assert!(sql.contains("ON t0.id = t1.uid"));
}

/// 测试带 order_by_desc + range 的 right_join 生成 LATERAL JOIN SQL
async fn test_lateral_right_join_with_order_and_range_impl(config: &_test_common::DbConfig) {
    let select = Select::<TestUser>::new()
        .right_join::<TestRole>(|p, q| p.id.eq(q.uid).order_by_desc(q.role_name).range(..1));
    let (sql, _) = select.to_sql_with_params(config.0);

    println!("Lateral RIGHT JOIN SQL: {}", sql);
    assert!(sql.contains("RIGHT JOIN LATERAL"));
    assert!(sql.contains("SELECT * FROM test_lateral_roles"));
    assert!(sql.contains("WHERE t0.id = uid"));
    assert!(sql.contains("ORDER BY role_name DESC"));
    assert!(sql.contains("LIMIT 1"));
    assert!(sql.contains(") AS t1 ON true"));
}

test_on_all_dbs!(test_plain_left_join_impl);
test_on_all_dbs!(test_lateral_join_with_order_and_range_impl);
test_on_all_dbs!(test_lateral_join_order_only_impl);
test_on_all_dbs!(test_lateral_join_range_only_impl);
test_on_all_dbs!(test_lateral_join_with_outer_filter_impl);
test_on_all_dbs!(test_plain_inner_join_impl);
test_on_all_dbs!(test_lateral_inner_join_with_order_and_range_impl);
test_on_all_dbs!(test_plain_right_join_impl);
test_on_all_dbs!(test_lateral_right_join_with_order_and_range_impl);
