#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::query::builder::Select;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_join!(User, "test_range_users_1");
define_test_role!(Role, "test_range_roles_1");

async fn test_basic_range_impl(config: &_test_common::DbConfig) {
    // 测试 range(0..10)
    let select = Select::<User>::new().range(0..10);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with range(0..10): {}", sql);
}

async fn test_range_with_offset_impl(config: &_test_common::DbConfig) {
    // 测试 range(10..20)
    let select = Select::<User>::new().range(10..20);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 10"));
    println!("SQL with range(10..20): {}", sql);
}

async fn test_range_with_filter_impl(config: &_test_common::DbConfig) {
    // 测试 range 与 filter 组合
    let select = Select::<User>::new().filter(|p| p.age.ge(18)).range(5..15);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("WHERE"));
    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 5"));
    println!("SQL with filter and range: {}", sql);
}

async fn test_range_with_order_by_impl(config: &_test_common::DbConfig) {
    // 测试 range 与 order_by 组合
    let select = Select::<User>::new().order_by(|p| p.age.desc()).range(0..5);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("ORDER BY"));
    assert!(sql.contains("LIMIT 5"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with order_by and range: {}", sql);
}

async fn test_range_single_record_impl(config: &_test_common::DbConfig) {
    // 测试单条记录 range(0..1)
    let select = Select::<User>::new().range(0..1);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("LIMIT 1"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with range(0..1): {}", sql);
}

async fn test_no_range_impl(config: &_test_common::DbConfig) {
    // 测试不使用 range 时不生成 LIMIT/OFFSET
    let select = Select::<User>::new();
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(!sql.contains("LIMIT"));
    assert!(!sql.contains("OFFSET"));
    println!("SQL without range: {}", sql);
}

async fn test_related_select_range_impl(config: &_test_common::DbConfig) {
    // 测试 RelatedSelect 的 range 功能
    let select = Select::<User>::new()
        .from::<User, Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .range(0..10);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("RelatedSelect SQL with range: {}", sql);
}

async fn test_left_join_range_impl(config: &_test_common::DbConfig) {
    // 测试 LEFT JOIN 的 range 功能
    let select = Select::<User>::new()
        .left_join::<Role>(|p, q| p.id.eq(q.uid))
        .range(0..10);
    let (sql, _) = select.to_sql_with_params(config.0);

    assert!(sql.contains("LEFT JOIN"));
    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("LeftJoin SQL with range: {}", sql);
}

async fn test_range_calculation_impl(config: &_test_common::DbConfig) {
    // 验证 LIMIT 和 OFFSET 的计算是否正确
    let test_cases = vec![
        (0..10, 10, 0),
        (5..15, 10, 5),
        (10..20, 10, 10),
        (0..1, 1, 0),
        (100..200, 100, 100),
    ];

    for (range, expected_limit, expected_offset) in test_cases {
        let select = Select::<User>::new().range(range);
        let (sql, _) = select.to_sql_with_params(config.0);

        assert!(
            sql.contains(&format!("LIMIT {}", expected_limit)),
            "Expected LIMIT {} in SQL: {}",
            expected_limit,
            sql
        );
        assert!(
            sql.contains(&format!("OFFSET {}", expected_offset)),
            "Expected OFFSET {} in SQL: {}",
            expected_offset,
            sql
        );
    }
    println!("All range calculation tests passed!");
}

test_on_all_dbs!(test_basic_range_impl);
test_on_all_dbs!(test_range_with_offset_impl);
test_on_all_dbs!(test_range_with_filter_impl);
test_on_all_dbs!(test_range_with_order_by_impl);
test_on_all_dbs!(test_range_single_record_impl);
test_on_all_dbs!(test_no_range_impl);
test_on_all_dbs!(test_related_select_range_impl);
test_on_all_dbs!(test_left_join_range_impl);
test_on_all_dbs!(test_range_calculation_impl);
