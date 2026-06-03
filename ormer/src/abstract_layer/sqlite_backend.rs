use super::common::common_helpers;
use crate::abstract_layer::DbType;
use crate::model::{DbBackendTypeMapper, Model, Row, Value};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MappedSelect,
    MultiTableSelect, RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use crate::query::filter::FilterExpr;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

// 导入宏
use crate::impl_backend_executor_methods;
use crate::impl_backend_join_executor_methods;
use crate::impl_backend_related_executor_methods;

/// Sqlite 类型映射器
pub struct SqliteTypeMapper;

impl DbBackendTypeMapper for SqliteTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        // SQLite 不支持原生 ENUM,降级为 TEXT
        if enum_variants.is_some() {
            let base_type = "TEXT";
            let mut sql_type = base_type.to_string();
            if !is_nullable && !is_primary {
                sql_type.push_str(" NOT NULL");
            }
            return sql_type;
        }

        // 首先处理主键类型
        if is_primary {
            if is_auto_increment {
                return "INTEGER PRIMARY KEY AUTOINCREMENT".to_string();
            } else {
                return "INTEGER PRIMARY KEY".to_string();
            }
        }

        // 基础类型映射（SQLite 类型系统更简单）
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

/// Sqlite 数据库连接封装
pub struct Database {
    conn: Arc<turso::Connection>,
}

// SAFETY: turso::Connection uses internal synchronization mechanisms
// that make it safe to share between threads. The turso library
// doesn't explicitly implement Send, but the local connection mode
// is safe to share because all operations are serialized through
// async/await.
unsafe impl Send for Database {}
unsafe impl Sync for Database {}

// Wrapper type to make turso::Connection explicitly Send
#[allow(dead_code)]
struct SendableConnection(turso::Connection);

unsafe impl Send for SendableConnection {}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: Model> {
    db: &'a Database,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> CreateTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        // 表不存在，创建新表
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::Sqlite,
            self.table_name.as_deref(),
        )?;

        self.db.conn.execute(&create_sql, ()).await?;

        Ok(())
    }
}

/// 删除表执行器（基于Model）
pub struct DropTableExecutor<'a, T: Model> {
    db: &'a Database,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> DropTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let sql = format!("DROP TABLE IF EXISTS {}", T::TABLE_NAME);
        self.db.conn.execute(&sql, ()).await?;
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
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.db.insert_impl::<I::Model>(&refs).await
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
    /// 连接到 Sqlite 数据库 (本地模式)
    pub async fn connect(_db_type: super::DbType, path: &str) -> anyhow::Result<Self> {
        let db = turso::Builder::new_local(path).build().await?;

        let conn = Arc::new(db.connect()?);

        Ok(Self { conn })
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            db: self,
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: Model>(&self) -> anyhow::Result<()> {
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().await?;

        if !table_exists {
            return Err(anyhow::anyhow!(
                "Schema mismatch: table {} does not exist",
                T::TABLE_NAME
            ));
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>().await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> anyhow::Result<bool> {
        let sql = "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?";

        let mut rows = self.conn.query(sql, [T::TABLE_NAME]).await?;

        if let Some(row) = rows.next().await? {
            let count = row.get_value(0)?;

            match count {
                turso::Value::Integer(c) => Ok(c > 0),
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(&self) -> anyhow::Result<()> {
        // 查询表的列信息
        let sql = format!("PRAGMA table_info({})", T::TABLE_NAME);

        let mut rows = self.conn.query(&sql, ()).await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool)> = Vec::new();
        while let Some(row) = rows.next().await? {
            let name = row.get_value(1)?;
            let col_type = row.get_value(2)?;
            let notnull = row.get_value(3)?;
            let pk = row.get_value(5)?;

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

            let (actual_name, actual_type, actual_notnull, actual_pk) = &actual_columns[i];

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

            // 检查主键约束
            if expected_col.is_primary != *actual_pk {
                return Err(anyhow::anyhow!(
                    "Schema mismatch: table {}, reason: Primary key mismatch for '{}': expected {}primary key, but actual is {}primary key",
                    T::TABLE_NAME,
                    expected_col.name,
                    if expected_col.is_primary { "" } else { "not " },
                    if *actual_pk { "" } else { "not " }
                ));
            }

            // 检查列类型（只比较基础类型，不包含 NOT NULL 约束）
            let expected_type = crate::abstract_layer::DbType::Sqlite.sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
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
                let full_type = crate::abstract_layer::DbType::Sqlite.sql_type(
                    expected_col.rust_type,
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

            // 检查 NOT NULL 约束（主键列自动 NOT NULL，所以不需要额外检查）
            if !expected_col.is_primary {
                let expected_notnull = !expected_col.is_nullable;
                if *actual_notnull != expected_notnull {
                    return Err(anyhow::anyhow!(
                        "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                        T::TABLE_NAME,
                        expected_col.name,
                        if expected_notnull { "NOT " } else { "" },
                        if *actual_notnull { "NOT " } else { "" }
                    ));
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

    /// 批量插入记录
    pub(crate) async fn insert_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_with_columns(
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );
        let all_params = values_to_params(&all_values)?;

        self.conn.execute(&sql, all_params).await?;

        Ok(())
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let primary_key_columns = T::primary_key_columns();
        let primary_key = primary_key_columns.join(", ");

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ",
            T::TABLE_NAME,
            columns.join(", ")
        );
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.insert_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
        let mut first = true;
        for col_name in columns.iter() {
            if primary_key_columns.contains(col_name) {
                continue; // 跳过主键
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{col_name} = excluded.{col_name}"));
            first = false;
        }

        self.conn.execute(&sql, all_params).await?;

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
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.insert_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        // 添加 ON CONFLICT DO NOTHING 子句
        sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));

        self.conn.execute(&sql, all_params).await?;

        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
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
    pub async fn begin(&self) -> anyhow::Result<Transaction> {
        self.conn.execute("BEGIN", ()).await?;
        Ok(Transaction {
            conn: self.conn.clone(),
            committed: false,
            rolled_back: false,
        })
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: Model>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            db: self,
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    /// 执行原生 SQL 查询并返回模型列表
    pub async fn execute<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        let mut rows = self.conn.query(sql, ()).await?;

        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i)?;
                let ormer_value = convert_turso_value(&value)?;
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
        let result = self.conn.execute(sql, ()).await?;
        Ok(result)
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        self.conn.execute("SELECT 1", ()).await.is_ok()
    }
}

/// Sqlite 事务对象
pub struct Transaction {
    conn: Arc<turso::Connection>,
    committed: bool,
    rolled_back: bool,
}

/// 事务中的插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.txn.insert_impl::<I::Model>(&refs).await
    }
}

/// 事务中的插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.txn.insert_or_update_impl::<I::Model>(&refs).await
    }
}

/// 事务中的插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    txn: &'a mut Transaction,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        self.txn.insert_or_ignore_impl::<I::Model>(&refs).await
    }
}

impl Transaction {
    /// 提交事务
    pub async fn commit(mut self) -> anyhow::Result<()> {
        if self.committed || self.rolled_back {
            return Err(anyhow::anyhow!(
                "Transaction already committed or rolled back"
            ));
        }
        self.conn.execute("COMMIT", ()).await?;
        self.committed = true;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> anyhow::Result<()> {
        if self.committed || self.rolled_back {
            return Err(anyhow::anyhow!(
                "Transaction already committed or rolled back"
            ));
        }
        self.conn.execute("ROLLBACK", ()).await?;
        self.rolled_back = true;
        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            conn: self.conn.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
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

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            txn: self,
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
            txn: self,
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
            txn: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入记录（内部使用）
    async fn insert_impl<T: Model>(&mut self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_with_columns(
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );
        let all_params = values_to_params(&all_values)?;

        self.conn.execute(&sql, all_params).await?;

        Ok(())
    }

    /// 批量插入或更新记录（遇到重复键时更新）（内部使用）
    async fn insert_or_update_impl<T: Model>(&mut self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let col_count = columns.len();
        let primary_key_columns = T::primary_key_columns();
        let primary_key = primary_key_columns.join(", ");

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO UPDATE SET ...
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ",
            T::TABLE_NAME,
            columns.join(", ")
        );
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.insert_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        // 添加 ON CONFLICT DO UPDATE 子句
        sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", primary_key));
        let mut first = true;
        for col_name in columns.iter() {
            if primary_key_columns.contains(col_name) {
                continue; // 跳过主键
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{col_name} = excluded.{col_name}"));
            first = false;
        }

        self.conn.execute(&sql, all_params).await?;

        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略）（内部使用）
    async fn insert_or_ignore_impl<T: Model>(&mut self, models: &[&T]) -> anyhow::Result<()> {
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
        let mut all_params = Vec::new();

        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            let placeholders: Vec<String> = (1..=col_count).map(|_| "?".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));

            let values = model.insert_values();
            let params = values_to_params(&values)?;
            all_params.extend(params);
        }

        // 添加 ON CONFLICT DO NOTHING 子句
        sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));

        self.conn.execute(&sql, all_params).await?;

        Ok(())
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    conn: Arc<turso::Connection>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> Clone for SelectExecutor<'a, T> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for LeftJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for InnerJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, J)>,
}

impl<T: Model, J: Model> Clone for RightJoinedSelectExecutor<T, J> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

/// Related 查询执行器（支持多表关联查询）
pub struct RelatedSelectExecutor<T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

/// MultiTable 查询执行器（支持3个表关联查询）
#[allow(dead_code)]
pub struct MultiTableSelectExecutor<T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2)>,
}

/// FourTable 查询执行器（支持4个表关联查询）
#[allow(dead_code)]
pub struct FourTableSelectExecutor<T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

/// Mapped 查询执行器（字段投影查询）
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: MappedSelect<T, V>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<&'a (T, V)>,
}

/// Grouped 查询执行器（分组聚合查询）
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<&'a (T, V)>,
}

impl<'a, T: Model, V> Clone for MappedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, V> Clone for GroupedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        Self {
            select: self.select.clone(),
            conn: Arc::clone(&self.conn),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
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

    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持:
    /// - 单字段:map_to(|r| r.uid) -> MappedSelectExecutor<T, i32>
    /// - 元组:map_to(|r| (r.uid, r.id)) -> MappedSelectExecutor<T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        let mapped_select = self.select.map_to(f);
        MappedSelectExecutor {
            select: mapped_select,
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 选择列(支持聚合函数)- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        let grouped_select = self.select.select_column(f);
        GroupedSelectExecutor {
            select: grouped_select,
            conn: self.conn,
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

    /// 执行查询并返回 Vec<T>
    pub fn exec(self) -> CollectFuture<'a, T, Vec<T>>
    where
        T: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// 执行查询并返回 Vec<T> (exec 的别名)
    pub fn execute(self) -> CollectFuture<'a, T, Vec<T>>
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
    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询(支持4个表)
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            conn: self.conn,
            _marker: PhantomData,
        }
    }

    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            select: self.select,
            conn: super::common::StreamConnection::Sqlite(self.conn),
            _marker: std::marker::PhantomData,
        }
    }
}

// 使用宏生成通用的 filter/order_by/range 方法
impl_backend_executor_methods!(SelectExecutor, conn, Arc<turso::Connection>, Select);

// LEFT JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    LeftJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    LeftJoinedSelect
);

impl<T: Model, J: Model> LeftJoinedSelectExecutor<T, J> {
    /// 获取 SQL（用于调试）
    pub fn to_sql(&self) -> String {
        self.select.to_sql_with_params(DbType::Sqlite).0
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<T, J> {
        LeftJoinCollectFuture {
            executor: self.clone(),
        }
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

    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().await? {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i)?;
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
// INNER JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    InnerJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    InnerJoinedSelect
);

impl<T: Model, J: Model> InnerJoinedSelectExecutor<T, J> {
    pub fn exec(self) -> InnerJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        InnerJoinCollectFuture { executor: self }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        InnerJoinCollectFuture {
            executor: self.clone(),
        }
    }

    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().await? {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i)?;
                t_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let value = row.get_value(idx)?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

// RIGHT JOIN Executor
// RIGHT JOIN Executor
// 使用宏生成通用的 filter/range 方法
impl_backend_join_executor_methods!(
    RightJoinedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    RightJoinedSelect
);

impl<T: Model, J: Model> RightJoinedSelectExecutor<T, J> {
    pub fn exec(self) -> RightJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        RightJoinCollectFuture { executor: self }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(&self) -> RightJoinCollectFuture<T, J>
    where
        T: 'static,
        J: 'static,
    {
        RightJoinCollectFuture {
            executor: self.clone(),
        }
    }

    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        while let Some(row) = rows.next().await? {
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
                let value = row.get_value(idx)?;
                j_data.insert(col_name.to_string(), convert_turso_value(&value)?);
            }
            let j_model = J::from_row(&Row::new(j_data))?;

            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: std::marker::PhantomData<C>,
}

// SAFETY: CollectFuture contains SelectExecutor which references Database (Send + Sync),
// and the async operations are all await-based which ensures thread safety
unsafe impl<'a, T: Model + Send, C: FromIterator<T> + Send> Send for CollectFuture<'a, T, C> {}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    conn: Arc<turso::Connection>,
    _marker: PhantomData<(T, R)>,
}

impl<
    T: Model + 'static + std::marker::Send,
    R: crate::model::FromValue + 'static + std::marker::Send,
> std::future::IntoFuture for AggregateFuture<T, R>
{
    type Output = anyhow::Result<R>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self.aggregate_select.to_sql_with_params(DbType::Sqlite);

            let values: Vec<turso::Value> = params
                .into_iter()
                .map(|v| match v {
                    crate::model::Value::Integer(i) => turso::Value::Integer(i),
                    crate::model::Value::Text(t) => turso::Value::Text(t),
                    crate::model::Value::Real(r) => turso::Value::Real(r),
                    crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                    crate::model::Value::Bytes(b) => turso::Value::Blob(b),
                    crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                    crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                    crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                    crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64),
                    crate::model::Value::Null => turso::Value::Null,
                })
                .collect();

            let mut rows = if values.is_empty() {
                self.conn.query(&sql, ()).await?
            } else {
                self.conn.query(&sql, values).await?
            };

            if let Some(row) = rows.next().await? {
                let value = row.get_value(0)?;

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

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for LeftJoinCollectFuture<T, J> {}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<T, J>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for InnerJoinCollectFuture<T, J> {}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<T, J>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, J: Model + Send> Send for RightJoinCollectFuture<T, J> {}

/// Grouped Collect future（分组聚合查询）
pub struct GroupedCollectFuture<'a, T: Model, V, C> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(T, V, C)>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<'a, T: Model + Send, V: Send, C: Send> Send for GroupedCollectFuture<'a, T, V, C> {}

impl<'a, T: Model + 'static + std::marker::Send + std::marker::Sync, C: FromIterator<T> + 'static>
    std::future::IntoFuture for CollectFuture<'a, T, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for LeftJoinCollectFuture<T, J>
{
    type Output = anyhow::Result<Vec<(T, Option<J>)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for InnerJoinCollectFuture<T, J>
{
    type Output = anyhow::Result<Vec<(T, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RightJoinCollectFuture<T, J>
{
    type Output = anyhow::Result<Vec<(Option<T>, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

// RelatedSelectExecutor
// 使用宏生成通用的 filter/range 方法
impl_backend_related_executor_methods!(
    RelatedSelectExecutor,
    conn,
    Arc<turso::Connection>,
    RelatedSelect
);

impl<T: Model, R: Model> RelatedSelectExecutor<T, R> {
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

    async fn collect_inner<C: FromIterator<T>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i)?;
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

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<T: Model + Send, R: Model + Send> Send for RelatedCollectFuture<T, R> {}

impl<T: Model + 'static + std::marker::Send, R: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RelatedCollectFuture<T, R>
{
    type Output = anyhow::Result<Vec<T>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> anyhow::Result<C> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        // 将 ormer::Value 转换为 turso::Value
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            let mut data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let value = row.get_value(i)?;
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
    pub async fn execute(self) -> anyhow::Result<u64> {
        let (sql, params) = self.build_sql();

        let result = self.conn.execute(&sql, params).await?;

        Ok(result)
    }

    /// 执行删除操作并返回影响的行数（execute 的别名）
    pub async fn exec(self) -> anyhow::Result<u64> {
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
                let _ = common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut ormer_params,
                    DbType::Sqlite,
                );
            }
        }

        let turso_params = values_to_params(&ormer_params).unwrap_or_default();
        (sql, turso_params)
    }
}

impl<T: Model + 'static + std::marker::Send> std::future::IntoFuture for DeleteExecutor<T> {
    type Output = anyhow::Result<u64>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

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
    pub async fn execute(self) -> anyhow::Result<u64> {
        let (sql, params) = self.build_sql()?;

        let result = self.conn.execute(&sql, params).await?;

        Ok(result)
    }

    /// 执行更新操作（execute 的别名）
    pub async fn exec(self) -> anyhow::Result<u64> {
        self.execute().await
    }

    fn build_sql(&self) -> anyhow::Result<(String, Vec<turso::Value>)> {
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
                let _ = common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut ormer_params,
                    DbType::Sqlite,
                );
            }
        }

        let turso_params = values_to_params(&ormer_params)?;
        Ok((sql, turso_params))
    }
}

impl<T: Model + 'static + std::marker::Send> std::future::IntoFuture for UpdateExecutor<T> {
    type Output = anyhow::Result<u64>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 将 ormer Value 转换为 turso 参数
fn values_to_params(values: &[Value]) -> anyhow::Result<Vec<turso::Value>> {
    let mut params = Vec::new();

    for value in values {
        let param = match value {
            Value::Integer(v) => turso::Value::Integer(*v),
            Value::Text(v) => turso::Value::Text(v.clone()),
            Value::Real(v) => turso::Value::Real(*v),
            Value::Boolean(v) => turso::Value::Integer(if *v { 1 } else { 0 }),
            Value::Bytes(v) => turso::Value::Blob(v.clone()),
            Value::DateTime(v) => turso::Value::Text(v.to_rfc3339()),
            Value::Json(v) => turso::Value::Text(v.to_string()),
            Value::Uuid(v) => turso::Value::Text(v.to_string()),
            Value::BigInt(v) => turso::Value::Integer(*v as i64),
            Value::Null => turso::Value::Null,
        };
        params.push(param);
    }

    Ok(params)
}

/// 将 turso Value 转换为 ormer Value
fn convert_turso_value(value: &turso::Value) -> anyhow::Result<Value> {
    match value {
        turso::Value::Integer(v) => Ok(Value::Integer(*v)),
        turso::Value::Text(v) => {
            // 尝试解析为 DateTime
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                return Ok(Value::DateTime(dt.with_timezone(&chrono::Utc)));
            }
            Ok(Value::Text(v.clone()))
        }
        turso::Value::Real(v) => Ok(Value::Real(*v)),
        turso::Value::Null => Ok(Value::Null),
        turso::Value::Blob(v) => Ok(Value::Bytes(v.clone())),
        _ => Err(anyhow::anyhow!("Unsupported turso value type: {:?}", value)),
    }
}

/// Mapped Select Collect future
pub struct MappedCollectFuture<'a, T: Model + 'static, V: 'static, C: FromIterator<V> + 'static> {
    executor: MappedSelectExecutor<'a, T, V>,
    _marker: PhantomData<C>,
}

// SAFETY: Contains executor which references Database (Send + Sync)
unsafe impl<'a, T: Model + Send, V: Send, C: FromIterator<V> + Send> Send
    for MappedCollectFuture<'a, T, V, C>
{
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// ModelCollectWithFuture - 用于collect_with的Future,支持类型转换
pub struct ModelCollectWithFuture<'a, T: Model, V, C, M, F> {
    executor: MappedSelectExecutor<'a, T, V>,
    transform: F,
    _marker: PhantomData<(C, M)>,
}

// SAFETY: Contains executor which references Database (Send + Sync), and transform function is Send
unsafe impl<'a, T: Model + Send, V: Send, C: Send, M: Send, F: Send> Send
    for ModelCollectWithFuture<'a, T, V, C, M, F>
{
}

impl<'a, T, V, C, M, F> std::future::IntoFuture for ModelCollectWithFuture<'a, T, V, C, M, F>
where
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<M> + 'static,
    M: 'static + std::marker::Send,
    F: Fn(V) -> M + Clone + Send + 'static,
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<V> = self.executor.collect_inner().await?;
            Ok(results.into_iter().map(|v| (self.transform)(v)).collect())
        })
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 获取子查询的 SQL 和参数
    pub fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        self.select.to_sql_with_params(DbType::Sqlite)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(self) -> MappedCollectFuture<'a, T, V, C> {
        MappedCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果，同时应用转换函数
    /// 用于将查询结果转换为其他类型（如Model）
    /// 示例：collect_with(|v| Uids { id: v })
    pub fn collect_with<C, F, M>(self, f: F) -> ModelCollectWithFuture<'a, T, V, C, M, F>
    where
        C: FromIterator<M> + 'static,
        F: Fn(V) -> M + Clone + 'static,
        M: 'static,
    {
        ModelCollectWithFuture {
            executor: self.clone(),
            transform: f,
            _marker: PhantomData,
        }
    }

    async fn collect_inner<C: FromIterator<V>>(self) -> anyhow::Result<C>
    where
        V: crate::model::FromRowValues,
    {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            // 获取行中的所有值
            let column_count = self.select.column_names().len();
            let mut values = Vec::with_capacity(column_count);
            for i in 0..column_count {
                let value = row.get_value(i)?;
                let ormer_value = convert_turso_value(&value)?;
                values.push(ormer_value);
            }

            // 使用 FromRowValues 从多个值构建类型
            let typed_value = V::from_row_values(&values)?;
            results.push(typed_value);
        }

        Ok(results.into_iter().collect())
    }
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        GroupedCollectFuture {
            executor: self.clone(),
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
            conn: self.conn,
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
            conn: self.conn,
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
            conn: self.conn,
            _marker: PhantomData,
        }
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for GroupedCollectFuture<'a, T, V, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<V> = self.executor.collect_inner().await?;
            Ok(results.into_iter().collect())
        })
    }
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    async fn collect_inner<C: FromIterator<V>>(self) -> anyhow::Result<C>
    where
        V: crate::model::FromRowValues,
    {
        let (sql, params) = self.select.build_sql(DbType::Sqlite);

        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let mut rows = if turso_params.is_empty() {
            self.conn.query(&sql, ()).await?
        } else {
            self.conn.query(&sql, turso_params).await?
        };

        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            // 获取行中的所有值（从 column_count 获取列数）
            let column_count = self.select.column_count();
            let mut values = Vec::with_capacity(column_count);
            for i in 0..column_count {
                let value = row.get_value(i)?;
                let ormer_value = convert_turso_value(&value)?;
                values.push(ormer_value);
            }

            // 使用 FromRowValues 从多个值构建类型
            let typed_value = V::from_row_values(&values)?;
            results.push(typed_value);
        }

        Ok(results.into_iter().collect())
    }
}

/// SelectStream - 流式查询执行器 (SQLite/Turso)
///
/// 该执行器用于创建流式查询，允许逐行读取数据而不是一次性加载所有结果到内存中。
/// 适用于处理大量数据的场景，内存占用为 O(1)。
///
/// # 示例
///
/// ```rust,ignore
/// let mut stream = db.select::<User>().stream().into_iter().await?;
/// while let Some(result) = stream.next().await {
///     let user = result?;
///     println!("User: {:?}", user);
/// }
/// ```
///
/// # 连接管理
///
/// 该执行器持有 `Arc<turso::Connection>` 的克隆，确保在流式查询期间连接保持活跃。
/// 当 `SelectStreamIterator` 被 drop 时，连接会自动释放（通过 Arc 的引用计数）。
pub struct SelectStream<'a, T: Model> {
    select: Select<T>,
    conn: super::common::StreamConnection<'a>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器
    pub async fn into_iter(self) -> anyhow::Result<SelectStreamIterator<'a, T>> {
        let (sql, params) = self.select.to_sql_with_params(DbType::Sqlite);

        // 从 StreamConnection 获取连接
        let conn = match &self.conn {
            super::common::StreamConnection::Sqlite(c) => c.clone(),
            _ => unreachable!("Expected Sqlite connection"),
        };

        // 将 ormer::Value 转换为 turso::Value
        let turso_params: Vec<turso::Value> = params
            .into_iter()
            .map(|v| match v {
                crate::model::Value::Integer(i) => turso::Value::Integer(i),
                crate::model::Value::Text(t) => turso::Value::Text(t),
                crate::model::Value::Real(r) => turso::Value::Real(r),
                crate::model::Value::Boolean(b) => turso::Value::Integer(if b { 1 } else { 0 }),
                crate::model::Value::Bytes(b) => turso::Value::Blob(b.clone()),
                crate::model::Value::DateTime(dt) => turso::Value::Text(dt.to_rfc3339()),
                crate::model::Value::Json(j) => turso::Value::Text(j.to_string()),
                crate::model::Value::Uuid(u) => turso::Value::Text(u.to_string()),
                crate::model::Value::BigInt(b) => turso::Value::Integer(b as i64), // 可能丢失精度
                crate::model::Value::Null => turso::Value::Null,
            })
            .collect();

        let rows = if turso_params.is_empty() {
            conn.query(&sql, ()).await?
        } else {
            conn.query(&sql, turso_params).await?
        };

        Ok(SelectStreamIterator {
            conn: super::common::StreamConnection::Sqlite(conn),
            rows,
            polluted: false,
            _marker: std::marker::PhantomData,
        })
    }
}

/// SelectStreamIterator - 流式查询迭代器 (SQLite/Turso)
///
/// 该迭代器用于逐行读取流式查询的结果。
/// 每次调用 `next()` 方法会从数据库中获取下一行数据。
///
/// # 错误处理
///
/// 如果在解析行数据时发生错误，迭代器会被标记为"污染"状态，
/// 后续调用 `next()` 将直接返回 `None`，避免连续错误。
///
/// # 资源释放
///
/// 当迭代器被 drop 时（无论是正常完成、提前终止还是发生错误），
/// 底层的 turso::Rows 会自动关闭游标，连接会通过 Arc 的引用计数自动释放。
pub struct SelectStreamIterator<'a, T: Model> {
    #[allow(dead_code)]
    conn: super::common::StreamConnection<'a>,
    rows: turso::Rows,
    polluted: bool, // 标记是否发生解析错误，污染后不再尝试读取
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> Drop for SelectStreamIterator<'a, T> {
    fn drop(&mut self) {
        // turso::Rows 会在 Drop 时自动关闭游标并释放相关资源
        // StreamConnection 中的 Arc<turso::Connection> 会在最后一个引用释放时自动清理
        // 不需要显式操作，Rust 的 RAII 机制会确保资源正确释放
    }
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<anyhow::Result<T>> {
        // 如果已经污染，直接返回 None
        if self.polluted {
            return None;
        }

        match self.rows.next().await {
            Ok(Some(row)) => {
                // 解析行数据为 Model
                let mut data = HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    match row.get_value(i) {
                        Ok(value) => match convert_turso_value(&value) {
                            Ok(ormer_value) => {
                                data.insert(col_name.to_string(), ormer_value);
                            }
                            Err(e) => {
                                self.polluted = true;
                                return Some(Err(e));
                            }
                        },
                        Err(e) => {
                            self.polluted = true;
                            return Some(Err(
                                anyhow::anyhow!(e).context("Database operation failed")
                            ));
                        }
                    }
                }
                let ormer_row = Row::new(data);
                Some(T::from_row(&ormer_row))
            }
            Ok(None) => None,
            Err(e) => {
                self.polluted = true;
                Some(Err(anyhow::anyhow!(e).context("Database operation failed")))
            }
        }
    }
}
