use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{Select, WhereExpr};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// Turso 类型映射器
pub struct TursoTypeMapper;

impl DbBackendTypeMapper for TursoTypeMapper {
    fn sql_type(rust_type: &str, is_primary: bool, is_nullable: bool) -> String {
        // 首先处理主键类型
        if is_primary {
            return "INTEGER PRIMARY KEY".to_string();
        }

        // 基础类型映射（SQLite/Turso 类型系统更简单）
        let base_type = match rust_type {
            // 整数类型
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => "INTEGER",
            // 浮点类型
            "f32" | "f64" => "REAL",
            // 字符串类型
            "String" => "TEXT",
            // 布尔类型（SQLite 没有原生 bool，用 INTEGER 存储）
            "bool" => "INTEGER",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BLOB",
            // 日期时间类型（SQLite 存储为 TEXT 或 INTEGER）
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => "TEXT",
            "NaiveDate" | "chrono::NaiveDate" => "TEXT",
            "NaiveTime" | "chrono::NaiveTime" => "TEXT",
            // JSON 类型（SQLite 存储为 TEXT）
            "JsonValue" | "serde_json::Value" => "TEXT",
            // 默认使用 TEXT
            _ => "TEXT",
        };

        let mut sql_type = base_type.to_string();

        // 非主键字段根据 is_nullable 决定是否添加 NOT NULL
        if !is_nullable {
            sql_type.push_str(" NOT NULL");
        }

        sql_type
    }
}

/// Turso 数据库连接封装
pub struct Database {
    conn: Arc<turso::Connection>,
}

impl Database {
    /// 连接到 Turso 数据库 (本地模式)
    pub async fn connect(_db_type: super::DbType, path: &str) -> Result<Self, crate::Error> {
        let db = turso::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let conn = Arc::new(
            db.connect()
                .map_err(|e| crate::Error::Database(e.to_string()))?,
        );

        Ok(Self { conn })
    }

    /// 创建表
    pub async fn create_table<T: Model>(&self) -> Result<(), crate::Error> {
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().await?;

        if table_exists {
            // 表已存在，验证表结构
            self.validate_table_schema::<T>().await?;
            // 结构匹配，无需创建
            return Ok(());
        }

        // 表不存在，创建新表
        let create_sql =
            crate::generate_create_table_sql::<T>(crate::abstract_layer::DbType::Turso);

        self.conn
            .execute(&create_sql, ())
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> Result<bool, crate::Error> {
        let sql = "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1";

        let mut rows = self
            .conn
            .query(sql, [T::TABLE_NAME])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let count = row
                .get_value(0)
                .map_err(|e| crate::Error::Database(e.to_string()))?;

            match count {
                turso::Value::Integer(c) => Ok(c > 0),
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    /// 验证表结构是否与模型定义匹配
    async fn validate_table_schema<T: Model>(&self) -> Result<(), crate::Error> {
        // 查询表的列信息
        let sql = format!("PRAGMA table_info({})", T::TABLE_NAME);

        let mut rows = self
            .conn
            .query(&sql, ())
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool)> = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let name = row
                .get_value(1)
                .map_err(|e| crate::Error::Database(e.to_string()))?;
            let col_type = row
                .get_value(2)
                .map_err(|e| crate::Error::Database(e.to_string()))?;
            let notnull = row
                .get_value(3)
                .map_err(|e| crate::Error::Database(e.to_string()))?;
            let pk = row
                .get_value(5)
                .map_err(|e| crate::Error::Database(e.to_string()))?;

            if let (
                turso::Value::Text(name),
                turso::Value::Text(col_type),
                turso::Value::Integer(notnull),
                turso::Value::Integer(pk),
            ) = (name, col_type, notnull, pk)
            {
                actual_columns.push((name, col_type, notnull != 0, pk != 0));
            }
        }

        // 比较列数量
        if actual_columns.len() != T::COLUMNS.len() {
            return Err(crate::Error::SchemaMismatch {
                table: T::TABLE_NAME.to_string(),
                reason: format!(
                    "Column count mismatch: expected {}, but actual is {}",
                    T::COLUMNS.len(),
                    actual_columns.len()
                ),
            });
        }

        // 比较每一列的定义
        for (i, expected_col) in T::COLUMN_SCHEMA.iter().enumerate() {
            if i >= actual_columns.len() {
                return Err(crate::Error::SchemaMismatch {
                    table: T::TABLE_NAME.to_string(),
                    reason: format!("Missing column: {}", expected_col.name),
                });
            }

            let (actual_name, actual_type, actual_notnull, actual_pk) = &actual_columns[i];

            // 检查列名
            if actual_name != expected_col.name {
                return Err(crate::Error::SchemaMismatch {
                    table: T::TABLE_NAME.to_string(),
                    reason: format!(
                        "Column name mismatch at position {}: expected '{}', but actual is '{}'",
                        i, expected_col.name, actual_name
                    ),
                });
            }

            // 检查主键约束
            if expected_col.is_primary != *actual_pk {
                return Err(crate::Error::SchemaMismatch {
                    table: T::TABLE_NAME.to_string(),
                    reason: format!(
                        "Primary key mismatch for '{}': expected {}primary key, but actual is {}primary key",
                        expected_col.name,
                        if expected_col.is_primary { "" } else { "not " },
                        if *actual_pk { "" } else { "not " }
                    ),
                });
            }

            // 检查列类型（只比较基础类型，不包含 NOT NULL 约束）
            let expected_type = crate::abstract_layer::DbType::Turso.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_nullable,
            );

            // 对于类型比较，我们需要提取基础类型（不包含约束）
            let type_to_compare = if expected_col.is_primary {
                // 主键的基础类型，不包含任何约束
                match expected_col.rust_type {
                    "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                        "INTEGER".to_string()
                    }
                    "f32" | "f64" => "REAL".to_string(),
                    "String" => "TEXT".to_string(),
                    "bool" => "INTEGER".to_string(),
                    "Vec<u8>" | "&[u8]" => "BLOB".to_string(),
                    _ => "TEXT".to_string(),
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::Turso.sql_type(
                    expected_col.rust_type,
                    false,
                    expected_col.is_nullable,
                );
                // 去掉 " NOT NULL" 后缀
                full_type.replace(" NOT NULL", "")
            };

            if !self.types_compatible(actual_type, &type_to_compare) {
                return Err(crate::Error::SchemaMismatch {
                    table: T::TABLE_NAME.to_string(),
                    reason: format!(
                        "Column type mismatch for '{}': expected '{}', but actual is '{}'",
                        expected_col.name, expected_type, actual_type
                    ),
                });
            }

            // 检查 NOT NULL 约束（主键列自动 NOT NULL，所以不需要额外检查）
            if !expected_col.is_primary {
                let expected_notnull = !expected_col.is_nullable;
                if *actual_notnull != expected_notnull {
                    return Err(crate::Error::SchemaMismatch {
                        table: T::TABLE_NAME.to_string(),
                        reason: format!(
                            "Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                            expected_col.name,
                            if expected_notnull { "NOT " } else { "" },
                            if *actual_notnull { "NOT " } else { "" }
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// 检查 SQL 类型是否兼容
    fn types_compatible(&self, actual: &str, expected: &str) -> bool {
        // 标准化类型名称（SQLite 类型别名）
        fn normalize(s: &str) -> String {
            match s.to_uppercase().as_str() {
                "INT" | "INTEGER" | "MEDIUMINT" | "BIGINT" | "INT64" => "INTEGER".to_string(),
                "VARCHAR" | "CHARACTER" | "NCHAR" | "NVARCHAR" | "TEXT" | "CLOB" => {
                    "TEXT".to_string()
                }
                "BLOB" => "BLOB".to_string(),
                "REAL" | "FLOAT" | "DOUBLE" | "DECIMAL" | "NUMERIC" => "REAL".to_string(),
                _ => s.to_string(),
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        let columns = T::COLUMNS.join(", ");
        let placeholders: Vec<String> = (1..=T::COLUMNS.len()).map(|i| format!("?{}", i)).collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            T::TABLE_NAME,
            columns,
            placeholders.join(", ")
        );

        let values = model.field_values();
        let params = values_to_params(&values)?;

        self.conn
            .execute(&sql, params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<T> {
        SelectExecutor {
            select: Select::<T>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<T> {
        DeleteExecutor {
            filters: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }
}

/// Select 查询执行器
pub struct SelectExecutor<T: Model> {
    select: Select<T>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

impl<T: Model> SelectExecutor<T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加排序
    pub fn order_by<F>(self, f: F) -> Self
    where
        F: FnOnce(crate::WhereColumn<T>) -> crate::OrderBy,
    {
        Self {
            select: self.select.order_by(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 限制结果数量
    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 设置偏移量
    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(self) -> CollectFuture<T, C> {
        CollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<T>,
    _marker: PhantomData<C>,
}

impl<T: Model + 'static, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<T, C>
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model> SelectExecutor<T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        // 将 ormer::Value 转换为 turso::Value
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn
                .query(&sql, ())
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?
        } else {
            self.conn
                .query(&sql, turso_params)
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?
        };

        let mut results = Vec::new();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row
                    .get_value(i)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;
                let ormer_value = convert_turso_value(&value)?;
                data.insert(col_name.to_string(), ormer_value);
            }

            let ormer_row = Row::new(data);
            let model = T::from_row(&ormer_row)?;
            results.push(model);
        }

        Ok(results.into_iter().collect())
    }
}

/// Delete 执行器
pub struct DeleteExecutor<T: Model> {
    filters: Vec<FilterExpr>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

impl<T: Model> DeleteExecutor<T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    /// 执行删除操作并返回影响的行数
    pub async fn execute(self) -> Result<u64, crate::Error> {
        let sql = self.build_sql();

        let result = self
            .conn
            .execute(&sql, ())
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    fn build_sql(&self) -> String {
        let mut sql = format!("DELETE FROM {}", T::TABLE_NAME);

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                format_filter(filter, &mut sql, &mut param_idx);
            }
        }

        sql
    }
}

impl<T: Model + 'static> std::future::IntoFuture for DeleteExecutor<T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<T: Model> {
    sets: Vec<(String, Value)>,
    filters: Vec<FilterExpr>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

impl<T: Model> UpdateExecutor<T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    /// 设置要更新的字段
    pub fn set<F, V>(mut self, field_fn: F, value: V) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::NumericColumn,
        V: Into<Value>,
    {
        let where_obj = T::Where::default();
        let column = field_fn(where_obj);
        let column_name = column.column_name().to_string();
        self.sets.push((column_name, value.into()));
        self
    }

    /// 执行更新操作
    pub async fn execute(self) -> Result<u64, crate::Error> {
        let (sql, params) = self.build_sql()?;

        let result = self
            .conn
            .execute(&sql, params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    fn build_sql(&self) -> Result<(String, Vec<turso::Value>), crate::Error> {
        let mut sql = format!("UPDATE {} SET ", T::TABLE_NAME);
        let mut ormer_params = Vec::new();

        // 构建 SET 子句
        let mut first = true;
        for (col_name, value) in &self.sets {
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = ?{}", col_name, ormer_params.len() + 1));
            ormer_params.push(value.clone());
            first = false;
        }

        // 构建 WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = ormer_params.len() + 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                format_filter_with_params(filter, &mut sql, &mut param_idx, &mut ormer_params);
            }
        }

        let turso_params = values_to_params(&ormer_params)?;
        Ok((sql, turso_params))
    }
}

impl<T: Model + 'static> std::future::IntoFuture for UpdateExecutor<T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 turso 参数
fn values_to_params(values: &[Value]) -> Result<Vec<turso::Value>, crate::Error> {
    let mut params = Vec::new();

    for value in values {
        let param = match value {
            Value::Integer(v) => turso::Value::Integer(*v),
            Value::Text(v) => turso::Value::Text(v.clone()),
            Value::Real(v) => turso::Value::Real(*v),
            Value::Null => turso::Value::Null,
        };
        params.push(param);
    }

    Ok(params)
}

/// 将 turso Value 转换为 ormer Value
fn convert_turso_value(value: &turso::Value) -> Result<Value, crate::Error> {
    match value {
        turso::Value::Integer(v) => Ok(Value::Integer(*v)),
        turso::Value::Text(v) => Ok(Value::Text(v.clone())),
        turso::Value::Real(v) => Ok(Value::Real(*v)),
        turso::Value::Null => Ok(Value::Null),
        _ => Err(crate::Error::Database(format!(
            "Unsupported turso value type: {:?}",
            value
        ))),
    }
}

/// 格式化过滤器 (不包含参数值,仅用于 DELETE)
fn format_filter(filter: &FilterExpr, sql: &mut String, param_idx: &mut i32) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value: _,
        } => {
            use std::fmt::Write;
            write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
            *param_idx += 1;
        }
        FilterExpr::And(left, right) => {
            format_filter(left, sql, param_idx);
            sql.push_str(" AND ");
            format_filter(right, sql, param_idx);
        }
        FilterExpr::Or(left, right) => {
            format_filter(left, sql, param_idx);
            sql.push_str(" OR ");
            format_filter(right, sql, param_idx);
        }
    }
}

/// 格式化过滤器并收集参数 (用于 UPDATE)
fn format_filter_with_params(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut usize,
    params: &mut Vec<Value>,
) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            use std::fmt::Write;
            write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
            // 将 filter::Value 转换为 ormer::Value
            let ormer_value = match value {
                crate::query::filter::Value::Integer(v) => Value::Integer(*v),
                crate::query::filter::Value::Text(v) => Value::Text(v.clone()),
                crate::query::filter::Value::Real(v) => Value::Real(*v),
                crate::query::filter::Value::Null => Value::Null,
            };
            params.push(ormer_value);
            *param_idx += 1;
        }
        FilterExpr::And(left, right) => {
            format_filter_with_params(left, sql, param_idx, params);
            sql.push_str(" AND ");
            format_filter_with_params(right, sql, param_idx, params);
        }
        FilterExpr::Or(left, right) => {
            format_filter_with_params(left, sql, param_idx, params);
            sql.push_str(" OR ");
            format_filter_with_params(right, sql, param_idx, params);
        }
    }
}
