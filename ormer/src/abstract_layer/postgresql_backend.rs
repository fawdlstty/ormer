use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{Select, WhereExpr};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio_postgres::NoTls;

/// PostgreSQL 类型映射器
pub struct PostgreSQLTypeMapper;

impl DbBackendTypeMapper for PostgreSQLTypeMapper {
    fn sql_type(rust_type: &str, is_primary: bool, is_nullable: bool) -> String {
        // 首先处理主键类型（主键自动 NOT NULL）
        if is_primary {
            let serial_type = match rust_type {
                "i8" | "i16" | "i32" => "SERIAL",
                "i64" | "u16" | "u32" | "u64" => "BIGSERIAL",
                "u8" => "SMALLSERIAL", // PostgreSQL 最小序列类型
                _ => "SERIAL",         // 默认使用 SERIAL
            };
            return format!("{} PRIMARY KEY", serial_type);
        }

        // 基础类型映射
        let base_type = match rust_type {
            // 整数类型
            "i8" => "SMALLINT",
            "i16" => "SMALLINT",
            "i32" => "INTEGER",
            "i64" => "BIGINT",
            // 无符号整数（PostgreSQL 不原生支持，使用有符号类型模拟）
            "u8" => "SMALLINT",
            "u16" => "INTEGER",
            "u32" => "BIGINT",
            "u64" => "BIGINT",
            // 浮点类型
            "f32" => "REAL",
            "f64" => "DOUBLE PRECISION",
            // 字符串类型
            "String" => "VARCHAR",
            // 布尔类型
            "bool" => "BOOLEAN",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BYTEA",
            // UUID 类型（如果使用 uuid crate）
            "Uuid" | "uuid::Uuid" => "UUID",
            // 日期时间类型（如果使用 chrono crate）
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                "TIMESTAMP"
            }
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            // JSON 类型
            "JsonValue" | "serde_json::Value" => "JSONB",
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

/// PostgreSQL 数据库连接封装
pub struct Database {
    client: tokio_postgres::Client,
    connection_handle: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}

impl Database {
    /// 连接到 PostgreSQL 数据库
    pub async fn connect(
        _db_type: super::DbType,
        connection_string: &str,
    ) -> Result<Self, crate::Error> {
        let (client, connection) = tokio_postgres::connect(connection_string, NoTls)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 在后台运行连接
        let connection_handle = tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("PostgreSQL connection error: {}", e);
            }
            Ok(())
        });

        Ok(Self {
            client,
            connection_handle,
        })
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
            crate::generate_create_table_sql::<T>(crate::abstract_layer::DbType::PostgreSQL);

        self.client
            .execute(&create_sql, &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> Result<bool, crate::Error> {
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_type='BASE TABLE' AND table_name=$1";

        let row = self
            .client
            .query_one(sql, &[&T::TABLE_NAME])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let count: i64 = row
            .try_get(0)
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(count > 0)
    }

    /// 验证表结构是否与模型定义匹配
    async fn validate_table_schema<T: Model>(&self) -> Result<(), crate::Error> {
        // 查询表的列信息
        let sql = r#"
            SELECT column_name, data_type, is_nullable
            FROM information_schema.columns
            WHERE table_name = $1
            ORDER BY ordinal_position
        "#;

        let rows = self
            .client
            .query(sql, &[&T::TABLE_NAME])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool)> = Vec::new();
        for row in rows {
            let name: String = row
                .try_get(0)
                .map_err(|e| crate::Error::Database(e.to_string()))?;
            let col_type: String = row
                .try_get(1)
                .map_err(|e| crate::Error::Database(e.to_string()))?;
            let is_nullable: String = row
                .try_get(2)
                .map_err(|e| crate::Error::Database(e.to_string()))?;

            actual_columns.push((name, col_type, is_nullable == "YES"));
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

            let (actual_name, actual_type, actual_nullable) = &actual_columns[i];

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

            // 检查列类型（只比较基础类型，不包含约束）
            let expected_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_nullable,
            );

            // 对于类型比较，我们需要提取基础类型（不包含 SERIAL, PRIMARY KEY, NOT NULL 等约束）
            let type_to_compare = if expected_col.is_primary {
                // 主键的基础类型
                match expected_col.rust_type {
                    "i8" | "i16" | "i32" => "SMALLINT".to_string(),
                    "i64" | "u16" | "u32" | "u64" => "BIGINT".to_string(),
                    "u8" => "SMALLINT".to_string(),
                    _ => "INTEGER".to_string(),
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
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

            // 检查 NOT NULL 约束（主键列除外，因为主键自动 NOT NULL）
            if !expected_col.is_primary {
                let expected_nullable = expected_col.is_nullable;
                if *actual_nullable != expected_nullable {
                    return Err(crate::Error::SchemaMismatch {
                        table: T::TABLE_NAME.to_string(),
                        reason: format!(
                            "Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                            expected_col.name,
                            if expected_nullable { "" } else { "NOT " },
                            if *actual_nullable { "" } else { "NOT " }
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// 检查 SQL 类型是否兼容
    fn types_compatible(&self, actual: &str, expected: &str) -> bool {
        // 标准化类型名称
        fn normalize(s: &str) -> String {
            let upper = s.to_uppercase();
            match upper.as_str() {
                // 整数类型
                "SMALLINT" | "INT2" => "SMALLINT".to_string(),
                "INTEGER" | "INT" | "INT4" | "SERIAL" => "INTEGER".to_string(),
                "BIGINT" | "INT8" | "BIGSERIAL" => "BIGINT".to_string(),
                // 字符串类型
                "CHARACTER VARYING" | "VARCHAR" | "TEXT" | "CHAR" | "CHARACTER" | "BPCHAR" => {
                    "VARCHAR".to_string()
                }
                // 布尔类型
                "BOOLEAN" | "BOOL" => "BOOLEAN".to_string(),
                // 浮点类型
                "REAL" | "FLOAT4" => "REAL".to_string(),
                "DOUBLE PRECISION" | "FLOAT8" | "FLOAT" => "DOUBLE PRECISION".to_string(),
                // 字节类型
                "BYTEA" | "BLOB" => "BYTEA".to_string(),
                // 其他
                _ => upper,
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        let columns = T::COLUMNS.join(", ");
        let placeholders: Vec<String> = (1..=T::COLUMNS.len()).map(|i| format!("${}", i)).collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            T::TABLE_NAME,
            columns,
            placeholders.join(", ")
        );

        let values = model.field_values();
        let params = values_to_params(&values)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        self.client
            .execute(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<T> {
        SelectExecutor {
            select: Select::<T>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<T> {
        DeleteExecutor {
            filters: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 添加 WHERE 条件
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
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
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 限制结果数量
    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 设置偏移量
    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<C>,
}

impl<'a, T: Model + 'static, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<'a, T, C>
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        // 将 SQL 从 Turso 格式转换为 PostgreSQL 格式 (?N -> $N)
        let pg_sql = convert_sql_to_pg(&sql);

        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self
            .client
            .query(&pg_sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();

        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                // 根据列的类型获取值
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.rust_type;
                let is_nullable = column_info.is_nullable;

                let ormer_value = if is_nullable {
                    // 处理可空类型
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: Option<i64> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Integer(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "String" => {
                            let v: Option<String> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Text(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "f32" | "f64" => {
                            let v: Option<f64> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Real(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "bool" => {
                            let v: Option<bool> = row.get(i);
                            match v {
                                Some(true) => crate::model::Value::Integer(1),
                                Some(false) => crate::model::Value::Integer(0),
                                None => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported nullable column type: {}",
                                rust_type
                            )));
                        }
                    }
                } else {
                    // 处理非空类型
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: i64 = row.get(i);
                            crate::model::Value::Integer(v)
                        }
                        "String" => {
                            let v: String = row.get(i);
                            crate::model::Value::Text(v)
                        }
                        "f32" | "f64" => {
                            let v: f64 = row.get(i);
                            crate::model::Value::Real(v)
                        }
                        "bool" => {
                            let v: bool = row.get(i);
                            if v {
                                crate::model::Value::Integer(1)
                            } else {
                                crate::model::Value::Integer(0)
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported column type: {}",
                                rust_type
                            )));
                        }
                    }
                };
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
pub struct DeleteExecutor<'a, T: Model> {
    filters: Vec<FilterExpr>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> DeleteExecutor<'a, T> {
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
        let pg_sql = convert_sql_to_pg(&sql);

        let result = self
            .client
            .execute(&pg_sql, &[])
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

impl<'a, T: Model + 'static> std::future::IntoFuture for DeleteExecutor<'a, T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<(String, Value)>,
    filters: Vec<FilterExpr>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> UpdateExecutor<'a, T> {
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
        let pg_sql = convert_sql_to_pg(&sql);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let result = self
            .client
            .execute(&pg_sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    fn build_sql(&self) -> Result<(String, Vec<crate::model::Value>), crate::Error> {
        let mut sql = format!("UPDATE {} SET ", T::TABLE_NAME);
        let mut params = Vec::new();

        // 构建 SET 子句
        let mut first = true;
        for (col_name, value) in &self.sets {
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = ${}", col_name, params.len() + 1));
            params.push(value.clone());
            first = false;
        }

        // 构建 WHERE 子句
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = params.len() + 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                format_filter_with_params(filter, &mut sql, &mut param_idx, &mut params);
            }
        }

        Ok((sql, params))
    }
}

impl<'a, T: Model + 'static> std::future::IntoFuture for UpdateExecutor<'a, T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 tokio-postgres 参数
fn values_to_params(
    values: &[crate::model::Value],
) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync>>, crate::Error> {
    use tokio_postgres::types::{ToSql, Type};

    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();

    for value in values {
        let param: Box<dyn tokio_postgres::types::ToSql + Sync> = match value {
            crate::model::Value::Integer(v) => Box::new(*v),
            crate::model::Value::Text(v) => Box::new(v.clone()),
            crate::model::Value::Real(v) => Box::new(*v),
            crate::model::Value::Null => {
                // 使用 Option<i64> 的 None 来表示 NULL
                let null_val: Option<i64> = None;
                Box::new(null_val)
            }
        };
        params.push(param);
    }

    Ok(params)
}

/// 将 SQL 从 Turso 格式转换为 PostgreSQL 格式
/// Turso: ?1, ?2, ?3
/// PostgreSQL: $1, $2, $3
fn convert_sql_to_pg(sql: &str) -> String {
    // 简单的替换，将 ?N 替换为 $N
    let mut result = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '?' {
            result.push('$');
        } else {
            result.push(c);
        }
    }

    result
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
    params: &mut Vec<crate::model::Value>,
) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            use std::fmt::Write;
            write!(sql, "{} {} ${}", column, operator, param_idx).unwrap();
            params.push(value.clone().into());
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
