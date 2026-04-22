use ormer::Model;
use ormer::abstract_layer::DbType;
use ormer::query::builder::{LeftJoinedSelect, RelatedSelect, Select};

// 定义测试用的 User 模型
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

// 定义测试用的 Role 模型
#[derive(Debug, Model)]
#[table = "roles"]
struct Role {
    #[primary]
    id: i32,
    uid: i32,
    name: String,
}

#[test]
fn test_basic_range() {
    // 测试 range(0..10)
    let select = Select::<User>::new().range(0..10);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with range(0..10): {}", sql);
}

#[test]
fn test_range_with_offset() {
    // 测试 range(10..20)
    let select = Select::<User>::new().range(10..20);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 10"));
    println!("SQL with range(10..20): {}", sql);
}

#[test]
fn test_range_with_filter() {
    // 测试 range 与 filter 组合
    let select = Select::<User>::new().filter(|p| p.age.ge(18)).range(5..15);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("WHERE"));
    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 5"));
    println!("SQL with filter and range: {}", sql);
}

#[test]
fn test_range_with_order_by() {
    // 测试 range 与 order_by 组合
    let select = Select::<User>::new().order_by(|p| p.age.desc()).range(0..5);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("ORDER BY"));
    assert!(sql.contains("LIMIT 5"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with order_by and range: {}", sql);
}

#[test]
fn test_range_single_record() {
    // 测试单条记录 range(0..1)
    let select = Select::<User>::new().range(0..1);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("LIMIT 1"));
    assert!(sql.contains("OFFSET 0"));
    println!("SQL with range(0..1): {}", sql);
}

#[test]
fn test_no_range() {
    // 测试不使用 range 时不生成 LIMIT/OFFSET
    let select = Select::<User>::new();
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(!sql.contains("LIMIT"));
    assert!(!sql.contains("OFFSET"));
    println!("SQL without range: {}", sql);
}

#[test]
fn test_related_select_range() {
    // 测试 RelatedSelect 的 range 功能
    let select = Select::<User>::new()
        .from::<User, Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .range(0..10);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("RelatedSelect SQL with range: {}", sql);
}

#[test]
fn test_left_join_range() {
    // 测试 LEFT JOIN 的 range 功能
    let select = Select::<User>::new()
        .left_join::<Role>(|p, q| p.id.eq(q.uid))
        .range(0..10);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("LEFT JOIN"));
    assert!(sql.contains("LIMIT 10"));
    assert!(sql.contains("OFFSET 0"));
    println!("LeftJoin SQL with range: {}", sql);
}

#[test]
fn test_range_calculation() {
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
        let (sql, _) = select.to_sql_with_params(DbType::Turso);

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
