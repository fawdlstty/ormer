use super::super::DbType;
use crate::model::{
    FromRowValues, Model, Row, Value, quote_identifier, quote_qualified_identifier,
};
use crate::query::filter::FilterExpr;
use crate::query::filter_formatter::FilterFormatter;
use std::collections::HashMap;

pub fn placeholder(db_type: DbType, _param_idx: usize) -> String {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => format!("${_param_idx}"),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => format!("@P{_param_idx}"),
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => "?".to_string(),
        #[cfg(feature = "mysql")]
        DbType::MySQL => "?".to_string(),
    }
}

pub fn placeholder_list(db_type: DbType, start_idx: usize, count: usize) -> String {
    (0..count)
        .map(|offset| placeholder(db_type, start_idx + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_table_name<T: Model>(db_type: DbType) -> String {
    quote_qualified_identifier(db_type, T::table_name_for_db(db_type))
}

pub fn quote_column_list(db_type: DbType, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| quote_identifier(db_type, column))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_column_with_prefix(db_type: DbType, prefix: &str, column: &str) -> String {
    format!("{}.{}", prefix, quote_identifier(db_type, column))
}

pub fn quote_assignment(db_type: DbType, column: &str, value_sql: &str) -> String {
    format!("{} = {}", quote_identifier(db_type, column), value_sql)
}

pub fn quote_postgres_excluded_assignment(db_type: DbType, column: &str) -> String {
    quote_assignment(
        db_type,
        column,
        &quote_column_with_prefix(db_type, "EXCLUDED", column),
    )
}

pub fn quote_mysql_values_assignment(db_type: DbType, column: &str) -> String {
    quote_assignment(
        db_type,
        column,
        &format!("VALUES({})", quote_identifier(db_type, column)),
    )
}

pub fn sql_type_with_nullability(base_type: &str, is_nullable: bool) -> String {
    format!("{base_type}{}", if is_nullable { "" } else { " NOT NULL" })
}

/// 通用过滤器格式化函数（不包含参数值，用于 DELETE）
pub fn format_filter(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut i32,
    db_type: DbType,
) -> anyhow::Result<()> {
    let mut params = Vec::new();
    sql.push_str(&FilterFormatter::new(db_type).format(filter, param_idx, &mut params));
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
    let mut next_param_idx = *param_idx as i32;
    sql.push_str(&FilterFormatter::new(db_type).format(filter, &mut next_param_idx, params));
    *param_idx = next_param_idx as usize;
    Ok(())
}

/// 通用行数据提取函数 - 从数据库行中提取模型数据
pub fn extract_model_from_row<T: Model>(row_data: &HashMap<String, Value>) -> anyhow::Result<T> {
    let row = Row::new(row_data.clone());
    T::from_row(&row)
}

pub fn decode_model_from_indexed_values<T, F>(offset: usize, mut value_at: F) -> anyhow::Result<T>
where
    T: Model,
    F: FnMut(usize) -> anyhow::Result<Value>,
{
    let mut data = HashMap::new();
    for (i, col_name) in T::COLUMNS.iter().enumerate() {
        data.insert(col_name.to_string(), value_at(offset + i)?);
    }

    T::from_row(&Row::new(data))
}

pub fn decode_optional_model_from_indexed_values<T, F>(
    offset: usize,
    mut value_at: F,
) -> anyhow::Result<Option<T>>
where
    T: Model,
    F: FnMut(usize) -> anyhow::Result<Value>,
{
    let mut data = HashMap::new();
    let mut is_null = true;
    for (i, col_name) in T::COLUMNS.iter().enumerate() {
        let value = value_at(offset + i)?;
        if !matches!(value, Value::Null) {
            is_null = false;
        }
        data.insert(col_name.to_string(), value);
    }

    if is_null {
        Ok(None)
    } else {
        Ok(Some(T::from_row(&Row::new(data))?))
    }
}

pub fn decode_row_values_from_indexed_values<V, F>(
    column_count: usize,
    mut value_at: F,
) -> anyhow::Result<V>
where
    V: FromRowValues,
    F: FnMut(usize) -> anyhow::Result<Value>,
{
    let mut values = Vec::with_capacity(column_count);
    for i in 0..column_count {
        values.push(value_at(i)?);
    }

    V::from_row_values(&values)
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
                Ok(Value::DateTime(get_datetime().unwrap_or({
                    chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
                })))
            }
            _ => Err(anyhow::anyhow!("Unsupported column type: {rust_type}")),
        }
    }
}

/// 将 model::Value 转换为 filter::Value
pub fn value_to_filter_value(val: &Value) -> crate::query::filter::Value {
    val.clone().into()
}

fn downcast_auto_increment_key<K: 'static, T: 'static>(value: T) -> K {
    let boxed: Box<dyn std::any::Any> = Box::new(value);
    match boxed.downcast::<K>() {
        Ok(value) => *value,
        Err(_) => unreachable!("auto-increment key type checked before downcast"),
    }
}

/// 将数据库返回的自增 ID 转换为模型指定的 AutoIncrementKeyType。
pub fn convert_auto_increment_key<K: Default + 'static>(
    last_id: impl Into<i128>,
) -> anyhow::Result<K> {
    let last_id = last_id.into();
    let key_type = std::any::TypeId::of::<K>();
    if key_type == std::any::TypeId::of::<()>() {
        Ok(downcast_auto_increment_key(()))
    } else if key_type == std::any::TypeId::of::<i32>() {
        Ok(downcast_auto_increment_key(last_id as i32))
    } else if key_type == std::any::TypeId::of::<i64>() {
        Ok(downcast_auto_increment_key(last_id as i64))
    } else if key_type == std::any::TypeId::of::<u32>() {
        Ok(downcast_auto_increment_key(last_id as u32))
    } else if key_type == std::any::TypeId::of::<u64>() {
        Ok(downcast_auto_increment_key(last_id as u64))
    } else if key_type == std::any::TypeId::of::<usize>() {
        Ok(downcast_auto_increment_key(last_id as usize))
    } else if key_type == std::any::TypeId::of::<Option<i64>>() {
        Ok(downcast_auto_increment_key(Some(last_id as i64)))
    } else {
        Err(anyhow::anyhow!(
            "Unsupported auto-increment key type. Only i32, i64, u32, u64, usize, Option<i64> and () are supported."
        ))
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

fn build_batch_insert_sql_with_prefix(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let table_name = quote_qualified_identifier(db_type, table_name);
    let columns_str = quote_column_list(db_type, columns);
    let col_count = columns.len();

    let mut sql = format!("{insert_prefix} {table_name} ({columns_str}) VALUES ");

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let start_idx = idx * col_count + 1;
        sql.push_str(&format!(
            "({})",
            placeholder_list(db_type, start_idx, col_count)
        ));
    }

    (sql, col_count)
}

/// 构建批量插入 SQL 的公共函数（使用自定义列名列表，排除自增主键）
pub fn build_batch_insert_sql_with_columns(
    db_type: DbType,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    build_batch_insert_sql_with_prefix(db_type, "INSERT INTO", table_name, columns, models_count)
}

#[derive(Debug, Clone, Copy)]
pub enum BatchInsertValuesMode {
    All,
    WithoutAutoIncrement,
}

/// 构建批量 INSERT 主体并收集对应参数，冲突子句由各后端追加。
pub fn build_batch_insert_statement<T: Model>(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models: &[&T],
    values_mode: BatchInsertValuesMode,
) -> (String, Vec<Value>) {
    let (sql, _) = build_batch_insert_sql_with_prefix(
        db_type,
        insert_prefix,
        table_name,
        columns,
        models.len(),
    );
    let values = match values_mode {
        BatchInsertValuesMode::All => collect_batch_insert_values(models),
        BatchInsertValuesMode::WithoutAutoIncrement => {
            collect_batch_insert_values_with_auto_increment(models)
        }
    };
    (sql, values)
}

pub fn auto_increment_column<T: Model>() -> Option<&'static str> {
    T::COLUMN_SCHEMA
        .iter()
        .find(|column| column.is_auto_increment)
        .map(|column| column.name)
}

pub fn append_auto_increment_returning<T: Model>(db_type: DbType, sql: String) -> String {
    let Some(_pk_col) = auto_increment_column::<T>() else {
        return sql;
    };

    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => format!("{sql} RETURNING rowid"),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let pk_col = quote_column_list(DbType::PostgreSQL, &[_pk_col]);
            format!("{sql} RETURNING {pk_col}")
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => sql,
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            format!(
                "{} OUTPUT {}",
                sql,
                quote_column_with_prefix(DbType::MSSQL, "inserted", _pk_col)
            )
        }
    }
}

pub fn build_insert_statement<T: Model>(db_type: DbType, models: &[&T]) -> (String, Vec<Value>) {
    let columns = T::insert_columns();
    build_batch_insert_statement::<T>(
        db_type,
        "INSERT INTO",
        T::table_name_for_db(db_type),
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    )
}

pub fn build_insert_statement_with_auto_increment_returning<T: Model>(
    db_type: DbType,
    models: &[&T],
) -> (String, Vec<Value>) {
    let (sql, values) = build_insert_statement::<T>(db_type, models);
    (append_auto_increment_returning::<T>(db_type, sql), values)
}

#[cfg(feature = "mssql")]
pub fn build_mssql_merge_source<T: Model>(models: &[&T]) -> (String, Vec<Value>) {
    let columns = quote_column_list(DbType::MSSQL, T::COLUMNS);
    let col_count = T::COLUMNS.len();
    let mut sql = format!(
        "MERGE INTO {} AS target USING (VALUES ",
        quote_table_name::<T>(DbType::MSSQL)
    );
    let mut all_values = Vec::new();

    for (idx, model) in models.iter().enumerate() {
        if idx > 0 {
            sql.push_str(", ");
        }
        let placeholders = placeholder_list(DbType::MSSQL, all_values.len() + 1, col_count);
        sql.push_str(&format!("({placeholders})"));
        all_values.extend(model.field_values());
    }

    sql.push_str(&format!(") AS source ({columns}) ON "));
    for (i, pk) in T::primary_key_columns().iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_column_with_prefix(DbType::MSSQL, "target", pk),
            quote_column_with_prefix(DbType::MSSQL, "source", pk)
        ));
    }

    (sql, all_values)
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_update_clause<T: Model>(sql: &mut String) {
    sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
    let pks = T::primary_key_columns();
    let mut first = true;
    for col_name in T::COLUMNS.iter() {
        if pks.contains(col_name) {
            continue;
        }
        if !first {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            DbType::MSSQL,
            col_name,
            &quote_column_with_prefix(DbType::MSSQL, "source", col_name),
        ));
        first = false;
    }
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_insert_clause<T: Model>(sql: &mut String) {
    let columns = quote_column_list(DbType::MSSQL, T::COLUMNS);
    sql.push_str(&format!(
        " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
    ));
    for (i, col_name) in T::COLUMNS.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_column_with_prefix(DbType::MSSQL, "source", col_name));
    }
    sql.push_str(");");
}

/// 构建批量插入 SQL（MSSQL 使用 @P1, @P2 占位符）
pub fn build_batch_insert_sql_mssql_with_columns(
    db_type: DbType,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    build_batch_insert_sql_with_columns(db_type, table_name, columns, models_count)
}

/// 构建批量插入 SQL（PostgreSQL 使用 $1, $2 占位符）
#[cfg(feature = "postgresql")]
pub fn build_batch_insert_sql_postgresql<T: Model>(models_count: usize) -> (String, usize) {
    build_batch_insert_sql_with_columns(
        DbType::PostgreSQL,
        T::table_name_for_db(DbType::PostgreSQL),
        T::COLUMNS,
        models_count,
    )
}

/// 构建批量插入 SQL（PostgreSQL 使用 $1, $2 占位符，使用自定义列名列表）
pub fn build_batch_insert_sql_postgresql_with_columns(
    db_type: DbType,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    build_batch_insert_sql_with_columns(db_type, table_name, columns, models_count)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_insert_builder_uses_backend_placeholder_rules() {
        let columns = &["id", "name", "age"];

        #[cfg(feature = "sqlite")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::Sqlite, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "mysql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MySQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "postgresql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::PostgreSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES ($1, $2, $3), ($4, $5, $6)"
            );
        }

        #[cfg(feature = "mssql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MSSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (@P1, @P2, @P3), (@P4, @P5, @P6)"
            );
        }
    }
}
