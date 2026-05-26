use crate::abstract_layer::DbType;
use crate::model::{DbBackendTypeMapper, Model, Value};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect,
    RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use std::marker::PhantomData;
use std::sync::Arc;
use tiberius::{Client, Config, Query};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

/// MSSQL 类型映射器
pub struct MSSQLTypeMapper;

impl DbBackendTypeMapper for MSSQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        if enum_variants.is_some() {
            let mut sql_type = "VARCHAR(255)".to_string();
            if !is_nullable {
                sql_type.push_str(" NOT NULL");
            }
            return sql_type;
        }

        if is_primary {
            let int_type = match rust_type {
                "i8" | "i16" | "u8" => "SMALLINT",
                "i32" | "u16" => "INT",
                "i64" | "u32" | "u64" => "BIGINT",
                _ => "INT",
            };
            if is_auto_increment {
                return format!("{int_type} PRIMARY KEY IDENTITY(1,1)");
            } else {
                return format!("{int_type} PRIMARY KEY");
            }
        }

        let base_type = match rust_type {
            "i8" => "SMALLINT",
            "i16" => "SMALLINT",
            "i32" => "INT",
            "i64" => "BIGINT",
            "u8" => "SMALLINT",
            "u16" => "INT",
            "u32" => "BIGINT",
            "u64" => "BIGINT",
            "f32" => "REAL",
            "f64" => "FLOAT",
            "String" => "NVARCHAR(255)",
            "bool" => "BIT",
            "Vec<u8>" | "&[u8]" => "VARBINARY(MAX)",
            "DateTime" | "chrono::DateTime" | "NaiveDateTime" | "chrono::NaiveDateTime" => {
                "DATETIME2"
            }
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            "JsonValue" | "serde_json::Value" => "NVARCHAR(MAX)",
            _ => "NVARCHAR(255)",
        };

        let mut sql_type = base_type.to_string();
        if !is_nullable {
            sql_type.push_str(" NOT NULL");
        }
        sql_type
    }
}

pub type Pool = Arc<Mutex<Client<tokio_util::compat::Compat<TcpStream>>>>;

/// MSSQL 数据库连接封装
pub struct Database {
    pool: Pool,
}

impl Database {
    pub async fn connect(_db_type: super::DbType, connection_string: &str) -> anyhow::Result<Self> {
        let config = Config::from_ado_string(connection_string)?;
        let tcp = TcpStream::connect(config.get_addr()).await?;
        tcp.set_nodelay(true)?;
        let client = Client::connect(config, tcp.compat_write()).await?;
        Ok(Self {
            pool: Arc::new(Mutex::new(client)),
        })
    }

    pub fn get_pool(&self) -> Pool {
        self.pool.clone()
    }

    pub fn is_valid(&self) -> bool {
        true
    }

    pub async fn exec_sql(&self, sql: &str) -> anyhow::Result<u64> {
        let mut client = self.pool.lock().await;
        let query = Query::new(sql);
        let result = query.execute(&mut *client).await?;
        Ok(result.total())
    }

    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            pool: self.pool.clone(),
            table_name: None,
            _marker: PhantomData,
        }
    }

    pub fn drop_table<T: Model>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        InsertOrUpdateExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        InsertOrIgnoreExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub async fn insert_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }
        let columns = T::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_mssql_with_columns(
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        query.execute(&mut *client).await?;
        Ok(())
    }

    pub async fn insert_or_update_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<()> {
        if models.is_empty() {
            return Ok(());
        }
        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let pks = T::primary_key_columns();

        // 构建 MERGE SQL（MSSQL 的 INSERT OR UPDATE 语法）
        let mut sql = format!("MERGE INTO {} AS target USING (VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON ",));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
        let mut first = true;
        for col_name in T::COLUMNS.iter() {
            if pks.contains(col_name) {
                continue;
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = source.{}", col_name, col_name));
            first = false;
        }

        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in T::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        query.execute(&mut *client).await?;
        Ok(())
    }

    pub async fn insert_or_ignore_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let pks = T::primary_key_columns();

        let mut sql = format!("MERGE INTO {} AS target USING (VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON "));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        // 只插入不匹配的记录，不更新已存在的记录
        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in T::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }

    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_multi_table<T: Model + 'static, R1: Model, R2: Model>(
        &self,
    ) -> MultiTableSelectExecutor<'_, T, R1, R2> {
        MultiTableSelectExecutor {
            select: Select::<T>::new().from3::<T, R1, R2>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_four_table<T: Model + 'static, R1: Model, R2: Model, R3: Model>(
        &self,
    ) -> FourTableSelectExecutor<'_, T, R1, R2, R3> {
        FourTableSelectExecutor {
            select: Select::<T>::new().from4::<T, R1, R2, R3>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_mapped<T: Model, V>(
        &self,
        mapped: crate::query::builder::MappedSelect<T, V>,
    ) -> MappedSelectExecutor<'_, T, V> {
        MappedSelectExecutor {
            select: mapped,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_grouped<T: Model, V>(
        &self,
        grouped: GroupedSelect<T, V>,
    ) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: grouped,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub async fn transaction(&self) -> anyhow::Result<Transaction<'_>> {
        let _client = self.pool.lock().await;
        Ok(Transaction {
            pool: self.pool.clone(),
            _marker: PhantomData,
        })
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: Model>(&self) -> anyhow::Result<()> {
        let mut client = self.pool.lock().await;

        // 检查表是否存在
        let check_sql = format!(
            "SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = '{}'",
            T::TABLE_NAME
        );
        {
            let query = Query::new(&check_sql);
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;
            if rows.is_empty() {
                return Err(anyhow::anyhow!("Table {} does not exist", T::TABLE_NAME));
            }
            // 尝试读取 COUNT 结果
            if let Ok(Some(count)) = rows[0].try_get::<i32, _>(0) {
                if count == 0 {
                    return Err(anyhow::anyhow!("Table {} does not exist", T::TABLE_NAME));
                }
            }
        }

        // 查询表的列信息
        let col_sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            T::TABLE_NAME
        );
        let query = Query::new(&col_sql);
        let stream = query.query(&mut *client).await?;
        let rows = stream.into_first_result().await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool)> = Vec::new();
        for row in rows {
            let name: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            let col_type: String = row.get::<&str, _>(1).unwrap_or("").to_string();
            let nullable: String = row.get::<&str, _>(2).unwrap_or("").to_string();
            actual_columns.push((
                name.to_lowercase(),
                col_type.to_lowercase(),
                nullable == "YES",
            ));
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

            let (actual_name, actual_type, _actual_nullable) = &actual_columns[i];

            // 检查列名
            if actual_name != &expected_col.name.to_lowercase() {
                return Err(anyhow::anyhow!(
                    "Schema mismatch: table {}, reason: Column name mismatch at position {}: expected '{}', but actual is '{}'",
                    T::TABLE_NAME,
                    i,
                    expected_col.name,
                    actual_name
                ));
            }

            // 提取预期的 SQL 类型
            let expected_sql_type = MSSQLTypeMapper::sql_type(
                expected_col.rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                false,
                expected_col.enum_variants,
            );
            // 提取基础类型（第一个单词，去除约束和大小）
            let base_expected = expected_sql_type
                .split(|c: char| c == ' ' || c == '(')
                .next()
                .unwrap_or("")
                .to_lowercase();

            // 检查列类型
            if &base_expected != actual_type {
                // 处理特殊情况：MSSQL 的 INT/INTEGER
                if !(base_expected == "int" && actual_type == "integer")
                    && !(base_expected == "integer" && actual_type == "int")
                    && !(base_expected == "nvarchar" && actual_type == "varchar")
                    && !(base_expected == "nchar" && actual_type == "char")
                {
                    return Err(anyhow::anyhow!(
                        "Schema mismatch: table {}, reason: Column type mismatch at column '{}': expected '{}', but actual is '{}'",
                        T::TABLE_NAME,
                        expected_col.name,
                        base_expected,
                        actual_type
                    ));
                }
            }
        }

        Ok(())
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> anyhow::Result<Transaction<'_>> {
        let mut client = self.pool.lock().await;
        let query = Query::new("BEGIN TRANSACTION");
        query.execute(&mut *client).await?;
        Ok(Transaction {
            pool: self.pool.clone(),
            _marker: PhantomData,
        })
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn execute<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        let mut client = self.pool.lock().await;
        let query = Query::new(sql);
        let results = query.query(&mut *client).await?;
        let rows = results.into_first_result().await?;
        let mut result = Vec::new();
        for row in rows {
            let mut data = std::collections::HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = extract_value_from_row(&row, i)?;
                data.insert(col_name.to_string(), ormer_value);
            }
            let ormer_row = crate::model::Row::new(data);
            let model = T::from_row(&ormer_row)?;
            result.push(model);
        }
        Ok(result)
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn exec_non_query(&self, sql: &str) -> anyhow::Result<u64> {
        let mut client = self.pool.lock().await;
        let query = Query::new(sql);
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: Model> {
    pool: Pool,
    table_name: Option<String>,
    _marker: PhantomData<(T, &'a ())>,
}

impl<'a, T: Model> CreateTableExecutor<'a, T> {
    pub fn with_table_name(mut self, table_name: &str) -> Self {
        self.table_name = Some(table_name.to_string());
        self
    }

    pub async fn execute(self) -> anyhow::Result<()> {
        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            crate::abstract_layer::DbType::MSSQL,
            self.table_name.as_deref(),
        )?;
        let mut client = self.pool.lock().await;
        let query = Query::new(&create_sql);
        query.execute(&mut *client).await?;
        Ok(())
    }
}

/// 删除表执行器
pub struct DropTableExecutor<'a, T: Model> {
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

impl<'a, T: Model> DropTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let sql = format!("DROP TABLE IF EXISTS {}", T::TABLE_NAME);
        let mut client = self.pool.lock().await;
        let query = Query::new(&sql);
        query.execute(&mut *client).await?;
        Ok(())
    }
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> InsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<u64> {
        let refs = self.models.as_refs();
        self.insert_impl::<I::Model>(&refs).await
    }

    async fn insert_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let columns = T::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_mssql_with_columns(
            T::TABLE_NAME,
            &columns,
            models.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<T>(
                models,
            );
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> InsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<u64> {
        let refs = self.models.as_refs();
        self.insert_or_update_batch::<I::Model>(&refs).await
    }

    async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> anyhow::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let pks = T::primary_key_columns();

        let mut sql = format!("MERGE INTO {} AS target USING (VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON "));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
        let mut first = true;
        for col_name in T::COLUMNS.iter() {
            if pks.contains(col_name) {
                continue;
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = source.{}", col_name, col_name));
            first = false;
        }

        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in T::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> InsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<u64> {
        let refs = self.models.as_refs();
        self.insert_or_ignore_impl::<I::Model>(&refs).await
    }

    pub async fn insert_or_ignore_impl<T: Model>(&self, models: &[&T]) -> anyhow::Result<u64> {
        if models.is_empty() {
            return Ok(0);
        }
        let columns = T::COLUMNS.join(", ");
        let col_count = T::COLUMNS.len();
        let pks = T::primary_key_columns();

        // MSSQL: 使用 MERGE + WHEN NOT MATCHED BY SOURCE 来模拟 INSERT OR IGNORE
        let mut sql = format!("MERGE INTO {} AS target USING (VALUES ", T::TABLE_NAME);
        let mut all_values = Vec::new();
        for (idx, model) in models.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON "));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        // 只插入不匹配的记录，不更新已存在的记录
        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in T::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 关联查询执行器
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 多表查询执行器
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 四表查询执行器
pub struct FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 左连接查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 内连接查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 右连接查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 映射查询结果执行器
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: crate::query::builder::MappedSelect<T, V>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 分组查询执行器
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 删除执行器
pub struct DeleteExecutor<'a, T: Model> {
    filters: Vec<crate::query::filter::FilterExpr>,
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

/// 更新执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<(String, Value)>,
    filters: Vec<crate::query::filter::FilterExpr>,
    pool: Pool,
    _marker: PhantomData<(T, &'a ())>,
}

/// 事务
pub struct Transaction<'a> {
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Transaction<'a> {
    pub async fn commit(self) -> anyhow::Result<()> {
        let mut client = self.pool.lock().await;
        let query = Query::new("COMMIT");
        query.execute(&mut *client).await?;
        Ok(())
    }

    pub async fn rollback(self) -> anyhow::Result<()> {
        let mut client = self.pool.lock().await;
        let query = Query::new("ROLLBACK");
        query.execute(&mut *client).await?;
        Ok(())
    }

    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        TransactionInsertOrUpdateExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }

    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        TransactionInsertOrIgnoreExecutor {
            pool: self.pool.clone(),
            models,
            _marker: PhantomData,
        }
    }
}

/// 事务插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(());
        }
        let columns = I::Model::insert_columns();
        let (sql, _) = super::common::common_helpers::build_batch_insert_sql_mssql_with_columns(
            I::Model::TABLE_NAME,
            &columns,
            refs.len(),
        );
        let all_values =
            super::common::common_helpers::collect_batch_insert_values_with_auto_increment::<
                I::Model,
            >(&refs);
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        query.execute(&mut *client).await?;
        Ok(())
    }
}

/// 事务插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(());
        }
        let columns = I::Model::COLUMNS.join(", ");
        let col_count = I::Model::COLUMNS.len();
        let pks = I::Model::primary_key_columns();

        let mut sql = format!(
            "MERGE INTO {} AS target USING (VALUES ",
            I::Model::TABLE_NAME
        );
        let mut all_values = Vec::new();
        for (idx, model) in refs.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON "));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
        let mut first = true;
        for col_name in I::Model::COLUMNS.iter() {
            if pks.contains(col_name) {
                continue;
            }
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = source.{}", col_name, col_name));
            first = false;
        }

        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in I::Model::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        query.execute(&mut *client).await?;
        Ok(())
    }
}

/// 事务插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pool: Pool,
    models: I,
    _marker: PhantomData<&'a ()>,
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(());
        }
        let columns = I::Model::COLUMNS.join(", ");
        let col_count = I::Model::COLUMNS.len();
        let pks = I::Model::primary_key_columns();

        let mut sql = format!(
            "MERGE INTO {} AS target USING (VALUES ",
            I::Model::TABLE_NAME
        );
        let mut all_values = Vec::new();
        for (idx, model) in refs.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let placeholders: Vec<String> = (1..=col_count).map(|_| "@P".to_string()).collect();
            sql.push_str(&format!("({})", placeholders.join(", ")));
            let values = model.field_values();
            all_values.extend(values);
        }

        sql.push_str(&format!(") AS source ({columns}) ON "));
        for (i, pk) in pks.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&format!("target.{} = source.{}", pk, pk));
        }

        // 只插入不匹配的记录，不更新已存在的记录
        sql.push_str(&format!(
            " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
        ));
        for (i, col_name) in I::Model::COLUMNS.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("source.{}", col_name));
        }
        sql.push_str(");");

        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &all_values {
            bind_value(&mut query, param);
        }
        query.execute(&mut *client).await?;
        Ok(())
    }
}

/// 收集 Future
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<(fn() -> C, &'a ())>,
}

impl<'a, T: Model + 'static, C: FromIterator<T> + 'static> CollectFuture<'a, T, C> {
    pub async fn into_future(self) -> anyhow::Result<C> {
        // TODO: 实现实际的查询执行
        Ok(Vec::new().into_iter().collect())
    }
}

/// 聚合 Future
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    pool: Pool,
    _marker: PhantomData<&'a ()>,
}

/// 左连接收集 Future
pub struct LeftJoinCollectFuture<'a, T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 内连接收集 Future
pub struct InnerJoinCollectFuture<'a, T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 右连接收集 Future
pub struct RightJoinCollectFuture<'a, T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<&'a ()>,
}

/// 关联收集 Future
pub struct RelatedCollectFuture<'a, T: Model, R: Model> {
    executor: RelatedSelectExecutor<'a, T, R>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, T: Model + 'static, R: Model + 'static> RelatedCollectFuture<'a, T, R> {
    pub async fn into_future(self) -> anyhow::Result<Vec<T>> {
        // TODO: 实现实际的关联查询执行
        Ok(Vec::new())
    }
}

/// 映射收集 Future
pub struct MappedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    executor: MappedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(fn() -> C, &'a ())>,
}

/// 分组收集 Future
pub struct GroupedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(fn() -> C, &'a ())>,
}

/// 流式查询
pub struct SelectStream<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<&'a ()>,
}

// SelectExecutor 实现 - 基础方法（不需要 'static）
impl<'a, T: Model> SelectExecutor<'a, T> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn order_by<F, O>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<crate::query::filter::OrderBy>,
    {
        Self {
            select: self.select.order_by(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn order_by_desc<F, O>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> O,
        O: Into<crate::query::filter::OrderBy>,
    {
        Self {
            select: self.select.order_by_desc(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
    {
        AggregateFuture {
            aggregate_select: self.select.count(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.sum(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.avg(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.max(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(T::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        AggregateFuture {
            aggregate_select: self.select.min(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        MappedSelectExecutor {
            select: self.select.map_to(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(T::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        GroupedSelectExecutor {
            select: self.select.select_column(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

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

    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            executor: self,
            _marker: PhantomData,
        }
    }
}

// SelectExecutor 实现 - 需要 'static 的方法
impl<'a, T: Model + 'static> SelectExecutor<'a, T> {
    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> CollectFuture<'a, T, Vec<T>> {
        self.collect::<Vec<T>>()
    }
}

// GroupedSelectExecutor 实现
impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        Self {
            select: self.select.group_by(f),
            pool: self.pool,
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
            pool: self.pool,
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
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model + 'static, V: crate::model::FromRowValues + 'static>
    GroupedSelectExecutor<'a, T, V>
{
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C> {
        GroupedCollectFuture {
            executor: GroupedSelectExecutor {
                select: self.select.clone(),
                pool: self.pool.clone(),
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

// DeleteExecutor 实现
impl<'a, T: Model> DeleteExecutor<'a, T> {
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

    pub async fn execute(self) -> anyhow::Result<u64> {
        let (sql, params) = self.build_sql_with_params();
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &params {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
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
                let _ = crate::abstract_layer::common::common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    DbType::MSSQL,
                );
            }
        }

        (sql, params)
    }
}

// UpdateExecutor 实现
impl<'a, T: Model> UpdateExecutor<'a, T> {
    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        let where_obj = T::Where::default();
        let expr = f(where_obj);
        self.filters.push(expr.into());
        self
    }

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

    pub async fn execute(self) -> anyhow::Result<u64> {
        let (sql, params) = self.build_sql()?;
        let mut client = self.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &params {
            bind_value(&mut query, param);
        }
        let result = query.execute(&mut *client).await?;
        Ok(result.total() as u64)
    }

    fn build_sql(&self) -> anyhow::Result<(String, Vec<Value>)> {
        let mut sql = format!("UPDATE {} SET ", T::TABLE_NAME);
        let mut params = Vec::new();

        let mut first = true;
        for (col_name, value) in &self.sets {
            if !first {
                sql.push_str(", ");
            }
            sql.push_str(&format!("{} = @P", col_name));
            params.push(value.clone());
            first = false;
        }

        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut param_idx: usize = 1;
            for (i, filter) in self.filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let _ = crate::abstract_layer::common::common_helpers::format_filter_with_params(
                    filter,
                    &mut sql,
                    &mut param_idx,
                    &mut params,
                    DbType::MSSQL,
                );
            }
        }

        Ok((sql, params))
    }
}

// Join Executors 实现
impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J> {
        LeftJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J> {
        InnerJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }

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

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J> {
        RightJoinCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 生成子查询SQL和参数
    pub fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        self.select.to_sql_with_params(DbType::MSSQL)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        MappedCollectFuture {
            executor: self.clone_with_pool(),
            _marker: PhantomData,
        }
    }

    /// 克隆executor（保持相同的pool引用）
    pub fn clone_with_pool(&self) -> Self {
        Self {
            select: self.select.clone(),
            pool: self.pool.clone(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model + 'static, R: Model + 'static> RelatedSelectExecutor<'a, T, R> {
    pub fn filter(self, f: impl FnOnce(T::Where, R::Where) -> WhereExpr) -> Self {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<'a, T, R> {
        RelatedCollectFuture {
            executor: self,
            _marker: PhantomData,
        }
    }

    pub fn exec(self) -> RelatedCollectFuture<'a, T, R> {
        self.collect::<Vec<T>>()
    }
}

impl<'a, T: Model, R1: Model, R2: Model> MultiTableSelectExecutor<'a, T, R1, R2> {
    pub fn filter(self, f: impl FnOnce(T::Where, R1::Where, R2::Where) -> WhereExpr) -> Self {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

impl<'a, T: Model, R1: Model, R2: Model, R3: Model> FourTableSelectExecutor<'a, T, R1, R2, R3> {
    pub fn filter(
        self,
        f: impl FnOnce(T::Where, R1::Where, R2::Where, R3::Where) -> WhereExpr,
    ) -> Self {
        Self {
            select: self.select.filter(f),
            pool: self.pool,
            _marker: PhantomData,
        }
    }

    pub fn range<RR: Into<crate::query::builder::RangeBounds>>(self, range: RR) -> Self {
        Self {
            select: self.select.range(range),
            pool: self.pool,
            _marker: PhantomData,
        }
    }
}

// IntoFuture 实现
impl<'a, T: Model + 'static + std::marker::Send, C: FromIterator<T> + 'static>
    std::future::IntoFuture for CollectFuture<'a, T, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        let SelectExecutor {
            select,
            pool,
            _marker: _,
        } = self.executor;
        Box::pin(async move {
            let (sql, params) = select.to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let mut results = Vec::new();
            for row in rows {
                let mut data = std::collections::HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    data.insert(col_name.to_string(), ormer_value);
                }
                let ormer_row = crate::model::Row::new(data);
                let model = T::from_row(&ormer_row)?;
                results.push(model);
            }
            Ok(results.into_iter().collect())
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send,
    R: crate::model::FromValue + 'static + std::marker::Send,
> std::future::IntoFuture for AggregateFuture<'a, T, R>
{
    type Output = anyhow::Result<R>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .aggregate_select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;
            if rows.is_empty() {
                return Err(anyhow::anyhow!("Aggregate query returned no rows"));
            }
            let ormer_value = extract_value_from_row(&rows[0], 0)?;
            R::from_value(&ormer_value)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for LeftJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(T, Option<J>)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let mut t_data = std::collections::HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    t_data.insert(col_name.to_string(), ormer_value);
                }
                let t_model = T::from_row(&crate::model::Row::new(t_data))?;

                let mut j_data = std::collections::HashMap::new();
                let mut j_is_null = true;
                for (i, col_name) in J::COLUMNS.iter().enumerate() {
                    let idx = t_col_count + i;
                    let ormer_value = extract_value_from_row(&row, idx)?;
                    if !matches!(ormer_value, crate::model::Value::Null) {
                        j_is_null = false;
                    }
                    j_data.insert(col_name.to_string(), ormer_value);
                }

                let j_model = if j_is_null {
                    None
                } else {
                    Some(J::from_row(&crate::model::Row::new(j_data))?)
                };
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for InnerJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(T, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let mut t_data = std::collections::HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    t_data.insert(col_name.to_string(), ormer_value);
                }
                let t_model = T::from_row(&crate::model::Row::new(t_data))?;

                let mut j_data = std::collections::HashMap::new();
                for (i, col_name) in J::COLUMNS.iter().enumerate() {
                    let idx = t_col_count + i;
                    let ormer_value = extract_value_from_row(&row, idx)?;
                    j_data.insert(col_name.to_string(), ormer_value);
                }
                let j_model = J::from_row(&crate::model::Row::new(j_data))?;
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<'a, T: Model + 'static + std::marker::Send, J: Model + 'static + std::marker::Send>
    std::future::IntoFuture for RightJoinCollectFuture<'a, T, J>
{
    type Output = anyhow::Result<Vec<(Option<T>, J)>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let t_col_count = T::COLUMNS.len();
            let mut results = Vec::new();
            for row in rows {
                let mut t_data = std::collections::HashMap::new();
                let mut t_is_null = true;
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    if !matches!(ormer_value, crate::model::Value::Null) {
                        t_is_null = false;
                    }
                    t_data.insert(col_name.to_string(), ormer_value);
                }
                let t_model = if t_is_null {
                    None
                } else {
                    Some(T::from_row(&crate::model::Row::new(t_data))?)
                };

                let mut j_data = std::collections::HashMap::new();
                for (i, col_name) in J::COLUMNS.iter().enumerate() {
                    let idx = t_col_count + i;
                    let ormer_value = extract_value_from_row(&row, idx)?;
                    j_data.insert(col_name.to_string(), ormer_value);
                }
                let j_model = J::from_row(&crate::model::Row::new(j_data))?;
                results.push((t_model, j_model));
            }
            Ok(results)
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    R: Model + 'static + std::marker::Send + std::marker::Sync,
> std::future::IntoFuture for RelatedCollectFuture<'a, T, R>
where
    Self: 'static,
{
    type Output = anyhow::Result<Vec<T>>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let mut results = Vec::new();
            for row in rows {
                let mut data = std::collections::HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    data.insert(col_name.to_string(), ormer_value);
                }
                let ormer_row = crate::model::Row::new(data);
                let model = T::from_row(&ormer_row)?;
                results.push(model);
            }
            Ok(results)
        })
    }
}

impl<
    'a,
    T: Model + 'static + std::marker::Send + std::marker::Sync,
    V: crate::model::FromRowValues + 'static + std::marker::Send + std::marker::Sync,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = anyhow::Result<C>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let mut results = Vec::new();
            for row in rows {
                let mut values = Vec::new();
                for i in 0..row.columns().len() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    values.push(ormer_value);
                }
                let v = V::from_row_values(&values)?;
                results.push(v);
            }
            Ok(results.into_iter().collect())
        })
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
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
            let mut client = self.executor.pool.lock().await;
            let mut query = Query::new(&sql);
            for param in &params {
                bind_value(&mut query, param);
            }
            let stream = query.query(&mut *client).await?;
            let rows = stream.into_first_result().await?;

            let mut results = Vec::new();
            for row in rows {
                let mut values = Vec::new();
                for i in 0..row.columns().len() {
                    let ormer_value = extract_value_from_row(&row, i)?;
                    values.push(ormer_value);
                }
                let v = V::from_row_values(&values)?;
                results.push(v);
            }
            Ok(results.into_iter().collect())
        })
    }
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    pub async fn into_iter(self) -> anyhow::Result<SelectStreamIterator<'a, T>> {
        let (sql, params) = self
            .executor
            .select
            .to_sql_with_params(crate::abstract_layer::DbType::MSSQL);
        let mut client = self.executor.pool.lock().await;
        let mut query = Query::new(&sql);
        for param in &params {
            bind_value(&mut query, param);
        }
        let stream = query.query(&mut *client).await?;
        let rows = stream.into_first_result().await?;

        let mut results = Vec::new();
        for row in rows {
            let mut data = std::collections::HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = extract_value_from_row(&row, i)?;
                data.insert(col_name.to_string(), ormer_value);
            }
            let ormer_row = crate::model::Row::new(data);
            let model = T::from_row(&ormer_row)?;
            results.push(model);
        }
        Ok(SelectStreamIterator {
            iter: results.into_iter(),
            _marker: PhantomData,
        })
    }
}

pub struct SelectStreamIterator<'a, T: Model> {
    iter: std::vec::IntoIter<T>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    pub async fn next(&mut self) -> Option<anyhow::Result<T>> {
        self.iter.next().map(Ok)
    }
}

// 辅助函数：从 tiberius Row 中提取 Value
fn extract_value_from_row(row: &tiberius::Row, idx: usize) -> anyhow::Result<Value> {
    // 尝试 i32 (INT)
    if let Ok(Some(v)) = row.try_get::<i32, _>(idx) {
        return Ok(Value::Integer(v as i64));
    }
    // 尝试 i64 (BIGINT)
    if let Ok(Some(v)) = row.try_get::<i64, _>(idx) {
        return Ok(Value::Integer(v));
    }
    // 尝试 i16 (SMALLINT)
    if let Ok(Some(v)) = row.try_get::<i16, _>(idx) {
        return Ok(Value::Integer(v as i64));
    }
    // 尝试 &str (NVARCHAR, VARCHAR, CHAR)
    if let Ok(Some(v)) = row.try_get::<&str, _>(idx) {
        return Ok(Value::Text(v.to_string()));
    }
    // 尝试 f64 (FLOAT)
    if let Ok(Some(v)) = row.try_get::<f64, _>(idx) {
        return Ok(Value::Real(v));
    }
    // 尝试 f32 (REAL)
    if let Ok(Some(v)) = row.try_get::<f32, _>(idx) {
        return Ok(Value::Real(v as f64));
    }
    // 尝试 bool (BIT)
    if let Ok(Some(v)) = row.try_get::<bool, _>(idx) {
        return Ok(if v {
            Value::Integer(1)
        } else {
            Value::Integer(0)
        });
    }
    // 尝试 NaiveDateTime (DATETIME2, DATETIME)
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
        return Ok(Value::DateTime(
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(v, chrono::Utc),
        ));
    }
    // 尝试 &[u8] (VARBINARY, BINARY)
    if let Ok(Some(v)) = row.try_get::<&[u8], _>(idx) {
        return Ok(Value::Bytes(v.to_vec()));
    }
    // 值为 NULL 或无法识别的类型
    Ok(Value::Null)
}

// 辅助函数：从 tiberius Row 中提取 i64 值（用于聚合查询）
#[allow(dead_code)]
fn extract_i64_from_row(row: &tiberius::Row, idx: usize) -> anyhow::Result<Option<i64>> {
    if let Ok(Some(v)) = row.try_get::<i32, _>(idx) {
        return Ok(Some(v as i64));
    }
    if let Ok(Some(v)) = row.try_get::<i64, _>(idx) {
        return Ok(Some(v));
    }
    if let Ok(Some(v)) = row.try_get::<i16, _>(idx) {
        return Ok(Some(v as i64));
    }
    Ok(None)
}

// 辅助函数：从 tiberius Row 中提取 f64 值（用于 AVG 聚合）
#[allow(dead_code)]
fn extract_f64_from_row(row: &tiberius::Row, idx: usize) -> anyhow::Result<Option<f64>> {
    if let Ok(Some(v)) = row.try_get::<f64, _>(idx) {
        return Ok(Some(v));
    }
    if let Ok(Some(v)) = row.try_get::<f32, _>(idx) {
        return Ok(Some(v as f64));
    }
    if let Ok(Some(v)) = row.try_get::<i32, _>(idx) {
        return Ok(Some(v as f64));
    }
    if let Ok(Some(v)) = row.try_get::<i64, _>(idx) {
        return Ok(Some(v as f64));
    }
    Ok(None)
}

// 辅助函数：将 Value 绑定到 Query
fn bind_value<'a>(query: &mut Query<'a>, value: &'a Value) {
    match value {
        Value::Null => query.bind(Option::<&str>::None),
        Value::Boolean(v) => query.bind(*v),
        Value::Integer(v) => query.bind(*v),
        Value::BigInt(v) => query.bind(*v as i64),
        Value::Real(v) => query.bind(*v),
        Value::Text(v) => query.bind(v.as_str()),
        Value::Bytes(v) => query.bind(v.as_slice()),
        Value::DateTime(v) => query.bind(v.naive_utc()),
        Value::Json(v) => query.bind(v.to_string()),
        Value::Uuid(v) => query.bind(v.as_bytes().as_slice()),
    }
}
