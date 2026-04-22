use std::collections::HashMap;

/// 字段元数据
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub name: &'static str,
    pub rust_type: &'static str,
    pub is_primary: bool,
    pub is_auto_increment: bool,
    pub is_nullable: bool,
    pub unique_group: Option<i32>, // None表示不唯一，Some(group_id)表示属于哪个唯一键组
    pub is_indexed: bool,
    pub foreign_key: Option<ForeignKeyInfo>, // 外键信息
}

/// 外键信息
#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub ref_table: &'static str,  // 引用的表名
    pub ref_column: &'static str, // 引用的列名
}

/// 数据库后端 trait - 用于 SQL 类型映射
pub trait DbBackendTypeMapper {
    /// 根据 Rust 类型获取 SQL 类型
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
    ) -> String;
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

/// 用于 insert/insert_or_update 的参数类型 trait
pub trait Insertable {
    type Model: crate::model::Model;
    fn as_refs(&self) -> Vec<&Self::Model>;
}

impl<T: crate::model::Model> Insertable for &T {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        vec![*self]
    }
}

impl<T: crate::model::Model> Insertable for Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
}

impl<T: crate::model::Model> Insertable for &Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
}

impl<T: crate::model::Model> Insertable for &[T] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
}

impl<T: crate::model::Model, const N: usize> Insertable for &[T; N] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
}

/// 为具体的 Model 类型生成引用的集合类型的 Insertable 实现
/// 这个宏用于解决 orphan rule 问题
#[macro_export]
macro_rules! impl_insertable_for_ref_collections {
    ($model_type:ty) => {
        impl Insertable for Vec<&$model_type> {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.as_slice().to_vec()
            }
        }

        impl Insertable for &Vec<&$model_type> {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.as_slice().to_vec()
            }
        }

        impl<const N: usize> Insertable for &[&$model_type; N] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
        }

        impl Insertable for &[&$model_type] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
        }
    };
}

/// 运行时动态生成 CREATE TABLE SQL
pub fn generate_create_table_sql<T: Model>(db_type: crate::abstract_layer::DbType) -> String {
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (", T::TABLE_NAME);

    for (i, column) in T::COLUMN_SCHEMA.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }

        let sql_type = db_type.sql_type(
            column.rust_type,
            column.is_primary,
            column.is_auto_increment,
            column.is_nullable,
        );

        sql.push_str(&format!("{} {sql_type}", column.name));

        // 添加单列 UNIQUE 约束（group 中只有一个字段的情况）
        if column.unique_group.is_some() {
            // 检查这个 group 中是否有多个字段
            let group_count = T::COLUMN_SCHEMA
                .iter()
                .filter(|c| c.unique_group == column.unique_group)
                .count();

            if group_count == 1 {
                // 单列唯一约束
                sql.push_str(" UNIQUE");
            }
        }
    }

    // 添加外键约束
    let foreign_key_constraints = generate_foreign_key_constraints::<T>();
    if !foreign_key_constraints.is_empty() {
        sql.push_str(", ");
        sql.push_str(&foreign_key_constraints.join(", "));
    }

    // 添加联合 UNIQUE 约束
    let unique_constraints = generate_unique_constraints::<T>();
    if !unique_constraints.is_empty() {
        sql.push_str(", ");
        sql.push_str(&unique_constraints.join(", "));
    }

    sql.push(')');

    // 添加索引
    let index_sql = generate_indexes::<T>(db_type);
    if !index_sql.is_empty() {
        sql.push_str(";");
        sql.push_str(&index_sql);
    }

    sql
}

/// 生成 UNIQUE 约束
fn generate_unique_constraints<T: Model>() -> Vec<String> {
    let mut constraints = Vec::new();

    // 收集所有 unique_group
    let mut group_map: std::collections::BTreeMap<i32, Vec<&str>> =
        std::collections::BTreeMap::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if let Some(group_id) = column.unique_group {
            group_map.entry(group_id).or_default().push(column.name);
        }
    }

    // 生成约束
    for (_group_id, columns) in group_map {
        if columns.len() == 1 {
            // 单列唯一约束已经在列定义中处理
        } else {
            // 联合唯一约束
            let cols = columns.join(", ");
            constraints.push(format!("UNIQUE ({cols})"));
        }
    }

    constraints
}

/// 生成索引 SQL
fn generate_indexes<T: Model>(_db_type: crate::abstract_layer::DbType) -> String {
    let mut sqls = Vec::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if column.is_indexed {
            let index_name = format!("idx_{}_{}", T::TABLE_NAME, column.name);
            sqls.push(format!(
                "CREATE INDEX IF NOT EXISTS {} ON {} ({})",
                index_name,
                T::TABLE_NAME,
                column.name
            ));
        }
    }

    sqls.join(";")
}

/// 生成外键约束 SQL
fn generate_foreign_key_constraints<T: Model>() -> Vec<String> {
    let mut constraints = Vec::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if let Some(fk) = &column.foreign_key {
            constraints.push(format!(
                "FOREIGN KEY ({}) REFERENCES {} ({})",
                column.name, fk.ref_table, fk.ref_column
            ));
        }
    }

    constraints
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

impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Real(v) => Ok(*v),
            Value::Integer(v) => Ok(*v as f64),
            _ => Err(Error::TypeMismatch("f64".to_string())),
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

impl FromValue for usize {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Integer(v) => Ok(*v as usize),
            _ => Err(Error::TypeMismatch("usize".to_string())),
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

impl FromValue for Option<f64> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Real(v) => Ok(Some(*v)),
            Value::Integer(v) => Ok(Some(*v as f64)),
            _ => Err(Error::TypeMismatch("Option<f64>".to_string())),
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

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
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
