use crate::abstract_layer::DbType;
use crate::model::{Model, Row, Value};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::fmt::Write;

/// 通用过滤器格式化函数（不包含参数值，用于 DELETE）
pub fn format_filter(filter: &FilterExpr, sql: &mut String, param_idx: &mut i32, db_type: DbType) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value: _,
        } => {
            match db_type {
                DbType::PostgreSQL => {
                    write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                }
                DbType::Turso | DbType::MySQL => {
                    write!(sql, "{} {} ?", column, operator).unwrap();
                }
            }
            *param_idx += 1;
        }
        FilterExpr::ColumnComparison {
            left_column,
            operator,
            right_column,
        } => {
            write!(sql, "{} {} {}", left_column, operator, right_column).unwrap();
        }
        FilterExpr::In { column, values } => {
            // 生成 IN 语句: column IN (?, ?, ...)
            write!(sql, "{} IN (", column).unwrap();
            for (i, _) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "${}", param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        sql.push('?');
                    }
                }
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::And(left, right) => {
            format_filter(left, sql, param_idx, db_type);
            sql.push_str(" AND ");
            format_filter(right, sql, param_idx, db_type);
        }
        FilterExpr::Or(left, right) => {
            format_filter(left, sql, param_idx, db_type);
            sql.push_str(" OR ");
            format_filter(right, sql, param_idx, db_type);
        }
    }
}

/// 通用过滤器格式化函数并收集参数（用于 UPDATE/SELECT）
pub fn format_filter_with_params(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut usize,
    params: &mut Vec<Value>,
    db_type: DbType,
) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            match db_type {
                DbType::PostgreSQL => {
                    write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
                }
                DbType::Turso | DbType::MySQL => {
                    write!(sql, "{} {} ?", column, operator).unwrap();
                }
            }
            params.push(value.clone().into());
            *param_idx += 1;
        }
        FilterExpr::ColumnComparison {
            left_column,
            operator,
            right_column,
        } => {
            write!(sql, "{} {} {}", left_column, operator, right_column).unwrap();
        }
        FilterExpr::In { column, values } => {
            // 生成 IN 语句: column IN (?, ?, ...)
            write!(sql, "{} IN (", column).unwrap();
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    DbType::PostgreSQL => {
                        write!(sql, "${}", param_idx).unwrap();
                    }
                    DbType::Turso | DbType::MySQL => {
                        sql.push('?');
                    }
                }
                params.push(value.clone().into());
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::And(left, right) => {
            format_filter_with_params(left, sql, param_idx, params, db_type);
            sql.push_str(" AND ");
            format_filter_with_params(right, sql, param_idx, params, db_type);
        }
        FilterExpr::Or(left, right) => {
            format_filter_with_params(left, sql, param_idx, params, db_type);
            sql.push_str(" OR ");
            format_filter_with_params(right, sql, param_idx, params, db_type);
        }
    }
}

/// 通用行数据提取函数 - 从数据库行中提取模型数据
pub fn extract_model_from_row<T: Model>(
    row_data: &HashMap<String, Value>,
) -> Result<T, crate::Error> {
    let row = Row::new(row_data.clone());
    T::from_row(&row)
}

/// 通用列值转换助手 - 根据 rust_type 转换数据库值到 ormer Value
pub fn convert_column_value(
    rust_type: &str,
    is_nullable: bool,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
) -> Result<Value, crate::Error> {
    if is_nullable {
        match rust_type {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => match get_int() {
                Some(val) => Ok(Value::Integer(val)),
                None => Ok(Value::Null),
            },
            "String" => match get_string() {
                Some(val) => Ok(Value::Text(val)),
                None => Ok(Value::Null),
            },
            "f32" | "f64" => match get_real() {
                Some(val) => Ok(Value::Real(val)),
                None => Ok(Value::Null),
            },
            "bool" => match get_bool() {
                Some(1) => Ok(Value::Integer(1)),
                Some(0) => Ok(Value::Integer(0)),
                _ => Ok(Value::Null),
            },
            _ => Err(crate::Error::Database(format!(
                "Unsupported nullable column type: {rust_type}"
            ))),
        }
    } else {
        match rust_type {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                Ok(Value::Integer(get_int().unwrap_or(0)))
            }
            "String" => Ok(Value::Text(get_string().unwrap_or_default())),
            "f32" | "f64" => Ok(Value::Real(get_real().unwrap_or(0.0))),
            "bool" => {
                let v = get_bool().unwrap_or(0);
                Ok(Value::Integer(if v == 1 { 1 } else { 0 }))
            }
            _ => Err(crate::Error::Database(format!(
                "Unsupported column type: {rust_type}"
            ))),
        }
    }
}

/// 构建批量插入 SQL 的公共函数（使用 ? 占位符）
pub fn build_batch_insert_sql<T: Model>(models_count: usize) -> (String, usize) {
    let columns = T::COLUMNS.join(", ");
    let col_count = T::COLUMNS.len();

    let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
        sql.push_str(&format!("({})", placeholders.join(", ")));
    }

    (sql, col_count)
}

/// 构建批量插入 SQL（PostgreSQL 使用 $1, $2 占位符）
pub fn build_batch_insert_sql_postgresql<T: Model>(models_count: usize) -> (String, usize) {
    let columns = T::COLUMNS.join(", ");
    let col_count = T::COLUMNS.len();

    let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
    let mut param_idx = 1;

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let placeholders: Vec<String> = (1..=col_count)
            .map(|i| {
                let idx = param_idx + i - 1;
                format!("${}", idx)
            })
            .collect();
        sql.push_str(&format!("({})", placeholders.join(", ")));
        param_idx += col_count;
    }

    (sql, col_count)
}

/// 收集批量插入的所有模型值
pub fn collect_batch_insert_values<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.field_values();
        all_values.extend(values);
    }
    all_values
}
