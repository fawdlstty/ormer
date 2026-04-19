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
