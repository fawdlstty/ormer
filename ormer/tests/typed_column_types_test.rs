use ormer::query::builder::{ColumnValueType, TypedColumn};
use ormer::query::filter::FilterExpr;

// 辅助函数：从 WhereExpr 提取 FilterExpr
fn get_filter_expr(where_expr: ormer::query::builder::WhereExpr) -> FilterExpr {
    where_expr.into()
}

// 测试各种整数类型
#[test]
fn test_typed_column_i8() {
    let col: TypedColumn<i8> = TypedColumn::new("test_col");
    let expr = col.ge(10);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">="),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_i16() {
    let col: TypedColumn<i16> = TypedColumn::new("test_col");
    let expr = col.gt(100);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">"),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_u32() {
    let col: TypedColumn<u32> = TypedColumn::new("test_col");
    let expr = col.le(1000);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<="),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_u64() {
    let col: TypedColumn<u64> = TypedColumn::new("test_col");
    let expr = col.lt(10000);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<"),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_usize() {
    let col: TypedColumn<usize> = TypedColumn::new("test_col");
    let expr = col.eq(42);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试浮点类型
#[test]
fn test_typed_column_f32() {
    let col: TypedColumn<f32> = TypedColumn::new("test_col");
    let expr = col.ge(3.14);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, ">="),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_f64() {
    let col: TypedColumn<f64> = TypedColumn::new("test_col");
    let expr = col.le(2.718);
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "<="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试字符串类型
#[test]
fn test_typed_column_string() {
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let expr = col.eq("hello".to_string());
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

#[test]
fn test_typed_column_str_ref() {
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let expr = col.eq("world");
    match get_filter_expr(expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, "="),
        _ => panic!("Expected Comparison"),
    }
}

// 测试 IN 语句支持各种类型
#[test]
fn test_is_in_i32() {
    let col: TypedColumn<i32> = TypedColumn::new("test_col");
    let values = vec![1, 2, 3];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

#[test]
fn test_is_in_i64() {
    let col: TypedColumn<i64> = TypedColumn::new("test_col");
    let values = vec![100i64, 200, 300];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

#[test]
fn test_is_in_u32() {
    let col: TypedColumn<u32> = TypedColumn::new("test_col");
    let values = vec![10u32, 20, 30];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

#[test]
fn test_is_in_f64() {
    let col: TypedColumn<f64> = TypedColumn::new("test_col");
    let values = vec![1.1, 2.2, 3.3];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

#[test]
fn test_is_in_string() {
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let values = vec![
        "apple".to_string(),
        "banana".to_string(),
        "cherry".to_string(),
    ];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

#[test]
fn test_is_in_str_ref() {
    let col: TypedColumn<String> = TypedColumn::new("test_col");
    let values = vec!["red", "green", "blue"];
    let expr = col.is_in(values);
    match get_filter_expr(expr) {
        FilterExpr::In { .. } => {} // Success
        _ => panic!("Expected In"),
    }
}

// 测试列引用比较
#[test]
fn test_column_ref_comparison() {
    let col1: TypedColumn<i64> = TypedColumn::new("col1");
    let col2: TypedColumn<i64> = TypedColumn::new("col2");
    let expr = col1.eq(col2);
    match get_filter_expr(expr) {
        FilterExpr::ColumnComparison { .. } => {} // Success
        _ => panic!("Expected ColumnComparison"),
    }
}

// 验证 ColumnValueType trait 实现
#[test]
fn test_column_value_type_i32() {
    let value = ColumnValueType::to_filter_value(42i32);
    match value {
        ormer::query::filter::Value::Integer(v) => assert_eq!(v, 42),
        _ => panic!("Expected Integer value"),
    }
}

#[test]
fn test_column_value_type_f64() {
    let value = ColumnValueType::to_filter_value(3.14f64);
    match value {
        ormer::query::filter::Value::Real(v) => assert!((v - 3.14).abs() < 0.001),
        _ => panic!("Expected Real value"),
    }
}

#[test]
fn test_column_value_type_string() {
    let value = ColumnValueType::to_filter_value("hello".to_string());
    match value {
        ormer::query::filter::Value::Text(v) => assert_eq!(v, "hello"),
        _ => panic!("Expected Text value"),
    }
}

#[test]
fn test_column_value_type_supports_comparison() {
    assert!(i32::supports_comparison());
    assert!(i64::supports_comparison());
    assert!(f64::supports_comparison());
    assert!(u32::supports_comparison());
    assert!(!String::supports_comparison());
}
