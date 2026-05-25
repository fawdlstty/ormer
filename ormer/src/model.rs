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
    pub data_type: Option<&'static str>, // 数据库类型覆盖
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
    fn from_row(row: &Row) -> anyhow::Result<Self>;
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self>;

    /// 获取字段值 (用于 INSERT/UPDATE)
    fn field_values(&self) -> Vec<Value>;

    /// 获取主键字段名列表（支持单主键和复合主键）
    fn primary_key_columns() -> &'static [&'static str] {
        // 默认实现返回空，要求派生宏生成
        &[]
    }

    /// 获取主键值列表（支持单主键和复合主键）
    fn primary_key_values(&self) -> Vec<Value>;

    /// 获取主键字段名（已废弃，请使用 primary_key_columns）
    #[deprecated(since = "0.2.0", note = "Please use `primary_key_columns()` instead")]
    fn primary_key_column() -> &'static str {
        Self::primary_key_columns()[0]
    }

    /// 获取主键值（已废弃，请使用 primary_key_values）
    #[deprecated(since = "0.2.0", note = "Please use `primary_key_values()` instead")]
    fn primary_key_value(&self) -> Value {
        self.primary_key_values()[0].clone()
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
                Self::COLUMNS
                    .iter()
                    .position(|&c| c == col.name)
                    .and_then(|original_idx| {
                        if original_idx < all_values.len() {
                            Some(all_values[original_idx].clone())
                        } else {
                            None
                        }
                    })
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
    fn from_name(name: &str) -> anyhow::Result<Self>
    where
        Self: Sized;

    /// 获取当前变体的数值表示（用于数值枚举）
    /// 默认返回 0，数值枚举应重写此方法
    fn as_i64(&self) -> i64 {
        0
    }

    /// 从数值构造枚举值（用于数值枚举）
    /// 默认返回错误，数值枚举应重写此方法
    fn from_i64(_value: i64) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        Err(anyhow::anyhow!(
            "This enum does not support numeric conversion"
        ))
    }

    /// 判断是否为数值枚举
    /// 默认返回 false，数值枚举应重写此方法返回 true
    fn is_numeric_enum() -> bool {
        false
    }
}

/// 为 `Option<T>` 实现 ModelEnumProvider (如果 T 实现了 ModelEnum)
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
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Text(s) => {
                // 使用 ModelEnum::from_name 构造枚举值
                match T::from_name(s) {
                    Ok(enum_val) => Ok(Some(enum_val)),
                    Err(_) => Err(anyhow::anyhow!("Unknown enum variant: {}", s)),
                }
            }
            _ => Err(anyhow::anyhow!(
                "Expected Text value for Option<{}>",
                std::any::type_name::<T>()
            )),
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
    fn as_refs_mut(&mut self) -> Vec<&mut Self::Model>;
}

impl<T: crate::model::Model> Insertable for &T {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        vec![*self]
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &T 无法提供 &mut T，返回空向量（仅在需要可变引用时会使用其他实现）
        vec![]
    }
}

impl<T: crate::model::Model> Insertable for Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }
}

impl<T: crate::model::Model> Insertable for &Vec<T> {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &Vec<T> 无法提供 &mut T，返回空向量
        vec![]
    }
}

impl<T: crate::model::Model> Insertable for &[T] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &[T] 无法提供 &mut T，返回空向量
        vec![]
    }
}

impl<T: crate::model::Model, const N: usize> Insertable for &[T; N] {
    type Model = T;
    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }
    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        // &[T; N] 无法提供 &mut T，返回空向量
        vec![]
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
pub fn generate_create_table_sql<T: Model>(
    db_type: crate::abstract_layer::DbType,
) -> anyhow::Result<String> {
    generate_create_table_sql_with_name::<T>(db_type, None)
}

/// 生成 CREATE TABLE SQL 语句，支持自定义表名
pub fn generate_create_table_sql_with_name<T: Model>(
    db_type: crate::abstract_layer::DbType,
    table_name: Option<&str>,
) -> anyhow::Result<String> {
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
        let effective_rust_type = column.data_type.unwrap_or(column.rust_type);
        let sql_type = if is_composite_primary && column.is_primary {
            db_type.sql_type(
                effective_rust_type,
                false, // 不在列级别标记为主键
                column.is_auto_increment,
                column.is_nullable,
                column.enum_variants,
            )
        } else {
            db_type.sql_type(
                effective_rust_type,
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
        sql.push(';');
        sql.push_str(&index_sql);
    }

    Ok(sql)
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
                // PostgreSQL 和 Sqlite (SQLite) 支持 IF NOT EXISTS
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

    pub fn get<T: FromValue>(&self, column: &str) -> anyhow::Result<T> {
        self.data
            .get(column)
            .ok_or_else(|| anyhow::anyhow!("Column not found: {}", column))
            .and_then(|v| T::from_value(v))
    }
}

/// 值类型
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    BigInt(i128),
    Text(String),
    Real(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
    DateTime(chrono::DateTime<chrono::Utc>),
    Json(serde_json::Value),
    Uuid(uuid::Uuid),
    Null,
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> anyhow::Result<Self>;
}

/// FromRowValues trait - 用于从一行中的多个值构建类型(如元组、Model)
pub trait FromRowValues: Sized {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self>;
}

/// FromSingleValue trait - 用于从单个值构建Model(用于map_to后的转换)
/// 当查询单列结果并想转换为Model时使用
pub trait FromSingleValue<V>: Sized {
    fn from_single_value(value: V, column_name: &str) -> anyhow::Result<Self>;
}

// 为所有可以转换为Value的类型实现FromSingleValue的blanket implementation
impl<T, V> FromSingleValue<V> for T
where
    T: Model,
    V: Into<Value>,
    T: FromValue,
{
    fn from_single_value(value: V, _column_name: &str) -> anyhow::Result<Self> {
        let ormer_value: Value = value.into();
        Self::from_value(&ormer_value)
    }
}

// 使用宏生成 FromValue 实现，减少重复代码
macro_rules! impl_from_value_for {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl FromValue for $type {
                fn from_value(value: &Value) -> anyhow::Result<Self> {
                    match value {
                        Value::$variant(v) => Ok(*v as $type),
                        _ => Err(anyhow::anyhow!("Type mismatch: expected {}", stringify!($type))),
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
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected i32"));
        }
        Self::from_value(&values[0])
    }
}

impl FromRowValues for i64 {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected i64"));
        }
        Self::from_value(&values[0])
    }
}

impl FromRowValues for usize {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected usize"));
        }
        Self::from_value(&values[0])
    }
}

// f64 特殊处理（支持 Integer 和 Real）
impl FromValue for f64 {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Real(v) => Ok(*v),
            Value::Integer(v) => Ok(*v as f64),
            _ => Err(anyhow::anyhow!("Type mismatch: expected f64")),
        }
    }
}

impl FromRowValues for f64 {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected f64"));
        }
        Self::from_value(&values[0])
    }
}

// String 特殊处理（需要 clone）
impl FromValue for String {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected String")),
        }
    }
}

impl FromRowValues for String {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected String"));
        }
        Self::from_value(&values[0])
    }
}

// bool 特殊处理（从 Boolean 读取）
impl FromValue for bool {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Boolean(v) => Ok(*v),
            Value::Integer(v) => Ok(*v != 0), // 向后兼容
            _ => Err(anyhow::anyhow!("Type mismatch: expected bool")),
        }
    }
}

impl FromRowValues for bool {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected bool"));
        }
        Self::from_value(&values[0])
    }
}

// 为二元组实现 FromValue
impl<T1: FromValue, T2: FromValue> FromValue for (T1, T2) {
    fn from_value(_value: &Value) -> anyhow::Result<Self> {
        // 元组不能从单个Value构建，这个实现仅用于类型系统完整性
        // 实际上元组应该从多个Value构建
        Err(anyhow::anyhow!("Type mismatch: expected tuple"))
    }
}

// 为二元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues> FromRowValues for (T1, T2) {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.len() < 2 {
            return Err(anyhow::anyhow!("Type mismatch: expected tuple (T1, T2)"));
        }
        let v1 = T1::from_row_values(&values[0..1])?;
        let v2 = T2::from_row_values(&values[1..2])?;
        Ok((v1, v2))
    }
}

// 为三元组实现 FromValue
impl<T1: FromValue, T2: FromValue, T3: FromValue> FromValue for (T1, T2, T3) {
    fn from_value(_value: &Value) -> anyhow::Result<Self> {
        Err(anyhow::anyhow!("Type mismatch: expected tuple"))
    }
}

// 为三元组实现 FromRowValues
impl<T1: FromRowValues, T2: FromRowValues, T3: FromRowValues> FromRowValues for (T1, T2, T3) {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.len() < 3 {
            return Err(anyhow::anyhow!(
                "Type mismatch: expected tuple (T1, T2, T3)"
            ));
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
                fn from_value(value: &Value) -> anyhow::Result<Self> {
                    match value {
                        Value::Null => Ok(None),
                        Value::$variant(v) => Ok(Some(*v as $type)),
                        _ => Err(anyhow::anyhow!("Type mismatch: expected Option<{}>", stringify!($type))),
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
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Text(v) => Ok(Some(v.clone())),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Option<String>")),
        }
    }
}

impl FromValue for Option<bool> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Boolean(v) => Ok(Some(*v)),
            Value::Integer(v) => Ok(Some(*v != 0)), // 向后兼容
            _ => Err(anyhow::anyhow!("Type mismatch: expected Option<bool>")),
        }
    }
}

impl FromValue for Option<f64> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Real(v) => Ok(Some(*v)),
            Value::Integer(v) => Ok(Some(*v as f64)),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Option<f64>")),
        }
    }
}

// 为 Option 类型实现 FromRowValues
impl<T: FromValue> FromRowValues for Option<T> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!(
                "Type mismatch: expected Option<{}>",
                std::any::type_name::<T>()
            ));
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

// bool 特殊处理（转为 Boolean）
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Boolean(v)
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
            Some(true) => Value::Boolean(true),
            Some(false) => Value::Boolean(false),
            None => Value::Null,
        }
    }
}

// 为 FilterValue 实现 Into<Value>
impl From<crate::query::filter::Value> for Value {
    fn from(value: crate::query::filter::Value) -> Self {
        match value {
            crate::query::filter::Value::Integer(v) => Value::Integer(v),
            crate::query::filter::Value::BigInt(v) => Value::BigInt(v),
            crate::query::filter::Value::Text(v) => Value::Text(v),
            crate::query::filter::Value::Real(v) => Value::Real(v),
            crate::query::filter::Value::Boolean(v) => Value::Boolean(v),
            crate::query::filter::Value::Bytes(v) => Value::Bytes(v),
            crate::query::filter::Value::DateTime(v) => Value::DateTime(v),
            crate::query::filter::Value::Json(v) => Value::Json(v),
            crate::query::filter::Value::Uuid(v) => Value::Uuid(v),
            crate::query::filter::Value::Null => Value::Null,
        }
    }
}

// Vec<u8> (Bytes) 特殊处理
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(v)
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Vec<u8>")),
        }
    }
}

impl From<Option<Vec<u8>>> for Value {
    fn from(v: Option<Vec<u8>>) -> Self {
        match v {
            Some(bytes) => Value::Bytes(bytes),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<Vec<u8>> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Bytes(v) => Ok(Some(v.clone())),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Option<Vec<u8>>")),
        }
    }
}

// chrono::DateTime<Utc> 特殊处理
impl From<chrono::DateTime<chrono::Utc>> for Value {
    fn from(v: chrono::DateTime<chrono::Utc>) -> Self {
        Value::DateTime(v)
    }
}

impl FromValue for chrono::DateTime<chrono::Utc> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::DateTime(v) => Ok(*v),
            _ => Err(anyhow::anyhow!("Type mismatch: expected DateTime<Utc>")),
        }
    }
}

impl From<Option<chrono::DateTime<chrono::Utc>>> for Value {
    fn from(v: Option<chrono::DateTime<chrono::Utc>>) -> Self {
        match v {
            Some(dt) => Value::DateTime(dt),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<chrono::DateTime<chrono::Utc>> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::DateTime(v) => Ok(Some(*v)),
            _ => Err(anyhow::anyhow!(
                "Type mismatch: expected Option<DateTime<Utc>>"
            )),
        }
    }
}

// chrono::NaiveDateTime 特殊处理
impl From<chrono::NaiveDateTime> for Value {
    fn from(v: chrono::NaiveDateTime) -> Self {
        Value::DateTime(v.and_utc())
    }
}

impl FromValue for chrono::NaiveDateTime {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::DateTime(v) => Ok(v.naive_utc()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected NaiveDateTime")),
        }
    }
}

impl FromRowValues for chrono::NaiveDateTime {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected NaiveDateTime"));
        }
        Self::from_value(&values[0])
    }
}

impl From<Option<chrono::NaiveDateTime>> for Value {
    fn from(v: Option<chrono::NaiveDateTime>) -> Self {
        match v {
            Some(dt) => Value::DateTime(dt.and_utc()),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<chrono::NaiveDateTime> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::DateTime(v) => Ok(Some(v.naive_utc())),
            _ => Err(anyhow::anyhow!(
                "Type mismatch: expected Option<NaiveDateTime>"
            )),
        }
    }
}

// serde_json::Value 特殊处理
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        Value::Json(v)
    }
}

impl FromValue for serde_json::Value {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Json(v) => Ok(v.clone()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected serde_json::Value")),
        }
    }
}

impl From<Option<serde_json::Value>> for Value {
    fn from(v: Option<serde_json::Value>) -> Self {
        match v {
            Some(json) => Value::Json(json),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<serde_json::Value> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Json(v) => Ok(Some(v.clone())),
            _ => Err(anyhow::anyhow!(
                "Type mismatch: expected Option<serde_json::Value>"
            )),
        }
    }
}

// uuid::Uuid 特殊处理
impl From<uuid::Uuid> for Value {
    fn from(v: uuid::Uuid) -> Self {
        Value::Uuid(v)
    }
}

impl FromValue for uuid::Uuid {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Uuid(v) => Ok(*v),
            _ => Err(anyhow::anyhow!("Type mismatch: expected uuid::Uuid")),
        }
    }
}

impl From<Option<uuid::Uuid>> for Value {
    fn from(v: Option<uuid::Uuid>) -> Self {
        match v {
            Some(uuid) => Value::Uuid(uuid),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<uuid::Uuid> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Uuid(v) => Ok(Some(*v)),
            _ => Err(anyhow::anyhow!(
                "Type mismatch: expected Option<uuid::Uuid>"
            )),
        }
    }
}

// 重新导出钩子 traits 以保持向后兼容
pub use crate::hooks::{
    AfterDelete, AfterInsert, AfterUpdate, BeforeDelete, BeforeInsert, BeforeUpdate,
};
