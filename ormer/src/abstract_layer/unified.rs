/// 统一的数据库抽象层
/// 使用枚举包装不同数据库后端,对外提供统一接口
/// 通过条件编译控制枚举变体
use crate::model::Model;
use crate::query::builder::WhereExpr;

// 根据启用的 feature 导入后端实现
#[cfg(feature = "turso")]
use super::turso_backend;

#[cfg(feature = "postgresql")]
use super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::mysql_backend;

/// 统一的 Database 枚举
pub enum Database {
    #[cfg(feature = "turso")]
    Turso(turso_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
}

impl Database {
    /// 连接到数据库,根据 DbType 选择后端
    pub async fn connect(
        db_type: super::DbType,
        connection_string: &str,
    ) -> Result<Self, crate::Error> {
        match db_type {
            #[cfg(feature = "turso")]
            super::DbType::Turso => {
                let db = turso_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::Turso(db))
            }
            #[cfg(feature = "postgresql")]
            super::DbType::PostgreSQL => {
                let db = postgresql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::PostgreSQL(db))
            }
            #[cfg(feature = "mysql")]
            super::DbType::MySQL => {
                let db = mysql_backend::Database::connect(db_type, connection_string).await?;
                Ok(Database::MySQL(db))
            }
        }
    }

    /// 创建表
    pub async fn create_table<T: Model>(&self) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.create_table::<T>().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.create_table::<T>().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.create_table::<T>().await,
        }
    }

    /// 插入记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.insert::<T>(model).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.insert::<T>(model).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.insert::<T>(model).await,
        }
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => {
                SelectExecutor::Turso(db.select::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => SelectExecutor::PostgreSQL(db.select::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => SelectExecutor::MySQL(db.select::<T>()),
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => {
                DeleteExecutor::Turso(db.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => DeleteExecutor::PostgreSQL(db.delete::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => DeleteExecutor::MySQL(db.delete::<T>()),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => {
                UpdateExecutor::Turso(db.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => UpdateExecutor::PostgreSQL(db.update::<T>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => UpdateExecutor::MySQL(db.update::<T>()),
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn from<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => {
                RelatedSelectExecutor::Turso(db.related::<T, R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => RelatedSelectExecutor::PostgreSQL(db.related::<T, R>()),
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => RelatedSelectExecutor::MySQL(db.related::<T, R>()),
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> Result<Transaction<'_>, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => {
                let txn = db.begin().await?;
                Ok(Transaction::Turso(txn, std::marker::PhantomData))
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
        }
    }
}

/// 统一的 SelectExecutor 枚举
pub enum SelectExecutor<'a, T: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::SelectExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::SelectExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::SelectExecutor<'a, T>),
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                SelectExecutor::Turso(exec.filter(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.filter(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.filter(f)),
        }
    }

    pub fn order_by<F>(self, f: F) -> Self
    where
        F: FnOnce(crate::WhereColumn<T>) -> crate::OrderBy,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                SelectExecutor::Turso(exec.order_by(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.order_by(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.order_by(f)),
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                SelectExecutor::Turso(exec.limit(limit), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.limit(limit)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.limit(limit)),
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                SelectExecutor::Turso(exec.offset(offset), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => SelectExecutor::PostgreSQL(exec.offset(offset)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => SelectExecutor::MySQL(exec.offset(offset)),
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2: Model, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                RelatedSelectExecutor::Turso(exec.from::<T2, R>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RelatedSelectExecutor::PostgreSQL(exec.from::<T2, R>())
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => RelatedSelectExecutor::MySQL(exec.from::<T2, R>()),
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2: Model, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => MultiTableSelectExecutor::Turso(
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
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => FourTableSelectExecutor::Turso(
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
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                LeftJoinedSelectExecutor::Turso(exec.left_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.left_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => LeftJoinedSelectExecutor::MySQL(exec.left_join::<J>(f)),
        }
    }

    pub fn collect<C: FromIterator<T> + 'static>(self) -> CollectFuture<'a, T, C> {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                CollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => CollectFuture::PostgreSQL(exec.collect::<C>()),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => CollectFuture::MySQL(exec.collect::<C>()),
        }
    }

    /// 执行查询并返回 Vec<T> (collect 的别名)
    pub fn execute(self) -> CollectFuture<'a, T, Vec<T>>
    where
        T: 'static,
    {
        self.collect::<Vec<T>>()
    }
}

/// 统一的 DeleteExecutor 枚举
pub enum DeleteExecutor<'a, T: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::DeleteExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::DeleteExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::DeleteExecutor<'a, T>),
}

impl<'a, T: Model> DeleteExecutor<'a, T> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        match self {
            #[cfg(feature = "turso")]
            DeleteExecutor::Turso(exec, _) => {
                DeleteExecutor::Turso(exec.filter(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            DeleteExecutor::PostgreSQL(exec) => DeleteExecutor::PostgreSQL(exec.filter(f)),
            #[cfg(feature = "mysql")]
            DeleteExecutor::MySQL(exec) => DeleteExecutor::MySQL(exec.filter(f)),
        }
    }

    pub async fn execute(self) -> Result<u64, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            DeleteExecutor::Turso(exec, _) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            DeleteExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            DeleteExecutor::MySQL(exec) => exec.execute().await,
        }
    }
}

impl<'a, T: Model + 'static> std::future::IntoFuture for DeleteExecutor<'a, T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 统一的 UpdateExecutor 枚举
pub enum UpdateExecutor<'a, T: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::UpdateExecutor<T>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::UpdateExecutor<'a, T>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::UpdateExecutor<'a, T>),
}

impl<'a, T: Model> UpdateExecutor<'a, T> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        match self {
            #[cfg(feature = "turso")]
            UpdateExecutor::Turso(exec, _) => {
                UpdateExecutor::Turso(exec.filter(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            UpdateExecutor::PostgreSQL(exec) => UpdateExecutor::PostgreSQL(exec.filter(f)),
            #[cfg(feature = "mysql")]
            UpdateExecutor::MySQL(exec) => UpdateExecutor::MySQL(exec.filter(f)),
        }
    }

    pub fn set<F, V>(self, field_fn: F, value: V) -> Self
    where
        F: FnOnce(T::Where) -> crate::query::builder::NumericColumn,
        V: Into<crate::model::Value>,
    {
        match self {
            #[cfg(feature = "turso")]
            UpdateExecutor::Turso(exec, _) => {
                UpdateExecutor::Turso(exec.set(field_fn, value), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            UpdateExecutor::PostgreSQL(exec) => {
                UpdateExecutor::PostgreSQL(exec.set(field_fn, value))
            }
            #[cfg(feature = "mysql")]
            UpdateExecutor::MySQL(exec) => UpdateExecutor::MySQL(exec.set(field_fn, value)),
        }
    }

    pub async fn execute(self) -> Result<u64, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            UpdateExecutor::Turso(exec, _) => exec.execute().await,
            #[cfg(feature = "postgresql")]
            UpdateExecutor::PostgreSQL(exec) => exec.execute().await,
            #[cfg(feature = "mysql")]
            UpdateExecutor::MySQL(exec) => exec.execute().await,
        }
    }
}

impl<'a, T: Model + 'static> std::future::IntoFuture for UpdateExecutor<'a, T> {
    type Output = Result<u64, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// 统一的 CollectFuture 枚举
pub enum CollectFuture<'a, T: Model, C: FromIterator<T>> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::CollectFuture<T, C>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::CollectFuture<'a, T, C>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::CollectFuture<'a, T, C>),
}

/// 统一的 RelatedSelectExecutor 枚举
pub enum RelatedSelectExecutor<'a, T: Model, R: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::RelatedSelectExecutor<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedSelectExecutor<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedSelectExecutor<'a, T, R>),
}

/// 统一的 MultiTableSelectExecutor 枚举
pub enum MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::MultiTableSelectExecutor<T, R1, R2>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::MultiTableSelectExecutor<'a, T, R1, R2>),
}

/// 统一的 FourTableSelectExecutor 枚举
pub enum FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::FourTableSelectExecutor<T, R1, R2, R3>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::FourTableSelectExecutor<'a, T, R1, R2, R3>),
}

/// 统一的 LeftJoinedSelectExecutor 枚举
pub enum LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::LeftJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinedSelectExecutor<'a, T, J>),
}

/// 统一的 LeftJoinCollectFuture 枚举
pub enum LeftJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::LeftJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::LeftJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::LeftJoinCollectFuture<'a, T, J>),
}

impl<'a, T: Model + 'static, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<'a, T, C>
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "turso")]
            CollectFuture::Turso(future, _) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            CollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            CollectFuture::MySQL(future) => Box::pin(future.into_future()),
        }
    }
}

impl<'a, T: Model, R: Model> RelatedSelectExecutor<'a, T, R> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where, R::Where) -> WhereExpr,
    {
        match self {
            #[cfg(feature = "turso")]
            RelatedSelectExecutor::Turso(exec, _) => {
                RelatedSelectExecutor::Turso(exec.filter(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            RelatedSelectExecutor::PostgreSQL(exec) => {
                RelatedSelectExecutor::PostgreSQL(exec.filter(f))
            }
            #[cfg(feature = "mysql")]
            RelatedSelectExecutor::MySQL(exec) => RelatedSelectExecutor::MySQL(exec.filter(f)),
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        match self {
            #[cfg(feature = "turso")]
            RelatedSelectExecutor::Turso(exec, _) => {
                RelatedSelectExecutor::Turso(exec.limit(limit), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            RelatedSelectExecutor::PostgreSQL(exec) => {
                RelatedSelectExecutor::PostgreSQL(exec.limit(limit))
            }
            #[cfg(feature = "mysql")]
            RelatedSelectExecutor::MySQL(exec) => RelatedSelectExecutor::MySQL(exec.limit(limit)),
        }
    }

    pub fn collect<C: FromIterator<T> + 'static>(self) -> RelatedCollectFuture<'a, T, R>
    where
        T: 'static,
        R: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            RelatedSelectExecutor::Turso(exec, _) => {
                RelatedCollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            RelatedSelectExecutor::PostgreSQL(exec) => {
                RelatedCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            RelatedSelectExecutor::MySQL(exec) => RelatedCollectFuture::MySQL(exec.collect::<C>()),
        }
    }

    pub fn exec(self) -> RelatedCollectFuture<'a, T, R>
    where
        T: 'static,
        R: 'static,
    {
        self.collect::<Vec<T>>()
    }

    /// 执行查询并返回 Vec<T> (exec 的别名)
    pub fn execute(self) -> RelatedCollectFuture<'a, T, R>
    where
        T: 'static,
        R: 'static,
    {
        self.collect::<Vec<T>>()
    }
}

/// 统一的 RelatedCollectFuture 枚举
pub enum RelatedCollectFuture<'a, T: Model, R: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::RelatedCollectFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RelatedCollectFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RelatedCollectFuture<'a, T, R>),
}

impl<'a, T: Model + 'static, R: Model + 'static> std::future::IntoFuture
    for RelatedCollectFuture<'a, T, R>
{
    type Output = Result<Vec<T>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "turso")]
            RelatedCollectFuture::Turso(future, _) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            RelatedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            RelatedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
        }
    }
}

/// 统一的 Transaction 枚举
pub enum Transaction<'a> {
    #[cfg(feature = "turso")]
    Turso(turso_backend::Transaction, std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Transaction<'a>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Transaction),
}

impl<'a> Transaction<'a> {
    /// 提交事务
    pub async fn commit(self) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => txn.commit().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.commit().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.commit().await,
        }
    }

    /// 回滚事务
    pub async fn rollback(self) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => txn.rollback().await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.rollback().await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.rollback().await,
        }
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => {
                SelectExecutor::Turso(txn.select::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => SelectExecutor::PostgreSQL(txn.select::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => SelectExecutor::MySQL(txn.select::<T>()),
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> DeleteExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => {
                DeleteExecutor::Turso(txn.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => DeleteExecutor::PostgreSQL(txn.delete::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => DeleteExecutor::MySQL(txn.delete::<T>()),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> UpdateExecutor<'_, T> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => {
                UpdateExecutor::Turso(txn.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => UpdateExecutor::PostgreSQL(txn.update::<T>()),
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => UpdateExecutor::MySQL(txn.update::<T>()),
        }
    }

    /// 插入记录
    pub async fn insert<T: Model>(&self, model: &T) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => txn.insert::<T>(model).await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.insert::<T>(model).await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.insert::<T>(model).await,
        }
    }
}

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    pub fn filter<F>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> WhereExpr,
    {
        match self {
            #[cfg(feature = "turso")]
            LeftJoinedSelectExecutor::Turso(exec, _) => {
                LeftJoinedSelectExecutor::Turso(exec.filter(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.filter(f))
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinedSelectExecutor::MySQL(exec.filter(f))
            }
        }
    }

    pub fn limit(self, limit: i64) -> Self {
        match self {
            #[cfg(feature = "turso")]
            LeftJoinedSelectExecutor::Turso(exec, _) => {
                LeftJoinedSelectExecutor::Turso(exec.limit(limit), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.limit(limit))
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinedSelectExecutor::MySQL(exec.limit(limit))
            }
        }
    }

    pub fn offset(self, offset: i64) -> Self {
        match self {
            #[cfg(feature = "turso")]
            LeftJoinedSelectExecutor::Turso(exec, _) => {
                LeftJoinedSelectExecutor::Turso(exec.offset(offset), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinedSelectExecutor::PostgreSQL(exec.offset(offset))
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinedSelectExecutor::MySQL(exec.offset(offset))
            }
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        self,
    ) -> LeftJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            LeftJoinedSelectExecutor::Turso(exec, _) => {
                LeftJoinCollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            LeftJoinedSelectExecutor::PostgreSQL(exec) => {
                LeftJoinCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            LeftJoinedSelectExecutor::MySQL(exec) => {
                LeftJoinCollectFuture::MySQL(exec.collect::<C>())
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

impl<'a, T: Model + 'static, J: Model + 'static> std::future::IntoFuture
    for LeftJoinCollectFuture<'a, T, J>
{
    type Output = Result<Vec<(T, Option<J>)>, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "turso")]
            LeftJoinCollectFuture::Turso(future, _) => Box::pin(future.into_future()),
            #[cfg(feature = "postgresql")]
            LeftJoinCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            #[cfg(feature = "mysql")]
            LeftJoinCollectFuture::MySQL(future) => Box::pin(future.into_future()),
        }
    }
}
