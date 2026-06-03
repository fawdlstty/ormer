#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

use ormer::query::builder::Select;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_simple!(User, "test_between_users_1");

async fn test_between_sql_generation_impl(config: &_test_common::DbConfig) {
    // 测试 between 生成正确的 SQL
    let select = Select::<User>::new().filter(|p| p.age.between(18, 30));
    let (sql, params) = select.to_sql_with_params(config.0);

    assert!(
        sql.contains("BETWEEN"),
        "SQL should contain BETWEEN: {}",
        sql
    );
    assert!(sql.contains("AND"), "SQL should contain AND: {}", sql);
    assert_eq!(params.len(), 2, "BETWEEN should have 2 params");
    println!("SQL with between: {}", sql);
    println!("Params: {:?}", params);
}

async fn test_between_with_other_filters_impl(config: &_test_common::DbConfig) {
    // 测试 between 与其他 filter 组合
    let select = Select::<User>::new()
        .filter(|p| p.age.between(18, 30))
        .filter(|p| p.name.eq("alice"));
    let (sql, params) = select.to_sql_with_params(config.0);

    assert!(
        sql.contains("BETWEEN"),
        "SQL should contain BETWEEN: {}",
        sql
    );
    assert!(sql.contains("AND"), "SQL should contain AND: {}", sql);
    assert!(sql.contains("="), "SQL should contain =: {}", sql);
    // between 2 params + eq 1 param = 3 params
    assert_eq!(params.len(), 3, "Should have 3 params total");
    println!("SQL with between and eq: {}", sql);
}

async fn test_between_with_range_impl(config: &_test_common::DbConfig) {
    // 测试 between 与 range 组合
    let select = Select::<User>::new()
        .filter(|p| p.age.between(18, 30))
        .range(0..10);
    let (sql, params) = select.to_sql_with_params(config.0);

    assert!(
        sql.contains("BETWEEN"),
        "SQL should contain BETWEEN: {}",
        sql
    );
    assert!(sql.contains("LIMIT"), "SQL should contain LIMIT: {}", sql);
    assert_eq!(params.len(), 2, "BETWEEN should have 2 params");
    println!("SQL with between and range: {}", sql);
}

async fn test_between_param_values_impl(config: &_test_common::DbConfig) {
    // 验证参数值是否正确
    let select = Select::<User>::new().filter(|p| p.age.between(18, 30));
    let (_sql, params) = select.to_sql_with_params(config.0);

    assert_eq!(params.len(), 2);
    match &params[0] {
        ormer::Value::Integer(v) => assert_eq!(*v, 18, "First param should be 18"),
        _ => panic!("First param should be Integer(18), got {:?}", params[0]),
    }
    match &params[1] {
        ormer::Value::Integer(v) => assert_eq!(*v, 30, "Second param should be 30"),
        _ => panic!("Second param should be Integer(30), got {:?}", params[1]),
    }
}

test_on_all_dbs!(test_between_sql_generation_impl);
test_on_all_dbs!(test_between_with_other_filters_impl);
test_on_all_dbs!(test_between_with_range_impl);
test_on_all_dbs!(test_between_param_values_impl);
