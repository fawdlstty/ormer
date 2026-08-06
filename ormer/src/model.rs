use crate::time::{naive_local_to_utc, utc_to_naive_local};
use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;

/// 为 Duration 扩展 PostgreSQL INTERVAL 格式化能力
pub trait DurationToInterval {
    /// 将 Duration 转换为 PostgreSQL INTERVAL 字符串
    /// 支持小数形式，自动选择最适配的单位（milliseconds/seconds/minutes/hours/days）
    fn to_interval_string(&self) -> String;
}

impl DurationToInterval for std::time::Duration {
    fn to_interval_string(&self) -> String {
        let total_secs = self.as_secs_f64();
        let millis = self.subsec_millis();

        if total_secs < 1.0 && millis > 0 {
            format!("{} milliseconds", millis)
        } else if total_secs < 60.0 {
            // 秒级，支持小数
            if total_secs.fract() == 0.0 {
                format!("{} seconds", total_secs as u64)
            } else {
                format!("{:.3} seconds", total_secs)
            }
        } else if total_secs < 3600.0 {
            // 分钟级，支持小数
            let minutes = total_secs / 60.0;
            if minutes.fract() == 0.0 {
                format!("{} minutes", minutes as u64)
            } else {
                format!("{:.3} minutes", minutes)
            }
        } else if total_secs < 86400.0 {
            // 小时级，支持小数
            let hours = total_secs / 3600.0;
            if hours.fract() == 0.0 {
                format!("{} hours", hours as u64)
            } else {
                format!("{:.3} hours", hours)
            }
        } else {
            // 天级，支持小数
            let days = total_secs / 86400.0;
            if days.fract() == 0.0 {
                format!("{} days", days as u64)
            } else {
                format!("{:.3} days", days)
            }
        }
    }
}

/// 字段元数据
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub rust_name: &'static str,
    pub name: &'static str,
    pub rust_type: &'static str,
    pub is_primary: bool,
    pub is_auto_increment: bool,
    pub is_nullable: bool,
    pub unique_group: Option<i32>, // None表示不唯一，Some(group_id)表示属于哪个唯一键组
    pub unique_name: Option<&'static str>,
    pub is_indexed: bool,
    pub index_group: Option<i32>,
    pub index_name: Option<&'static str>,
    pub index_order: Option<&'static str>,
    pub index_where: Option<&'static str>,
    pub foreign_key: Option<ForeignKeyInfo>, // 外键信息
    pub enum_variants: Option<&'static [&'static str]>, // 枚举类型的变体列表
    pub data_type: Option<&'static str>,     // 数据库类型覆盖
    pub default: Option<ColumnDefault>,      // 数据库端默认值
    pub check: Option<CheckConstraint>,      // CHECK 约束
    pub hypertable: Option<std::time::Duration>, // TimescaleDB hypertable 分片时长
    pub compress: bool,                      // 是否启用数据库级压缩（PostgreSQL: COMPRESSION pglz）
}

/// 字段默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnDefault {
    String(&'static str),
    Number(&'static str),
    Boolean(bool),
    Expression(&'static str),
}

impl ColumnDefault {
    pub fn to_sql(self, db_type: crate::abstract_layer::DbType) -> String {
        match self {
            Self::String(value) => quote_sql_literal(value),
            Self::Number(value) => value.to_string(),
            Self::Boolean(value) => match db_type {
                #[cfg(feature = "sqlite")]
                crate::abstract_layer::DbType::Sqlite => if value { "1" } else { "0" }.to_string(),
                #[cfg(feature = "postgresql")]
                crate::abstract_layer::DbType::PostgreSQL => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
                #[cfg(feature = "mysql")]
                crate::abstract_layer::DbType::MySQL => {
                    if value { "TRUE" } else { "FALSE" }.to_string()
                }
                #[cfg(feature = "mssql")]
                crate::abstract_layer::DbType::MSSQL => if value { "1" } else { "0" }.to_string(),
            },
            Self::Expression(expr) => expr.to_string(),
        }
    }
}

/// 字段 CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckConstraint {
    pub name: Option<&'static str>,
    pub expr: &'static str,
}

/// 外键信息
#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub name: Option<&'static str>,                  // 外键约束名
    pub ref_table: &'static str,                     // 引用的表名
    pub ref_column: &'static str,                    // 引用的列名（对于静态指定的情况）
    pub ref_column_fn: Option<fn() -> &'static str>, // 运行时获取列名的函数（对于自动关联主键的情况）
    pub on_delete: Option<ForeignKeyAction>,         // ON DELETE 动作
    pub on_update: Option<ForeignKeyAction>,         // ON UPDATE 动作
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

/// 外键动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignKeyAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ForeignKeyAction {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

/// 关系方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    HasMany,
    BelongsTo,
}

/// 关系元数据
#[derive(Debug, Clone)]
pub struct RelationInfo {
    pub name: &'static str,
    pub kind: RelationKind,
    pub target_table: &'static str,
    pub local_key: &'static str,
    pub target_key: &'static str,
}

/// 类型化关系句柄，用于 include/preload/find_related。
#[derive(Debug, Clone, Copy)]
pub struct Relation<Owner: Model, Target: Model> {
    name: &'static str,
    _marker: PhantomData<(Owner, Target)>,
}

impl<Owner: Model, Target: Model> Relation<Owner, Target> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _marker: PhantomData,
        }
    }

    pub fn info(&self) -> anyhow::Result<&'static RelationInfo> {
        Owner::RELATIONS
            .iter()
            .find(|relation| {
                relation.name == self.name && relation.target_table == Target::TABLE_NAME
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Relation {} -> {} not found on {}",
                    self.name,
                    Target::TABLE_NAME,
                    Owner::TABLE_NAME
                )
            })
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

/// 主键 trait - 用于 find_by_id 方法
/// 支持单主键和复合主键
pub trait PrimaryKey: Sized {
    fn into_values(self) -> Vec<Value>;
}

// 为 i32 实现 PrimaryKey（最常见的单主键类型）
impl PrimaryKey for i32 {
    fn into_values(self) -> Vec<Value> {
        vec![Value::from(self)]
    }
}

// 为 String 实现 PrimaryKey
impl PrimaryKey for String {
    fn into_values(self) -> Vec<Value> {
        vec![Value::from(self)]
    }
}

// 为 &str 实现 PrimaryKey（方便使用字符串字面量）
impl PrimaryKey for &str {
    fn into_values(self) -> Vec<Value> {
        vec![Value::from(self.to_string())]
    }
}

// 为两元素元组实现 PrimaryKey（复合主键）
impl<A, B> PrimaryKey for (A, B)
where
    A: Into<Value>,
    B: Into<Value>,
{
    fn into_values(self) -> Vec<Value> {
        vec![self.0.into(), self.1.into()]
    }
}

// 为三元素元组实现 PrimaryKey（复合主键）
impl<A, B, C> PrimaryKey for (A, B, C)
where
    A: Into<Value>,
    B: Into<Value>,
    C: Into<Value>,
{
    fn into_values(self) -> Vec<Value> {
        vec![self.0.into(), self.1.into(), self.2.into()]
    }
}

/// 模型 trait,所有 ORM 模型必须实现
pub trait Model: Sized {
    const TABLE_NAME: &'static str;
    const COLUMNS: &'static [&'static str];
    const COLUMN_SCHEMA: &'static [ColumnSchema];
    const RELATIONS: &'static [RelationInfo] = &[];

    /// 获取指定数据库后端实际使用的表名。
    fn table_name_for_db(db_type: crate::abstract_layer::DbType) -> &'static str {
        normalize_table_name_for_db(db_type, Self::TABLE_NAME)
    }

    /// 获取 hypertable 时间字段名和分片时长（如果有）
    fn hypertable_info() -> Option<(&'static str, std::time::Duration)> {
        for col in Self::COLUMN_SCHEMA {
            if let Some(duration) = col.hypertable {
                return Some((col.name, duration));
            }
        }
        None
    }

    /// 自增主键类型
    /// 如果模型有自增主键（#[primary(auto)]），此类型为主键的 Rust 类型（如 i32, i64）
    /// 如果没有自增主键，此类型为 ()
    type AutoIncrementKeyType: Default + 'static;

    type QueryBuilder;
    type Where: Default;

    fn query() -> Self::QueryBuilder;
    fn select() -> Self::QueryBuilder;
    fn from_row(row: &Row) -> anyhow::Result<Self>;
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self>;

    /// 获取字段值 (用于 INSERT/UPDATE)
    fn field_values(&self) -> Vec<Value>;

    /// 获取指定列的值。
    fn column_value(&self, _column: &str) -> Option<Value> {
        None
    }

    /// 通过 Rust 字段名查找实际 SQL 列名。
    fn column_name_for_field(field: &str) -> Option<&'static str> {
        Self::COLUMN_SCHEMA
            .iter()
            .find(|column| column.rust_name == field || column.name == field)
            .map(|column| column.name)
    }

    /// 获取关系本地键值。
    fn relation_key_value(&self, relation: &RelationInfo) -> anyhow::Result<Value> {
        self.column_value(relation.local_key).ok_or_else(|| {
            anyhow::anyhow!(
                "Column {} not found on {} for relation {}",
                relation.local_key,
                Self::TABLE_NAME,
                relation.name
            )
        })
    }

    /// 写入预加载后的关系对象。
    fn assign_relation<Target: Model + 'static>(
        &mut self,
        relation_name: &'static str,
        values: Vec<Target>,
    ) -> anyhow::Result<()> {
        let _ = values;
        Err(anyhow::anyhow!(
            "Relation {} is not assignable on {}",
            relation_name,
            Self::TABLE_NAME
        ))
    }

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

    /// 获取非主键字段的 (列名, 值) 对，用于 set_model
    fn non_pk_field_values(&self) -> Vec<(&'static str, Value)> {
        let all_values = self.field_values();
        Self::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_primary)
            .filter_map(|col| {
                Self::COLUMNS
                    .iter()
                    .position(|&c| c == col.name)
                    .and_then(|idx| {
                        if idx < all_values.len() {
                            Some((col.name, all_values[idx].clone()))
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    /// 获取指定非主键字段的 (列名, 值) 对，用于 set_model_fields。
    fn non_pk_field_values_for_columns(&self, columns: &[String]) -> Vec<(&'static str, Value)> {
        self.non_pk_field_values()
            .into_iter()
            .filter(|(col, _)| columns.iter().any(|selected| selected == col))
            .collect()
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
    const ENUM_VARIANTS: Option<&'static [&'static str]>;

    /// 获取枚举的所有变体名称
    fn enum_variants() -> Option<&'static [&'static str]> {
        Self::ENUM_VARIANTS
    }
}

/// 去掉 schema 前缀，返回最后一段表名。
pub fn table_name_without_schema(table_name: &str) -> &str {
    table_name
        .rsplit_once('.')
        .map(|(_, table)| table)
        .unwrap_or(table_name)
}

/// 按数据库后端返回实际 SQL 中使用的表名。
pub fn normalize_table_name_for_db(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
) -> &str {
    match db_type {
        #[cfg(feature = "sqlite")]
        crate::abstract_layer::DbType::Sqlite => table_name_without_schema(table_name),
        #[cfg(feature = "mysql")]
        crate::abstract_layer::DbType::MySQL => table_name_without_schema(table_name),
        #[cfg(feature = "postgresql")]
        crate::abstract_layer::DbType::PostgreSQL => table_name,
        #[cfg(feature = "mssql")]
        crate::abstract_layer::DbType::MSSQL => table_name,
    }
}

/// 拆分支持 schema 的数据库表名，未指定 schema 时使用默认 schema。
pub fn split_schema_table_name<'a>(
    table_name: &'a str,
    default_schema: &'a str,
) -> (&'a str, &'a str) {
    table_name
        .rsplit_once('.')
        .unwrap_or((default_schema, table_name))
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

/// 为 `Option<T>` 实现 ModelEnumProvider，透传内部类型的枚举信息
impl<T: ModelEnumProvider> ModelEnumProvider for Option<T> {
    const ENUM_VARIANTS: Option<&'static [&'static str]> = T::ENUM_VARIANTS;
}

// 为 Option<T> where T: ModelEnum 实现 From<Option<T>> for Value
impl<T: ModelEnum> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(enum_val) if T::is_numeric_enum() => Value::Integer(enum_val.as_i64()),
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
            Value::Integer(v) if T::is_numeric_enum() => T::from_i64(*v).map(Some),
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
                const ENUM_VARIANTS: Option<&'static [&'static str]> = None;
            }
        )*
    };
}

impl_enum_provider_for_non_enum!(
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    isize,
    usize,
    f32,
    f64,
    bool,
    String,
    &str,
    Vec<u8>,
    Vec<i32>,
    Vec<i64>,
    Vec<Option<i64>>,
    Vec<String>,
    std::time::Duration,
    chrono::DateTime<chrono::Utc>,
    chrono::NaiveDateTime,
    serde_json::Value,
    uuid::Uuid,
);

/// 用于 insert/insert_or_update 的参数类型 trait
#[async_trait::async_trait]
pub trait Insertable {
    type Model: crate::model::Model;
    fn as_refs(&self) -> Vec<&Self::Model>;
    fn as_refs_mut(&mut self) -> Vec<&mut Self::Model>;

    /// Run insert hooks for mutable hook-aware inputs.
    ///
    /// Immutable inputs intentionally keep the historical no-hook behavior.
    async fn run_before_insert(
        &mut self,
        _ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn run_after_insert(
        &self,
        _ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
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

/// A mutable single model opts into automatic insert hooks.
#[async_trait::async_trait]
impl<T> Insertable for &mut T
where
    T: crate::model::Model + crate::hooks::BeforeInsert + crate::hooks::AfterInsert + Send + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        vec![&**self]
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        vec![&mut **self]
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        let mut ctx = ctx;
        (**self).before_insert(&mut ctx).await
    }

    async fn run_after_insert(
        &self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        let mut ctx = ctx;
        (**self).after_insert(&mut ctx).await
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

/// A mutable batch opts into automatic per-row insert hooks.
#[async_trait::async_trait]
impl<T> Insertable for &mut Vec<T>
where
    T: crate::model::Model + crate::hooks::BeforeInsert + crate::hooks::AfterInsert + Send + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(
        &self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
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

#[async_trait::async_trait]
impl<T> Insertable for &mut [T]
where
    T: crate::model::Model + crate::hooks::BeforeInsert + crate::hooks::AfterInsert + Send + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(
        &self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
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

#[async_trait::async_trait]
impl<T, const N: usize> Insertable for &mut [T; N]
where
    T: crate::model::Model + crate::hooks::BeforeInsert + crate::hooks::AfterInsert + Send + Sync,
{
    type Model = T;

    fn as_refs(&self) -> Vec<&T> {
        self.iter().collect()
    }

    fn as_refs_mut(&mut self) -> Vec<&mut T> {
        self.iter_mut().collect()
    }

    async fn run_before_insert(
        &mut self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter_mut().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.before_insert(&mut row_ctx).await?;
        }
        Ok(())
    }

    async fn run_after_insert(
        &self,
        ctx: crate::hooks::HookContext<'static>,
    ) -> anyhow::Result<()> {
        for (index, model) in self.iter().enumerate() {
            let mut row_ctx = ctx.for_batch(index);
            model.after_insert(&mut row_ctx).await?;
        }
        Ok(())
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
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl Insertable for &Vec<&$model_type> {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.as_slice().to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl<const N: usize> Insertable for &[&$model_type; N] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }

        impl Insertable for &[&$model_type] {
            type Model = $model_type;
            fn as_refs(&self) -> Vec<&$model_type> {
                self.to_vec()
            }
            fn as_refs_mut(&mut self) -> Vec<&mut $model_type> {
                vec![]
            }
        }
    };
}

pub fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn needs_identifier_quote(identifier: &str) -> bool {
    if identifier.is_empty() {
        return true;
    }

    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !(first == '_' || first.is_ascii_lowercase()) {
        return true;
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_lowercase() || ch.is_ascii_digit())) {
        return true;
    }

    matches!(
        identifier,
        "all"
            | "and"
            | "as"
            | "by"
            | "check"
            | "column"
            | "constraint"
            | "create"
            | "default"
            | "delete"
            | "desc"
            | "from"
            | "group"
            | "index"
            | "insert"
            | "into"
            | "key"
            | "not"
            | "null"
            | "order"
            | "primary"
            | "references"
            | "select"
            | "table"
            | "unique"
            | "update"
            | "user"
            | "where"
    )
}

pub fn quote_identifier(db_type: crate::abstract_layer::DbType, identifier: &str) -> String {
    if !needs_identifier_quote(identifier) {
        return identifier.to_string();
    }

    match db_type {
        #[cfg(feature = "mysql")]
        crate::abstract_layer::DbType::MySQL => {
            format!("`{}`", identifier.replace('`', "``"))
        }
        #[cfg(feature = "mssql")]
        crate::abstract_layer::DbType::MSSQL => {
            format!("[{}]", identifier.replace(']', "]]"))
        }
        #[cfg(feature = "sqlite")]
        crate::abstract_layer::DbType::Sqlite => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
        #[cfg(feature = "postgresql")]
        crate::abstract_layer::DbType::PostgreSQL => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
    }
}

pub fn quote_qualified_identifier(db_type: crate::abstract_layer::DbType, name: &str) -> String {
    name.split('.')
        .map(|part| quote_identifier(db_type, part))
        .collect::<Vec<_>>()
        .join(".")
}

pub fn quote_column_reference(db_type: crate::abstract_layer::DbType, column: &str) -> String {
    if column.contains('(') || column.contains(' ') {
        return column.to_string();
    }
    quote_qualified_identifier(db_type, column)
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
    let table_name = normalize_table_name_for_db(db_type, table_name.unwrap_or(T::TABLE_NAME));
    let quoted_table_name = quote_qualified_identifier(db_type, table_name);
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (", quoted_table_name);

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

        // 添加压缩属性（仅 PostgreSQL 支持，且必须在 NOT NULL 之前）
        let is_postgresql = {
            #[cfg(feature = "postgresql")]
            {
                matches!(db_type, crate::abstract_layer::DbType::PostgreSQL)
            }
            #[cfg(not(feature = "postgresql"))]
            {
                false
            }
        };
        if column.compress && is_postgresql {
            if sql_type.ends_with(" NOT NULL") {
                let base = &sql_type[..sql_type.len() - " NOT NULL".len()];
                sql.push_str(&format!(
                    "{} {base} COMPRESSION pglz NOT NULL",
                    quote_identifier(db_type, column.name)
                ));
            } else {
                sql.push_str(&format!(
                    "{} {sql_type} COMPRESSION pglz",
                    quote_identifier(db_type, column.name)
                ));
            }
        } else {
            sql.push_str(&format!(
                "{} {sql_type}",
                quote_identifier(db_type, column.name)
            ));
        }

        if let Some(default) = column.default {
            sql.push_str(" DEFAULT ");
            sql.push_str(&default.to_sql(db_type));
        }

        // 添加单列 UNIQUE 约束（group 中只有一个字段的情况）
        if column.unique_group.is_some() {
            // 检查这个 group 中是否有多个字段
            let group_count = T::COLUMN_SCHEMA
                .iter()
                .filter(|c| c.unique_group == column.unique_group)
                .count();

            if group_count == 1 {
                // 单列唯一约束
                if column.unique_name.is_none() {
                    sql.push_str(" UNIQUE");
                }
            }
        }

        if let Some(check) = column.check {
            sql.push(' ');
            if let Some(name) = check.name {
                sql.push_str(&format!("CONSTRAINT {} ", quote_identifier(db_type, name)));
            }
            sql.push_str(&format!("CHECK ({})", check.expr));
        }
    }

    // 添加外键约束
    let foreign_key_constraints = generate_foreign_key_constraints::<T>(db_type);
    if !foreign_key_constraints.is_empty() {
        sql.push_str(", ");
        sql.push_str(&foreign_key_constraints.join(", "));
    }

    // 添加复合主键约束（如果有多个主键字段）
    let composite_primary_constraint = generate_composite_primary_key_constraint::<T>(db_type);
    if !composite_primary_constraint.is_empty() {
        sql.push_str(", ");
        sql.push_str(&composite_primary_constraint);
    }

    // 添加联合 UNIQUE 约束
    let unique_constraints = generate_unique_constraints::<T>(db_type);
    if !unique_constraints.is_empty() {
        sql.push_str(", ");
        sql.push_str(&unique_constraints.join(", "));
    }

    sql.push(')');

    // 添加索引
    let index_sql = generate_indexes_with_name::<T>(db_type, table_name)?;
    if !index_sql.is_empty() {
        sql.push(';');
        sql.push_str(&index_sql);
    }

    Ok(sql)
}

/// 生成 UNIQUE 约束
fn generate_unique_constraints<T: Model>(db_type: crate::abstract_layer::DbType) -> Vec<String> {
    let mut constraints = Vec::new();

    // 收集所有 unique_group
    let mut group_map: std::collections::BTreeMap<i32, Vec<&ColumnSchema>> =
        std::collections::BTreeMap::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if let Some(group_id) = column.unique_group {
            group_map.entry(group_id).or_default().push(column);
        }
    }

    // 生成约束
    for (_group_id, columns) in group_map {
        let unique_name = columns.iter().find_map(|column| column.unique_name);
        if columns.len() == 1 && unique_name.is_none() {
            // 单列唯一约束已经在列定义中处理
        } else {
            // 联合唯一约束
            let cols = columns
                .iter()
                .map(|column| quote_identifier(db_type, column.name))
                .collect::<Vec<_>>()
                .join(", ");
            let prefix = unique_name
                .map(|name| format!("CONSTRAINT {} ", quote_identifier(db_type, name)))
                .unwrap_or_default();
            constraints.push(format!("{prefix}UNIQUE ({cols})"));
        }
    }

    constraints
}

/// 生成索引 SQL，支持自定义表名
fn generate_indexes_with_name<T: Model>(
    db_type: crate::abstract_layer::DbType,
    table_name: &str,
) -> anyhow::Result<String> {
    let mut sqls = Vec::new();

    // 检查是否为 MySQL 数据库（通过调试字符串）
    let is_mysql = format!("{:?}", db_type).contains("MySQL");

    let mut grouped_indexes: std::collections::BTreeMap<i32, Vec<&ColumnSchema>> =
        std::collections::BTreeMap::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if !column.is_indexed {
            continue;
        }

        if let Some(group_id) = column.index_group {
            grouped_indexes.entry(group_id).or_default().push(column);
            continue;
        }

        let index_name = column
            .index_name
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("idx_{}_{}", table_name.replace('.', "_"), column.name));
        sqls.push(render_index_sql(
            db_type,
            is_mysql,
            table_name,
            &index_name,
            &[column],
        )?);
    }

    for (group_id, columns) in grouped_indexes {
        if columns.is_empty() {
            continue;
        }

        let index_name = columns
            .iter()
            .find_map(|column| column.index_name)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("idx_{}_{}", table_name.replace('.', "_"), group_id));
        sqls.push(render_index_sql(
            db_type,
            is_mysql,
            table_name,
            &index_name,
            &columns,
        )?);
    }

    Ok(sqls.join(";"))
}

fn render_index_sql(
    db_type: crate::abstract_layer::DbType,
    is_mysql: bool,
    table_name: &str,
    index_name: &str,
    columns: &[&ColumnSchema],
) -> anyhow::Result<String> {
    let where_clause = columns.iter().find_map(|column| column.index_where);
    if where_clause.is_some() && is_mysql {
        return Err(anyhow::anyhow!(
            "MySQL does not support partial index WHERE clauses"
        ));
    }

    let columns_sql = columns
        .iter()
        .map(|column| {
            let mut col = quote_identifier(db_type, column.name);
            if let Some(order) = column.index_order {
                col.push(' ');
                col.push_str(order);
            }
            col
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = if is_mysql {
        format!(
            "CREATE INDEX {} ON {} ({})",
            quote_identifier(db_type, index_name),
            quote_qualified_identifier(db_type, table_name),
            columns_sql
        )
    } else {
        format!(
            "CREATE INDEX IF NOT EXISTS {} ON {} ({})",
            quote_identifier(db_type, index_name),
            quote_qualified_identifier(db_type, table_name),
            columns_sql
        )
    };

    Ok(if let Some(where_clause) = where_clause {
        format!("{sql} WHERE {where_clause}")
    } else {
        sql
    })
}

/// 生成外键约束 SQL
fn generate_foreign_key_constraints<T: Model>(
    db_type: crate::abstract_layer::DbType,
) -> Vec<String> {
    let mut constraints = Vec::new();

    for column in T::COLUMN_SCHEMA.iter() {
        if let Some(fk) = &column.foreign_key {
            let ref_column = fk.get_ref_column();
            let ref_table = normalize_table_name_for_db(db_type, fk.ref_table);
            let mut constraint = String::new();
            if let Some(name) = fk.name {
                constraint.push_str(&format!("CONSTRAINT {} ", quote_identifier(db_type, name)));
            }
            constraint.push_str(&format!(
                "FOREIGN KEY ({}) REFERENCES {} ({})",
                quote_identifier(db_type, column.name),
                quote_qualified_identifier(db_type, ref_table),
                quote_identifier(db_type, ref_column)
            ));
            if let Some(action) = fk.on_delete {
                constraint.push_str(&format!(" ON DELETE {}", action.as_sql()));
            }
            if let Some(action) = fk.on_update {
                constraint.push_str(&format!(" ON UPDATE {}", action.as_sql()));
            }
            constraints.push(constraint);
        }
    }

    constraints
}

/// 生成复合主键约束 SQL
fn generate_composite_primary_key_constraint<T: Model>(
    db_type: crate::abstract_layer::DbType,
) -> String {
    let primary_keys: Vec<&str> = T::COLUMN_SCHEMA
        .iter()
        .filter(|c| c.is_primary)
        .map(|c| c.name)
        .collect();

    if primary_keys.len() > 1 {
        // 复合主键：PRIMARY KEY (col1, col2, ...)
        format!(
            "PRIMARY KEY ({})",
            primary_keys
                .into_iter()
                .map(|column| quote_identifier(db_type, column))
                .collect::<Vec<_>>()
                .join(", ")
        )
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

pub(crate) fn normalize_string_vec(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn parse_string_vec_text(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    if let Ok(values) = serde_json::from_str::<Vec<String>>(raw) {
        return normalize_string_vec(values);
    }

    vec![raw.to_string()]
}

#[cfg(any(feature = "sqlite", feature = "mysql", feature = "mssql"))]
pub(crate) fn stringify_string_vec(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

/// 值类型
#[derive(Debug, Clone)]
pub enum Value {
    Integer(i64),
    BigInt(i128),
    Duration(std::time::Duration),
    Text(String),
    TextArray(Vec<String>),
    Real(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
    IntegerArray(Vec<i32>),
    BigIntArray(Vec<i64>),
    NullableBigIntArray(Vec<Option<i64>>),
    DateTime(chrono::DateTime<chrono::Utc>),
    Json(serde_json::Value),
    Uuid(uuid::Uuid),
    Null,
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> anyhow::Result<Self>;
}

pub fn downcast_relation_vec_as<Concrete: Model + 'static, Target: Model + 'static>(
    values: Vec<Target>,
) -> anyhow::Result<Vec<Concrete>> {
    let boxed: Box<dyn Any> = Box::new(values);
    boxed
        .downcast::<Vec<Concrete>>()
        .map(|values| *values)
        .map_err(|_| anyhow::anyhow!("Relation target type mismatch"))
}

#[doc(hidden)]
pub struct I32DataTypeEncoder<T>(std::marker::PhantomData<T>);

impl<T> I32DataTypeEncoder<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for I32DataTypeEncoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub trait I32DataTypeEncode<T> {
    fn encode(self, value: T, column_name: &'static str, target_type: &'static str) -> i32;
}

impl<T> I32DataTypeEncode<T> for I32DataTypeEncoder<T>
where
    T: Into<i32>,
{
    fn encode(self, value: T, _column_name: &'static str, _target_type: &'static str) -> i32 {
        value.into()
    }
}

impl<T> I32DataTypeEncode<T> for &I32DataTypeEncoder<T>
where
    T: Copy,
{
    fn encode(self, value: T, column_name: &'static str, target_type: &'static str) -> i32 {
        if std::mem::size_of::<T>() != std::mem::size_of::<i32>() {
            panic!(
                "Failed to convert column '{}' from {}: target type must be i32-sized when it does not implement Into<i32>",
                column_name, target_type
            );
        }

        // Fallback for #[data_type(i32)] C-like enums that intentionally do not
        // implement Into<i32>. Valid discriminants are the caller's contract.
        unsafe { std::mem::transmute_copy::<T, i32>(&value) }
    }
}

#[doc(hidden)]
pub struct I32DataTypeDecoder<T>(std::marker::PhantomData<T>);

impl<T> I32DataTypeDecoder<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for I32DataTypeDecoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub trait I32DataTypeDecode<T> {
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> anyhow::Result<T>;
}

impl<T> I32DataTypeDecode<T> for I32DataTypeDecoder<T>
where
    T: TryFrom<i32>,
    <T as TryFrom<i32>>::Error: std::fmt::Display,
{
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> anyhow::Result<T> {
        T::try_from(value).map_err(|err| {
            anyhow::anyhow!(
                "Failed to convert column '{}' to {}: {}",
                column_name,
                target_type,
                err
            )
        })
    }
}

impl<T> I32DataTypeDecode<T> for &I32DataTypeDecoder<T>
where
    T: Copy,
{
    fn decode(
        self,
        value: i32,
        column_name: &'static str,
        target_type: &'static str,
    ) -> anyhow::Result<T> {
        if std::mem::size_of::<T>() != std::mem::size_of::<i32>() {
            return Err(anyhow::anyhow!(
                "Failed to convert column '{}' to {}: target type must be i32-sized when it does not implement TryFrom<i32>",
                column_name,
                target_type
            ));
        }

        // Fallback for #[data_type(i32)] C-like enums that intentionally do not
        // implement TryFrom<i32>. Valid discriminants are the caller's contract.
        Ok(unsafe { std::mem::transmute_copy::<i32, T>(&value) })
    }
}

/// FromRowValues trait - 用于从一行中的多个值构建类型(如元组、Model)
pub trait FromRowValues: Sized {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self>;
}

impl<T: Model> FromRowValues for T {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        T::from_row_values(values)
    }
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

impl FromValue for std::time::Duration {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Duration(v) => Ok(*v),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Duration")),
        }
    }
}

impl FromRowValues for std::time::Duration {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Duration"));
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

impl From<Vec<String>> for Value {
    fn from(v: Vec<String>) -> Self {
        Value::TextArray(v)
    }
}

impl FromValue for Vec<String> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::TextArray(v) => Ok(normalize_string_vec(v.clone())),
            Value::Text(v) => Ok(parse_string_vec_text(v)),
            Value::Json(v) => {
                if v.is_null() {
                    return Ok(Vec::new());
                }
                if let Some(user) = v.as_str() {
                    let user = user.trim();
                    return Ok((!user.is_empty())
                        .then(|| user.to_string())
                        .into_iter()
                        .collect());
                }
                let users = serde_json::from_value::<Vec<String>>(v.clone())?;
                Ok(normalize_string_vec(users))
            }
            _ => Err(anyhow::anyhow!("Type mismatch: expected Vec<String>")),
        }
    }
}

impl From<Option<Vec<String>>> for Value {
    fn from(v: Option<Vec<String>>) -> Self {
        match v {
            Some(values) => Value::TextArray(values),
            None => Value::Null,
        }
    }
}

impl FromRowValues for Vec<String> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Vec<String>"));
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

impl From<std::time::Duration> for Value {
    fn from(v: std::time::Duration) -> Self {
        Value::Duration(v)
    }
}

// String 特殊处理
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
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

impl From<Option<std::time::Duration>> for Value {
    fn from(v: Option<std::time::Duration>) -> Self {
        match v {
            Some(duration) => Value::Duration(duration),
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
            crate::query::filter::Value::Duration(v) => Value::Duration(v),
            crate::query::filter::Value::Text(v) => Value::Text(v),
            crate::query::filter::Value::TextArray(v) => Value::TextArray(v),
            crate::query::filter::Value::Real(v) => Value::Real(v),
            crate::query::filter::Value::Boolean(v) => Value::Boolean(v),
            crate::query::filter::Value::Bytes(v) => Value::Bytes(v),
            crate::query::filter::Value::IntegerArray(v) => Value::IntegerArray(v),
            crate::query::filter::Value::BigIntArray(v) => Value::BigIntArray(v),
            crate::query::filter::Value::NullableBigIntArray(v) => Value::NullableBigIntArray(v),
            crate::query::filter::Value::DateTime(v) => Value::DateTime(v),
            crate::query::filter::Value::Json(v) => Value::Json(v),
            crate::query::filter::Value::Uuid(v) => Value::Uuid(v),
            crate::query::filter::Value::Null => Value::Null,
        }
    }
}

impl FromValue for Option<std::time::Duration> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::Duration(v) => Ok(Some(*v)),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Option<Duration>")),
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

impl FromRowValues for Vec<u8> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Vec<u8>"));
        }
        Self::from_value(&values[0])
    }
}

impl From<Vec<i32>> for Value {
    fn from(v: Vec<i32>) -> Self {
        Value::IntegerArray(v)
    }
}

impl FromValue for Vec<i32> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::IntegerArray(v) => Ok(v.clone()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Vec<i32>")),
        }
    }
}

impl FromRowValues for Vec<i32> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Vec<i32>"));
        }
        Self::from_value(&values[0])
    }
}

impl From<Vec<i64>> for Value {
    fn from(v: Vec<i64>) -> Self {
        Value::BigIntArray(v)
    }
}

impl FromValue for Vec<i64> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::BigIntArray(v) => Ok(v.clone()),
            Value::NullableBigIntArray(v) => v
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| anyhow::anyhow!("Type mismatch: expected Vec<i64>")),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Vec<i64>")),
        }
    }
}

impl FromRowValues for Vec<i64> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Vec<i64>"));
        }
        Self::from_value(&values[0])
    }
}

impl From<Vec<Option<i64>>> for Value {
    fn from(v: Vec<Option<i64>>) -> Self {
        Value::NullableBigIntArray(v)
    }
}

impl FromValue for Vec<Option<i64>> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::BigIntArray(v) => Ok(v.iter().copied().map(Some).collect()),
            Value::NullableBigIntArray(v) => Ok(v.clone()),
            _ => Err(anyhow::anyhow!("Type mismatch: expected Vec<Option<i64>>")),
        }
    }
}

impl FromRowValues for Vec<Option<i64>> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected Vec<Option<i64>>"));
        }
        Self::from_value(&values[0])
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

impl FromRowValues for chrono::DateTime<chrono::Utc> {
    fn from_row_values(values: &[Value]) -> anyhow::Result<Self> {
        if values.is_empty() {
            return Err(anyhow::anyhow!("Type mismatch: expected DateTime<Utc>"));
        }
        Self::from_value(&values[0])
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
        Value::DateTime(naive_local_to_utc(v))
    }
}

impl FromValue for chrono::NaiveDateTime {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::DateTime(v) => Ok(utc_to_naive_local(*v)),
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
            Some(dt) => Value::DateTime(naive_local_to_utc(dt)),
            None => Value::Null,
        }
    }
}

impl FromValue for Option<chrono::NaiveDateTime> {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Null => Ok(None),
            Value::DateTime(v) => Ok(Some(utc_to_naive_local(*v))),
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
