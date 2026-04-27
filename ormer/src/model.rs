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
    pub enum_variants: Option<&'static [&'static str]>, // 枚举类型的变体列表
}

/// 外键信息
#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub ref_table: &'static str,                     // 引用的表名
    pub ref_column: &'static str,                    // 引用的列名（对于静态指定的情况）
    pub ref_column_fn: Option<fn() -> &'static str>, // 运行时获取列名的函数（对于自动关联主键的情况）
}

impl ForeignKeyInfo {
    /// 获取引用列名
    pub fn get_ref_column(&self) -> &'static str {
        if let Some(fn_get) = self.ref_column_fn {
            fn_get()
        } else {
            self.ref_column
        }
    }
}

/// 数据库后端 trait - 用于 SQL 类型映射
pub trait DbBackendTypeMapper {
    /// 根据 Rust 类型获取 SQL 类型
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
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
    fn from_row_values(values: &[Value]) -> Result<Self, Error>;

    /// 获取字段值 (用于 INSERT/UPDATE)
    fn field_values(&self) -> Vec<Value>;

    /// 获取主键字段名列表 (用于 UPDATE/DELETE，支持复合主键)
    fn primary_key_columns() -> &'static [&'static str] {
        // 默认实现：返回单个主键，保持向后兼容
        const PRIMARY_KEY: &[&str] = &[""];
        PRIMARY_KEY
    }

    /// 获取主键值列表 (支持复合主键)
    fn primary_key_values(&self) -> Vec<Value> {
        // 默认实现：返回单个主键值，保持向后兼容
        vec![self.primary_key_value()]
    }

    /// 获取主键字段名 (用于 UPDATE/DELETE，已废弃，请使用 primary_key_columns)
    fn primary_key_column() -> &'static str {
        ""
    }

    /// 获取主键值 (已废弃，请使用 primary_key_values)
    fn primary_key_value(&self) -> Value {
        Value::Null
    }

    /// 获取需要插入的列名（排除自增主键）
    fn insert_columns() -> Vec<&'static str> {
        Self::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.name)
            .collect()
    }

    /// 获取需要插入的字段值（排除自增主键）
    fn insert_values(&self) -> Vec<Value> {
        let all_values = self.field_values();
        Self::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .filter_map(|col| {
                // 找到原始字段值中对应的索引
                let original_idx = Self::COLUMNS.iter().position(|&c| c == col.name).unwrap();
                if original_idx < all_values.len() {
                    Some(all_values[original_idx].clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// 枚举类型提供者 trait (可选实现)
/// 如果类型实现了此 trait,则会被识别为枚举类型并生成 ENUM SQL
pub trait ModelEnumProvider {
    /// 获取枚举的所有变体名称
    fn enum_variants() -> Option<&'static [&'static str]>;
}

/// ModelEnum trait - 用于标记枚举类型 (由派生宏自动实现)
pub trait ModelEnum: ModelEnumProvider {
    /// 获取枚举的所有变体名称  
    const VARIANTS: &'static [&'static str];

    /// 获取当前变体的名称
    fn name(&self) -> &'static str;

    /// 从名称构造枚举值
    fn from_name(name: &str) -> Result<Self, Error>
    where
        Self: Sized;
}

/// 为 Option<T> 实现 ModelEnumProvider (如果 T 实现了 ModelEnum)
impl<T: ModelEnum> ModelEnumProvider for Option<T> {
    fn enum_variants() -> Option<&'static [&'static str]> {
        Some(T::VARIANTS)
    }
}

// 为 Option<T> where T: ModelEnum 实现 From<Option<T>> for Value
impl<T: ModelEnum> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(enum_val) => Value::Text(enum_val.name().to_string()),
            None => Value::Null,
        }
    }
}

// 为 Option<T> where T: ModelEnum 实现 FromValue
impl<T: ModelEnum> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            Value::Text(s) => {
                // 使用 ModelEnum::from_name 构造枚举值
                match T::from_name(s) {
                    Ok(enum_val) => Ok(Some(enum_val)),
                    Err(_) => Err(Error::TypeMismatch(format!("Unknown enum variant: {}", s))),
                }
            }
            _ => Err(Error::TypeMismatch(format!(
                "Expected Text value for Option<{}>",
                std::any::type_name::<T>()
            ))),
        }
    }
}

// 为常见非枚举类型实现 ModelEnumProvider，返回 None
macro_rules! impl_enum_provider_for_non_enum {
    ($($t:ty),* $(,)?) => {
        $(
            impl ModelEnumProvider for $t {
                fn enum_variants() -> Option<&'static [&'static str]> {
                    None
                }
            }
        )*
    };
}

impl_enum_provider_for_non_enum!(
    i8, i16, i32, i64, u8, u16, u32, u64, isize, usize, f32, f64, bool, String, &str,
);

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
    generate_create_table_sql_with_name::<T>(db_type, None)
}

/// 生成 CREATE TABLE SQL 语句，支持自定义表名
pub fn generate_create_table_sql_with_name<T: Model>(
    db_type: crate::abstract_layer::DbType,
    table_name: Option<&str>,
) -> String {
    let table_name = table_name.unwrap_or(T::TABLE_NAME);
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (", table_name);

    for (i, column) in T::COLUMN_SCHEMA.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }

        // 检查是否有复合主键（多个主键字段）
        let primary_key_count = T::COLUMN_SCHEMA.iter().filter(|c| c.is_primary).count();
        let is_composite_primary = primary_key_count > 1;

        // 对于复合主键，不在列定义中添加 PRIMARY KEY，而是在最后添加表级约束
        let sql_type = if is_composite_primary && column.is_primary {
            db_type.sql_type(
                column.rust_type,
                false, // 不在列级别标记为主键
                column.is_auto_increment,
                column.is_nullable,
                column.enum_variants,
            )
        } else {
            db_type.sql_type(
                column.rust_type,
                column.is_primary,
                column.is_auto_increment,
                column.is_nullable,
                column.enum_variants,
            )
        };

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

    // 添加复合主键约束（如果有多个主键字段）
    let composite_primary_constraint = generate_composite_primary_key_constraint::<T>();
    if !composite_primary_constraint.is_empty() {
        sql.push_str(", ");
        sql.push_str(&composite_primary_constraint);
    }

    // 添加联合 UNIQUE 约束
    let unique_constraints = generate_unique_constraints::<T>();
    if !unique_constraints.is_empty() {
        sql.push_str(", ");
        sql.push_str(&unique_constraints.join(", "));
    }

    sql.push(')');

    // 添加索引
    let index_sql = generate_indexes_with_name::<T>(db_type, table_name);
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
#[allow(dead_code)]
fn generate_indexes<T: Model>(db_type: crate::abstract_layer::DbType) -> String {
    generate_indexes_with_name::<T>(db_type, T::TABLE_NAME)
}

/// 生成索引 SQL，支持自定义表名
fn generate_indexes_with_name<T: Model>(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
) -> String {
    let mut sqls = Vec::new();

    // 检查是否为 MySQL 数据库（通过调试字符串）
    let is_mysql = format!("{:?}", db_type).contains("MySQL");

    for column in T::COLUMN_SCHEMA.iter() {
        if column.is_indexed {
            let index_name = format!("idx_{}_{}", table_name, column.name);
            // MySQL 不支持 CREATE INDEX IF NOT EXISTS，需要特殊处理
            let sql = if is_mysql {
                format!(
                    "CREATE INDEX {} ON {} ({})",
                    index_name, table_name, column.name
                )
            } else {
                // PostgreSQL 和 Turso (SQLite) 支持 IF NOT EXISTS
                format!(
                    "CREATE INDEX IF NOT EXISTS {} ON {} ({})",
                    index_name, table_name, column.name
                )
            };
            sqls.push(sql);
        }
    }

    sqls.join(";")
}

/// 生成外键约束 SQL
fn generate_foreign_key_constraints<T: Model>() -> Vec<String> {
    let mut constraints = Vec::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if let Some(fk) = &column.foreign_key {
            let ref_column = fk.get_ref_column();
            constraints.push(format!(
                "FOREIGN KEY ({}) REFERENCES {} ({})",
                column.name, fk.ref_table, ref_column
            ));
        }
    }

    constraints
}

/// 生成复合主键约束 SQL
fn generate_composite_primary_key_constraint<T: Model>() -> String {
    let primary_keys: Vec<&str> = T::COLUMN_SCHEMA
        .iter()
        .filter(|c| c.is_primary)
        .map(|c| c.name)
        .collect();

    if primary_keys.len() > 1 {
        // 复合主键：PRIMARY KEY (col1, col2, ...)
        format!("PRIMARY KEY ({})", primary_keys.join(", "))
    } else {
        // 单主键或无主键：不需要表级约束
        String::new()
    }
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

/// FromRowValues trait - 用于从一行中的多个值构建类型（如元组、Model）
pub trait FromRowValues: Sized {
    fn from_row_values(values: &[Value]) -> Result<Self, Error>;
}

/// FromSingleValue trait - 用于从单个值构建Model（用于map_to后的转换）
/// 当查询单列结果并想转换为Model时使用
pub trait FromSingleValue<V>: Sized {
    fn from_single_value(value: V, column_name: &str) -> Result<Self, Error>;
}

// 为所有可以转换为Value的类型实现FromSingleValue的blanket implementation
impl<T, V> FromSingleValue<V> for T
where
    T: Model,
    V: Into<Value>,
    T: FromValue,
{
    fn from_single_value(value: V, _column_name: &str) -> Result<Self, Error> {
        let ormer_value: Value = value.into();
        Self::from_value(&ormer_value)
    }
}

// 使用宏生成 FromValue 实现，减少重复代码
macro_rules! impl_from_value_for {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl FromValue for $type {
                fn from_value(value: &Value) -> Result<Self, Error> {
                    match value {
                        Value::$variant(v) => Ok(*v as $type),
                        _ => Err(Error::TypeMismatch(stringify!($type).to_string())),
                    }
                }
            }
        )*
    };
}

// 为基本类型生成 FromValue 实现
impl_from_value_for!(
    i32 => Integer,
    i64 => Integer,
    usize => Integer,
);

// 为基本类型实现 FromRowValues（从单列构建）
impl FromRowValues for i32 {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("i32".to_string()));
        }
        Self::from_value(&values[0])
    }
}

impl FromRowValues for i64 {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("i64".to_string()));
        }
        Self::from_value(&values[0])
    }
}

impl FromRowValues for usize {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("usize".to_string()));
        }
        Self::from_value(&values[0])
    }
}

// f64 特殊处理（支持 Integer 和 Real）
impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Real(v) => Ok(*v),
            Value::Integer(v) => Ok(*v as f64),
            _ => Err(Error::TypeMismatch("f64".to_string())),
        }
    }
}

impl FromRowValues for f64 {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("f64".to_string()));
        }
        Self::from_value(&values[0])
    }
}

// String 特殊处理（需要 clone）
impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            _ => Err(Error::TypeMismatch("String".to_string())),
        }
    }
}

impl FromRowValues for String {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("String".to_string()));
        }
        Self::from_value(&values[0])
    }
}

// bool 特殊处理（0/1 转换）
impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Integer(v) => Ok(*v != 0),
            _ => Err(Error::TypeMismatch("bool".to_string())),
        }
    }
}

impl FromRowValues for bool {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch("bool".to_string()));
        }
        Self::from_value(&values[0])
    }
}

// 为二元组实现 FromValue
impl<T1: FromValue, T2: FromValue> FromValue for (T1, T2) {
    fn from_value(_value: &Value) -> Result<Self, Error> {
        // 元组不能从单个Value构建，这个实现仅用于类型系统完整性
        // 实际上元组应该从多个Value构建
        Err(Error::TypeMismatch("tuple".to_string()))
    }
}

// 为二元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues> FromRowValues for (T1, T2) {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.len() < 2 {
            return Err(Error::TypeMismatch("tuple (T1, T2)".to_string()));
        }
        let v1 = T1::from_row_values(&values[0..1])?;
        let v2 = T2::from_row_values(&values[1..2])?;
        Ok((v1, v2))
    }
}

// 为三元组实现 FromValue
impl<T1: FromValue, T2: FromValue, T3: FromValue> FromValue for (T1, T2, T3) {
    fn from_value(_value: &Value) -> Result<Self, Error> {
        Err(Error::TypeMismatch("tuple".to_string()))
    }
}

// 为三元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues, T3: FromRowValues> FromRowValues for (T1, T2, T3) {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.len() < 3 {
            return Err(Error::TypeMismatch("tuple (T1, T2, T3)".to_string()));
        }
        let v1 = T1::from_row_values(&values[0..1])?;
        let v2 = T2::from_row_values(&values[1..2])?;
        let v3 = T3::from_row_values(&values[2..3])?;
        Ok((v1, v2, v3))
    }
}

// 使用宏生成 Option<T> 的 FromValue 实现
macro_rules! impl_from_value_for_option {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl FromValue for Option<$type> {
                fn from_value(value: &Value) -> Result<Self, Error> {
                    match value {
                        Value::Null => Ok(None),
                        Value::$variant(v) => Ok(Some(*v as $type)),
                        _ => Err(Error::TypeMismatch(concat!("Option<", stringify!($type), ">").to_string())),
                    }
                }
            }
        )*
    };
}

// 为 Option 类型生成 FromValue 实现
impl_from_value_for_option!(
    i32 => Integer,
    i64 => Integer,
);

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

// 为 Option 类型实现 FromRowValues
impl<T: FromValue> FromRowValues for Option<T> {
    fn from_row_values(values: &[Value]) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::TypeMismatch(format!(
                "Option<{}>",
                std::any::type_name::<T>()
            )));
        }
        // 直接使用 Option<T> 的 from_value 实现
        match &values[0] {
            Value::Null => Ok(None),
            _ => {
                let inner = T::from_value(&values[0])?;
                Ok(Some(inner))
            }
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

// 使用宏生成 From<T> for Value 实现
macro_rules! impl_from_for_value {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$type> for Value {
                fn from(v: $type) -> Self {
                    Value::$variant(v as i64)
                }
            }
        )*
    };
}

// 为整数类型生成 From 实现
impl_from_for_value!(
    i32 => Integer,
    i64 => Integer,
);

// f64 特殊处理
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
    }
}

// String 特殊处理
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

// bool 特殊处理（转为 0/1）
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        if v {
            Value::Integer(1)
        } else {
            Value::Integer(0)
        }
    }
}

// 使用宏生成 Option<T> 的 From 实现
macro_rules! impl_from_option_for_value {
    ($($type:ty => { Some($variant:ident), None => Null }),* $(,)?) => {
        $(
            impl From<Option<$type>> for Value {
                fn from(v: Option<$type>) -> Self {
                    match v {
                        Some(val) => Value::$variant(val as i64),
                        None => Value::Null,
                    }
                }
            }
        )*
    };
}

// 为 Option 整数类型生成 From 实现
impl_from_option_for_value!(
    i32 => { Some(Integer), None => Null },
    i64 => { Some(Integer), None => Null },
);

// Option<String> 特殊处理
impl From<Option<String>> for Value {
    fn from(v: Option<String>) -> Self {
        match v {
            Some(s) => Value::Text(s),
            None => Value::Null,
        }
    }
}

// Option<bool> 特殊处理
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
