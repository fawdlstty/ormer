use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{Select, WhereExpr};
use crate::query::filter::FilterExpr;
use mysql_async::Pool;
use mysql_async::prelude::*;
use std::collections::HashMap;
use std::marker::PhantomData;

/// MySQL 类型映射器
pub struct MySQLTypeMapper;

impl DbBackendTypeMapper for MySQLTypeMapper {
    fn sql_type(rust_type: &str, is_primary: bool, is_nullable: bool) -> String {
        // 首先处理主键类型
        if is_primary {
            let int_type = match rust_type {
                "i8" | "i16" | "u8" => "TINYINT",
                "i32" | "u16" => "INT",
                "i64" | "u32" | "u64" => "BIGINT",
                _ => "INT", // 默认使用 INT
            };
            return format!("{} PRIMARY KEY AUTO_INCREMENT", int_type);
        }

        // 基础类型映射
        let base_type = match rust_type {
            // 整数类型
            "i8" => "TINYINT",
            "i16" => "SMALLINT",
            "i32" => "INT",
            "i64" => "BIGINT",
            // 无符号整数
            "u8" => "TINYINT UNSIGNED",
            "u16" => "SMALLINT UNSIGNED",
            "u32" => "INT UNSIGNED",
            "u64" => "BIGINT UNSIGNED",
            // 浮点类型
            "f32" => "FLOAT",
            "f64" => "DOUBLE",
            // 字符串类型
            "String" => "VARCHAR(255)",
            // 布尔类型
            "bool" => "TINYINT(1)",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BLOB",
            // 日期时间类型
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                "DATETIME"
            }
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            // JSON 类型
            "JsonValue" | "serde_json::Value" => "JSON",
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

/// MySQL 数据库连接封装
pub struct Database {
    pool: Pool,
}

impl Database {
    /// 连接到 MySQL 数据库
    pub async fn connect(
        _db_type: super::DbType,
        connection_string: &str,
    ) -> Result<Self, crate::Error> {
        // 解析连接字符串
        let opts = mysql_async::Opts::from_url(connection_string)
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let pool = Pool::new(opts);

        Ok(Self { pool })
    }

    /// 创建表
    pub async fn create_table<T: Model>(&self) -> Result<(), crate::Error> {
        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>(&mut conn).await?;

        if table_exists {
            // 表已存在，验证表结构
            self.validate_table_schema::<T>(&mut conn).await?;
            // 结构匹配，无需创建
            return Ok(());
        }

        // 表不存在，创建新表
        let create_sql =
            crate::generate_create_table_sql::<T>(crate::abstract_layer::DbType::MySQL);

        conn.query_drop(&create_sql)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(
        &self,
        conn: &mut mysql_async::Conn,
    ) -> Result<bool, crate::Error> {
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?";

        let result: Option<u64> = conn
            .query_first(sql)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result.unwrap_or(0) > 0)
    }

    /// 验证表结构是否与模型定义匹配
    async fn validate_table_schema<T: Model>(
        &self,
        conn: &mut mysql_async::Conn,
    ) -> Result<(), crate::Error> {
        // 查询表的列信息
        let sql = r#"
            SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE
            FROM INFORMATION_SCHEMA.COLUMNS
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
            ORDER BY ORDINAL_POSITION
        "#;

        let rows: Vec<(String, String, String)> = conn
            .query_map(sql, (T::TABLE_NAME,), |(name, col_type, is_nullable)| {
                (name, col_type, is_nullable)
            })
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool)> = Vec::new();
        for (name, col_type, is_nullable) in rows {
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
            let expected_type = crate::abstract_layer::DbType::MySQL.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_nullable,
            );

            // 对于类型比较，我们需要提取基础类型（不包含 AUTO_INCREMENT, PRIMARY KEY, NOT NULL 等约束）
            let type_to_compare = if expected_col.is_primary {
                // 主键的基础类型
                match expected_col.rust_type {
                    "i8" | "i16" | "u8" => "TINYINT".to_string(),
                    "i32" | "u16" => "INT".to_string(),
                    "i64" | "u32" | "u64" => "BIGINT".to_string(),
                    _ => "INT".to_string(),
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::MySQL.sql_type(
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
                "TINYINT" => "TINYINT".to_string(),
                "SMALLINT" => "SMALLINT".to_string(),
                "MEDIUMINT" => "MEDIUMINT".to_string(),
                "INT" | "INTEGER" => "INT".to_string(),
                "BIGINT" => "BIGINT".to_string(),
                // 无符号整数
                "TINYINT UNSIGNED" => "TINYINT UNSIGNED".to_string(),
                "SMALLINT UNSIGNED" => "SMALLINT UNSIGNED".to_string(),
                "MEDIUMINT UNSIGNED" => "MEDIUMINT UNSIGNED".to_string(),
                "INT UNSIGNED" | "INTEGER UNSIGNED" => "INT UNSIGNED".to_string(),
                "BIGINT UNSIGNED" => "BIGINT UNSIGNED".to_string(),
                // 浮点类型
                "FLOAT" => "FLOAT".to_string(),
                "DOUBLE" | "DOUBLE PRECISION" => "DOUBLE".to_string(),
                // 字符串类型
                "VARCHAR" | "CHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" => {
                    "VARCHAR".to_string()
                }
                // 布尔类型
                "TINYINT(1)" | "BOOL" | "BOOLEAN" => "TINYINT(1)".to_string(),
                // 字节类型
                "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "VARBINARY" | "BINARY" => {
                    "BLOB".to_string()
                }
                // 其他
                _ => upper,
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let columns = T::COLUMNS.join(", ");
        let placeholders: Vec<String> = (1..=T::COLUMNS.len()).map(|_| "?".to_string()).collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            T::TABLE_NAME,
            columns,
            placeholders.join(", ")
        );

        let values = model.field_values();
        let params = values_to_params(&values)?;

        conn.exec_drop(&sql, params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: &self.pool,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            pool: &self.pool,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            pool: &self.pool,
            _marker: PhantomData,
        }
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    pool: &'a Pool,
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
            pool: self.pool,
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
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 限制结果数量
    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 设置偏移量
    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
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

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mysql_params = values_to_params(&params)?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();

        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                // 根据列的类型获取值
                let rust_type = T::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" | "i64" | "u32" | "u64" => {
                        let v: i64 = row.get(i).unwrap_or(0);
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(i).unwrap_or(String::new());
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(i).unwrap_or(0.0);
                        crate::model::Value::Real(v)
                    }
                    _ => {
                        return Err(crate::Error::Database(format!(
                            "Unsupported column type: {}",
                            rust_type
                        )));
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
    pool: &'a Pool,
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

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let result = conn
            .query_iter(&sql)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result.affected_rows())
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
    pool: &'a Pool,
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

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mysql_params = values_to_params(&params)?;

        let result = conn
            .exec_iter(&sql, mysql_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result.affected_rows())
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
            sql.push_str(&format!("{} = ?", col_name));
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

/// 将 ormer Value 转换为 mysql_async 参数
fn values_to_params(
    values: &[crate::model::Value],
) -> Result<Vec<mysql_async::Value>, crate::Error> {
    let mut params: Vec<mysql_async::Value> = Vec::new();

    for value in values {
        let param = match value {
            crate::model::Value::Integer(v) => mysql_async::Value::Int(*v),
            crate::model::Value::Text(v) => mysql_async::Value::Bytes(v.as_bytes().to_vec()),
            crate::model::Value::Real(v) => mysql_async::Value::Double(*v),
            crate::model::Value::Null => mysql_async::Value::NULL,
        };
        params.push(param);
    }

    Ok(params)
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
            write!(sql, "{} {} ?", column, operator).unwrap();
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
            write!(sql, "{} {} ?", column, operator).unwrap();
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
