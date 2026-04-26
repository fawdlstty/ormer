#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::query::builder::TypedColumn;
use ormer::query::filter::FilterExpr;

mod _test_common;

// 此测试不需要模型定义，仅测试 TypedColumn 类型

// 辅助函数：从 WhereExpr 提取 FilterExpr
fn get_filter_expr(where_expr: ormer::query::builder::WhereExpr) -> FilterExpr {
    where_expr.into()
}

// 测试各种整数类型
async fn test_typed_column_i8_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<i8> = TypedColumn::new("test_col");
    let expr = col.ge(10);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">="),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_i16_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<i16> = TypedColumn::new("test_col");
    let expr = col.gt(100);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">"),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_u32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<u32> = TypedColumn::new("test_col");
    let expr = col.le(1000);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<="),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_u64_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<u64> = TypedColumn::new("test_col");
    let expr = col.lt(10000);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<"),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_usize_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<usize> = TypedColumn::new("test_col");
    let expr = col.eq(42);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试浮点类型
async fn test_typed_column_f32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<f32> = TypedColumn::new("test_col");
    let expr = col.ge(3.14);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">="),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_f64_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<f64> = TypedColumn::new("test_col");
    let expr = col.le(2.718);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试字符串类型
async fn test_typed_column_string_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let expr = col.eq("hello".to_string());
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

async fn test_typed_column_str_ref_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let expr = col.eq("world");
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试 IN 语句支持各种类型
async fn test_is_in_i32_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<i32> = TypedColumn::new("test_col");
    let values = vec![1, 2, 3];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

async fn test_is_in_i64_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<i64> = TypedColumn::new("test_col");
    let values = vec![100i64, 200, 300];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

async fn test_is_in_string_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let values = vec!["a".to_string(), "b".to_string()];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

test_on_all_dbs!(test_typed_column_i8_impl);
test_on_all_dbs!(test_typed_column_i16_impl);
test_on_all_dbs!(test_typed_column_u32_impl);
test_on_all_dbs!(test_typed_column_u64_impl);
test_on_all_dbs!(test_typed_column_usize_impl);
test_on_all_dbs!(test_typed_column_f32_impl);
test_on_all_dbs!(test_typed_column_f64_impl);
test_on_all_dbs!(test_typed_column_string_impl);
test_on_all_dbs!(test_typed_column_str_ref_impl);
test_on_all_dbs!(test_is_in_i32_impl);
test_on_all_dbs!(test_is_in_i64_impl);
test_on_all_dbs!(test_is_in_string_impl);
