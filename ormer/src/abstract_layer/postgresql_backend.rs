use super::common::common_helpers;
use crate::abstract_layer::DbType;
use crate::model::{DbBackendTypeMapper, DurationToInterval, Model, Row, Value};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect,
    RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use crate::utils::{AnyhowFutureTraceExt, FutureTraceExt, ResultTraceExt};
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio_postgres::NoTls;
use tokio_postgres::types::Type;

/// 将数据库返回的自增ID（i64）转换为模型指定的 AutoIncrementKeyType
/// 支持 i32, i64, u32, u64 等整数类型，以及 ()
fn convert_auto_increment_key<K: Default + 'static>(last_id: i64) -> anyhow::Result<K> {
    let result = std::any::TypeId::of::<K>();
    if result == std::any::TypeId::of::<()>() {
        let val: () = ();
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<i32>() {
        let val: i32 = last_id as i32;
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<i64>() {
        let val: i64 = last_id;
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<u32>() {
        let val: u32 = last_id as u32;
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<u64>() {
        let val: u64 = last_id as u64;
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<usize>() {
        let val: usize = last_id as usize;
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else if result == std::any::TypeId::of::<Option<i64>>() {
        let val: Option<i64> = Some(last_id);
        Ok(unsafe { std::mem::transmute_copy(&val) })
    } else {
        Err(anyhow::anyhow!(
            "Unsupported auto-increment key type. Only i32, i64, u32, u64, usize and () are supported."
        ))
    }
}

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
        enum_variants: Option<&[&str]>,
    ) -> String {
        // PostgreSQL 支持 ENUM 类型
        if enum_variants.is_some() {
            // 使用 rust_type 作为 ENUM 类型名（需要小蛇形命名）
            let enum_name = to_snake_case(rust_type);
            return format!(
                "{}{}",
                enum_name,
                if !is_nullable { " NOT NULL" } else { "" }
            );
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
            "DateTime" | "chrono::DateTime" => "TIMESTAMPTZ",
            "NaiveDateTime" | "chrono::NaiveDateTime" => "TIMESTAMP",
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            // JSON 类型
            "JsonValue" | "serde_json::Value" => "JSONB",
            // 默认使用 TEXT
            _ => "TEXT",
        };

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
                return format!("{base_type} PRIMARY KEY");
            }
        }

        let mut sql_type = base_type.to_string();

        // 非主键字段根据 is_nullable 决定是否添加 NOT NULL
        if !is_nullable {
            sql_type.push_str(" NOT NULL");
        }

        sql_type
    }
}

/// 将驼峰命名转换为蛇形命名
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// PostgreSQL 数据库连接封装
pub struct Database {
    client: tokio_postgres::Client,
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: Model> {
    client: &'a tokio_postgres::Client,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> CreateTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let table_name = self.table_name.as_deref().unwrap_or(T::TABLE_NAME);

        // 创建所有需要的 ENUM 类型（幂等：使用 IF NOT EXISTS）
        for column in T::COLUMN_SCHEMA.iter() {
            if let Some(variants) = column.enum_variants {
                let enum_name = to_snake_case(column.rust_type);
                let variants_str = variants
                    .iter()
                    .map(|v| format!("'{}'", v))
                    .collect::<Vec<_>>()
                    .join(", ");
                let create_enum_sql = format!(
                    "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = '{}') THEN CREATE TYPE {} AS ENUM ({}); END IF; END $$",
                    enum_name, enum_name, variants_str
                );

                self.client.execute(&create_enum_sql, &[]).trace().await?;
            }
        }

        // 生成 CREATE TABLE SQL
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::PostgreSQL,
            self.table_name.as_deref(),
        )?;

        // 分离 CREATE TABLE 和 CREATE INDEX 语句
        let sql_parts: Vec<&str> = create_sql.split(';').collect();

        // 执行 CREATE TABLE IF NOT EXISTS（幂等操作，不会删除已有数据）
        let first_part = sql_parts[0].trim();
        if !first_part.is_empty() {
            self.client.execute(first_part, &[]).trace().await?;
        }

        // 执行剩余的 CREATE INDEX 语句（使用 IF NOT EXISTS，幂等）
        for sql_part in sql_parts.iter().skip(1) {
            let sql_part = sql_part.trim();
            if sql_part.is_empty() {
                continue;
            }

            self.client.execute(sql_part, &[]).trace().await?;
        }

        // If the model is marked as a hypertable, create a TimescaleDB hypertable
        if let Some((time_column, chunk_interval)) = T::hypertable_info() {
            // Check whether the TimescaleDB extension is enabled
            let ext_check = self
                .client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')",
                    &[],
                )
                .trace()
                .await?;
            let tsdb_enabled: bool = ext_check.get(0);
            if !tsdb_enabled {
                anyhow::bail!(
                    "Model '{}' is marked as hypertable, but the TimescaleDB extension is not installed",
                    T::TABLE_NAME
                );
            }

            // Check whether this table is already a hypertable
            let ht_check = self
                .client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM _timescaledb_catalog.hypertable WHERE schema_name = 'public' AND table_name = $1)",
                    &[&table_name],
                )
                .trace().await?;
            let is_hypertable: bool = ht_check.get(0);

            if !is_hypertable {
                let interval_str = chunk_interval.to_interval_string();
                let hypertable_sql = format!(
                    "SELECT create_hypertable('{}', '{}', chunk_time_interval => INTERVAL '{}')",
                    table_name, time_column, interval_str
                );
                self.client.execute(&hypertable_sql, &[]).trace().await?;
            }
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
    pub async fn execute(self) -> anyhow::Result<()> {
        let sql = format!("DROP TABLE IF EXISTS {} CASCADE", T::TABLE_NAME);
        self.client.execute(&sql, &[]).trace().await?;
        Ok(())
    }
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> InsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<<I::Model as Model>::AutoIncrementKeyType> {
        let refs = self.models.as_refs();
        self.db.insert_impl::<I::Model>(&refs).await
    }

    /// 执行插入并返回插入的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(self) -> anyhow::Result<Vec<I::Model>> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(Vec::new());
        }

        let columns = I::Model::insert_columns();
        let (sql, _) =
            super::common::common_helpers::build_batch_insert_sql_postgresql_with_columns(
                I::Model::TABLE_NAME,
                &columns,
                refs.len(),
            );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<
                I::Model,
            >(&refs);

        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let sql_with_returning = format!("{} RETURNING *", sql);
        let rows = self
            .db
            .client
            .query(&sql_with_returning, &param_refs)
            .await?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in I::Model::COLUMNS.iter().enumerate() {
                let ormer_value = convert_postgres_value(&row, i)?;
                data.insert(col_name.to_string(), ormer_value);
            }
            let ormer_row = Row::new(data);
            let model = I::Model::from_row(&ormer_row)?;
            results.push(model);
        }

        Ok(results)
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> InsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.db.insert_or_update_batch::<I::Model>(&refs).await
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> InsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.db.insert_or_ignore_batch::<I::Model>(&refs).await
    }
}

impl Database {
    /// 连接到 PostgreSQL 数据库
    pub async fn connect(_db_type: super::DbType, connection_string: &str) -> anyhow::Result<Self> {
        let (client, connection) = tokio_postgres::connect(connection_string, NoTls)
            .trace()
            .await?;

        // 在后台运行连接
        tokio::spawn(async move {
            if let Err(err) = connection
                .trace_for("tokio_postgres::Connection::poll")
                .await
            {
                eprintln!("[ormer] {err}");
            }
        });

        // 将服务端消息级别设为 WARNING，过滤掉 NOTICE/INFO/LOG/DEBUG（如 "关系已存在, 跳过"）
        client
            .execute("SET client_min_messages TO WARNING;", &[])
            .trace()
            .await?;

        Ok(Self { client })
    }

    /// 从 bb8 PooledConnection 创建 Database
    ///
    /// bb8-postgres 的 PooledConnection 通过 Deref 提供 &Client 访问。
    /// 由于 tokio_postgres::Client 不实现 Clone，我们使用 std::ops::Deref
    /// 获取引用后，通过 unsafe ptr::read 复制 Client（Client 内部使用 Arc，
    /// 复制是安全的，只是增加 Arc 引用计数），然后 forget PooledConnection
    /// 防止其 drop 时关闭连接。
    pub fn from_pooled_connection(
        pooled: bb8::PooledConnection<'_, bb8_postgres::PostgresConnectionManager<NoTls>>,
    ) -> Self {
        use std::ops::Deref;
        let client_ref: &tokio_postgres::Client = pooled.deref();
        let client = unsafe { std::ptr::read(client_ref as *const _) };
        std::mem::forget(pooled);
        Self { client }
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            client: &self.client,
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        InsertOrUpdateExecutor {
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        InsertOrIgnoreExecutor {
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: Model>(&self) -> anyhow::Result<()> {
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().trace().await?;

        if !table_exists {
            return Err(anyhow::anyhow!(
                "Schema mismatch: table {}, reason: Table does not exist",
                T::TABLE_NAME
            ));
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>().await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> anyhow::Result<bool> {
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_type='BASE TABLE' AND table_schema='public' AND table_name=$1";

        let row = self
            .client
            .query_one(sql, &[&T::TABLE_NAME])
            .trace()
            .await?;

        let count: i64 = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;

        Ok(count > 0)
    }

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(&self) -> anyhow::Result<()> {
        // 查询表的列信息
        let sql = r#"
            SELECT column_name, data_type, is_nullable
            FROM information_schema.columns
            WHERE table_schema='public' AND table_name = $1
            ORDER BY ordinal_position
        "#;

        let rows = self.client.query(sql, &[&T::TABLE_NAME]).trace().await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool)> = Vec::new();
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let col_type: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let is_nullable: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;

            actual_columns.push((name, col_type, is_nullable == "YES"));
        }

        // 比较列数量
        if actual_columns.len() != T::COLUMNS.len() {
            return Err(anyhow::anyhow!(
                "Schema mismatch: table {}, reason: Column count mismatch: expected {}, but actual is {}",
                T::TABLE_NAME,
                T::COLUMNS.len(),
                actual_columns.len()
            ));
        }

        // 比较每一列的定义
        for (i, expected_col) in T::COLUMN_SCHEMA.iter().enumerate() {
            if i >= actual_columns.len() {
                return Err(anyhow::anyhow!(
                    "Schema mismatch: table {}, reason: Missing column: {}",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            let (actual_name, actual_type, actual_nullable) = &actual_columns[i];

            // 检查列名
            if actual_name != expected_col.name {
                return Err(anyhow::anyhow!(
                    "Schema mismatch: table {}, reason: Column name mismatch at position {}: expected '{}', but actual is '{}'",
                    T::TABLE_NAME,
                    i,
                    expected_col.name,
                    actual_name
                ));
            }

            let effective_rust_type = expected_col.data_type.unwrap_or(expected_col.rust_type);

            // 检查列类型（只比较基础类型，不包含约束）
            let expected_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                effective_rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
            );

            // 对于类型比较，我们需要提取基础类型（不包含 SERIAL, PRIMARY KEY, NOT NULL 等约束）
            let type_to_compare = if expected_col.is_primary && expected_col.is_auto_increment {
                // SERIAL类型在PostgreSQL中实际存储为integer/bigint
                match effective_rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(), // SMALLSERIAL -> SMALLINT
                    "i32" | "u16" | "u32" => "INTEGER".to_string(), // SERIAL -> INTEGER
                    "i64" | "u64" => "BIGINT".to_string(),         // BIGSERIAL -> BIGINT
                    _ => "INTEGER".to_string(),
                }
            } else if expected_col.is_primary {
                // 主键的基础类型
                match effective_rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(),
                    "i32" | "u16" | "u32" => "INTEGER".to_string(),
                    "i64" | "u64" => "BIGINT".to_string(),
                    // 非整数主键（如 NaiveDateTime）使用 sql_type 获取基础类型
                    _ => {
                        let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                            effective_rust_type,
                            false,
                            expected_col.is_auto_increment,
                            expected_col.is_nullable,
                            expected_col.enum_variants,
                        );
                        full_type.replace(" NOT NULL", "")
                    }
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                    effective_rust_type,
                    false,
                    expected_col.is_auto_increment,
                    expected_col.is_nullable,
                    expected_col.enum_variants,
                );
                // 去掉 " NOT NULL" 后缀
                full_type.replace(" NOT NULL", "")
            };

            if !self.types_compatible(actual_type, &type_to_compare) {
                return Err(anyhow::anyhow!(
                    "Schema mismatch: table {}, reason: Column type mismatch for '{}': expected '{expected_type}', but actual is '{actual_type}'",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            // 检查 NOT NULL 约束（主键列除外，因为主键自动 NOT NULL）
            if !expected_col.is_primary {
                let expected_nullable = expected_col.is_nullable;
                if *actual_nullable != expected_nullable {
                    return Err(anyhow::anyhow!(
                        "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                        T::TABLE_NAME,
                        expected_col.name,
                        if expected_nullable { "" } else { "NOT " },
                        if *actual_nullable { "" } else { "NOT " }
                    ));
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

    /// 批量插入记录，返回自增主键值（如果有自增主键）或 ()
    /// 对于批量插入，返回的是第一条插入记录的自增ID（即最小值）
    pub(crate) async fn insert_impl<T: Model>(
        &self,
        models: &[&T],
    ) -> anyhow::Result<T::AutoIncrementKeyType> {
        if models.is_empty() {
            return Ok(T::AutoIncrementKeyType::default());
        }

        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);

        let columns = T::insert_columns();
        let (sql, _) =
            super::common::common_helpers::build_batch_insert_sql_postgresql_with_columns(
                T::TABLE_NAME,
                &columns,
                models.len(),
            );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );

        // 获取列的rust_type信息（排除自增主键，优先使用data_type覆盖）
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        if has_auto_increment {
            // 获取自增主键列名
            let pk_col = T::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            // 使用 RETURNING 子句获取插入的ID
            let sql_with_returning = format!("{} RETURNING {}", sql, pk_col);
            let rows = self
                .client
                .query(&sql_with_returning, &param_refs)
                .trace()
                .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => {
                    return Err(anyhow::anyhow!("No rows returned from batch insert"));
                }
            };
            // 根据列类型读取自增主键值（SERIAL = INT4, BIGSERIAL = INT8）
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            let result = convert_auto_increment_key::<T::AutoIncrementKeyType>(id)?;
            Ok(result)
        } else {
            self.client.execute(&sql, &param_refs).trace().await?;
            Ok(T::AutoIncrementKeyType::default())
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let primary_key = T::primary_key_columns()[0];

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
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

            let values = model.field_values();
            all_values.extend(values);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO UPDATE SET "));
        let mut first = true;
        for col_name in T::COLUMNS.iter() {
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
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        self.client.execute(&sql, &param_refs).trace().await?;
        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略）
    pub async fn insert_or_ignore_batch<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let primary_key_columns = T::primary_key_columns();
        let primary_key = primary_key_columns.join(", ");

        // 构建批量插入或忽略的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO NOTHING
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ",
            T::TABLE_NAME,
            columns.join(", ")
        );
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

        // 添加 ON CONFLICT DO NOTHING 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO NOTHING"));

        // 获取列的rust_type信息（排除自增主键，优先使用data_type覆盖）
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        self.client.execute(&sql, &param_refs).trace().await?;
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

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
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
            model_updates: Vec::new(),
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
    pub async fn begin(&self) -> anyhow::Result<Transaction<'_>> {
        self.client.execute("BEGIN", &[]).trace().await?;
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
    /// 执行原生 SQL 查询并返回模型列表
    pub async fn execute<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        let rows = self.client.query(sql, &[]).trace().await?;

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

    /// 执行原生 SQL 查询并返回模型列表（向后兼容）
    #[deprecated(since = "0.1.0", note = "请使用 execute 方法")]
    pub async fn exec_table<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        self.execute::<T>(sql).await
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn exec_non_query(&self, sql: &str) -> anyhow::Result<u64> {
        let result = self.client.execute(sql, &[]).trace().await?;
        Ok(result)
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        self.client.execute("SELECT 1", &[]).trace().await.is_ok()
    }
}

/// PostgreSQL 事务对象
pub struct Transaction<'a> {
    client: &'a tokio_postgres::Client,
    committed: bool,
    rolled_back: bool,
}

/// 事务中的插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<<I::Model as Model>::AutoIncrementKeyType> {
        let refs = self.models.as_refs();
        // 直接调用批量插入逻辑，使用 client 引用
        if refs.is_empty() {
            return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default());
        }

        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);

        let columns = I::Model::insert_columns();
        let (sql, _) =
            super::common::common_helpers::build_batch_insert_sql_postgresql_with_columns(
                I::Model::TABLE_NAME,
                &columns,
                refs.len(),
            );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<
                I::Model,
            >(&refs);

        // 获取列的rust_type信息（排除自增主键）
        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        if has_auto_increment {
            // 获取自增主键列名
            let pk_col = I::Model::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            // 使用 RETURNING 子句获取插入的ID
            let sql_with_returning = format!("{} RETURNING {}", sql, pk_col);
            let rows = self
                .client
                .query(&sql_with_returning, &param_refs)
                .trace()
                .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => {
                    return Err(anyhow::anyhow!("No rows returned from batch insert"));
                }
            };
            // 根据列类型读取自增主键值（SERIAL = INT4, BIGSERIAL = INT8）
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            let result =
                convert_auto_increment_key::<<I::Model as Model>::AutoIncrementKeyType>(id)?;
            Ok(result)
        } else {
            self.client.execute(&sql, &param_refs).trace().await?;
            Ok(<<I::Model as Model>::AutoIncrementKeyType>::default())
        }
    }
}

/// 事务中的插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(());
        }
        let columns = I::Model::insert_columns();
        let col_count = columns.len();
        let primary_key_columns = I::Model::primary_key_columns();
        let primary_key = primary_key_columns.join(", ");
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ",
            I::Model::TABLE_NAME,
            columns.join(", ")
        );
        let mut all_values = Vec::new();
        let mut param_idx = 1;
        for (idx, _model) in refs.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count)
                .map(|i| format!("${}", param_idx + i - 1))
                .collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            param_idx += col_count;
        }
        sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
        let mut first = true;
        for col_name in I::Model::COLUMNS.iter() {
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = EXCLUDED.{}", col_name, col_name));
            first = false;
        }
        for model in &refs {
            let values = (*model).field_values();
            all_values.extend(values);
        }

        // 获取列的rust类型信息（排除自增主键）
        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        self.client.execute(&sql, &param_refs).trace().await?;
        Ok(())
    }
}

/// 事务中的插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(());
        }
        let columns = I::Model::insert_columns();
        let col_count = columns.len();
        let primary_key_columns = I::Model::primary_key_columns();
        let primary_key = primary_key_columns.join(", ");
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ",
            I::Model::TABLE_NAME,
            columns.join(", ")
        );
        let mut all_values = Vec::new();
        let mut param_idx = 1;
        for (idx, _model) in refs.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count)
                .map(|i| format!("${}", param_idx + i - 1))
                .collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            param_idx += col_count;
        }
        sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));
        for model in &refs {
            let values = (*model).insert_values();
            all_values.extend(values);
        }

        // 获取列的rust类型信息（排除自增主键）
        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.rust_type)
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        self.client.execute(&sql, &param_refs).trace().await?;
        Ok(())
    }
}

impl<'a> Transaction<'a> {
    /// 提交事务
    pub async fn commit(mut self) -> anyhow::Result<()> {
        if self.committed || self.rolled_back {
            return Err(anyhow::anyhow!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        self.client.execute("COMMIT", &[]).trace().await?;
        self.committed = true;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> anyhow::Result<()> {
        if self.committed || self.rolled_back {
            return Err(anyhow::anyhow!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        self.client.execute("ROLLBACK", &[]).trace().await?;
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

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
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
            model_updates: Vec::new(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            client: self.client,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        TransactionInsertOrUpdateExecutor {
            client: self.client,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        TransactionInsertOrIgnoreExecutor {
            client: self.client,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入记录
    #[allow(dead_code)]
    pub(crate) async fn insert_impl<T: Model>(
        &self,
        models: &[&T],
    ) -> anyhow::Result<T::AutoIncrementKeyType> {
        if models.is_empty() {
            return Ok(T::AutoIncrementKeyType::default());
        }

        let has_auto_increment = T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let columns = T::insert_columns();
        let (sql, _) =
            super::common::common_helpers::build_batch_insert_sql_postgresql_with_columns(
                T::TABLE_NAME,
                &columns,
                models.len(),
            );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );

        // 获取列的rust_type信息（排除自增主键，优先使用data_type覆盖）
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        if has_auto_increment {
            // 获取自增主键列名
            let pk_col = T::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            // 使用 RETURNING 子句获取插入的ID
            let sql_with_returning = format!("{} RETURNING {}", sql, pk_col);
            let rows = self
                .client
                .query(&sql_with_returning, &param_refs)
                .trace()
                .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => {
                    return Err(anyhow::anyhow!("No rows returned from batch insert"));
                }
            };
            // 根据列类型读取自增主键值（SERIAL = INT4, BIGSERIAL = INT8）
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            let result = convert_auto_increment_key::<T::AutoIncrementKeyType>(id)?;
            Ok(result)
        } else {
            self.client.execute(&sql, &param_refs).trace().await?;
            Ok(T::AutoIncrementKeyType::default())
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let primary_key = T::primary_key_columns()[0];

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!("INSERT INTO {} ({columns}) VALUES ", T::TABLE_NAME);
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

            let values = model.field_values();
            all_values.extend(values);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO UPDATE SET "));
        let mut first = true;
        for col_name in T::COLUMNS.iter() {
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
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        let params = values_to_params_with_types(&all_values, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        self.client.execute(&sql, &param_refs).trace().await?;

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

/// 分组聚合查询执行器
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V)>,
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 生成子查询SQL和参数
    pub fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        self.select.to_sql_with_params(DbType::PostgreSQL)
    }

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

impl<
    'a,
    T: Model + 'static + Send,
    V: crate::model::FromRowValues + 'static + Send,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let param_rust_types = self.select.param_rust_types();
            let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
            let pg_params = values_to_params_for_query(&params, &param_rust_types)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let rows = self.client.query(&sql, &param_refs).trace().await?;

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

    /// 启用 DISTINCT 去重
    pub fn distinct(self) -> Self {
        Self {
            select: self.select.distinct(),
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

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(T::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        let grouped_select = self.select.select_column(f);
        GroupedSelectExecutor {
            select: grouped_select,
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

    /// 执行查询并返回第一条记录
    pub fn first(self) -> FirstFuture<'a, T> {
        FirstFuture { executor: self }
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

/// First future for单条记录查询
pub struct FirstFuture<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model + 'static + Send, R: crate::model::FromValue + 'static + Send>
    std::future::IntoFuture for AggregateFuture<'a, T, R>
{
    type Output = anyhow::Result<R>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (mut sql, params) = self.aggregate_select.to_sql_with_params(DbType::PostgreSQL);

            // 对于 AVG 聚合,PostgreSQL 返回 NUMERIC 类型,需要 CAST 为 FLOAT8
            // 这样可以避免 tokio-postgres 不支持 NUMERIC 类型的问题
            if sql.contains("SELECT AVG(") {
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
            let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync + Send>> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => {
                        // PostgreSQL INTEGER (Int4) 是 32 位，需要将 i64 转换为 i32
                        if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                            Box::new(i as i32) as Box<dyn postgres_types::ToSql + Sync + Send>
                        } else {
                            Box::new(i) as Box<dyn postgres_types::ToSql + Sync + Send>
                        }
                    }
                    crate::model::Value::Text(t) => {
                        Box::new(t) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Real(r) => {
                        Box::new(r) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Boolean(b) => {
                        Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Bytes(b) => {
                        Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::DateTime(dt) => {
                        Box::new(dt) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Json(j) => {
                        Box::new(j.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Uuid(u) => {
                        Box::new(u.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::BigInt(b) => {
                        Box::new(b as i64) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Null => {
                        Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync + Send>
                    }
                })
                .collect();

            let params_ref: Vec<&(dyn postgres_types::ToSql + Sync)> = pg_params
                .iter()
                .map(|p| p.as_ref() as &(dyn postgres_types::ToSql + Sync))
                .collect();

            let row = self.client.query_one(&sql, &params_ref).trace().await?;

            // 获取第一列的值
            use tokio_postgres::types::Type;
            let column_type = row.columns()[0].type_();

            // 根据类型获取值
            let ormer_value = match *column_type {
                Type::INT2 => {
                    let val: Option<i16> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT4 => {
                    let val: Option<i32> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT8 => {
                    let val: Option<i64> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Integer)
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT4 => {
                    let val: Option<f32> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Real(v as f64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT8 => {
                    let val: Option<f64> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Real)
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
                    let val: Option<String> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Text)
                        .unwrap_or(crate::model::Value::Null)
                }
                _ => crate::model::Value::Null,
            };

            // 使用 FromValue 转换为目标类型
            R::from_value(&ormer_value)
        })
    }
}

impl<'a, T: Model + 'static + Send, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<'a, T, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model + 'static + Send + std::marker::Sync> std::future::IntoFuture
    for FirstFuture<'a, T>
{
    type Output = anyhow::Result<Option<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<T> = self.executor.collect_inner().await?;
            Ok(results.into_iter().next())
        })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> anyhow::Result<C> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params = values_to_params_for_query(&params, &param_rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &param_refs).trace().await?;

        let mut results = Vec::new();

        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                // 根据列的类型获取值
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.data_type.unwrap_or(column_info.rust_type);
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
                        Type::BYTEA => {
                            let v: Option<Vec<u8>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Bytes(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::TIMESTAMP => {
                            let v: Option<chrono::NaiveDateTime> = row.get(i);
                            match v {
                                Some(val) => {
                                    let utc = chrono::DateTime::from_naive_utc_and_offset(
                                        val,
                                        chrono::Utc,
                                    );
                                    crate::model::Value::DateTime(utc)
                                }
                                None => crate::model::Value::Null,
                            }
                        }
                        Type::TIMESTAMPTZ => {
                            let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::DateTime(val),
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
                                "Vec<u8>"
                                | "std::vec::Vec<u8>"
                                | "alloc::vec::Vec<u8>"
                                | "&[u8]" => {
                                    let v: Option<Vec<u8>> = row.get(i);
                                    match v {
                                        Some(val) => crate::model::Value::Bytes(val),
                                        None => crate::model::Value::Null,
                                    }
                                }
                                "NaiveDateTime" | "chrono::NaiveDateTime" => {
                                    let v: Option<chrono::NaiveDateTime> = row.get(i);
                                    match v {
                                        Some(val) => {
                                            let utc = chrono::DateTime::from_naive_utc_and_offset(
                                                val,
                                                chrono::Utc,
                                            );
                                            crate::model::Value::DateTime(utc)
                                        }
                                        None => crate::model::Value::Null,
                                    }
                                }
                                "DateTime" | "chrono::DateTime" => {
                                    let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                                    match v {
                                        Some(val) => crate::model::Value::DateTime(val),
                                        None => crate::model::Value::Null,
                                    }
                                }
                                _ => {
                                    return Err(anyhow::anyhow!(format!(
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
                        Type::BYTEA => {
                            let v: Vec<u8> = row.get(i);
                            crate::model::Value::Bytes(v)
                        }
                        Type::TIMESTAMP => {
                            let v: chrono::NaiveDateTime = row.get(i);
                            let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                            crate::model::Value::DateTime(utc)
                        }
                        Type::TIMESTAMPTZ => {
                            let v: chrono::DateTime<chrono::Utc> = row.get(i);
                            crate::model::Value::DateTime(v)
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
                                "Vec<u8>"
                                | "std::vec::Vec<u8>"
                                | "alloc::vec::Vec<u8>"
                                | "&[u8]" => {
                                    let v: Vec<u8> = row.get(i);
                                    crate::model::Value::Bytes(v)
                                }
                                "NaiveDateTime" | "chrono::NaiveDateTime" => {
                                    let v: chrono::NaiveDateTime = row.get(i);
                                    let utc =
                                        chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                                    crate::model::Value::DateTime(utc)
                                }
                                "DateTime" | "chrono::DateTime" => {
                                    let v: chrono::DateTime<chrono::Utc> = row.get(i);
                                    crate::model::Value::DateTime(v)
                                }
                                _ => {
                                    return Err(anyhow::anyhow!(format!(
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
    pub async fn execute(self) -> anyhow::Result<u64> {
        let (sql, params) = self.build_sql_with_params();

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        let pg_params = values_to_params_with_types(&params, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let result = self.client.execute(&sql, &param_refs).trace().await?;

        Ok(result)
    }

    /// 执行删除并返回被删除的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(self) -> anyhow::Result<Vec<T>> {
        let (sql, params) = self.build_sql_with_params();

        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        let pg_params = values_to_params_with_types(&params, &rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let sql_with_returning = format!("{} RETURNING *", sql);
        let rows = self
            .client
            .query(&sql_with_returning, &param_refs)
            .trace()
            .await?;

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
                let _ = common_helpers::format_filter_with_params(
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

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for DeleteExecutor<'a, T> {
    type Output = anyhow::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<(String, Value)>,
    filters: Vec<FilterExpr>,
    model_updates: Vec<(Vec<(String, Value)>, Vec<FilterExpr>)>,
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

    /// 从模型实例设置所有非主键字段，并自动添加主键作为 WHERE 条件
    ///
    /// ```ignore
    /// let user = User { id: 1, name: "Bob".into(), age: 25, email: Some("bob@test.com".into()) };
    /// db.update::<User>().set_model(&user).execute().await?;
    /// ```
    pub fn set_model(mut self, model: &T) -> Self {
        let mut model_sets = Vec::new();
        for (col_name, value) in model.non_pk_field_values() {
            model_sets.push((col_name.to_string(), value));
        }
        let pk_columns = T::primary_key_columns();
        let pk_values = model.primary_key_values();
        let mut model_filters = Vec::new();
        for (col, val) in pk_columns.iter().zip(pk_values.into_iter()) {
            let filter_val =
                crate::abstract_layer::common::common_helpers::value_to_filter_value(&val);
            model_filters.push(crate::query::filter::FilterExpr::Comparison {
                column: col.to_string(),
                operator: "=".to_string(),
                value: filter_val,
            });
        }
        self.model_updates.push((model_sets, model_filters));
        self
    }

    /// 执行更新操作
    pub async fn execute(self) -> anyhow::Result<u64> {
        let statements = self.build_all_sql()?;
        let mut total: u64 = 0;
        for (sql, params) in &statements {
            let pg_params = values_to_params(params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let result = self.client.execute(sql, &param_refs).trace().await?;
            total += result;
        }
        Ok(total)
    }

    /// 执行更新并返回被更新的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(self) -> anyhow::Result<Vec<T>> {
        let statements = self.build_all_sql()?;
        let mut results = Vec::new();
        for (sql, params) in &statements {
            let pg_params = values_to_params(params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let sql_with_returning = format!("{} RETURNING *", sql);
            let rows = self
                .client
                .query(&sql_with_returning, &param_refs)
                .trace()
                .await?;
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
        }
        Ok(results)
    }

    fn build_all_sql(&self) -> anyhow::Result<Vec<(String, Vec<crate::model::Value>)>> {
        let mut statements = Vec::new();

        // Base UPDATE from sets/filters
        if !self.sets.is_empty() || (self.model_updates.is_empty() && !self.filters.is_empty()) {
            let mut sql = format!("UPDATE {} SET ", T::TABLE_NAME);
            let mut params = Vec::new();
            let mut first = true;
            for (col_name, value) in &self.sets {
                if !first {
                    sql.push_str(", ");
                }
                sql.push_str(&format!("{col_name} = ${}", params.len() + 1));
                params.push(value.clone());
                first = false;
            }
            if !self.filters.is_empty() {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                for (i, filter) in self.filters.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(" AND ");
                    }
                    let _ = common_helpers::format_filter_with_params(
                        filter,
                        &mut sql,
                        &mut param_idx,
                        &mut params,
                        DbType::PostgreSQL,
                    );
                }
            }
            statements.push((sql, params));
        }

        // Model UPDATE statements
        for (model_sets, model_filters) in &self.model_updates {
            let mut sql = format!("UPDATE {} SET ", T::TABLE_NAME);
            let mut params = Vec::new();
            let mut first = true;
            for (col_name, value) in model_sets {
                if !first {
                    sql.push_str(", ");
                }
                sql.push_str(&format!("{col_name} = ${}", params.len() + 1));
                params.push(value.clone());
                first = false;
            }
            if !model_filters.is_empty() {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                for (i, filter) in model_filters.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(" AND ");
                    }
                    let _ = common_helpers::format_filter_with_params(
                        filter,
                        &mut sql,
                        &mut param_idx,
                        &mut params,
                        DbType::PostgreSQL,
                    );
                }
            }
            statements.push((sql, params));
        }

        Ok(statements)
    }
}

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for UpdateExecutor<'a, T> {
    type Output = anyhow::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 tokio-postgres 参数
/// 根据列的rust_type选择正确的参数类型（i32或i64）
fn values_to_params_with_types(
    values: &[crate::model::Value],
    rust_types: &[&str],
) -> anyhow::Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>> {
    // ToSql trait is used in the trait object type above
    #[allow(unused_imports)]
    use tokio_postgres::types::ToSql;

    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();

    for (idx, value) in values.iter().enumerate() {
        // 循环使用rust_types，因为values可能包含多个记录的所有字段
        let rust_type = rust_types[idx % rust_types.len()];

        let param: Box<dyn tokio_postgres::types::ToSql + Sync + Send> = match value {
            crate::model::Value::Integer(v) => {
                // 根据列的rust_type选择合适的整数类型
                // tokio-postgres要求Rust类型与PostgreSQL类型严格匹配
                let use_i64 = matches!(rust_type, "i64" | "u64");
                let is_known_int = matches!(
                    rust_type,
                    "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize"
                );
                if use_i64 {
                    Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                } else if is_known_int {
                    // 对于i32列，将值转换为i32
                    Box::new(*v as i32) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                } else {
                    // 非标准整数类型（如 flatbuffers newtype），数据库列可能是 TEXT
                    // 转为字符串传递，避免类型不匹配导致序列化失败
                    Box::new(v.to_string()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                }
            }
            crate::model::Value::Text(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Real(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Boolean(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Bytes(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::DateTime(v) => {
                // NaiveDateTime 对应 PostgreSQL TIMESTAMP，DateTime<Utc> 对应 TIMESTAMPTZ
                // 根据列的 rust_type 决定传递哪种类型，避免类型不匹配导致序列化失败
                if rust_type == "NaiveDateTime" || rust_type == "chrono::NaiveDateTime" {
                    Box::new(v.naive_utc()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                } else {
                    // DateTime<Utc> 需要传递 chrono::DateTime<chrono::Utc> 类型
                    Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                }
            }
            crate::model::Value::Json(v) => {
                Box::new(v.to_string()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Uuid(v) => {
                Box::new(v.to_string()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::BigInt(v) => {
                Box::new(*v as i64) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Null => {
                // 根据列类型选择NULL的类型
                match rust_type {
                    "i64" | "u64" => {
                        let null_val: Option<i64> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "i32" | "i16" | "i8" | "u16" | "u32" | "u8" => {
                        let null_val: Option<i32> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "String" | "&str" => {
                        let null_val: Option<String> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "f32" | "f64" => {
                        let null_val: Option<f64> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "bool" => {
                        let null_val: Option<bool> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let null_val: Option<Vec<u8>> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let null_val: Option<chrono::DateTime<chrono::Utc>> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let null_val: Option<chrono::NaiveDateTime> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    _ => {
                        // 默认使用Option<i32>
                        let null_val: Option<i32> = None;
                        Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                }
            }
        };
        params.push(param);
    }

    Ok(params)
}

fn values_to_params_for_query(
    values: &[crate::model::Value],
    rust_types: &[&str],
) -> anyhow::Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>> {
    if values.len() == rust_types.len() {
        values_to_params_with_types(values, rust_types)
    } else {
        values_to_params(values)
    }
}

/// 将 ormer Value 转换为 tokio-postgres 参数（旧版本，根据值大小选择类型）
fn values_to_params(
    values: &[crate::model::Value],
) -> anyhow::Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>> {
    // ToSql trait is used in the trait object type above
    #[allow(unused_imports)]
    use tokio_postgres::types::ToSql;

    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();

    for value in values {
        let param: Box<dyn tokio_postgres::types::ToSql + Sync + Send> = match value {
            crate::model::Value::Integer(v) => {
                // 使用 i32 作为默认,因为大多数用户定义的列是 INTEGER
                // 对于聚合函数(COUNT等返回BIGINT)的比较,PostgreSQL会自动提升i32到i64
                Box::new(*v as i32) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Text(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Real(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Boolean(v) => {
                Box::new(*v) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Bytes(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::DateTime(v) => {
                Box::new(v.clone()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Json(v) => {
                Box::new(v.to_string()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Uuid(v) => {
                Box::new(v.to_string()) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::BigInt(v) => {
                Box::new(*v as i64) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
            }
            crate::model::Value::Null => {
                // 使用 Option<i32> 的 None 来表示 NULL
                let null_val: Option<i32> = None;
                Box::new(null_val) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
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

/// SelectStream - 流式查询执行器 (PostgreSQL)
pub struct SelectStream<'a, T: Model> {
    select: Select<T>,
    conn: super::common::StreamConnection<'a>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            select: self.select,
            conn: super::common::StreamConnection::PostgreSQL(self.client),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器  
    pub async fn into_iter(self) -> anyhow::Result<SelectStreamIterator<'a, T>> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params_for_query(&params, &param_rust_types)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        // 从 StreamConnection 获取 client 引用
        let client = match &self.conn {
            super::common::StreamConnection::PostgreSQL(c) => c,
            _ => unreachable!("Expected PostgreSQL connection"),
        };

        // 使用 query_raw 获取 RowStream
        let row_stream = client.query_raw(&sql, param_refs).trace().await?;

        Ok(SelectStreamIterator {
            conn: self.conn,
            row_stream: Box::pin(row_stream),
            _marker: std::marker::PhantomData,
        })
    }
}

/// SelectStreamIterator - 流式查询迭代器 (PostgreSQL)
pub struct SelectStreamIterator<'a, T: Model> {
    #[allow(dead_code)]
    conn: super::common::StreamConnection<'a>,
    row_stream: std::pin::Pin<Box<tokio_postgres::RowStream>>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<anyhow::Result<T>> {
        use futures::StreamExt;

        match self.row_stream.next().await {
            Some(Ok(row)) => {
                // 解析行数据为 Model
                let mut data = HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let column_info = &T::COLUMN_SCHEMA[i];
                    let rust_type = column_info.rust_type;
                    let is_nullable = column_info.is_nullable;

                    let ormer_value = if is_nullable {
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
                            _ => match rust_type {
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
                                "Vec<u8>"
                                | "std::vec::Vec<u8>"
                                | "alloc::vec::Vec<u8>"
                                | "&[u8]" => {
                                    let v: Option<Vec<u8>> = row.get(i);
                                    match v {
                                        Some(val) => crate::model::Value::Bytes(val),
                                        None => crate::model::Value::Null,
                                    }
                                }
                                "NaiveDateTime" | "chrono::NaiveDateTime" => {
                                    let v: Option<chrono::NaiveDateTime> = row.get(i);
                                    match v {
                                        Some(val) => {
                                            let utc = chrono::DateTime::from_naive_utc_and_offset(
                                                val,
                                                chrono::Utc,
                                            );
                                            crate::model::Value::DateTime(utc)
                                        }
                                        None => crate::model::Value::Null,
                                    }
                                }
                                "DateTime" | "chrono::DateTime" => {
                                    let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                                    match v {
                                        Some(val) => crate::model::Value::DateTime(val),
                                        None => crate::model::Value::Null,
                                    }
                                }
                                _ => {
                                    return Some(Err(anyhow::anyhow!(format!(
                                        "Unsupported nullable column type: {rust_type}"
                                    ))));
                                }
                            },
                        }
                    } else {
                        // 非空类型处理 - 解析失败时返回错误
                        match rust_type {
                            "i8" | "i16" | "i32" | "u8" | "u16" | "u32" => {
                                let v: Option<i32> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::Integer(val as i64),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected integer type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "i64" | "u64" => {
                                let v: Option<i64> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::Integer(val),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected i64 type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "String" => {
                                let v: Option<String> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::Text(val),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected String type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "f32" | "f64" => {
                                let v: Option<f64> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::Real(val),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected float type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "bool" => {
                                let v: Option<bool> = row.get(i);
                                match v {
                                    Some(true) => crate::model::Value::Integer(1),
                                    Some(false) => crate::model::Value::Integer(0),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected bool type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                                let v: Option<Vec<u8>> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::Bytes(val),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected Vec<u8> type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "NaiveDateTime" | "chrono::NaiveDateTime" => {
                                let v: Option<chrono::NaiveDateTime> = row.get(i);
                                match v {
                                    Some(val) => {
                                        let utc = chrono::DateTime::from_naive_utc_and_offset(
                                            val,
                                            chrono::Utc,
                                        );
                                        crate::model::Value::DateTime(utc)
                                    }
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected NaiveDateTime type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            "DateTime" | "chrono::DateTime" => {
                                let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                                match v {
                                    Some(val) => crate::model::Value::DateTime(val),
                                    None => {
                                        return Some(Err(anyhow::anyhow!(format!(
                                            "Failed to parse non-nullable column '{}' (expected DateTime type)",
                                            col_name
                                        ))));
                                    }
                                }
                            }
                            _ => {
                                return Some(Err(anyhow::anyhow!(format!(
                                    "Unsupported column type: {rust_type}"
                                ))));
                            }
                        }
                    };

                    data.insert(col_name.to_string(), ormer_value);
                }
                let ormer_row = crate::model::Row::new(data);
                Some(T::from_row(&ormer_row))
            }
            Some(Err(e)) => Some(Err(anyhow::anyhow!(
                "tokio_postgres::RowStream::next failed: {e}"
            ))),
            None => None,
        }
    }
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

    pub async fn collect<C: FromIterator<T>>(self) -> anyhow::Result<C> {
        let results = self.collect_inner().trace().await?;
        Ok(results.into_iter().collect())
    }

    async fn collect_inner(self) -> anyhow::Result<Vec<T>> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &param_refs).trace().await?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.data_type.unwrap_or(column_info.rust_type);
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Option<Vec<u8>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Bytes(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: Option<chrono::NaiveDateTime> = row.get(i);
                            match v {
                                Some(val) => {
                                    let utc = chrono::DateTime::from_naive_utc_and_offset(
                                        val,
                                        chrono::Utc,
                                    );
                                    crate::model::Value::DateTime(utc)
                                }
                                None => crate::model::Value::Null,
                            }
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::DateTime(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Vec<u8> = row.get(i);
                            crate::model::Value::Bytes(v)
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: chrono::NaiveDateTime = row.get(i);
                            let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                            crate::model::Value::DateTime(utc)
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: chrono::DateTime<chrono::Utc> = row.get(i);
                            crate::model::Value::DateTime(v)
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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

impl<'a, T: Model + 'static + Send, R: Model + 'static + Send> std::future::IntoFuture
    for RelatedCollectFuture<'a, T, R>
{
    type Output = anyhow::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

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

    async fn collect_inner(self) -> anyhow::Result<Vec<T>> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &param_refs).trace().await?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.data_type.unwrap_or(column_info.rust_type);
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Option<Vec<u8>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Bytes(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: Option<chrono::NaiveDateTime> = row.get(i);
                            match v {
                                Some(val) => {
                                    let utc = chrono::DateTime::from_naive_utc_and_offset(
                                        val,
                                        chrono::Utc,
                                    );
                                    crate::model::Value::DateTime(utc)
                                }
                                None => crate::model::Value::Null,
                            }
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::DateTime(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Vec<u8> = row.get(i);
                            crate::model::Value::Bytes(v)
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: chrono::NaiveDateTime = row.get(i);
                            let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                            crate::model::Value::DateTime(utc)
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: chrono::DateTime<chrono::Utc> = row.get(i);
                            crate::model::Value::DateTime(v)
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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

impl<'a, T: Model + 'static + Send, R1: Model + 'static + Send, R2: Model + 'static + Send>
    std::future::IntoFuture for MultiTableCollectFuture<'a, T, R1, R2>
{
    type Output = anyhow::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

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

    async fn collect_inner(self) -> anyhow::Result<Vec<T>> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let pg_params = values_to_params(&params)?;
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| &**p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &param_refs).trace().await?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let column_info = &T::COLUMN_SCHEMA[i];
                let rust_type = column_info.data_type.unwrap_or(column_info.rust_type);
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Option<Vec<u8>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::Bytes(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: Option<chrono::NaiveDateTime> = row.get(i);
                            match v {
                                Some(val) => {
                                    let utc = chrono::DateTime::from_naive_utc_and_offset(
                                        val,
                                        chrono::Utc,
                                    );
                                    crate::model::Value::DateTime(utc)
                                }
                                None => crate::model::Value::Null,
                            }
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: Option<chrono::DateTime<chrono::Utc>> = row.get(i);
                            match v {
                                Some(val) => crate::model::Value::DateTime(val),
                                None => crate::model::Value::Null,
                            }
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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
                        "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                            let v: Vec<u8> = row.get(i);
                            crate::model::Value::Bytes(v)
                        }
                        "NaiveDateTime" | "chrono::NaiveDateTime" => {
                            let v: chrono::NaiveDateTime = row.get(i);
                            let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                            crate::model::Value::DateTime(utc)
                        }
                        "DateTime" | "chrono::DateTime" => {
                            let v: chrono::DateTime<chrono::Utc> = row.get(i);
                            crate::model::Value::DateTime(v)
                        }
                        _ => {
                            return Err(anyhow::anyhow!(format!(
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

impl<
    'a,
    T: Model + 'static + Send,
    R1: Model + 'static + Send,
    R2: Model + 'static + Send,
    R3: Model + 'static + Send,
> std::future::IntoFuture for FourTableCollectFuture<'a, T, R1, R2, R3>
{
    type Output = anyhow::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

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

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for LeftJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(T, Option<J>)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync + Send>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Boolean(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Bytes(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::DateTime(dt) => {
                    Box::new(dt) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Json(j) => {
                    Box::new(j.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Uuid(u) => {
                    Box::new(u.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::BigInt(b) => {
                    Box::new(b as i64) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn postgres_types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &pg_params_refs).trace().await?;

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
                    "bool" => {
                        let v: Option<bool> = row.try_get(i).ok().flatten();
                        match v {
                            Some(true) => crate::model::Value::Integer(1),
                            Some(false) => crate::model::Value::Integer(0),
                            None => crate::model::Value::Null,
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Option<Vec<u8>> = row.try_get(i).ok().flatten();
                        crate::model::Value::Bytes(v.unwrap_or_default())
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: Option<chrono::NaiveDateTime> = row.try_get(i).ok().flatten();
                        match v {
                            Some(val) => {
                                let utc =
                                    chrono::DateTime::from_naive_utc_and_offset(val, chrono::Utc);
                                crate::model::Value::DateTime(utc)
                            }
                            None => crate::model::Value::Null,
                        }
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: Option<chrono::DateTime<chrono::Utc>> =
                            row.try_get(i).ok().flatten();
                        match v {
                            Some(val) => crate::model::Value::DateTime(val),
                            None => crate::model::Value::Null,
                        }
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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
                    "bool" => {
                        let v: Option<bool> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        match v {
                            Some(true) => crate::model::Value::Integer(1),
                            Some(false) => crate::model::Value::Integer(0),
                            None => crate::model::Value::Null,
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Option<Vec<u8>> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        crate::model::Value::Bytes(v.unwrap_or_default())
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: Option<chrono::NaiveDateTime> = row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        match v {
                            Some(val) => {
                                let utc =
                                    chrono::DateTime::from_naive_utc_and_offset(val, chrono::Utc);
                                crate::model::Value::DateTime(utc)
                            }
                            None => crate::model::Value::Null,
                        }
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: Option<chrono::DateTime<chrono::Utc>> =
                            row.try_get(idx).ok().flatten();
                        if v.is_some() {
                            j_is_null = false;
                        }
                        match v {
                            Some(val) => crate::model::Value::DateTime(val),
                            None => crate::model::Value::Null,
                        }
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for InnerJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(T, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync + Send>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Boolean(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Bytes(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::DateTime(dt) => {
                    Box::new(dt) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Json(j) => {
                    Box::new(j.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Uuid(u) => {
                    Box::new(u.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::BigInt(b) => {
                    Box::new(b as i64) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn postgres_types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &pg_params_refs).trace().await?;

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
                    "bool" => {
                        let v: bool = row.get(i);
                        if v {
                            crate::model::Value::Integer(1)
                        } else {
                            crate::model::Value::Integer(0)
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Vec<u8> = row.get(i);
                        crate::model::Value::Bytes(v)
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: chrono::NaiveDateTime = row.get(i);
                        let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                        crate::model::Value::DateTime(utc)
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: chrono::DateTime<chrono::Utc> = row.get(i);
                        crate::model::Value::DateTime(v)
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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
                    "bool" => {
                        let v: bool = row.get(idx);
                        if v {
                            crate::model::Value::Integer(1)
                        } else {
                            crate::model::Value::Integer(0)
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Vec<u8> = row.get(idx);
                        crate::model::Value::Bytes(v)
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: chrono::NaiveDateTime = row.get(idx);
                        let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                        crate::model::Value::DateTime(utc)
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: chrono::DateTime<chrono::Utc> = row.get(idx);
                        crate::model::Value::DateTime(v)
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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

/// Grouped Collect future（分组聚合查询）
pub struct GroupedCollectFuture<'a, T: Model, V, C> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(T, V, C)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for RightJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(Option<T>, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);

        let pg_params: Vec<Box<dyn postgres_types::ToSql + Sync + Send>> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => {
                    Box::new(i) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Text(t) => {
                    Box::new(t) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Real(r) => {
                    Box::new(r) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Boolean(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Bytes(b) => {
                    Box::new(b) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::DateTime(dt) => {
                    Box::new(dt) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Json(j) => {
                    Box::new(j.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Uuid(u) => {
                    Box::new(u.to_string()) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::BigInt(b) => {
                    Box::new(b as i64) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
                crate::model::Value::Null => {
                    Box::new(None::<i32>) as Box<dyn postgres_types::ToSql + Sync + Send>
                }
            })
            .collect();

        let pg_params_refs: Vec<&(dyn postgres_types::ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn postgres_types::ToSql + Sync))
            .collect();

        let rows = self.client.query(&sql, &pg_params_refs).trace().await?;

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
                    "bool" => {
                        let v: Option<bool> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        match v {
                            Some(true) => crate::model::Value::Integer(1),
                            Some(false) => crate::model::Value::Integer(0),
                            None => crate::model::Value::Null,
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Option<Vec<u8>> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        crate::model::Value::Bytes(v.unwrap_or_default())
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: Option<chrono::NaiveDateTime> = row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        match v {
                            Some(val) => {
                                let utc =
                                    chrono::DateTime::from_naive_utc_and_offset(val, chrono::Utc);
                                crate::model::Value::DateTime(utc)
                            }
                            None => crate::model::Value::Null,
                        }
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: Option<chrono::DateTime<chrono::Utc>> =
                            row.try_get(i).ok().flatten();
                        if v.is_some() {
                            t_is_null = false;
                        }
                        match v {
                            Some(val) => crate::model::Value::DateTime(val),
                            None => crate::model::Value::Null,
                        }
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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
                    "bool" => {
                        let v: bool = row.get(idx);
                        if v {
                            crate::model::Value::Integer(1)
                        } else {
                            crate::model::Value::Integer(0)
                        }
                    }
                    "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                        let v: Vec<u8> = row.get(idx);
                        crate::model::Value::Bytes(v)
                    }
                    "NaiveDateTime" | "chrono::NaiveDateTime" => {
                        let v: chrono::NaiveDateTime = row.get(idx);
                        let utc = chrono::DateTime::from_naive_utc_and_offset(v, chrono::Utc);
                        crate::model::Value::DateTime(utc)
                    }
                    "DateTime" | "chrono::DateTime" => {
                        let v: chrono::DateTime<chrono::Utc> = row.get(idx);
                        crate::model::Value::DateTime(v)
                    }
                    _ => {
                        return Err(anyhow::anyhow!(format!(
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
) -> anyhow::Result<crate::model::Value> {
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
        // 字节类型
        Type::BYTEA => {
            if let Ok(v) = row.try_get::<_, Option<Vec<u8>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Bytes(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 日期时间类型
        Type::TIMESTAMP => {
            if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDateTime>>(index) {
                return Ok(match v {
                    Some(val) => {
                        let utc = chrono::DateTime::from_naive_utc_and_offset(val, chrono::Utc);
                        crate::model::Value::DateTime(utc)
                    }
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::TIMESTAMPTZ => {
            if let Ok(v) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::DateTime(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        _ => {}
    }

    Err(anyhow::anyhow!(format!(
        "Unsupported column type {:?} at index {}",
        col_type, index
    )))
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        GroupedCollectFuture {
            executor: GroupedSelectExecutor {
                select: self.select.clone(),
                client: self.client,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        Self {
            select: self.select.group_by(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 HAVING 条件
    pub fn having<F>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::WhereExpr,
    {
        Self {
            select: self.select.having(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::WhereExpr,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
            _marker: PhantomData,
        }
    }
}

impl<
    'a,
    T: Model + 'static + Send,
    V: crate::model::FromRowValues + 'static + Send,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for GroupedCollectFuture<'a, T, V, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.executor.select.build_sql(DbType::PostgreSQL);

            // 对于PostgreSQL,我们需要智能地转换参数类型
            // 如果SQL中包含::bigint(通常在HAVING子句中),使用i64
            // 否则使用i32
            let use_i64 = sql.contains("::bigint");

            let pg_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => {
                        if use_i64 {
                            Box::new(i) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                        } else {
                            Box::new(i as i32)
                                as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                        }
                    }
                    crate::model::Value::Text(t) => {
                        Box::new(t) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Real(r) => {
                        Box::new(r) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Boolean(b) => {
                        Box::new(b) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Bytes(b) => {
                        Box::new(b) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::DateTime(dt) => {
                        Box::new(dt) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Json(j) => Box::new(j.to_string())
                        as Box<dyn tokio_postgres::types::ToSql + Sync + Send>,
                    crate::model::Value::Uuid(u) => Box::new(u.to_string())
                        as Box<dyn tokio_postgres::types::ToSql + Sync + Send>,
                    crate::model::Value::BigInt(b) => {
                        Box::new(b as i64) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                    }
                    crate::model::Value::Null => {
                        if use_i64 {
                            let null_val: Option<i64> = None;
                            Box::new(null_val)
                                as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                        } else {
                            let null_val: Option<i32> = None;
                            Box::new(null_val)
                                as Box<dyn tokio_postgres::types::ToSql + Sync + Send>
                        }
                    }
                })
                .collect();

            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pg_params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let rows = self
                .executor
                .client
                .query(&sql, &param_refs)
                .trace()
                .await?;

            let mut results = Vec::new();
            let column_count = self.executor.select.column_count();
            for row in rows {
                let mut values = Vec::with_capacity(column_count);
                for i in 0..column_count {
                    let value = convert_postgres_value(&row, i)?;
                    values.push(value);
                }

                let v = V::from_row_values(&values)?;
                results.push(v);
            }

            Ok(results.into_iter().collect())
        })
    }
}
