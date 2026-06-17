use super::super::DbType;
use crate::model::{Model, Row, Value};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::fmt::Write;

/// 通用过滤器格式化函数（不包含参数值，用于 DELETE）
pub fn format_filter(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut i32,
    db_type: DbType,
) -> anyhow::Result<()> {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value: _,
        } => {
            match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => {
                    write!(sql, "{column} {operator} ${param_idx}")?;
                }
                #[cfg(feature = "sqlite")]
                DbType::Sqlite => {
                    write!(sql, "{column} {operator} ?")?;
                }
                #[cfg(feature = "mysql")]
                DbType::MySQL => {
                    write!(sql, "{column} {operator} ?")?;
                }
                #[cfg(feature = "mssql")]
                DbType::MSSQL => {
                    write!(sql, "{column} {operator} @P")?;
                }
            }
            *param_idx += 1;
        }
        FilterExpr::ColumnComparison {
            left_column,
            operator,
            right_column,
        } => {
            write!(sql, "{left_column} {operator} {right_column}")?;
        }
        FilterExpr::In { column, values } => {
            // 生成 IN 语句: column IN (?, ?, ...)
            write!(sql, "{} IN (", column)?;
            for (i, _) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        write!(sql, "${param_idx}")?;
                    }
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        sql.push_str("@P");
                    }
                }
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::NotIn { column, values } => {
            // 生成 NOT IN 语句: column NOT IN (?, ?, ...)
            write!(sql, "{} NOT IN (", column)?;
            for (i, _) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        write!(sql, "${param_idx}")?;
                    }
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        sql.push_str("@P");
                    }
                }
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::InSubquery {
            column,
            subquery_sql,
            subquery_params: _,
        } => {
            // 生成子查询 IN 语句: column IN (SELECT ...)
            write!(sql, "{column} IN ({subquery_sql})")?;
            // 注意：子查询的参数数量需要累加到 param_idx
            // 但在这个函数中我们不处理参数值，只处理占位符
            // 子查询的 SQL 中已经包含了占位符，我们需要计算占位符数量
            let placeholder_count = subquery_sql.matches('?').count()
                + subquery_sql.matches('$').count()
                + subquery_sql.matches("@P").count();
            *param_idx += placeholder_count as i32;
        }
        FilterExpr::NotInSubquery {
            column,
            subquery_sql,
            subquery_params: _,
        } => {
            // 生成子查询 NOT IN 语句: column NOT IN (SELECT ...)
            write!(sql, "{column} NOT IN ({subquery_sql})")?;
            // 注意：子查询的参数数量需要累加到 param_idx
            let placeholder_count = subquery_sql.matches('?').count()
                + subquery_sql.matches('$').count()
                + subquery_sql.matches("@P").count();
            *param_idx += placeholder_count as i32;
        }
        FilterExpr::And(left, right) => {
            format_filter(left, sql, param_idx, db_type)?;
            sql.push_str(" AND ");
            format_filter(right, sql, param_idx, db_type)?;
        }
        FilterExpr::Or(left, right) => {
            format_filter(left, sql, param_idx, db_type)?;
            sql.push_str(" OR ");
            format_filter(right, sql, param_idx, db_type)?;
        }
        FilterExpr::IsNull { column } => {
            write!(sql, "{column} IS NULL")?;
        }
        FilterExpr::IsNotNull { column } => {
            write!(sql, "{column} IS NOT NULL")?;
        }
        FilterExpr::Between {
            column,
            min: _,
            max: _,
        } => {
            match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => {
                    write!(sql, "{column} BETWEEN ${param_idx} AND ${}", *param_idx + 1)?;
                }
                #[cfg(feature = "sqlite")]
                DbType::Sqlite => {
                    write!(sql, "{column} BETWEEN ? AND ?")?;
                }
                #[cfg(feature = "mysql")]
                DbType::MySQL => {
                    write!(sql, "{column} BETWEEN ? AND ?")?;
                }
                #[cfg(feature = "mssql")]
                DbType::MSSQL => {
                    write!(sql, "{column} BETWEEN @P AND @P")?;
                }
            }
            *param_idx += 2;
        }
        FilterExpr::Exists {
            subquery_sql,
            subquery_params: _,
        } => {
            write!(sql, "EXISTS ({subquery_sql})")?;
            let placeholder_count = subquery_sql.matches('?').count()
                + subquery_sql.matches('$').count()
                + subquery_sql.matches("@P").count();
            *param_idx += placeholder_count as i32;
        }
        FilterExpr::NotExists {
            subquery_sql,
            subquery_params: _,
        } => {
            write!(sql, "NOT EXISTS ({subquery_sql})")?;
            let placeholder_count = subquery_sql.matches('?').count()
                + subquery_sql.matches('$').count()
                + subquery_sql.matches("@P").count();
            *param_idx += placeholder_count as i32;
        }
    }
    Ok(())
}

/// 通用过滤器格式化函数并收集参数（用于 UPDATE/SELECT）
pub fn format_filter_with_params(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut usize,
    params: &mut Vec<Value>,
    db_type: DbType,
) -> anyhow::Result<()> {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => {
                    write!(sql, "{column} {operator} ${param_idx}")?;
                }
                #[cfg(feature = "sqlite")]
                DbType::Sqlite => {
                    write!(sql, "{column} {operator} ?")?;
                }
                #[cfg(feature = "mysql")]
                DbType::MySQL => {
                    write!(sql, "{column} {operator} ?")?;
                }
                #[cfg(feature = "mssql")]
                DbType::MSSQL => {
                    write!(sql, "{column} {operator} @P")?;
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
            write!(sql, "{} {} {}", left_column, operator, right_column)?;
        }
        FilterExpr::In { column, values } => {
            // 生成 IN 语句: column IN (?, ?, ...)
            write!(sql, "{} IN (", column)?;
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        sql.push('?');
                    }
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        write!(sql, "${}", param_idx)?;
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        sql.push_str("@P");
                    }
                }
                params.push(value.clone().into());
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::NotIn { column, values } => {
            // 生成 NOT IN 语句: column NOT IN (?, ?, ...)
            write!(sql, "{} NOT IN (", column)?;
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                match db_type {
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => {
                        sql.push('?');
                    }
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        write!(sql, "${}", param_idx)?;
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        sql.push('?');
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        sql.push_str("@P");
                    }
                }
                params.push(value.clone().into());
                *param_idx += 1;
            }
            sql.push(')');
        }
        FilterExpr::InSubquery {
            column,
            subquery_sql,
            subquery_params,
        } => {
            // 生成子查询 IN 语句: column IN (SELECT ...)
            write!(sql, "{} IN ({})", column, subquery_sql)?;
            // 添加子查询的参数
            for param in subquery_params {
                params.push(param.clone());
                *param_idx += 1;
            }
        }
        FilterExpr::NotInSubquery {
            column,
            subquery_sql,
            subquery_params,
        } => {
            // 生成子查询 NOT IN 语句: column NOT IN (SELECT ...)
            write!(sql, "{} NOT IN ({})", column, subquery_sql)?;
            // 添加子查询的参数
            for param in subquery_params {
                params.push(param.clone());
                *param_idx += 1;
            }
        }
        FilterExpr::And(left, right) => {
            format_filter_with_params(left, sql, param_idx, params, db_type)?;
            sql.push_str(" AND ");
            format_filter_with_params(right, sql, param_idx, params, db_type)?;
        }
        FilterExpr::Or(left, right) => {
            format_filter_with_params(left, sql, param_idx, params, db_type)?;
            sql.push_str(" OR ");
            format_filter_with_params(right, sql, param_idx, params, db_type)?;
        }
        FilterExpr::IsNull { column } => {
            write!(sql, "{column} IS NULL")?;
        }
        FilterExpr::IsNotNull { column } => {
            write!(sql, "{column} IS NOT NULL")?;
        }
        FilterExpr::Between { column, min, max } => {
            match db_type {
                #[cfg(feature = "postgresql")]
                DbType::PostgreSQL => {
                    write!(sql, "{column} BETWEEN ${param_idx} AND ${}", *param_idx + 1)?;
                }
                #[cfg(feature = "sqlite")]
                DbType::Sqlite => {
                    write!(sql, "{column} BETWEEN ? AND ?")?;
                }
                #[cfg(feature = "mysql")]
                DbType::MySQL => {
                    write!(sql, "{column} BETWEEN ? AND ?")?;
                }
                #[cfg(feature = "mssql")]
                DbType::MSSQL => {
                    write!(sql, "{column} BETWEEN @P AND @P")?;
                }
            }
            params.push(min.clone().into());
            params.push(max.clone().into());
            *param_idx += 2;
        }
        FilterExpr::Exists {
            subquery_sql,
            subquery_params,
        } => {
            write!(sql, "EXISTS ({})", subquery_sql)?;
            for param in subquery_params {
                params.push(param.clone());
                *param_idx += 1;
            }
        }
        FilterExpr::NotExists {
            subquery_sql,
            subquery_params,
        } => {
            write!(sql, "NOT EXISTS ({})", subquery_sql)?;
            for param in subquery_params {
                params.push(param.clone());
                *param_idx += 1;
            }
        }
    }
    Ok(())
}

/// 通用行数据提取函数 - 从数据库行中提取模型数据
pub fn extract_model_from_row<T: Model>(row_data: &HashMap<String, Value>) -> anyhow::Result<T> {
    let row = Row::new(row_data.clone());
    T::from_row(&row)
}

/// 通用列值转换助手 - 根据 rust_type 转换数据库值到 ormer Value
#[allow(clippy::too_many_arguments)]
pub fn convert_column_value(
    rust_type: &str,
    is_nullable: bool,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<Value> {
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
                Some(1) => Ok(Value::Boolean(true)),
                Some(0) => Ok(Value::Boolean(false)),
                _ => Ok(Value::Null),
            },
            "Vec<u8>" | "&[u8]" => match get_bytes() {
                Some(val) => Ok(Value::Bytes(val)),
                None => Ok(Value::Null),
            },
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                match get_datetime() {
                    Some(val) => Ok(Value::DateTime(val)),
                    None => Ok(Value::Null),
                }
            }
            _ => Err(anyhow::anyhow!(
                "Unsupported nullable column type: {rust_type}"
            )),
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
                Ok(Value::Boolean(v == 1))
            }
            "Vec<u8>" | "&[u8]" => Ok(Value::Bytes(get_bytes().unwrap_or_default())),
            "Duration" | "std::time::Duration" => Ok(Value::Duration(
                std::time::Duration::from_micros(get_int().unwrap_or(0).max(0) as u64),
            )),
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                Ok(Value::DateTime(get_datetime().unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap()
                })))
            }
            _ => Err(anyhow::anyhow!("Unsupported column type: {rust_type}")),
        }
    }
}

/// 将 model::Value 转换为 filter::Value
pub fn value_to_filter_value(val: &Value) -> crate::query::filter::Value {
    match val {
        Value::Integer(v) => crate::query::filter::Value::Integer(*v),
        Value::BigInt(v) => crate::query::filter::Value::BigInt(*v),
        Value::Duration(v) => crate::query::filter::Value::Duration(*v),
        Value::Text(v) => crate::query::filter::Value::Text(v.clone()),
        Value::Real(v) => crate::query::filter::Value::Real(*v),
        Value::Boolean(v) => crate::query::filter::Value::Boolean(*v),
        Value::Bytes(v) => crate::query::filter::Value::Bytes(v.clone()),
        Value::DateTime(v) => crate::query::filter::Value::DateTime(*v),
        Value::Json(v) => crate::query::filter::Value::Json(v.clone()),
        Value::Uuid(v) => crate::query::filter::Value::Uuid(*v),
        Value::Null => crate::query::filter::Value::Null,
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

/// 构建批量插入 SQL 的公共函数（使用自定义列名列表，排除自增主键）
pub fn build_batch_insert_sql_with_columns(
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let columns_str = columns.join(", ");
    let col_count = columns.len();

    let mut sql = format!("INSERT INTO {table_name} ({columns_str}) VALUES ");

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
        sql.push_str(&format!("({})", placeholders.join(", ")));
    }

    (sql, col_count)
}

/// 构建批量插入 SQL（MSSQL 使用 @P 占位符）
pub fn build_batch_insert_sql_mssql_with_columns(
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let columns_str = columns.join(", ");
    let col_count = columns.len();

    let mut sql = format!("INSERT INTO {table_name} ({columns_str}) VALUES ");

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
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

/// 构建批量插入 SQL（PostgreSQL 使用 $1, $2 占位符，使用自定义列名列表）
pub fn build_batch_insert_sql_postgresql_with_columns(
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let columns_str = columns.join(", ");
    let col_count = columns.len();

    let mut sql = format!("INSERT INTO {table_name} ({columns_str}) VALUES ");
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

/// 收集批量插入的所有模型值（排除自增主键）
pub fn collect_batch_insert_values_with_auto_increment<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.insert_values();
        all_values.extend(values);
    }
    all_values
}

/// 统一的列值解析函数 - 严格模式
///
/// 用于流式查询中解析列值,非空字段解析失败时返回错误而非默认值
#[allow(clippy::too_many_arguments)]
pub fn parse_column_value_strict(
    rust_type: &str,
    is_nullable: bool,
    column_name: &str,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<Value> {
    if is_nullable {
        // 可空字段:允许 None
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
                Some(1) => Ok(Value::Boolean(true)),
                Some(0) => Ok(Value::Boolean(false)),
                _ => Ok(Value::Null),
            },
            "Vec<u8>" | "&[u8]" => match get_bytes() {
                Some(val) => Ok(Value::Bytes(val)),
                None => Ok(Value::Null),
            },
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                match get_datetime() {
                    Some(val) => Ok(Value::DateTime(val)),
                    None => Ok(Value::Null),
                }
            }
            _ => Err(anyhow::anyhow!(
                "Unsupported nullable column type: {rust_type}"
            )),
        }
    } else {
        // 非空字段:解析失败时返回错误
        match rust_type {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => match get_int() {
                Some(val) => Ok(Value::Integer(val)),
                None => Err(anyhow::anyhow!(
                    "Failed to parse non-nullable column '{}' (expected integer type)",
                    column_name
                )),
            },
            "String" => match get_string() {
                Some(val) => Ok(Value::Text(val)),
                None => Err(anyhow::anyhow!(
                    "Failed to parse non-nullable column '{}' (expected String type)",
                    column_name
                )),
            },
            "f32" | "f64" => match get_real() {
                Some(val) => Ok(Value::Real(val)),
                None => Err(anyhow::anyhow!(
                    "Failed to parse non-nullable column '{}' (expected float type)",
                    column_name
                )),
            },
            "bool" => match get_bool() {
                Some(v) => Ok(Value::Boolean(v == 1)),
                None => Err(anyhow::anyhow!(
                    "Failed to parse non-nullable column '{}' (expected bool type)",
                    column_name
                )),
            },
            "Vec<u8>" | "&[u8]" => match get_bytes() {
                Some(val) => Ok(Value::Bytes(val)),
                None => Err(anyhow::anyhow!(
                    "Failed to parse non-nullable column '{}' (expected Vec<u8> type)",
                    column_name
                )),
            },
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                match get_datetime() {
                    Some(val) => Ok(Value::DateTime(val)),
                    None => Err(anyhow::anyhow!(
                        "Failed to parse non-nullable column '{}' (expected DateTime type)",
                        column_name
                    )),
                }
            }
            _ => Err(anyhow::anyhow!("Unsupported column type: {rust_type}")),
        }
    }
}
