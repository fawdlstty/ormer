#![cfg(feature = "sqlite")]

use ormer::query::filter_formatter::FilterFormatter;
use ormer::{DbType, FilterExpr, Value};

fn comparison(column: &str, value: i64) -> FilterExpr {
    FilterExpr::Comparison {
        column: column.to_string(),
        operator: "=".to_string(),
        value: Value::Integer(value),
    }
}

#[test]
fn and_or_groups_keep_ast_precedence() {
    let filter = FilterExpr::And(
        Box::new(FilterExpr::Or(
            Box::new(comparison("a", 1)),
            Box::new(comparison("b", 2)),
        )),
        Box::new(comparison("c", 3)),
    );
    let mut param_idx = 1;
    let mut params = Vec::new();

    let sql = FilterFormatter::new(DbType::Sqlite).format(&filter, &mut param_idx, &mut params);

    assert_eq!(sql, "((a = ? OR b = ?) AND c = ?)");
    assert_eq!(params.len(), 3);
    assert_eq!(param_idx, 4);
}

#[test]
fn nested_or_inside_and_is_parenthesized() {
    let filter = FilterExpr::Or(
        Box::new(comparison("a", 1)),
        Box::new(FilterExpr::And(
            Box::new(comparison("b", 2)),
            Box::new(comparison("c", 3)),
        )),
    );
    let mut param_idx = 1;
    let mut params = Vec::new();

    let sql = FilterFormatter::new(DbType::Sqlite).format(&filter, &mut param_idx, &mut params);

    assert_eq!(sql, "(a = ? OR (b = ? AND c = ?))");
    assert_eq!(params.len(), 3);
}
