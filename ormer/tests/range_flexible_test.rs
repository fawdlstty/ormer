use ormer::Model;
use ormer::abstract_layer::DbType;
use ormer::query::builder::Select;

// 定义测试用的 User 模型
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[test]
fn test_range_full() {
    // 测试完整范围 range(10..20) - 既有 offset 也有 limit
    let select = Select::<User>::new().range(10..20);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("OFFSET 10"));
    assert!(sql.contains("LIMIT 10"));
    println!("SQL with range(10..20): {}", sql);
}

#[test]
fn test_range_to() {
    // 测试只有上限 range(..10) - 只有 limit,没有 offset
    let select = Select::<User>::new().range(..10);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(!sql.contains("OFFSET"));
    assert!(sql.contains("LIMIT 10"));
    println!("SQL with range(..10): {}", sql);
}

#[test]
fn test_range_from() {
    // 测试只有下限 range(10..) - 只有 offset,没有 limit
    let select = Select::<User>::new().range(10..);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("OFFSET 10"));
    assert!(!sql.contains("LIMIT"));
    println!("SQL with range(10..): {}", sql);
}

#[test]
fn test_range_zero_to_ten() {
    // 测试 range(0..10)
    let select = Select::<User>::new().range(0..10);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("OFFSET 0"));
    assert!(sql.contains("LIMIT 10"));
    println!("SQL with range(0..10): {}", sql);
}

#[test]
fn test_range_with_filter() {
    // 测试 range 与 filter 组合
    let select = Select::<User>::new().filter(|p| p.age.ge(18)).range(..5);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("WHERE"));
    assert!(!sql.contains("OFFSET"));
    assert!(sql.contains("LIMIT 5"));
    println!("SQL with filter and range(..5): {}", sql);
}

#[test]
fn test_range_from_with_filter() {
    // 测试 range(10..) 与 filter 组合
    let select = Select::<User>::new().filter(|p| p.age.ge(18)).range(10..);
    let (sql, _) = select.to_sql_with_params(DbType::Turso);

    assert!(sql.contains("WHERE"));
    assert!(sql.contains("OFFSET 10"));
    assert!(!sql.contains("LIMIT"));
    println!("SQL with filter and range(10..): {}", sql);
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
fn test_range_boundary_cases() {
    // 测试边界情况
    // 测试 range(0..10)
    let select1 = Select::<User>::new().range(0..10);
    let (sql1, _) = select1.to_sql_with_params(DbType::Turso);
    assert!(sql1.contains("OFFSET 0"));
    assert!(sql1.contains("LIMIT 10"));

    // 测试 range(..5)
    let select2 = Select::<User>::new().range(..5);
    let (sql2, _) = select2.to_sql_with_params(DbType::Turso);
    assert!(!sql2.contains("OFFSET"));
    assert!(sql2.contains("LIMIT 5"));

    // 测试 range(20..)
    let select3 = Select::<User>::new().range(20..);
    let (sql3, _) = select3.to_sql_with_params(DbType::Turso);
    assert!(sql3.contains("OFFSET 20"));
    assert!(!sql3.contains("LIMIT"));

    // 测试 range(10..30)
    let select4 = Select::<User>::new().range(10..30);
    let (sql4, _) = select4.to_sql_with_params(DbType::Turso);
    assert!(sql4.contains("OFFSET 10"));
    assert!(sql4.contains("LIMIT 20"));

    println!("All boundary case tests passed!");
}
