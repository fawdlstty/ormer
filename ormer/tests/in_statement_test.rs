#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_direct!(TestUser, "test_in_stmt_users_1");

async fn test_in_statement_i32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[i32] 类型
    let values: &[i32] = &[2, 4, 6, 7, 8];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?, ?, ?)"));
    assert!(sql.contains("WHERE"));
}

async fn test_in_statement_i32_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&i32] 类型
    let v1: &i32 = &2;
    let v2: &i32 = &4;
    let v3: &i32 = &6;
    let values: &[&i32] = &[v1, v2, v3];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?)"));
}

async fn test_in_statement_string_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[String] 类型
    let names: &[String] = &[
        "Alice".to_string(),
        "Bob".to_string(),
        "Charlie".to_string(),
    ];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?, ?)"));
}

async fn test_in_statement_string_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&String] 类型
    let names: Vec<String> = vec!["Alice".to_string(), "Bob".to_string()];
    let name_refs: Vec<&String> = names.iter().collect();
    let name_refs_slice: &[&String] = &name_refs;
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(name_refs_slice))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

async fn test_in_statement_str_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&str] 类型
    let names: &[&str] = &["Alice", "Bob", "Charlie"];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?, ?)"));
}

async fn test_in_with_other_filters_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 IN 与其他过滤器组合
    let values: &[i32] = &[20, 25, 30];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.ge(18))
        .filter(|p| p.age.is_in(values))
        .range(..10)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age >= ?"));
    assert!(sql.contains("age IN (?, ?, ?)"));
    assert!(sql.contains("LIMIT 10"));
}

async fn test_in_empty_array_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试空数组
    let empty_vec: &[i32] = &[];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(empty_vec))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN ()"));
}

// ==================== Vec 类型测试 ====================

async fn test_in_vec_i32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &Vec<i32> 类型
    let values: Vec<i32> = vec![1, 2, 3, 4, 5];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(&values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?, ?, ?)"));
}

async fn test_in_vec_i32_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &Vec<&i32> 类型
    let v1 = 10;
    let v2 = 20;
    let v3 = 30;
    let values: Vec<&i32> = vec![&v1, &v2, &v3];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(&values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?)"));
}

async fn test_in_vec_string_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &Vec<String> 类型
    let names: Vec<String> = vec!["Alice".to_string(), "Bob".to_string()];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(&names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

async fn test_in_vec_string_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &Vec<&String> 类型
    let s1 = "Alice".to_string();
    let s2 = "Bob".to_string();
    let names: Vec<&String> = vec![&s1, &s2];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(&names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

async fn test_in_vec_str_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &Vec<&str> 类型
    let names: Vec<&str> = vec!["Alice", "Bob", "Charlie"];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(&names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?, ?)"));
}

// ==================== 数组类型测试 ====================

async fn test_in_array_i32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[i32; N] 类型
    let values: &[i32; 4] = &[1, 2, 3, 4];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?, ?)"));
}

async fn test_in_array_i32_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&i32; N] 类型
    let v1 = 100;
    let v2 = 200;
    let v3 = 300;
    let values: &[&i32; 3] = &[&v1, &v2, &v3];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.age.is_in(values))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?)"));
}

async fn test_in_array_string_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[String; N] 类型
    let names: &[String; 2] = &["Alice".to_string(), "Bob".to_string()];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

async fn test_in_array_string_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&String; N] 类型
    let s1 = "Alice".to_string();
    let s2 = "Bob".to_string();
    let names: &[&String; 2] = &[&s1, &s2];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

async fn test_in_array_str_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试 &[&str; N] 类型
    let names: &[&str; 3] = &["Alice", "Bob", "Charlie"];
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.is_in(names))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?, ?)"));
}

// ==================== 直接字面量测试 ====================

async fn test_in_literal_array_i32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试直接使用数组字面量 &[T; N]
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| {
            let values: &[i32; 5] = &[2, 4, 6, 7, 8];
            p.age.is_in(values)
        })
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("age IN (?, ?, ?, ?, ?)"));
}

async fn test_in_literal_array_str_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    // 测试直接使用 &str 数组字面量 &[&str; N]
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| {
            let names: &[&str; 2] = &["Alice", "Bob"];
            p.name.is_in(names)
        })
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name IN (?, ?)"));
}

test_on_all_dbs!(test_in_statement_i32_impl);
test_on_all_dbs!(test_in_statement_i32_ref_impl);
test_on_all_dbs!(test_in_statement_string_impl);
test_on_all_dbs!(test_in_statement_string_ref_impl);
test_on_all_dbs!(test_in_statement_str_impl);
test_on_all_dbs!(test_in_with_other_filters_impl);
test_on_all_dbs!(test_in_empty_array_impl);
test_on_all_dbs!(test_in_vec_i32_impl);
test_on_all_dbs!(test_in_vec_i32_ref_impl);
test_on_all_dbs!(test_in_vec_string_impl);
test_on_all_dbs!(test_in_vec_string_ref_impl);
test_on_all_dbs!(test_in_vec_str_impl);
test_on_all_dbs!(test_in_array_i32_impl);
test_on_all_dbs!(test_in_array_i32_ref_impl);
test_on_all_dbs!(test_in_array_string_impl);
test_on_all_dbs!(test_in_array_string_ref_impl);
test_on_all_dbs!(test_in_array_str_impl);
test_on_all_dbs!(test_in_literal_array_i32_impl);
test_on_all_dbs!(test_in_literal_array_str_impl);
