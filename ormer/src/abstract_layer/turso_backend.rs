use crate::abstract_layer::common_helpers;
use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{
    FourTableSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect, RelatedSelect,
    RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// Turso 类型映射器
pub struct TursoTypeMapper;

impl DbBackendTypeMapper for TursoTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
    ) -> String {
        // 首先处理主键类型
        if is_primary {
            if is_auto_increment {
                return "INTEGER PRIMARY KEY AUTOINCREMENT".to_string();
            } else {
                return "INTEGER PRIMARY KEY".to_string();
            }
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
        let sql = "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?";

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
                        "Column name mismatch at position {i}: expected '{}', but actual is '{actual_name}'",
                        expected_col.name
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
                expected_col.is_auto_increment,
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

    /// 插入单条记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();

        // 构建批量插入的 SQL: INSERT INTO table (cols) VALUES (...), (...), ...
        let (mut sql, col_count) = common_helpers::build_batch_insert_sql::<T>(models.len());
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.field_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        self.conn
            .execute(&sql, all_params)
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

    /// 创建 Related 查询执行器
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> Result<Transaction, crate::Error> {
        self.conn
            .execute("BEGIN", ())
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        Ok(Transaction {
            conn: self.conn.clone(),
            committed: false,
            rolled_back: false,
        })
    }
}

/// Turso 事务对象
pub struct Transaction {
    conn: Arc<turso::Connection>,
    committed: bool,
    rolled_back: bool,
}

impl Transaction {
    /// 提交事务
    pub async fn commit(mut self) -> Result<(), crate::Error> {
        if self.committed || self.rolled_back {
            return Err(crate::Error::Database(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        self.conn
            .execute("COMMIT", ())
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
            .execute("ROLLBACK", ())
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        self.rolled_back = true;
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

    /// 插入单条记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();

        let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.field_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        self.conn
            .execute(&sql, all_params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }
}

/// Select 查询执行器
pub struct SelectExecutor<T: Model> {
    select: Select<T>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<T>,
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

/// Related 查询执行器（支持多表关联查询）
pub struct RelatedSelectExecutor<T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTable 查询执行器（支持3个表关联查询）
pub struct MultiTableSelectExecutor<T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTable 查询执行器（支持4个表关联查询）
pub struct FourTableSelectExecutor<T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2, R3)>,
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

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join::<J>(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join::<J>(f),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join::<J>(f),
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

    /// 执行查询并返回 Vec<T>
    pub fn exec(self) -> CollectFuture<T, Vec<T>>
    where
        T: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// 执行查询并返回 Vec<T> (exec 的别名)
    pub fn execute(self) -> CollectFuture<T, Vec<T>>
    where
        T: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
    {
        let aggregate_select = self.select.count(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.sum(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.avg(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.max(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.min(f);
        AggregateFuture {
            aggregate_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2: Model, R: Model>(self) -> RelatedSelectExecutor<T, R>
    where
        T2: 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2: Model, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<T, R1, R2>
    where
        T2: 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2: Model, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<T, R1, R2, R3>
    where
        T2: 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }
}

// LEFT JOIN Executor
impl<T: Model, J: Model> LeftJoinedSelectExecutor<T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 获取 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        self.select.to_sql_with_params().0
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(self) -> LeftJoinCollectFuture<T, J> {
        LeftJoinCollectFuture { executor: self }
    }

    pub fn exec(self) -> LeftJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, Option<J>)>>()
    }

    /// 执行查询并返回 Vec<(T, Option<J>)> (exec 的别名)
    pub fn execute(self) -> LeftJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, Option<J>)>>()
    }

    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();
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
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row
                    .get_value(i)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;
                t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            // 尝试读取 J 的列（从 t_col_count 开始）
            let mut j_data = HashMap::new();
            let mut j_is_null = true;
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                if let Ok(value) = row.get_value(idx) {
                    let ormer_value = convert_turso_value(&value)?;
                    // 检查是否为 NULL，只有非 NULL 值才设置 j_is_null = false
                    if !matches!(ormer_value, Value::Null) {
                        j_is_null = false;
                    }
                    j_data.insert(col_name.to_string(), ormer_value);
                }
            }

            let j_model = if j_is_null {
                None
            } else {
                Some(J::from_row(&Row::new(j_data))?)
            };

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

// INNER JOIN Executor
impl<T: Model, J: Model> InnerJoinedSelectExecutor<T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> InnerJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        InnerJoinCollectFuture { executor: self }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(self) -> InnerJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        InnerJoinCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> Result<Vec<(T, J)>, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();
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
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row
                    .get_value(i)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;
                t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let value = row
                    .get_value(idx)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results)
    }
}

// RIGHT JOIN Executor
impl<T: Model, J: Model> RightJoinedSelectExecutor<T, J> {
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

    pub fn limit(self, limit: i64) -> Self {
        Self {
            select: self.select.limit(limit),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        Self {
            select: self.select.offset(offset),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> RightJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        RightJoinCollectFuture { executor: self }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(self) -> RightJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        RightJoinCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> Result<Vec<(Option<T>, J)>, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();
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
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?
        {
            let mut t_data = HashMap::new();
            let mut t_is_null = true;
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                if let Ok(value) = row.get_value(i) {
                    t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
                    t_is_null = false;
                }
            }
            let t_model = if t_is_null {
                None
            } else {
                Some(T::from_row(&Row::new(t_data))?)
            };

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let value = row
                    .get_value(idx)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results)
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<T>,
    _marker: PhantomData<C>,
}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

impl<T: Model + 'static, R: crate::model::FromValue + 'static> std::future::IntoFuture
    for AggregateFuture<T, R>
{
    type Output = Result<R, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.aggregate_select.to_sql_with_params();

            let values: Vec<turso::Value> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => turso::Value::Integer(i),
                    crate::model::Value::Text(t) => turso::Value::Text(t),
                    crate::model::Value::Real(r) => turso::Value::Real(r),
                    crate::model::Value::Null => turso::Value::Null,
                })
                .collect();

            let mut rows = if values.is_empty() {
                self.conn
                    .query(&sql, ())
                    .await
                    .map_err(|e| crate::Error::Database(e.to_string()))?
            } else {
                self.conn
                    .query(&sql, values)
                    .await
                    .map_err(|e| crate::Error::Database(e.to_string()))?
            };

            if let Some(row) = rows
                .next()
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?
            {
                let value = row
                    .get_value(0)
                    .map_err(|e| crate::Error::Database(e.to_string()))?;

                // 将turso::Value转换为ormer::Value
                let ormer_value = match value {
                    turso::Value::Integer(i) => crate::model::Value::Integer(i),
                    turso::Value::Real(r) => crate::model::Value::Real(r),
                    turso::Value::Text(t) => crate::model::Value::Text(t),
                    turso::Value::Blob(b) => {
                        crate::model::Value::Text(String::from_utf8_lossy(&b).to_string())
                    }
                    turso::Value::Null => crate::model::Value::Null,
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

/// LEFT JOIN Collect future
pub struct LeftJoinCollectFuture<T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<T, J>,
}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<T, J>,
}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<T, J>,
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

impl<T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for LeftJoinCollectFuture<T, J>
{
    type Output = Result<Vec<(T, Option<J>)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for InnerJoinCollectFuture<T, J>
{
    type Output = Result<Vec<(T, J)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for RightJoinCollectFuture<T, J>
{
    type Output = Result<Vec<(Option<T>, J)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model, R: Model> RelatedSelectExecutor<T, R> {
    /// 添加 WHERE 条件（支持两个表的字段比较）
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
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
    pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<T, R> {
        RelatedCollectFuture { executor: self }
    }

    /// 执行查询并返回 Vec<T>
    pub fn exec(self) -> RelatedCollectFuture<T, R>
    where
        T: 'static,
        R: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// 执行查询并返回 Vec<T> (exec 的别名)
    pub fn execute(self) -> RelatedCollectFuture<T, R>
    where
        T: 'static,
        R: 'static,
    {
        self.collect::<Vec<T>>()
    }

    async fn collect_inner<C: FromIterator<T>>(self) -> Result<C, crate::Error> {
        let (sql, params) = self.select.to_sql_with_params();
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

/// Related Collect future
pub struct RelatedCollectFuture<T: Model, R: Model> {
    executor: RelatedSelectExecutor<T, R>,
}

impl<T: Model + 'static, R: Model + 'static> std::future::IntoFuture
    for RelatedCollectFuture<T, R>
{
    type Output = Result<Vec<T>, crate::Error>;
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
        let (sql, params) = self.build_sql();

        let result = self
            .conn
            .execute(&sql, params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    /// 执行删除操作并返回影响的行数（execute 的别名）
    pub async fn exec(self) -> Result<u64, crate::Error> {
        self.execute().await
    }

    fn build_sql(&self) -> (String, Vec<turso::Value>) {
        let mut sql = format!("DELETE FROM {}", T::TABLE_NAME);
        let mut ormer_params = Vec::new();

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut ormer_params,
                );
            }
        }

        let turso_params = values_to_params(&ormer_params).unwrap_or_default();
        (sql, turso_params)
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

        let result = self
            .conn
            .execute(&sql, params)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    /// 执行更新操作（execute 的别名）
    pub async fn exec(self) -> Result<u64, crate::Error> {
        self.execute().await
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
            sql.push_str(&format!("{col_name} = ?"));
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
                common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut ormer_params,
                );
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
