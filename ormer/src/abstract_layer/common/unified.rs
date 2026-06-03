/// 统一的数据库抽象层
/// 使用枚举包装不同数据库后端,对外提供统一接口
/// 通过条件编译控制枚举变体
use crate::model::Model;
use crate::query::builder::WhereExpr;

// 根据启用的 feature 导入后端实现
#[cfg(feature = "sqlite")]
use super::super::sqlite_backend;

#[cfg(feature = "postgresql")]
use super::super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::super::mysql_backend;

#[cfg(feature = "mssql")]
use super::super::mssql_backend;

/// 统一的 Database 枚举
pub enum Database {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Database),
}

/// 统一的 CreateTableExecutor 枚举
pub enum CreateTableExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::CreateTableExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::CreateTableExecutor<'a, T>),
}

impl<'a, T: Model> CreateTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            CreateTableExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            CreateTableExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            CreateTableExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            CreateTableExecutor::MSSQL(exec) => exec.execute().await,
        }
    }
}

/// 统一的 DropTableExecutor 枚举
pub enum DropTableExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::DropTableExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::DropTableExecutor<'a, T>),
}

impl<'a, T: Model> DropTableExecutor<'a, T> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            DropTableExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            DropTableExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            DropTableExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            DropTableExecutor::MSSQL(exec) => exec.execute().await,
        }
    }
}

/// 统一的 InsertExecutor 枚举
pub enum InsertExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertExecutor<'a, I>),
}

impl<'a, I: crate::model::Insertable> InsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
        }
    }
}

/// 统一的 InsertOrUpdateExecutor 枚举
pub enum InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrUpdateExecutor<'a, I>),
}

impl<'a, I: crate::model::Insertable> InsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrUpdateExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrUpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrUpdateExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrUpdateExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
        }
    }
}

/// 统一的 InsertOrIgnoreExecutor 枚举
pub enum InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InsertOrIgnoreExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InsertOrIgnoreExecutor<'a, I>),
}

impl<'a, I: crate::model::Insertable> InsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            InsertOrIgnoreExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            InsertOrIgnoreExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            InsertOrIgnoreExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            InsertOrIgnoreExecutor::MSSQL(exec) => exec.execute().await.map(|_| ()),
        }
    }
}

impl Database {
    /// 连接到数据库,根据 DbType 选择后端
    pub async fn connect(
        db_type: super::super::DbType,
        connection_string: &str,
    ) -> anyhow::Result<Self> {
        match db_type {
            #[cfg(feature = "sqlite")]
            super::super::DbType::Sqlite => {
                let db = sqlite_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::Sqlite(db))
            }
            #[cfg(feature = "postgresql")]
            super::super::DbType::PostgreSQL => {
                let db = postgresql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::PostgreSQL(db))
            }
            #[cfg(feature = "mysql")]
            super::super::DbType::MySQL => {
                let db = mysql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::MySQL(db))
            }
            #[cfg(feature = "mssql")]
            super::super::DbType::MSSQL => {
                let db = mssql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::MSSQL(db))
            }
        }
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => CreateTableExecutor::Sqlite(db.create_table::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => CreateTableExecutor::PostgreSQL(db.create_table::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => CreateTableExecutor::MySQL(db.create_table::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => CreateTableExecutor::MSSQL(db.create_table::<T>()),
        }
    }

    /// 验证表结构
    pub async fn validate_table<T: Model>(&self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.validate_table::<T>().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.validate_table::<T>().await,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => InsertExecutor::Sqlite(db.insert::<I>(models)),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => InsertExecutor::PostgreSQL(db.insert::<I>(models)),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertExecutor::MySQL(db.insert::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertExecutor::MSSQL(db.insert::<I>(models)),
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertOrUpdateExecutor::Sqlite(db.insert_or_update::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                InsertOrUpdateExecutor::PostgreSQL(db.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertOrUpdateExecutor::MySQL(db.insert_or_update::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertOrUpdateExecutor::MSSQL(db.insert_or_update::<I>(models)),
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                InsertOrIgnoreExecutor::Sqlite(db.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                InsertOrIgnoreExecutor::PostgreSQL(db.insert_or_ignore::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => InsertOrIgnoreExecutor::MySQL(db.insert_or_ignore::<I>(models)),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => InsertOrIgnoreExecutor::MSSQL(db.insert_or_ignore::<I>(models)),
        }
    }

    /// 根据主键查找单条记录
    /// 支持单主键和复合主键
    /// ```
    /// // 单主键
    /// let user: Option<User> = db.find_by_id::<User>(1).await?;
    /// // 复合主键
    /// let record: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 2)).await?;
    /// ```
    pub async fn find_by_id<T: Model + 'static + std::marker::Send + std::marker::Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> anyhow::Result<Option<T>> {
        let pk_columns = T::primary_key_columns();
        let pk_values = key.into_values();

        if pk_columns.is_empty() {
            return Err(anyhow::anyhow!(
                "Model {} does not have a primary key",
                T::TABLE_NAME
            ));
        }
        if pk_columns.len() != pk_values.len() {
            return Err(anyhow::anyhow!(
                "Primary key column count ({}) does not match value count ({})",
                pk_columns.len(),
                pk_values.len()
            ));
        }

        // 构建 WHERE 条件（将 model::Value 转为 filter::Value）
        let mut filters: Vec<crate::query::filter::FilterExpr> = Vec::new();
        for (col, val) in pk_columns.iter().zip(pk_values.into_iter()) {
            let filter_val = match val {
                crate::model::Value::Integer(v) => crate::query::filter::Value::Integer(v),
                crate::model::Value::BigInt(v) => crate::query::filter::Value::BigInt(v),
                crate::model::Value::Text(v) => crate::query::filter::Value::Text(v),
                crate::model::Value::Real(v) => crate::query::filter::Value::Real(v),
                crate::model::Value::Boolean(v) => crate::query::filter::Value::Boolean(v),
                crate::model::Value::Bytes(v) => crate::query::filter::Value::Bytes(v),
                crate::model::Value::DateTime(v) => crate::query::filter::Value::DateTime(v),
                crate::model::Value::Json(v) => crate::query::filter::Value::Json(v),
                crate::model::Value::Uuid(v) => crate::query::filter::Value::Uuid(v),
                crate::model::Value::Null => crate::query::filter::Value::Null,
            };
            filters.push(crate::query::filter::FilterExpr::Comparison {
                column: col.to_string(),
                operator: "=".to_string(),
                value: filter_val,
            });
        }

        // 组合所有条件为 AND
        let filter = if filters.len() == 1 {
            filters.into_iter().next().unwrap()
        } else {
            filters
                .into_iter()
                .reduce(|a, b| crate::query::filter::FilterExpr::And(Box::new(a), Box::new(b)))
                .unwrap()
        };

        let where_expr = crate::query::builder::WhereExpr::from_filter(filter);

        // 执行查询并取第一条
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .execute()
            .await?;

        Ok(results.into_iter().next())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => SelectExecutor::Sqlite(db.select::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => SelectExecutor::PostgreSQL(db.select::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => SelectExecutor::MySQL(db.select::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => SelectExecutor::MSSQL(db.select::<T>()),
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => GroupedSelectExecutor::Sqlite(db.select_column::<T, V>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                GroupedSelectExecutor::PostgreSQL(db.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => GroupedSelectExecutor::MySQL(db.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => GroupedSelectExecutor::MSSQL(db.select_column::<T, V>()),
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => DeleteExecutor::PostgreSQL(db.delete::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => DeleteExecutor::MySQL(db.delete::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => DeleteExecutor::MSSQL(db.delete::<T>()),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                UpdateExecutor::Sqlite(db.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => UpdateExecutor::PostgreSQL(db.update::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => UpdateExecutor::MySQL(db.update::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => UpdateExecutor::MSSQL(db.update::<T>()),
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn from<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                RelatedSelectExecutor::Sqlite(db.related::<T, R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => RelatedSelectExecutor::PostgreSQL(db.related::<T, R>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => RelatedSelectExecutor::MySQL(db.related::<T, R>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => RelatedSelectExecutor::MSSQL(db.related::<T, R>()),
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> anyhow::Result<Transaction<'_>> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::Sqlite(txn))
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::PostgreSQL(txn))
            }
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::MySQL(txn))
            }
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::MSSQL(txn))
            }
        }
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: Model>(&self) -> DropTableExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => DropTableExecutor::Sqlite(db.drop_table::<T>()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => DropTableExecutor::PostgreSQL(db.drop_table::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => DropTableExecutor::MySQL(db.drop_table::<T>()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => DropTableExecutor::MSSQL(db.drop_table::<T>()),
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn execute<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.execute::<T>(sql).await,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表（向后兼容）
    #[deprecated(since = "0.1.0", note = "请使用 execute 方法")]
    pub async fn exec_table<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        self.execute::<T>(sql).await
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn exec_non_query(&self, sql: &str) -> anyhow::Result<u64> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.exec_non_query(sql).await,
        }
    }

    /// 创建连接池
    #[cfg(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    ))]
    pub fn create_pool(
        db_type: super::super::DbType,
        connection_string: &str,
    ) -> super::connection_pool::PoolBuilder {
        super::connection_pool::PoolBuilder::new(db_type, connection_string)
    }
}

/// 统一的 SelectExecutor 枚举
pub enum SelectExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectExecutor<'a, T>),
}

crate::impl_unified_select_executor_methods!(SelectExecutor);

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: Model + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                RelatedSelectExecutor::Sqlite(exec.from::<T2, R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RelatedSelectExecutor::PostgreSQL(exec.from::<T2, R>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => RelatedSelectExecutor::MySQL(exec.from::<T2, R>()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => RelatedSelectExecutor::MSSQL(exec.from::<T2, R>()),
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: Model + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => MultiTableSelectExecutor::Sqlite(
                exec.from3::<T2, R1, R2>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                MultiTableSelectExecutor::PostgreSQL(exec.from3::<T2, R1, R2>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                MultiTableSelectExecutor::MySQL(exec.from3::<T2, R1, R2>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                MultiTableSelectExecutor::MSSQL(exec.from3::<T2, R1, R2>())
            }
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
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => FourTableSelectExecutor::Sqlite(
                exec.from4::<T2, R1, R2, R3>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                FourTableSelectExecutor::PostgreSQL(exec.from4::<T2, R1, R2, R3>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                FourTableSelectExecutor::MySQL(exec.from4::<T2, R1, R2, R3>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                FourTableSelectExecutor::MSSQL(exec.from4::<T2, R1, R2, R3>())
            }
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                LeftJoinedSelectExecutor::Sqlite(exec.left_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.left_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => LeftJoinedSelectExecutor::MySQL(exec.left_join::<J>(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => LeftJoinedSelectExecutor::MSSQL(exec.left_join::<J>(f)),
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                InnerJoinedSelectExecutor::Sqlite(exec.inner_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                InnerJoinedSelectExecutor::PostgreSQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                InnerJoinedSelectExecutor::MySQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                InnerJoinedSelectExecutor::MSSQL(exec.inner_join::<J>(f))
            }
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                RightJoinedSelectExecutor::Sqlite(exec.right_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RightJoinedSelectExecutor::PostgreSQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                RightJoinedSelectExecutor::MySQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                RightJoinedSelectExecutor::MSSQL(exec.right_join::<J>(f))
            }
        }
    }

    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C>
    where
        T: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => CollectFuture::Sqlite(exec.clone().collect::<C>()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                CollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                CollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => {
                CollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
        }
    }

    /// 执行查询并返回 Vec<T> (collect 的别名)
    pub fn execute(self) -> CollectFuture<'a, T, Vec<T>>
    where
        T: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.count(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.count(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.count(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.count(f)),
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.sum(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.sum(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.sum(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.sum(f)),
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.avg(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.avg(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.avg(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.avg(f)),
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.max(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.max(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.max(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.max(f)),
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => {
                AggregateFuture::Sqlite(exec.min(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.min(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.min(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => AggregateFuture::MSSQL(exec.min(f)),
        }
    }
}

/// 统一的 DeleteExecutor 枚举
pub enum DeleteExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::DeleteExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::DeleteExecutor<'a, T>),
}

crate::impl_unified_delete_executor!(DeleteExecutor);

/// 统一的 UpdateExecutor 枚举
pub enum UpdateExecutor<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::UpdateExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::UpdateExecutor<'a, T>),
}

crate::impl_unified_update_executor!(UpdateExecutor);

/// 统一的 CollectFuture 枚举
pub enum CollectFuture<'a, T: Model, C: FromIterator<T>> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::CollectFuture<'a, T, C>),
}

/// 统一的 AggregateFuture 枚举
pub enum AggregateFuture<'a, T: Model, R> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::AggregateFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::AggregateFuture<'a, T, R>),
}

crate::impl_unified_aggregate_future!(AggregateFuture);

/// 统一的 RelatedSelectExecutor 枚举
pub enum RelatedSelectExecutor<'a, T: Model, R: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RelatedSelectExecutor<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RelatedSelectExecutor<'a, T, R>),
}

/// 统一的 MultiTableSelectExecutor 枚举
pub enum MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
}

/// 统一的 FourTableSelectExecutor 枚举
pub enum FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
}

/// 统一的 InnerJoinedSelectExecutor 枚举
pub enum InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::InnerJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InnerJoinedSelectExecutor<'a, T, J>),
}

/// 统一的 RightJoinedSelectExecutor 枚举
pub enum RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RightJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RightJoinedSelectExecutor<'a, T, J>),
}

/// 统一的 LeftJoinedSelectExecutor 枚举
pub enum LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::LeftJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::LeftJoinedSelectExecutor<'a, T, J>),
}

/// 统一的 LeftJoinCollectFuture 枚举
pub enum LeftJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::LeftJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::LeftJoinCollectFuture<'a, T, J>),
}

/// 统一的 InnerJoinCollectFuture 枚举
pub enum InnerJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::InnerJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::InnerJoinCollectFuture<'a, T, J>),
}

/// 统一的 RightJoinCollectFuture 枚举
pub enum RightJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RightJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RightJoinCollectFuture<'a, T, J>),
}

crate::impl_unified_collect_future!(CollectFuture);

crate::impl_unified_related_select_executor!(RelatedSelectExecutor);

/// 统一的 RelatedCollectFuture 枚举
pub enum RelatedCollectFuture<'a, T: Model, R: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(
        sqlite_backend::RelatedCollectFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::RelatedCollectFuture<'a, T, R>),
}

crate::impl_unified_related_collect_future!(RelatedCollectFuture);

/// 统一的 Transaction 枚举
pub enum Transaction<'a> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Transaction),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Transaction<'a>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Transaction<'a>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Transaction<'a>),
    // 使用 PhantomData 确保生命周期参数始终被使用
    _Phantom(std::marker::PhantomData<&'a ()>),
}

/// 事务中的插入执行器
pub enum TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TransactionInsertExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TransactionInsertExecutor<'a, I>),
}

impl<'a, I: crate::model::Insertable> TransactionInsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            TransactionInsertExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TransactionInsertExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TransactionInsertExecutor::MSSQL(exec) => exec.execute().await,
        }
    }
}

/// 事务中的插入或更新执行器
pub enum TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::TransactionInsertOrUpdateExecutor<'a, I>),
}

impl<'a, I: crate::model::Insertable> TransactionInsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            TransactionInsertOrUpdateExecutor::Sqlite(exec) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            TransactionInsertOrUpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            TransactionInsertOrUpdateExecutor::MySQL(exec) => exec.execute().await,
            #[cfg(feature = "mssql")]
            TransactionInsertOrUpdateExecutor::MSSQL(exec) => exec.execute().await,
        }
    }
}

impl<'a> Transaction<'a> {
    /// 提交事务
    pub async fn commit(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => txn.commit().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.commit().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.commit().await,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => txn.commit().await,
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 回滚事务
    pub async fn rollback(self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => txn.rollback().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.rollback().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.rollback().await,
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => txn.rollback().await,
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 根据主键查找单条记录（事务中）
    pub async fn find_by_id<T: Model + 'static + std::marker::Send + std::marker::Sync>(
        &self,
        key: impl crate::model::PrimaryKey,
    ) -> anyhow::Result<Option<T>> {
        let pk_columns = T::primary_key_columns();
        let pk_values = key.into_values();

        if pk_columns.is_empty() {
            return Err(anyhow::anyhow!(
                "Model {} does not have a primary key",
                T::TABLE_NAME
            ));
        }
        if pk_columns.len() != pk_values.len() {
            return Err(anyhow::anyhow!(
                "Primary key column count ({}) does not match value count ({})",
                pk_columns.len(),
                pk_values.len()
            ));
        }

        // 构建 WHERE 条件（将 model::Value 转为 filter::Value）
        let mut filters: Vec<crate::query::filter::FilterExpr> = Vec::new();
        for (col, val) in pk_columns.iter().zip(pk_values.into_iter()) {
            let filter_val = match val {
                crate::model::Value::Integer(v) => crate::query::filter::Value::Integer(v),
                crate::model::Value::BigInt(v) => crate::query::filter::Value::BigInt(v),
                crate::model::Value::Text(v) => crate::query::filter::Value::Text(v),
                crate::model::Value::Real(v) => crate::query::filter::Value::Real(v),
                crate::model::Value::Boolean(v) => crate::query::filter::Value::Boolean(v),
                crate::model::Value::Bytes(v) => crate::query::filter::Value::Bytes(v),
                crate::model::Value::DateTime(v) => crate::query::filter::Value::DateTime(v),
                crate::model::Value::Json(v) => crate::query::filter::Value::Json(v),
                crate::model::Value::Uuid(v) => crate::query::filter::Value::Uuid(v),
                crate::model::Value::Null => crate::query::filter::Value::Null,
            };
            filters.push(crate::query::filter::FilterExpr::Comparison {
                column: col.to_string(),
                operator: "=".to_string(),
                value: filter_val,
            });
        }

        // 组合所有条件为 AND
        let filter = if filters.len() == 1 {
            filters.into_iter().next().unwrap()
        } else {
            filters
                .into_iter()
                .reduce(|a, b| crate::query::filter::FilterExpr::And(Box::new(a), Box::new(b)))
                .unwrap()
        };

        let where_expr = crate::query::builder::WhereExpr::from_filter(filter);

        // 执行查询并取第一条
        let results = self
            .select::<T>()
            .filter(|_| where_expr)
            .range(..1)
            .execute()
            .await?;

        Ok(results.into_iter().next())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => SelectExecutor::Sqlite(txn.select::<T>()),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => SelectExecutor::PostgreSQL(txn.select::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => SelectExecutor::MySQL(txn.select::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => SelectExecutor::MSSQL(txn.select::<T>()),
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => GroupedSelectExecutor::Sqlite(txn.select_column::<T, V>()),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                GroupedSelectExecutor::PostgreSQL(txn.select_column::<T, V>())
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => GroupedSelectExecutor::MySQL(txn.select_column::<T, V>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => GroupedSelectExecutor::MSSQL(txn.select_column::<T, V>()),
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                DeleteExecutor::Sqlite(txn.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => DeleteExecutor::PostgreSQL(txn.delete::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => DeleteExecutor::MySQL(txn.delete::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => DeleteExecutor::MSSQL(txn.delete::<T>()),
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                UpdateExecutor::Sqlite(txn.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => UpdateExecutor::PostgreSQL(txn.update::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => UpdateExecutor::MySQL(txn.update::<T>()),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => UpdateExecutor::MSSQL(txn.update::<T>()),
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => TransactionInsertExecutor::Sqlite(txn.insert::<I>(models)),
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                TransactionInsertExecutor::PostgreSQL(txn.insert::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => TransactionInsertExecutor::MySQL(txn.insert::<I>(models)),
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => TransactionInsertExecutor::MSSQL(txn.insert::<I>(models)),
            Transaction::_Phantom(_) => unreachable!(),
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        match self {
            #[cfg(feature = "sqlite")]
            Transaction::Sqlite(txn) => {
                TransactionInsertOrUpdateExecutor::Sqlite(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => {
                TransactionInsertOrUpdateExecutor::PostgreSQL(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => {
                TransactionInsertOrUpdateExecutor::MySQL(txn.insert_or_update::<I>(models))
            }
            #[cfg(feature = "mssql")]
            Transaction::MSSQL(txn) => {
                TransactionInsertOrUpdateExecutor::MSSQL(txn.insert_or_update::<I>(models))
            }
            Transaction::_Phantom(_) => unreachable!(),
        }
    }
}

crate::impl_unified_join_executor!(LeftJoinedSelectExecutor);

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            LeftJoinedSelectExecutor::Sqlite(exec, phantom) => {
                LeftJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            LeftJoinedSelectExecutor::MSSQL(exec) => {
                LeftJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
        }
    }

    /// 执行查询并返回 Vec<(T, Option<J>)>
    pub fn execute(self) -> LeftJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, Option<J>)>>()
    }
}

crate::impl_unified_join_executor!(InnerJoinedSelectExecutor);

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            InnerJoinedSelectExecutor::Sqlite(exec, phantom) => {
                InnerJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            InnerJoinedSelectExecutor::PostgreSQL(exec) => {
                InnerJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            InnerJoinedSelectExecutor::MySQL(exec) => {
                InnerJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            InnerJoinedSelectExecutor::MSSQL(exec) => {
                InnerJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
        }
    }

    /// 执行查询并返回 Vec<(T, J)>
    pub fn execute(self) -> InnerJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(T, J)>>()
    }
}

crate::impl_unified_join_executor!(RightJoinedSelectExecutor);

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            RightJoinedSelectExecutor::Sqlite(exec, phantom) => {
                RightJoinCollectFuture::Sqlite(exec.clone().collect::<C>(), *phantom)
            }
            #[cfg(feature = "postgresql")]
            RightJoinedSelectExecutor::PostgreSQL(exec) => {
                RightJoinCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            RightJoinedSelectExecutor::MySQL(exec) => {
                RightJoinCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            RightJoinedSelectExecutor::MSSQL(exec) => {
                RightJoinCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
        }
    }

    /// 执行查询并返回 Vec<(Option<T>, J)>
    pub fn execute(self) -> RightJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        self.collect::<Vec<(Option<T>, J)>>()
    }
}

crate::impl_unified_join_collect_future!(
    LeftJoinCollectFuture,
    anyhow::Result<Vec<(T, Option<J>)>>
);

crate::impl_unified_join_collect_future!(InnerJoinCollectFuture, anyhow::Result<Vec<(T, J)>>);

crate::impl_unified_join_collect_future!(
    RightJoinCollectFuture,
    anyhow::Result<Vec<(Option<T>, J)>>
);

/// 统一的 MappedSelectExecutor 枚举
pub enum MappedSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MappedSelectExecutor<'a, T, V>),
    #[cfg(not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    )))]
    NotImplemented(std::marker::PhantomData<&'a (T, V)>),
}

/// 统一的 GroupedSelectExecutor 枚举
pub enum GroupedSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::GroupedSelectExecutor<'a, T, V>),
    #[cfg(not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    )))]
    NotImplemented(std::marker::PhantomData<&'a (T, V)>),
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 添加 GROUP BY 字段
    #[allow(unused_variables)]
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.group_by(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.group_by(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.group_by(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.group_by(f)),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            GroupedSelectExecutor::NotImplemented(_) => self,
        }
    }

    /// 添加 HAVING 条件
    #[allow(unused_variables)]
    pub fn having<F>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::WhereExpr,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.having(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.having(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.having(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.having(f)),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            GroupedSelectExecutor::NotImplemented(_) => self,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    #[allow(unused_variables)]
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::WhereExpr,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.filter(f)),
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.filter(f))
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.filter(f)),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.filter(f)),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            GroupedSelectExecutor::NotImplemented(_) => self,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            GroupedSelectExecutor::Sqlite(exec) => {
                GroupedCollectFuture::Sqlite(exec.collect::<C>())
            }
            #[cfg(feature = "postgresql")]
            GroupedSelectExecutor::PostgreSQL(exec) => {
                GroupedCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            GroupedSelectExecutor::MySQL(exec) => GroupedCollectFuture::MySQL(exec.collect::<C>()),
            #[cfg(feature = "mssql")]
            GroupedSelectExecutor::MSSQL(exec) => GroupedCollectFuture::MSSQL(exec.collect::<C>()),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            GroupedSelectExecutor::NotImplemented(_) => {
                unimplemented!("GroupedSelectExecutor::collect is not implemented for this backend")
            }
        }
    }

    /// 执行查询并返回 Vec<V>
    pub fn exec(self) -> GroupedCollectFuture<'a, T, V, Vec<V>>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        self.collect::<Vec<V>>()
    }
}

impl<'a, T: Model, V> Clone for MappedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => MappedSelectExecutor::Sqlite(exec.clone()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                MappedSelectExecutor::PostgreSQL(exec.clone_with_client())
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                MappedSelectExecutor::MySQL(exec.clone_with_pool())
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                MappedSelectExecutor::MSSQL(exec.clone_with_pool())
            }
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            MappedSelectExecutor::NotImplemented(phantom) => {
                MappedSelectExecutor::NotImplemented(*phantom)
            }
        }
    }
}

/// 统一的 MappedCollectFuture 枚举
pub enum MappedCollectFuture<'a, T: Model + 'static, V: 'static, C: FromIterator<V> + 'static> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::MappedCollectFuture<'a, T, V, C>),
    #[cfg(not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    )))]
    NotImplemented(std::marker::PhantomData<&'a (T, V, C)>),
}

/// 统一的 GroupedCollectFuture 枚举
pub enum GroupedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::GroupedCollectFuture<'a, T, V, C>),
    #[cfg(not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    )))]
    NotImplemented(std::marker::PhantomData<&'a (T, V, C)>),
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
        match self {
            #[cfg(feature = "sqlite")]
            GroupedCollectFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            GroupedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            GroupedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            GroupedCollectFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            GroupedCollectFuture::NotImplemented(_) => Box::pin(std::future::ready(Err(
                anyhow::anyhow!("No database backend available"),
            ))),
        }
    }
}

/// 统一的 ModelCollectWithFuture 枚举
pub enum ModelCollectWithFuture<'a, T: Model + 'static, V: 'static, C, M, F> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::ModelCollectWithFuture<'a, T, V, C, M, F>),
    #[cfg(feature = "postgresql")]
    PostgreSQLCollect(
        postgresql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mysql")]
    MySQLCollect(
        mysql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(feature = "mssql")]
    MSSQLCollect(
        mssql_backend::MappedCollectFuture<'a, T, V, Vec<V>>,
        F,
        std::marker::PhantomData<&'a (T, C, M)>,
    ),
    #[cfg(not(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "mssql"
    )))]
    NotImplemented(std::marker::PhantomData<&'a (T, V, C, M, F)>),
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持：
    /// - 单字段：map_to(|r| r.uid) -> MappedSelectExecutor<'a, T, i32>
    /// - 元组：map_to(|r| (r.uid, r.id)) -> MappedSelectExecutor<'a, T, (i32, i32)>
    #[allow(unused_variables)]
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => MappedSelectExecutor::Sqlite(exec.map_to(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => MappedSelectExecutor::PostgreSQL(exec.map_to(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => MappedSelectExecutor::MySQL(exec.map_to(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => MappedSelectExecutor::MSSQL(exec.map_to(f)),
            #[allow(unreachable_patterns)]
            _ => unreachable!("MappedSelectExecutor not implemented for this backend"),
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    #[allow(unused_variables)]
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(<T as Model>::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => GroupedSelectExecutor::Sqlite(exec.select_column(f)),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                GroupedSelectExecutor::PostgreSQL(exec.select_column(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => GroupedSelectExecutor::MySQL(exec.select_column(f)),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => GroupedSelectExecutor::MSSQL(exec.select_column(f)),
            #[allow(unreachable_patterns)]
            _ => unreachable!("GroupedSelectExecutor not implemented for this backend"),
        }
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    pub fn collect<C>(self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => MappedCollectFuture::Sqlite(exec.collect::<C>()),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                MappedCollectFuture::PostgreSQL(exec.clone_with_client().collect::<C>())
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                MappedCollectFuture::MySQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                MappedCollectFuture::MSSQL(exec.clone_with_pool().collect::<C>())
            }
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            MappedSelectExecutor::NotImplemented(_) => {
                unimplemented!("MappedSelectExecutor::collect is not implemented for this backend")
            }
        }
    }

    /// 执行查询并收集结果，同时应用转换函数
    /// 用于将查询结果转换为其他类型（如Model）
    /// 示例：collect_with(|v| Uids { id: v })
    #[allow(unused_variables)]
    pub fn collect_with<C, F, M>(self, f: F) -> ModelCollectWithFuture<'a, T, V, C, M, F>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<M> + 'static,
        F: Fn(V) -> M + Clone + 'static,
        M: 'static,
    {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => {
                ModelCollectWithFuture::Sqlite(exec.collect_with::<C, F, M>(f))
            }
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => {
                // PostgreSQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_client();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::PostgreSQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => {
                // MySQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_pool();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::MySQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => {
                // MSSQL也支持collect_with，通过clone exec然后调用collect实现
                let exec_clone = exec.clone_with_pool();
                let future = exec_clone.collect::<Vec<V>>();
                ModelCollectWithFuture::MSSQLCollect(future, f, std::marker::PhantomData)
            }
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            MappedSelectExecutor::NotImplemented(_) => {
                unimplemented!(
                    "MappedSelectExecutor::collect_with is not implemented for this backend"
                )
            }
        }
    }
}

// 为 MappedSelectExecutor 实现 Subquery trait
impl<'a, T: Model, V> crate::query::filter::Subquery for MappedSelectExecutor<'a, T, V> {
    fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        match self {
            #[cfg(feature = "sqlite")]
            MappedSelectExecutor::Sqlite(exec) => exec.to_subquery_sql(),
            #[cfg(feature = "postgresql")]
            MappedSelectExecutor::PostgreSQL(exec) => exec.to_subquery_sql(),
            #[cfg(feature = "mysql")]
            MappedSelectExecutor::MySQL(exec) => exec.to_subquery_sql(),
            #[cfg(feature = "mssql")]
            MappedSelectExecutor::MSSQL(exec) => exec.to_subquery_sql(),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            MappedSelectExecutor::NotImplemented(_) => {
                unimplemented!(
                    "MappedSelectExecutor::to_subquery_sql is not implemented for this backend"
                )
            }
        }
    }
}

// 为 MappedSelectExecutor 实现 IsInValues trait
impl<'a, T: Model, V: crate::query::builder::ColumnValueType> crate::query::builder::IsInValues<V>
    for MappedSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        use crate::query::filter::Subquery;

        let (sql, params) = self.to_subquery_sql();

        // 构造 FilterExpr::InSubquery
        let filter_expr = crate::query::filter::FilterExpr::InSubquery {
            column,
            subquery_sql: sql,
            subquery_params: params,
        };

        crate::query::builder::WhereExpr::from_filter(filter_expr)
    }
}

// 为 &MappedSelectExecutor 实现 IsInValues trait（引用版本）
impl<'a, 'b, T: Model, V: crate::query::builder::ColumnValueType>
    crate::query::builder::IsInValues<V> for &'b MappedSelectExecutor<'a, T, V>
{
    fn to_in_expr(self, column: String) -> crate::query::builder::WhereExpr {
        use crate::query::filter::Subquery;

        let (sql, params) = self.to_subquery_sql();

        // 构造 FilterExpr::InSubquery
        let filter_expr = crate::query::filter::FilterExpr::InSubquery {
            column,
            subquery_sql: sql,
            subquery_params: params,
        };

        crate::query::builder::WhereExpr::from_filter(filter_expr)
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
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "sqlite")]
            MappedCollectFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            MappedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            MappedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mssql")]
            MappedCollectFuture::MSSQL(future) => Box::pin(future.into_future()),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            MappedCollectFuture::NotImplemented(_) => {
                unimplemented!("MappedCollectFuture is not implemented for this backend")
            }
        }
    }
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
        match self {
            #[cfg(feature = "sqlite")]
            ModelCollectWithFuture::Sqlite(future) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            ModelCollectWithFuture::PostgreSQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
            #[cfg(feature = "mysql")]
            ModelCollectWithFuture::MySQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
            #[cfg(feature = "mssql")]
            ModelCollectWithFuture::MSSQLCollect(future, mapper, _) => Box::pin(async move {
                let vec = future.await?;
                Ok(vec.into_iter().map(mapper).collect())
            }),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            ModelCollectWithFuture::NotImplemented(_) => {
                unimplemented!("ModelCollectWithFuture is not implemented for this backend")
            }
        }
    }
}

/// 统一的 SelectStream 枚举
pub enum SelectStream<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectStream<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectStream<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectStream<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectStream<'a, T>),
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectExecutor::Sqlite(exec) => SelectStream::Sqlite(exec.stream()),
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectStream::PostgreSQL(exec.stream()),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectStream::MySQL(exec.stream()),
            #[cfg(feature = "mssql")]
            SelectExecutor::MSSQL(exec) => SelectStream::MSSQL(exec.stream()),
            #[allow(unreachable_patterns)]
            _ => unreachable!("SelectStream not implemented for this backend"),
        }
    }
}

/// 统一的 SelectStreamIterator 枚举
pub enum SelectStreamIterator<'a, T: Model> {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectStreamIterator<'a, T>),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::SelectStreamIterator<'a, T>),
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器
    pub async fn into_iter(self) -> anyhow::Result<SelectStreamIterator<'a, T>> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectStream::Sqlite(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::Sqlite(iter))
            }
            #[cfg(feature = "postgresql")]
            SelectStream::PostgreSQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::PostgreSQL(iter))
            }
            #[cfg(feature = "mysql")]
            SelectStream::MySQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::MySQL(iter))
            }
            #[cfg(feature = "mssql")]
            SelectStream::MSSQL(stream) => {
                let iter = stream.into_iter().await?;
                Ok(SelectStreamIterator::MSSQL(iter))
            }
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            _ => unimplemented!("SelectStream is not implemented for this backend"),
        }
    }
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<anyhow::Result<T>> {
        match self {
            #[cfg(feature = "sqlite")]
            SelectStreamIterator::Sqlite(iter) => iter.next().await,
            #[cfg(feature = "postgresql")]
            SelectStreamIterator::PostgreSQL(iter) => iter.next().await,
            #[cfg(feature = "mysql")]
            SelectStreamIterator::MySQL(iter) => iter.next().await,
            #[cfg(feature = "mssql")]
            SelectStreamIterator::MSSQL(iter) => iter.next().await,
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            _ => unimplemented!("SelectStreamIterator is not implemented for this backend"),
        }
    }
}
