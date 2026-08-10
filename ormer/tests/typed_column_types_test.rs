#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

use ormer::query::builder::TypedColumn;
use ormer::query::filter::FilterExpr;

mod _test_common;

// 此测试不需要模型定义，仅测试 TypedColumn 类型

// 辅助函数：从 WhereExpr 提取 FilterExpr
fn get_filter_expr(where_expr: ormer::query::builder::WhereExpr) -> FilterExpr {
    where_expr.into()
}

fn assert_comparison_operator(where_expr: ormer::query::builder::WhereExpr, expected: &str) {
    match get_filter_expr(where_expr) {
        FilterExpr::Comparison { operator, .. } => assert_eq!(operator, expected),
        _ => panic!("Expected Comparison"),
    }
}

fn assert_comparison_column_operator(
    where_expr: ormer::query::builder::WhereExpr,
    expected_column: &str,
    expected_operator: &str,
) {
    match get_filter_expr(where_expr) {
        FilterExpr::Comparison {
            column, operator, ..
        } => {
            assert_eq!(column, expected_column);
            assert_eq!(operator, expected_operator);
        }
        _ => panic!("Expected Comparison"),
    }
}

fn assert_in_expr(where_expr: ormer::query::builder::WhereExpr) {
    match get_filter_expr(where_expr) {
        FilterExpr::In { .. } => {}
        _ => panic!("Expected In"),
    }
}

macro_rules! comparison_operator_test {
    ($test_fn:ident, $ty:ty, $method:ident($($arg:expr),* $(,)?), $expected_operator:literal) => {
        async fn $test_fn(config: &_test_common::DbConfig) {
            let _config = config; // 仅用于获取数据库类型
            let col: TypedColumn<$ty> = TypedColumn::new("test_col");
            let expr = col.$method($($arg),*);

            assert_comparison_operator(expr, $expected_operator);
        }
    };
}

macro_rules! comparison_column_operator_test {
    ($test_fn:ident, $ty:ty, $column:literal, $method:ident($($arg:expr),* $(,)?), $expected_operator:literal) => {
        async fn $test_fn(config: &_test_common::DbConfig) {
            let _config = config;
            let col: TypedColumn<$ty> = TypedColumn::new($column);
            let expr = col.$method($($arg),*);

            assert_comparison_column_operator(expr, $column, $expected_operator);
        }
    };
}

macro_rules! in_expr_test {
    ($test_fn:ident, $ty:ty, { $($setup:tt)* }, $method:ident($($arg:expr),* $(,)?)) => {
        async fn $test_fn(config: &_test_common::DbConfig) {
            let _config = config; // 仅用于获取数据库类型
            let col: TypedColumn<$ty> = TypedColumn::new("test_col");
            $($setup)*
            let expr = col.$method($($arg),*);

            assert_in_expr(expr);
        }
    };
}

// 测试各种整数类型
comparison_operator_test!(test_typed_column_i8_impl, i8, ge(10), ">=");
comparison_operator_test!(test_typed_column_i16_impl, i16, gt(100), ">");
comparison_operator_test!(test_typed_column_u32_impl, u32, le(1000), "<=");
comparison_operator_test!(test_typed_column_u64_impl, u64, lt(10000), "<");
comparison_operator_test!(test_typed_column_usize_impl, usize, eq(42), "=");

// 测试浮点类型
comparison_operator_test!(
    test_typed_column_f32_impl,
    f32,
    ge(std::f32::consts::PI),
    ">="
);
comparison_operator_test!(
    test_typed_column_f64_impl,
    f64,
    le(std::f64::consts::E),
    "<="
);

// 测试字符串类型
comparison_operator_test!(
    test_typed_column_string_impl,
    String,
    eq("hello".to_string()),
    "="
);
comparison_operator_test!(test_typed_column_str_ref_impl, String, eq("world"), "=");

// 测试 IN 语句支持各种类型
in_expr_test!(
    test_is_in_i32_impl,
    i32,
    {
        let values = vec![1, 2, 3];
    },
    is_in(values)
);
in_expr_test!(
    test_is_in_i64_impl,
    i64,
    {
        let values = vec![100i64, 200, 300];
    },
    is_in(values)
);
in_expr_test!(
    test_is_in_string_impl,
    String,
    {
        let values = vec!["a".to_string(), "b".to_string()];
    },
    is_in(values)
);

// 测试 ne() 不等于
comparison_column_operator_test!(test_ne_i32_impl, i32, "status", ne(0), "!=");
comparison_column_operator_test!(test_ne_string_impl, String, "status", ne("deleted"), "!=");

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
test_on_all_dbs!(test_ne_i32_impl);
test_on_all_dbs!(test_ne_string_impl);
