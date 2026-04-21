use crate::abstract_layer::common_helpers;
use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{
    FourTableSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect, RelatedSelect,
    RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use mysql_async::Pool;
use mysql_async::prelude::*;
use std::collections::HashMap;
use std::marker::PhantomData;

/// MySQL 类型映射器
pub struct MySQLTypeMapper;

impl DbBackendTypeMapper for MySQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
    ) -> String {
        // 首先处理主键类型
        if is_primary {
            let int_type = match rust_type {
                "i8" | "i16" | "u8" => "TINYINT",
                "i32" | "u16" => "INT",
                "i64" | "u32" | "u64" => "BIGINT",
                _ => "INT", // 默认使用 INT
            };
            if is_auto_increment {
                return format!("{int_type} PRIMARY KEY AUTO_INCREMENT");
            } else {
                return format!("{int_type} PRIMARY KEY");
            }
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

        let rows: Vec<mysql_async::Row> = conn
            .exec(sql, (T::TABLE_NAME,))
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool)> = Vec::new();
        for row in rows {
            let name: String = row.get(0).unwrap_or_default();
            let col_type: String = row.get(1).unwrap_or_default();
            let is_nullable: String = row.get(2).unwrap_or_default();
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
                        "Column name mismatch at position {i}: expected '{}', but actual is '{actual_name}'",
                        expected_col.name
                    ),
                });
            }

            // 检查列类型（只比较基础类型，不包含约束）
            let expected_type = crate::abstract_layer::DbType::MySQL.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
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
                    expected_col.is_auto_increment,
                    expected_col.is_nullable,
                );
                // 去掉 " NOT NULL" 后缀
                full_type.replace(" NOT NULL", "")
            };

            if !self.types_compatible(actual_type, &type_to_compare) {
                return Err(crate::Error::SchemaMismatch {
                    table: T::TABLE_NAME.to_string(),
                    reason: format!(
                        "Column type mismatch for '{}': expected '{expected_type}', but actual is '{actual_type}'",
                        expected_col.name
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

    /// 插入单条记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();

        // 构建批量插入的 SQL: INSERT INTO table (cols) VALUES (...), (...), ...
        let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.field_values();
            all_values.extend(values);
        }

        let params = values_to_params(&all_values)?;

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

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            pool: &self.pool,
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> Result<Transaction<'_>, crate::Error> {
        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        conn.query_drop("START TRANSACTION")
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(Transaction {
            conn,
            pool: &self.pool,
            committed: false,
            rolled_back: false,
        })
    }
}

/// MySQL 事务对象
pub struct Transaction<'a> {
    conn: mysql_async::Conn,
    pool: &'a Pool,
    committed: bool,
    rolled_back: bool,
}

impl<'a> Transaction<'a> {
    /// 提交事务
    pub async fn commit(mut self) -> Result<(), crate::Error> {
        if self.committed || self.rolled_back {
            return Err(crate::Error::Database(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        self.conn
            .query_drop("COMMIT")
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        self.committed = true;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> Result<(), crate::Error> {
        if self.committed || self.rolled_back {
            return Err(crate::Error::Database(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        self.conn
            .query_drop("ROLLBACK")
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        self.rolled_back = true;
        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 插入单条记录
    pub async fn insert<T: Model>(&mut self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&mut self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();

        let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.field_values();
            all_values.extend(values);
        }

        let params = values_to_params(&all_values)?;

        self.conn
            .exec_drop(&sql, params)
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        Ok(())
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    pool: &'a Pool,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    pool: &'a Pool,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    pool: &'a Pool,
    _marker: PhantomData<(T, J)>,
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

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join::<J>(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join::<J>(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join::<J>(f),
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

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
    {
        let aggregate_select = self.select.count(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.sum(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.avg(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.max(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.min(f);
        AggregateFuture {
            aggregate_select,
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2: Model, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2: Model, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2: Model, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    where
        T2: 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<C>,
}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    pool: &'a Pool,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model + 'static, R: crate::model::FromValue + 'static> std::future::IntoFuture
    for AggregateFuture<'a, T, R>
{
    type Output = Result<R, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.aggregate_select.to_sql_with_params();

            // 将ormer::Value转换为mysql_async Value
            let mut conn = self
                .pool
                .get_conn()
                .await
                .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

            // 构建参数
            let mysql_params: Vec<mysql_async::Value> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => mysql_async::Value::Int(i),
                    crate::model::Value::Text(t) => mysql_async::Value::Bytes(t.into_bytes()),
                    crate::model::Value::Real(r) => mysql_async::Value::Double(r),
                    crate::model::Value::Null => mysql_async::Value::NULL,
                })
                .collect();

            let mut exec_result = conn
                .exec_iter(&sql, mysql_params)
                .await
                .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

            if let Some(row) = exec_result
                .next()
                .await
                .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?
            {
                // 获取第一个列的值
                let value: Option<mysql_async::Value> = row.get(0);
                let value = value.unwrap_or(mysql_async::Value::NULL);

                // 将mysql_async::Value转换为ormer::Value
                let ormer_value = match value {
                    mysql_async::Value::Int(i) => crate::model::Value::Integer(i),
                    mysql_async::Value::UInt(u) => crate::model::Value::Integer(u as i64),
                    mysql_async::Value::Float(f) => crate::model::Value::Real(f as f64),
                    mysql_async::Value::Double(d) => crate::model::Value::Real(d),
                    mysql_async::Value::Bytes(b) => String::from_utf8(b)
                        .map(|s| crate::model::Value::Text(s))
                        .unwrap_or(crate::model::Value::Null),
                    mysql_async::Value::Date(_, _, _, _, _, _, _)
                    | mysql_async::Value::Time(_, _, _, _, _, _) => crate::model::Value::Null,
                    mysql_async::Value::NULL => crate::model::Value::Null,
                };

                // 使用 FromValue 转换为目标类型
                R::from_value(&ormer_value)
            } else {
                // 如果没有结果，返回 NULL 的转换
                R::from_value(&crate::model::Value::Null)
            }
        })
    }
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
                            "Unsupported column type: {rust_type}"
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
                common_helpers::format_filter(filter, &mut sql, &mut param_idx);
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
    pub fn set<F, V, C>(mut self, field_fn: F, value: V) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
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
                common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                );
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

/// Related 查询执行器（支持2表关联查询）
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    pool: &'a Pool,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model, R: Model> RelatedSelectExecutor<'a, T, R> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> RelatedCollectFuture<'a, T, R>
    where
        T: 'static,
        R: 'static,
    {
        RelatedCollectFuture { executor: self }
    }

    pub async fn collect<C: FromIterator<T>>(self) -> Result<C, crate::Error> {
        let results = self.collect_inner().await?;
        Ok(results.into_iter().collect())
    }

    async fn collect_inner(self) -> Result<Vec<T>, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        let mysql_params: Vec<mysql_async::Value> = params
            .iter()
            .map(|v| match v {
                crate::model::Value::Integer(n) => mysql_async::Value::Int(*n),
                crate::model::Value::Text(s) => mysql_async::Value::Bytes(s.as_bytes().to_vec()),
                crate::model::Value::Real(f) => mysql_async::Value::Double(*f),
                crate::model::Value::Null => mysql_async::Value::NULL,
            })
            .collect();

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.rust_type;
                let is_nullable = column_info.is_nullable;

                let ormer_value = if is_nullable {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: Option<i64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Integer(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "String" => {
                            let v: Option<String> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Text(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "f32" | "f64" => {
                            let v: Option<f64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Real(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "bool" => {
                            let v: Option<i8> = row.get(i).unwrap_or(None);
                            match v {
                                Some(1) => crate::model::Value::Integer(1),
                                Some(0) => crate::model::Value::Integer(0),
                                None => crate::model::Value::Null,
                                _ => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: i64 = row.get(i).unwrap();
                            crate::model::Value::Integer(v)
                        }
                        "String" => {
                            let v: String = row.get(i).unwrap();
                            crate::model::Value::Text(v)
                        }
                        "f32" | "f64" => {
                            let v: f64 = row.get(i).unwrap();
                            crate::model::Value::Real(v)
                        }
                        "bool" => {
                            let v: i8 = row.get(i).unwrap();
                            if v == 1 {
                                crate::model::Value::Integer(1)
                            } else {
                                crate::model::Value::Integer(0)
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported column type: {rust_type}"
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
        Ok(results)
    }
}

pub struct RelatedCollectFuture<'a, T: Model, R: Model> {
    executor: RelatedSelectExecutor<'a, T, R>,
}

impl<'a, T: Model + 'static, R: Model + 'static> std::future::IntoFuture
    for RelatedCollectFuture<'a, T, R>
{
    type Output = Result<Vec<T>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// Multi-Table 查询执行器（支持3表关联查询）
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    pool: &'a Pool,
    _marker: PhantomData<(T, R1, R2)>,
}

impl<'a, T: Model, R1: Model, R2: Model> MultiTableSelectExecutor<'a, T, R1, R2> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> MultiTableCollectFuture<'a, T, R1, R2>
    where
        T: 'static,
        R1: 'static,
        R2: 'static,
    {
        MultiTableCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> Result<Vec<T>, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        let mysql_params: Vec<mysql_async::Value> = params
            .iter()
            .map(|v| match v {
                crate::model::Value::Integer(n) => mysql_async::Value::Int(*n),
                crate::model::Value::Text(s) => mysql_async::Value::Bytes(s.as_bytes().to_vec()),
                crate::model::Value::Real(f) => mysql_async::Value::Double(*f),
                crate::model::Value::Null => mysql_async::Value::NULL,
            })
            .collect();

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.rust_type;
                let is_nullable = column_info.is_nullable;

                let ormer_value = if is_nullable {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: Option<i64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Integer(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "String" => {
                            let v: Option<String> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Text(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "f32" | "f64" => {
                            let v: Option<f64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Real(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "bool" => {
                            let v: Option<i8> = row.get(i).unwrap_or(None);
                            match v {
                                Some(1) => crate::model::Value::Integer(1),
                                Some(0) => crate::model::Value::Integer(0),
                                None => crate::model::Value::Null,
                                _ => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: i64 = row.get(i).unwrap();
                            crate::model::Value::Integer(v)
                        }
                        "String" => {
                            let v: String = row.get(i).unwrap();
                            crate::model::Value::Text(v)
                        }
                        "f32" | "f64" => {
                            let v: f64 = row.get(i).unwrap();
                            crate::model::Value::Real(v)
                        }
                        "bool" => {
                            let v: i8 = row.get(i).unwrap();
                            if v == 1 {
                                crate::model::Value::Integer(1)
                            } else {
                                crate::model::Value::Integer(0)
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported column type: {rust_type}"
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
        Ok(results)
    }
}

pub struct MultiTableCollectFuture<'a, T: Model, R1: Model, R2: Model> {
    executor: MultiTableSelectExecutor<'a, T, R1, R2>,
}

impl<'a, T: Model + 'static, R1: Model + 'static, R2: Model + 'static> std::future::IntoFuture
    for MultiTableCollectFuture<'a, T, R1, R2>
{
    type Output = Result<Vec<T>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// FourTable 查询执行器（支持4表关联查询）
pub struct FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    pool: &'a Pool,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

impl<'a, T: Model, R1: Model, R2: Model, R3: Model> FourTableSelectExecutor<'a, T, R1, R2, R3> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where, R3::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> FourTableCollectFuture<'a, T, R1, R2, R3>
    where
        T: 'static,
        R1: 'static,
        R2: 'static,
        R3: 'static,
    {
        FourTableCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> Result<Vec<T>, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();
        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mysql_params: Vec<mysql_async::Value> = params
            .iter()
            .map(|v| match v {
                crate::model::Value::Integer(n) => mysql_async::Value::Int(*n),
                crate::model::Value::Text(s) => mysql_async::Value::Bytes(s.as_bytes().to_vec()),
                crate::model::Value::Real(f) => mysql_async::Value::Double(*f),
                crate::model::Value::Null => mysql_async::Value::NULL,
            })
            .collect();

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e: mysql_async::Error| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.rust_type;
                let is_nullable = column_info.is_nullable;

                let ormer_value = if is_nullable {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: Option<i64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Integer(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "String" => {
                            let v: Option<String> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Text(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "f32" | "f64" => {
                            let v: Option<f64> = row.get(i).unwrap_or(None);
                            match v {
                                Some(val) => crate::model::Value::Real(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "bool" => {
                            let v: Option<i8> = row.get(i).unwrap_or(None);
                            match v {
                                Some(1) => crate::model::Value::Integer(1),
                                Some(0) => crate::model::Value::Integer(0),
                                None => crate::model::Value::Null,
                                _ => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
                    match rust_type {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                            let v: i64 = row.get(i).unwrap();
                            crate::model::Value::Integer(v)
                        }
                        "String" => {
                            let v: String = row.get(i).unwrap();
                            crate::model::Value::Text(v)
                        }
                        "f32" | "f64" => {
                            let v: f64 = row.get(i).unwrap();
                            crate::model::Value::Real(v)
                        }
                        "bool" => {
                            let v: i8 = row.get(i).unwrap();
                            if v == 1 {
                                crate::model::Value::Integer(1)
                            } else {
                                crate::model::Value::Integer(0)
                            }
                        }
                        _ => {
                            return Err(crate::Error::Database(format!(
                                "Unsupported column type: {rust_type}"
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
        Ok(results)
    }
}

pub struct FourTableCollectFuture<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    executor: FourTableSelectExecutor<'a, T, R1, R2, R3>,
}

impl<'a, T: Model + 'static, R1: Model + 'static, R2: Model + 'static, R3: Model + 'static>
    std::future::IntoFuture for FourTableCollectFuture<'a, T, R1, R2, R3>
{
    type Output = Result<Vec<T>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// LeftJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        self,
    ) -> LeftJoinCollectFuture<'a, T, J> {
        LeftJoinCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    pub fn execute(self) -> LeftJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, Option<J>)>>()
    }
}

/// LEFT JOIN Collect future
pub struct LeftJoinCollectFuture<'a, T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for LeftJoinCollectFuture<'a, T, J>
{
    type Output = Result<Vec<(T, Option<J>)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        let mysql_params = values_to_params(&params)?;

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
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
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                t_data.insert(col_name.to_string(), ormer_value);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            // 尝试读取 J 的列
            let mut j_data = HashMap::new();
            let mut j_is_null = true;
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let rust_type = J::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" | "i64" | "u32" | "u64" => {
                        let v: i64 = row.get(idx).unwrap_or(0);
                        if v != 0 {
                            j_is_null = false;
                        }
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(idx).unwrap_or(String::new());
                        if !v.is_empty() {
                            j_is_null = false;
                        }
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(idx).unwrap_or(0.0);
                        if v != 0.0 {
                            j_is_null = false;
                        }
                        crate::model::Value::Real(v)
                    }
                    _ => {
                        return Err(crate::Error::Database(format!(
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                j_data.insert(col_name.to_string(), ormer_value);
            }

            if j_is_null {
                results.push((t_model, None));
            } else {
                let j_model = J::from_row(&Row::new(j_data))?;
                results.push((t_model, Some(j_model)));
            }
        }

        Ok(results.into_iter().collect())
    }
}

/// InnerJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(self) -> InnerJoinCollectFuture<'a, T, J> {
        InnerJoinCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    pub fn execute(self) -> InnerJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, J)>>()
    }
}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<'a, T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for InnerJoinCollectFuture<'a, T, J>
{
    type Output = Result<Vec<(T, J)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        let mysql_params = values_to_params(&params)?;

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
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
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                t_data.insert(col_name.to_string(), ormer_value);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let rust_type = J::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" | "i64" | "u32" | "u64" => {
                        let v: i64 = row.get(idx).unwrap_or(0);
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(idx).unwrap_or(String::new());
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(idx).unwrap_or(0.0);
                        crate::model::Value::Real(v)
                    }
                    _ => {
                        return Err(crate::Error::Database(format!(
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                j_data.insert(col_name.to_string(), ormer_value);
            }

            let j_model = J::from_row(&Row::new(j_data))?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// RightJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        self,
    ) -> RightJoinCollectFuture<'a, T, J> {
        RightJoinCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    pub fn execute(self) -> RightJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(Option<T>, J)>>()
    }
}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<'a, T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for RightJoinCollectFuture<'a, T, J>
{
    type Output = Result<Vec<(Option<T>, J)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();

        let mysql_params = values_to_params(&params)?;

        let mut conn = self
            .pool
            .get_conn()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let rows: Vec<mysql_async::Row> = conn
            .exec(&sql, mysql_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            let mut t_is_null = true;
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let rust_type = T::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" | "i64" | "u32" | "u64" => {
                        let v: Option<i64> = row.get(i);
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Integer(v.unwrap_or(0))
                    }
                    "String" => {
                        let v: Option<String> = row.get(i);
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Text(v.unwrap_or(String::new()))
                    }
                    "f32" | "f64" => {
                        let v: Option<f64> = row.get(i);
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Real(v.unwrap_or(0.0))
                    }
                    _ => {
                        return Err(crate::Error::Database(format!(
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                t_data.insert(col_name.to_string(), ormer_value);
            }

            let t_model = if t_is_null {
                None
            } else {
                Some(T::from_row(&Row::new(t_data))?)
            };

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let rust_type = J::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" | "i64" | "u32" | "u64" => {
                        let v: i64 = row.get(idx).unwrap_or(0);
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(idx).unwrap_or(String::new());
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(idx).unwrap_or(0.0);
                        crate::model::Value::Real(v)
                    }
                    _ => {
                        return Err(crate::Error::Database(format!(
                            "Unsupported column type: {rust_type}"
                        )));
                    }
                };
                j_data.insert(col_name.to_string(), ormer_value);
            }

            let j_model = J::from_row(&Row::new(j_data))?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}
