use crate::abstract_layer::DbType;
use crate::abstract_layer::common_helpers;
use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{
    FourTableSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect, RelatedSelect,
    RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio_postgres::NoTls;

// 导入宏
// use crate::impl_backend_executor_methods_with_lifetime;
// use crate::impl_backend_join_executor_methods_with_lifetime;
// use crate::impl_backend_related_executor_methods_with_lifetime;

/// PostgreSQL 类型映射器
pub struct PostgreSQLTypeMapper;

impl DbBackendTypeMapper for PostgreSQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
    ) -> String {
        // 首先处理主键类型（主键自动 NOT NULL）
        if is_primary {
            if is_auto_increment {
                let serial_type = match rust_type {
                    "i8" | "i16" | "i32" => "SERIAL",
                    "i64" | "u16" | "u32" | "u64" => "BIGSERIAL",
                    "u8" => "SMALLSERIAL", // PostgreSQL 最小序列类型
                    _ => "SERIAL",         // 默认使用 SERIAL
                };
                return format!("{serial_type} PRIMARY KEY");
            } else {
                let int_type = match rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT",
                    "i32" | "u16" | "u32" => "INTEGER",
                    "i64" | "u64" => "BIGINT",
                    _ => "INTEGER",
                };
                return format!("{int_type} PRIMARY KEY");
            }
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
            "String" => "TEXT",
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
#[allow(dead_code)]
pub struct Database {
    client: tokio_postgres::Client,
    connection_handle: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: Model> {
    client: &'a tokio_postgres::Client,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> CreateTableExecutor<'a, T> {
    pub async fn execute(self) -> Result<(), crate::Error> {
        // 表不存在，创建新表
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::PostgreSQL,
            self.table_name.as_deref(),
        );

        // 分离 CREATE TABLE 和 CREATE INDEX 语句
        let sql_parts: Vec<&str> = create_sql.split(';').collect();

        for sql_part in sql_parts.iter() {
            let sql_part = sql_part.trim();
            if sql_part.is_empty() {
                continue;
            }

            self.client
                .execute(sql_part, &[])
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?;
        }

        Ok(())
    }
}

/// 删除表执行器
pub struct DropTableExecutor<'a, T: Model> {
    client: &'a tokio_postgres::Client,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> DropTableExecutor<'a, T> {
    pub async fn execute(self) -> Result<(), crate::Error> {
        let sql = format!("DROP TABLE IF EXISTS {} CASCADE", T::TABLE_NAME);
        self.client
            .execute(&sql, &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        Ok(())
    }
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
            if let Err(_e) = connection.await {
                // PostgreSQL connection error silently handled
            }
            Ok(())
        });

        Ok(Self {
            client,
            connection_handle,
        })
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            client: &self.client,
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: Model>(&self) -> Result<(), crate::Error> {
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().await?;

        if !table_exists {
            return Err(crate::Error::SchemaMismatch {
                table: T::TABLE_NAME.to_string(),
                reason: "Table does not exist".to_string(),
            });
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>().await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> Result<bool, crate::Error> {
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_type='BASE TABLE' AND table_schema='public' AND table_name=$1";

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

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(&self) -> Result<(), crate::Error> {
        // 查询表的列信息
        let sql = r#"
            SELECT column_name, data_type, is_nullable
            FROM information_schema.columns
            WHERE table_schema='public' AND table_name = $1
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
                        "Column name mismatch at position {i}: expected '{}', but actual is '{actual_name}'",
                        expected_col.name
                    ),
                });
            }

            // 检查列类型（只比较基础类型，不包含约束）
            let expected_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
            );

            // 对于类型比较，我们需要提取基础类型（不包含 SERIAL, PRIMARY KEY, NOT NULL 等约束）
            let type_to_compare = if expected_col.is_primary && expected_col.is_auto_increment {
                // SERIAL类型在PostgreSQL中实际存储为integer/bigint
                match expected_col.rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(), // SMALLSERIAL -> SMALLINT
                    "i32" | "u16" | "u32" => "INTEGER".to_string(), // SERIAL -> INTEGER
                    "i64" | "u64" => "BIGINT".to_string(),         // BIGSERIAL -> BIGINT
                    _ => "INTEGER".to_string(),
                }
            } else if expected_col.is_primary {
                // 主键的基础类型
                match expected_col.rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(),
                    "i32" | "u16" | "u32" => "INTEGER".to_string(),
                    "i64" | "u64" => "BIGINT".to_string(),
                    _ => "INTEGER".to_string(),
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
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
        // 标准化类型名称 - 只提取基础类型，去除约束
        fn normalize(s: &str) -> String {
            let upper = s.to_uppercase();
            // 提取第一个单词作为基础类型
            let base_type = upper.split_whitespace().next().unwrap_or(&upper);

            match base_type {
                // 整数类型
                "SMALLINT" | "INT2" => "SMALLINT".to_string(),
                "INTEGER" | "INT" | "INT4" | "SERIAL" => "INTEGER".to_string(),
                "BIGINT" | "INT8" | "BIGSERIAL" => "BIGINT".to_string(),
                // 字符串类型
                "CHARACTER" => {
                    // CHARACTER VARYING 需要特殊处理
                    if upper.starts_with("CHARACTER VARYING") || upper.starts_with("CHARACTER(") {
                        "VARCHAR".to_string()
                    } else {
                        "CHAR".to_string()
                    }
                }
                "VARCHAR" | "TEXT" | "CHAR" | "BPCHAR" => "VARCHAR".to_string(),
                // 布尔类型
                "BOOLEAN" | "BOOL" => "BOOLEAN".to_string(),
                // 浮点类型
                "REAL" | "FLOAT4" => "REAL".to_string(),
                "DOUBLE" => "DOUBLE PRECISION".to_string(), // DOUBLE PRECISION
                "FLOAT8" | "FLOAT" => "DOUBLE PRECISION".to_string(),
                // 字节类型
                "BYTEA" | "BLOB" => "BYTEA".to_string(),
                // 其他
                _ => base_type.to_string(),
            }
        }

        normalize(actual) == normalize(expected)
    }

    /// 插入单条记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 插入或更新单条记录（遇到重复键时更新）
    pub async fn insert_or_update<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_or_update_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();

        // 构建批量插入的 SQL: INSERT INTO table (cols) VALUES (...), (...), ...
        let (sql, _) = common_helpers::build_batch_insert_sql_postgresql_with_columns(
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let mut all_values = Vec::new();

        for model in models.iter() {
            let values = model.insert_values();
            all_values.extend(values);
        }

        // 获取列的rust类型信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        self.client
            .execute(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(
        &self,
        models: &[&T],
    ) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let primary_key = T::primary_key_column();
        let columns_str = columns.join(", ");

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!("INSERT INTO {} ({columns_str}) VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        let mut param_idx = 1;

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count)
                .map(|i| format!("${}", param_idx + i - 1))
                .collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            param_idx += col_count;

            let values = model.insert_values();
            all_values.extend(values);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO UPDATE SET "));
        let mut first = true;
        for col_name in columns.iter() {
            if col_name == &primary_key {
                continue; // 跳过主键
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{col_name} = EXCLUDED.{col_name}"));
            first = false;
        }

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();
        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        self.client
            .execute(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(format!("db error: {}", e)))?;

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> Result<Transaction<'_>, crate::Error> {
        self.client
            .execute("BEGIN", &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        Ok(Transaction {
            client: &self.client,
            committed: false,
            rolled_back: false,
        })
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: Model>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            client: &self.client,
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn exec_table<T: Model>(&self, sql: &str) -> Result<Vec<T>, crate::Error> {
        let rows = self
            .client
            .query(sql, &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();

        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = convert_postgres_value(&row, i)?;
                data.insert(col_name.to_string(), ormer_value);
            }

            let ormer_row = Row::new(data);
            let model = T::from_row(&ormer_row)?;
            results.push(model);
        }

        Ok(results)
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn exec_non_query(&self, sql: &str) -> Result<u64, crate::Error> {
        let result = self
            .client
            .execute(sql, &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        Ok(result)
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        self.client.execute("SELECT 1", &[]).await.is_ok()
    }
}

/// PostgreSQL 事务对象
pub struct Transaction<'a> {
    client: &'a tokio_postgres::Client,
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
        self.client
            .execute("COMMIT", &[])
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
        self.client
            .execute("ROLLBACK", &[])
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;
        self.rolled_back = true;
        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 插入单条记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_batch::<T>(&[model]).await
    }

    /// 插入或更新单条记录（遇到重复键时更新）
    pub async fn insert_or_update<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        self.insert_or_update_batch::<T>(&[model]).await
    }

    /// 批量插入记录
    pub async fn insert_batch<T: Model>(&self, models: &[&T]) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let (sql, _) =
            crate::abstract_layer::common_helpers::build_batch_insert_sql_postgresql_with_columns(
                T::TABLE_NAME,
                &columns,
                models.len(),
            );
        let all_values =
            crate::abstract_layer::common_helpers::collect_batch_insert_values_with_auto_increment::<
                T,
            >(models);

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        self.client.execute(&sql, &param_refs).await.map_err(|e| {
            crate::Error::Database(format!(
                "Transaction insert_batch failed: {:?}, SQL: {}",
                e, sql
            ))
        })?;

        Ok(())
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(
        &self,
        models: &[&T],
    ) -> Result<(), crate::Error> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let primary_key = T::primary_key_column();
        let columns_str = columns.join(", ");

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!("INSERT INTO {} ({columns_str}) VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        let mut param_idx = 1;

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count)
                .map(|i| format!("${}", param_idx + i - 1))
                .collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            param_idx += col_count;

            let values = model.insert_values();
            all_values.extend(values);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO UPDATE SET "));
        let mut first = true;
        for col_name in columns.iter() {
            if col_name == &primary_key {
                continue; // 跳过主键
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{col_name} = EXCLUDED.{col_name}"));
            first = false;
        }

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();
        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        self.client
            .execute(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(())
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

/// 映射查询结果执行器
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: crate::query::builder::MappedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V)>,
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        MappedCollectFuture {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }
}

/// 映射查询收集Future
pub struct MappedCollectFuture<'a, T: Model, V, C> {
    select: crate::query::builder::MappedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V, C)>,
}

impl<'a, T: Model + 'static, V: crate::model::FromRowValues + 'static, C: FromIterator<V> + 'static>
    std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
            let pg_params = values_to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref()).collect();

            let rows = self
                .client
                .query(&sql, &param_refs)
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?;

            let mut results = Vec::new();
            for row in rows {
                // 将行数据转换为Vec<Value>
                let mut values = Vec::new();
                for i in 0..row.columns().len() {
                    let value = convert_postgres_value(&row, i)?;
                    values.push(value);
                }

                // 使用FromRowValues转换为V
                let v = V::from_row_values(&values)?;
                results.push(v);
            }

            Ok(results.into_iter().collect())
        })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

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
    pub fn order_by<F, O>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<crate::OrderBy>,
    {
        Self {
            select: self.select.order_by(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加降序排序
    pub fn order_by_desc<F, O>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<crate::OrderBy>,
    {
        Self {
            select: self.select.order_by_desc(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 设置范围
    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
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
            client: self.client,
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
            client: self.client,
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
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 映射查询结果到自定义类型
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        let mapped_select = self.select.map_to(f);
        MappedSelectExecutor {
            select: mapped_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self.clone_with_client(),
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
            client: self.client,
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
            client: self.client,
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
            client: self.client,
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
            client: self.client,
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
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            client: self.client,
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
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model + 'static, R: crate::model::FromValue + 'static> std::future::IntoFuture
    for AggregateFuture<'a, T, R>
{
    type Output = Result<R, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (mut sql, params) = self.aggregate_select.to_sql_with_params(DbType::PostgreSQL);

            // 对于 AVG 聚合，PostgreSQL 返回 NUMERIC 类型，需要 CAST 为 FLOAT8
            // 这样可以避免 tokio-postgres 不支持 NUMERIC 类型的问题
            if sql.contains("SELECT AVG(") {
                sql = sql.replace("SELECT AVG(", "SELECT AVG(");
                // 在 AVG 函数的闭括号后添加 ::FLOAT8
                // 找到 "AVG(column_name)" 并替换为 "AVG(column_name)::FLOAT8"
                if let Some(avg_start) = sql.find("AVG(") {
                    if let Some(paren_end) = sql[avg_start..].find(')') {
                        let insert_pos = avg_start + paren_end + 1;
                        sql.insert_str(insert_pos, "::FLOAT8");
                    }
                }
            }

            // 将ormer::Value转换为postgres_types::ToSql
            let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync>> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => {
                        // PostgreSQL INTEGER (Int4) 是 32 位，需要将 i64 转换为 i32
                        if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                            Box::new(i as i32) as Box<dyn postgres_types::ToSql + Sync>
                        } else {
                            Box::new(i) as Box<dyn postgres_types::ToSql + Sync>
                        }
                    }
                    crate::model::Value::Text(t) => {
                        Box::new(t) as Box<dyn postgres_types::ToSql + Sync>
                    }
                    crate::model::Value::Real(r) => {
                        Box::new(r) as Box<dyn postgres_types::ToSql + Sync>
                    }
                    crate::model::Value::Null => {
                        Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync>
                    }
                })
                .collect();

            let params_ref: Vec<&(dyn postgres_types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref()).collect();

            let row = self
                .client
                .query_one(&sql, &params_ref)
                .await
                .map_err(|e| crate::Error::Database(e.to_string()))?;

            // 获取第一列的值
            use tokio_postgres::types::Type;
            let column_type = row.columns()[0].type_();

            // 根据类型获取值
            let ormer_value = match *column_type {
                Type::INT2 => {
                    let val: Option<i16> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT4 => {
                    let val: Option<i32> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT8 => {
                    let val: Option<i64> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Integer(v))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT4 => {
                    let val: Option<f32> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Real(v as f64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT8 => {
                    let val: Option<f64> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Real(v))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::NUMERIC => {
                    // NUMERIC类型处理 - 由于tokio-postgres不直接支持NUMERIC到f64的转换
                    // 我们尝试直接读取为f64
                    let val_result: Result<Option<f64>, _> = row.try_get(0);
                    match val_result {
                        Ok(Some(v)) => crate::model::Value::Real(v),
                        Ok(None) => crate::model::Value::Null,
                        Err(_) => crate::model::Value::Null,
                    }
                }
                Type::TEXT | Type::VARCHAR => {
                    let val: Option<String> = row
                        .try_get(0)
                        .map_err(|e| crate::Error::Database(e.to_string()))?;
                    val.map(|v| crate::model::Value::Text(v))
                        .unwrap_or(crate::model::Value::Null)
                }
                _ => crate::model::Value::Null,
            };

            // 使用 FromValue 转换为目标类型
            R::from_value(&ormer_value)
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self
            .client
            .query(&sql, &param_refs)
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
                    // 处理可空类型 - 根据PostgreSQL的实际列类型读取
                    use tokio_postgres::types::Type;
                    let pg_type = row.columns()[i].type_();

                    match *pg_type {
                        Type::INT2 => {
                            let v: Option<i16> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Integer(val as i64),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::INT4 => {
                            let v: Option<i32> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Integer(val as i64),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::INT8 => {
                            let v: Option<i64> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Integer(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::TEXT | Type::VARCHAR => {
                            let v: Option<String> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Text(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::FLOAT4 => {
                            let v: Option<f32> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Real(val as f64),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::FLOAT8 => {
                            let v: Option<f64> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Real(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::BOOL => {
                            let v: Option<bool> = row.get(i);
                            match v {
                                Some(true) => crate::model::Value::Integer(1),
                                Some(false) => crate::model::Value::Integer(0),
                                None => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            // 备用方案：使用rust_type
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
                                        "Unsupported nullable column type: {rust_type}"
                                    )));
                                }
                            }
                        }
                    }
                } else {
                    // 处理非空类型 - 根据PostgreSQL的实际列类型读取
                    use tokio_postgres::types::Type;
                    let pg_type = row.columns()[i].type_();

                    match *pg_type {
                        Type::INT2 => {
                            let v: i16 = row.get(i);
                            crate::model::Value::Integer(v as i64)
                        }
                        Type::INT4 => {
                            let v: i32 = row.get(i);
                            crate::model::Value::Integer(v as i64)
                        }
                        Type::INT8 => {
                            let v: i64 = row.get(i);
                            crate::model::Value::Integer(v)
                        }
                        Type::TEXT | Type::VARCHAR => {
                            let v: String = row.get(i);
                            crate::model::Value::Text(v)
                        }
                        Type::FLOAT4 => {
                            let v: f32 = row.get(i);
                            crate::model::Value::Real(v as f64)
                        }
                        Type::FLOAT8 => {
                            let v: f64 = row.get(i);
                            crate::model::Value::Real(v)
                        }
                        Type::BOOL => {
                            let v: bool = row.get(i);
                            if v {
                                crate::model::Value::Integer(1)
                            } else {
                                crate::model::Value::Integer(0)
                            }
                        }
                        _ => {
                            // 备用方案：使用rust_type
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
                                        "Unsupported column type: {rust_type}"
                                    )));
                                }
                            }
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
        let (sql, params) = self.build_sql_with_params();

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA.iter().map(|col| col.rust_type).collect();
        let pg_params = values_to_params_with_types(&params, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let result = self
            .client
            .execute(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        Ok(result)
    }

    #[allow(dead_code)]
    fn build_sql(&self) -> String {
        let (sql, _) = self.build_sql_with_params();
        sql
    }

    fn build_sql_with_params(&self) -> (String, Vec<Value>) {
        let mut sql = format!("DELETE FROM {}", T::TABLE_NAME);
        let mut params = Vec::new();

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx: usize = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    DbType::PostgreSQL,
                );
            }
        }

        (sql, params)
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
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let result = self
            .client
            .execute(&sql, &param_refs)
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
            sql.push_str(&format!("{col_name} = ${}", params.len() + 1));
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
                    DbType::PostgreSQL,
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

/// 将 ormer Value 转换为 tokio-postgres 参数
/// 根据列的rust_type选择正确的参数类型（i32或i64）
fn values_to_params_with_types(
    values: &[crate::model::Value],
    rust_types: &[&str],
) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync>>, crate::Error> {
    // ToSql trait is used in the trait object type above
    #[allow(unused_imports)]
    use tokio_postgres::types::ToSql;

    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();

    for (idx, value) in values.iter().enumerate() {
        // 循环使用rust_types，因为values可能包含多个记录的所有字段
        let rust_type = rust_types[idx % rust_types.len()];

        let param: Box<dyn tokio_postgres::types::ToSql + Sync> = match value {
            crate::model::Value::Integer(v) => {
                // 根据列的rust_type选择合适的整数类型
                // tokio-postgres要求Rust类型与PostgreSQL类型严格匹配
                let use_i64 = matches!(rust_type, "i64" | "u64");
                if use_i64 {
                    Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync>
                } else {
                    // 对于i32列，将值转换为i32
                    Box::new(*v as i32) as Box<dyn tokio_postgres::types::ToSql + Sync>
                }
            }
            crate::model::Value::Text(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync>
            }
            crate::model::Value::Real(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync>
            }
            crate::model::Value::Null => {
                // 根据列类型选择NULL的类型
                match rust_type {
                    "i64" | "u64" => {
                        let null_val: Option<i64> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
                    }
                    "i32" | "i16" | "i8" | "u16" | "u32" | "u8" => {
                        let null_val: Option<i32> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
                    }
                    "String" | "&str" => {
                        let null_val: Option<String> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
                    }
                    "f32" | "f64" => {
                        let null_val: Option<f64> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
                    }
                    _ => {
                        // 默认使用Option<i32>
                        let null_val: Option<i32> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
                    }
                }
            }
        };
        params.push(param);
    }

    Ok(params)
}

/// 将 ormer Value 转换为 tokio-postgres 参数（旧版本，根据值大小选择类型）
fn values_to_params(
    values: &[crate::model::Value],
) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync>>, crate::Error> {
    // ToSql trait is used in the trait object type above
    #[allow(unused_imports)]
    use tokio_postgres::types::ToSql;

    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();

    for value in values {
        let param: Box<dyn tokio_postgres::types::ToSql + Sync> = match value {
            crate::model::Value::Integer(v) => {
                // PostgreSQL INTEGER (Int4) 是 32 位，需要将 i64 转换为 i32
                // 如果值超出 i32 范围，则使用 i64 (Int8/BIGINT)
                if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                    Box::new(*v as i32) as Box<dyn tokio_postgres::types::ToSql + Sync>
                } else {
                    Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync>
                }
            }
            crate::model::Value::Text(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync>
            }
            crate::model::Value::Real(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync>
            }
            crate::model::Value::Null => {
                // 使用 Option<i32> 的 None 来表示 NULL（默认用于INTEGER列）
                let null_val: Option<i32> = None;
                Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync>
            }
        };
        params.push(param);
    }

    Ok(params)
}

/// Related 查询执行器（支持2表关联查询）
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model, R: Model> RelatedSelectExecutor<'a, T, R> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self
            .client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

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
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
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

/// MultiTable 查询执行器（支持3表关联查询）
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R1, R2)>,
}

impl<'a, T: Model, R1: Model, R2: Model> MultiTableSelectExecutor<'a, T, R1, R2> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self
            .client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

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
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
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
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

impl<'a, T: Model, R1: Model, R2: Model, R3: Model> FourTableSelectExecutor<'a, T, R1, R2, R3> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R1::Where, R2::Where, R3::Where) -> WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self
            .client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

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
                                "Unsupported nullable column type: {rust_type}"
                            )));
                        }
                    }
                } else {
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
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J> {
        LeftJoinCollectFuture {
            executor: self.clone_with_client(),
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let rows = self
            .client
            .query(&sql, &pg_params_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let rust_type = T::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" => {
                        let v: Option<i32> = row.try_get(i).ok().flatten();
                        crate::model::Value::Integer(v.unwrap_or(0) as i64)
                    }
                    "i64" => {
                        let v: Option<i64> = row.try_get(i).ok().flatten();
                        crate::model::Value::Integer(v.unwrap_or(0))
                    }
                    "String" => {
                        let v: Option<String> = row.try_get(i).ok().flatten();
                        crate::model::Value::Text(v.unwrap_or_default())
                    }
                    "f32" | "f64" => {
                        let v: Option<f64> = row.try_get(i).ok().flatten();
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
            let t_model = T::from_row(&Row::new(t_data))?;

            // 尝试读取 J 的列
            let mut j_data = HashMap::new();
            let mut j_is_null = true;
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let rust_type = J::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" => {
                        let v: Option<i32> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        crate::model::Value::Integer(v.unwrap_or(0) as i64)
                    }
                    "i64" => {
                        let v: Option<i64> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        crate::model::Value::Integer(v.unwrap_or(0))
                    }
                    "String" => {
                        let v: Option<String> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        crate::model::Value::Text(v.unwrap_or_default())
                    }
                    "f32" | "f64" => {
                        let v: Option<f64> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        crate::model::Value::Real(v.unwrap_or(0.0))
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
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J> {
        InnerJoinCollectFuture {
            executor: self.clone_with_client(),
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let rows = self
            .client
            .query(&sql, &pg_params_refs)
            .await
            .map_err(|e| crate::Error::Database(e.to_string()))?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let rust_type = T::COLUMN_SCHEMA[i].rust_type;
                let ormer_value = match rust_type {
                    "i32" => {
                        let v: i32 = row.get(i);
                        crate::model::Value::Integer(v as i64)
                    }
                    "i64" => {
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
                    "i32" => {
                        let v: i32 = row.get(idx);
                        crate::model::Value::Integer(v as i64)
                    }
                    "i64" => {
                        let v: i64 = row.get(idx);
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(idx);
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(idx);
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
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J> {
        RightJoinCollectFuture {
            executor: self.clone_with_client(),
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
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let rows = self
            .client
            .query(&sql, &pg_params_refs)
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
                    "i32" => {
                        let v: Option<i32> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Integer(v.unwrap_or(0) as i64)
                    }
                    "i64" => {
                        let v: Option<i64> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Integer(v.unwrap_or(0))
                    }
                    "String" => {
                        let v: Option<String> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Text(v.unwrap_or_default())
                    }
                    "f32" | "f64" => {
                        let v: Option<f64> = row.try_get(i).ok().flatten();
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
                    "i32" => {
                        let v: i32 = row.get(idx);
                        crate::model::Value::Integer(v as i64)
                    }
                    "i64" => {
                        let v: i64 = row.get(idx);
                        crate::model::Value::Integer(v)
                    }
                    "String" => {
                        let v: String = row.get(idx);
                        crate::model::Value::Text(v)
                    }
                    "f32" | "f64" => {
                        let v: f64 = row.get(idx);
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

/// 将 PostgreSQL 行中的值转换为 ormer Value
fn convert_postgres_value(
    row: &tokio_postgres::Row,
    index: usize,
) -> Result<crate::model::Value, crate::Error> {
    use tokio_postgres::types::Type;

    let col_type = row.columns()[index].type_();

    // 根据PostgreSQL类型选择正确的Rust类型
    match *col_type {
        // 整数类型 - 需要根据实际大小选择
        Type::INT2 => {
            if let Ok(v) = row.try_get::<_, Option<i16>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val as i64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::INT4 => {
            if let Ok(v) = row.try_get::<_, Option<i32>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val as i64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::INT8 => {
            if let Ok(v) = row.try_get::<_, Option<i64>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 文本类型
        Type::TEXT | Type::VARCHAR | Type::CHAR | Type::BPCHAR | Type::NAME => {
            if let Ok(v) = row.try_get::<_, Option<String>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Text(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 浮点类型
        Type::FLOAT4 => {
            if let Ok(v) = row.try_get::<_, Option<f32>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Real(val as f64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::FLOAT8 => {
            if let Ok(v) = row.try_get::<_, Option<f64>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Real(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 布尔类型
        Type::BOOL => {
            if let Ok(v) = row.try_get::<_, Option<bool>>(index) {
                return Ok(match v {
                    Some(true) => crate::model::Value::Integer(1),
                    Some(false) => crate::model::Value::Integer(0),
                    None => crate::model::Value::Null,
                });
            }
        }
        _ => {}
    }

    Err(crate::Error::Database(format!(
        "Unsupported column type {:?} at index {}",
        col_type, index
    )))
}
