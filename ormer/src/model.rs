use std::collections::HashMap;

/// 字段元数据
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub name: &'static str,
    pub rust_type: &'static str,
    pub is_primary: bool,
    pub is_nullable: bool,
}

/// 数据库后端 trait - 用于 SQL 类型映射
pub trait DbBackendTypeMapper {
    /// 根据 Rust 类型获取 SQL 类型
    fn sql_type(rust_type: &str, is_primary: bool, is_nullable: bool) -> String;
}

/// 模型 trait,所有 ORM 模型必须实现
pub trait Model: Sized {
    const TABLE_NAME: &'static str;
    const COLUMNS: &'static [&'static str];
    const COLUMN_SCHEMA: &'static [ColumnSchema];

    type QueryBuilder;
    type Where: Default;

    fn query() -> Self::QueryBuilder;
    fn select() -> Self::QueryBuilder;
    fn from_row(row: &Row) -> Result<Self, Error>;

    /// 获取字段值 (用于 INSERT/UPDATE)
    fn field_values(&self) -> Vec<Value>;

    /// 获取主键字段名 (用于 UPDATE/DELETE)
    fn primary_key_column() -> &'static str;

    /// 获取主键值
    fn primary_key_value(&self) -> Value;
}

/// 运行时动态生成 CREATE TABLE SQL
pub fn generate_create_table_sql<T: Model>(db_type: crate::abstract_layer::DbType) -> String {
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (", T::TABLE_NAME);

    for (i, column) in T::COLUMN_SCHEMA.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }

        let sql_type = db_type.sql_type(column.rust_type, column.is_primary, column.is_nullable);

        sql.push_str(&format!("{} {}", column.name, sql_type));
    }

    sql.push(')');
    sql
}

/// 数据库行抽象
#[derive(Debug)]
pub struct Row {
    data: HashMap<String, Value>,
}

impl Row {
    pub fn new(data: HashMap<String, Value>) -> Self {
        Self { data }
    }

    pub fn get<T: FromValue>(&self, column: &str) -> Result<T, Error> {
        self.data
            .get(column)
            .ok_or_else(|| Error::ColumnNotFound(column.to_string()))
            .and_then(|v| T::from_value(v))
    }
}

/// 值类型
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    Text(String),
    Real(f64),
    Null,
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, Error>;
}

impl FromValue for i32 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Integer(v) => Ok(*v as i32),
            _ => Err(Error::TypeMismatch("i32".to_string())),
        }
    }
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Integer(v) => Ok(*v),
            _ => Err(Error::TypeMismatch("i64".to_string())),
        }
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            _ => Err(Error::TypeMismatch("String".to_string())),
        }
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Integer(v) => Ok(*v != 0),
            _ => Err(Error::TypeMismatch("bool".to_string())),
        }
    }
}

impl FromValue for Option<i32> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Integer(v) => Ok(Some(*v as i32)),
            _ => Err(Error::TypeMismatch("Option<i32>".to_string())),
        }
    }
}

impl FromValue for Option<i64> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Integer(v) => Ok(Some(*v)),
            _ => Err(Error::TypeMismatch("Option<i64>".to_string())),
        }
    }
}

impl FromValue for Option<String> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Text(v) => Ok(Some(v.clone())),
            _ => Err(Error::TypeMismatch("Option<String>".to_string())),
        }
    }
}

impl FromValue for Option<bool> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Integer(v) => Ok(Some(*v != 0)),
            _ => Err(Error::TypeMismatch("Option<bool>".to_string())),
        }
    }
}

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Type mismatch: expected {0}")]
    TypeMismatch(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Table schema mismatch for table '{table}': {reason}")]
    SchemaMismatch { table: String, reason: String },
}

/// 实现 Value 的 From trait
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        if v {
            Value::Integer(1)
        } else {
            Value::Integer(0)
        }
    }
}

impl From<Option<String>> for Value {
    fn from(v: Option<String>) -> Self {
        match v {
            Some(s) => Value::Text(s),
            None => Value::Null,
        }
    }
}

impl From<Option<i32>> for Value {
    fn from(v: Option<i32>) -> Self {
        match v {
            Some(n) => Value::Integer(n as i64),
            None => Value::Null,
        }
    }
}

impl From<Option<i64>> for Value {
    fn from(v: Option<i64>) -> Self {
        match v {
            Some(n) => Value::Integer(n),
            None => Value::Null,
        }
    }
}

impl From<Option<bool>> for Value {
    fn from(v: Option<bool>) -> Self {
        match v {
            Some(true) => Value::Integer(1),
            Some(false) => Value::Integer(0),
            None => Value::Null,
        }
    }
}

// 为 FilterValue 实现 Into<Value>
impl From<crate::query::filter::Value> for Value {
    fn from(value: crate::query::filter::Value) -> Self {
        match value {
            crate::query::filter::Value::Integer(v) => Value::Integer(v),
            crate::query::filter::Value::Text(v) => Value::Text(v),
            crate::query::filter::Value::Real(v) => Value::Real(v),
            crate::query::filter::Value::Null => Value::Null,
        }
    }
}
