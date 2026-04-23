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
    pub async fn insert<I: crate::model::Insertable>(&self, models: I) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.insert_batch::<I::Model>(&refs).await,
        }
    }

    /// 插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
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

    /// 删除表
    pub async fn drop_table<T: Model>(&self) -> Result<(), crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.drop_table::<T>().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.drop_table::<T>().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.drop_table::<T>().await,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn exec_table<T: Model>(&self, sql: &str) -> Result<Vec<T>, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.exec_table::<T>(sql).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.exec_table::<T>(sql).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.exec_table::<T>(sql).await,
        }
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn exec_non_query(&self, sql: &str) -> Result<u64, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            Database::Turso(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.exec_non_query(sql).await,
        }
    }

    /// 创建连接池
    #[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
    pub fn create_pool(
        db_type: super::DbType,
        connection_string: &str,
    ) -> super::connection_pool::PoolBuilder {
        super::connection_pool::PoolBuilder::new(db_type, connection_string)
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

crate::impl_unified_select_executor_methods!(SelectExecutor, std::marker::PhantomData);

impl<'a, T: Model> SelectExecutor<'a, T> {
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

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                InnerJoinedSelectExecutor::Turso(exec.inner_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                InnerJoinedSelectExecutor::PostgreSQL(exec.inner_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                InnerJoinedSelectExecutor::MySQL(exec.inner_join::<J>(f))
            }
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                RightJoinedSelectExecutor::Turso(exec.right_join::<J>(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => {
                RightJoinedSelectExecutor::PostgreSQL(exec.right_join::<J>(f))
            }
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => {
                RightJoinedSelectExecutor::MySQL(exec.right_join::<J>(f))
            }
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

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                AggregateFuture::Turso(exec.count(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.count(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.count(f)),
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                AggregateFuture::Turso(exec.sum(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.sum(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.sum(f)),
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                AggregateFuture::Turso(exec.avg(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.avg(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.avg(f)),
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                AggregateFuture::Turso(exec.max(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.max(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.max(f)),
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                AggregateFuture::Turso(exec.min(f), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            SelectExecutor::PostgreSQL(exec) => AggregateFuture::PostgreSQL(exec.min(f)),
            #[cfg(feature = "mysql")]
            SelectExecutor::MySQL(exec) => AggregateFuture::MySQL(exec.min(f)),
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

crate::impl_unified_delete_executor!(DeleteExecutor, std::marker::PhantomData);

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

crate::impl_unified_update_executor!(UpdateExecutor, std::marker::PhantomData);

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

/// 统一的 AggregateFuture 枚举
pub enum AggregateFuture<'a, T: Model, R> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::AggregateFuture<T, R>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::AggregateFuture<'a, T, R>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::AggregateFuture<'a, T, R>),
}

crate::impl_unified_aggregate_future!(AggregateFuture, std::marker::PhantomData);

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

/// 统一的 InnerJoinedSelectExecutor 枚举
pub enum InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::InnerJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinedSelectExecutor<'a, T, J>),
}

/// 统一的 RightJoinedSelectExecutor 枚举
pub enum RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::RightJoinedSelectExecutor<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinedSelectExecutor<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinedSelectExecutor<'a, T, J>),
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

/// 统一的 InnerJoinCollectFuture 枚举
pub enum InnerJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::InnerJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::InnerJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::InnerJoinCollectFuture<'a, T, J>),
}

/// 统一的 RightJoinCollectFuture 枚举
pub enum RightJoinCollectFuture<'a, T: Model, J: Model> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::RightJoinCollectFuture<T, J>,
        std::marker::PhantomData<&'a ()>,
    ),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::RightJoinCollectFuture<'a, T, J>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::RightJoinCollectFuture<'a, T, J>),
}

crate::impl_unified_collect_future!(CollectFuture, std::marker::PhantomData);

crate::impl_unified_related_select_executor!(RelatedSelectExecutor, std::marker::PhantomData);

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

crate::impl_unified_related_collect_future!(RelatedCollectFuture, std::marker::PhantomData);

/// 统一的 Transaction 枚举
pub enum Transaction<'a> {
    #[cfg(feature = "turso")]
    Turso(turso_backend::Transaction, std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Transaction<'a>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Transaction<'a>),
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
    pub async fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => txn.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.insert_batch::<I::Model>(&refs).await,
        }
    }

    /// 插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self {
            #[cfg(feature = "turso")]
            Transaction::Turso(txn, _) => txn.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            Transaction::PostgreSQL(txn) => txn.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            Transaction::MySQL(txn) => txn.insert_or_update_batch::<I::Model>(&refs).await,
        }
    }
}

crate::impl_unified_join_executor!(LeftJoinedSelectExecutor, std::marker::PhantomData);

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
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

crate::impl_unified_join_executor!(InnerJoinedSelectExecutor, std::marker::PhantomData);

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(T, J)> + 'static>(self) -> InnerJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            InnerJoinedSelectExecutor::Turso(exec, _) => {
                InnerJoinCollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            InnerJoinedSelectExecutor::PostgreSQL(exec) => {
                InnerJoinCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            InnerJoinedSelectExecutor::MySQL(exec) => {
                InnerJoinCollectFuture::MySQL(exec.collect::<C>())
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

crate::impl_unified_join_executor!(RightJoinedSelectExecutor, std::marker::PhantomData);

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        self,
    ) -> RightJoinCollectFuture<'a, T, J>
    where
        T: 'static,
        J: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            RightJoinedSelectExecutor::Turso(exec, _) => {
                RightJoinCollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            RightJoinedSelectExecutor::PostgreSQL(exec) => {
                RightJoinCollectFuture::PostgreSQL(exec.collect::<C>())
            }
            #[cfg(feature = "mysql")]
            RightJoinedSelectExecutor::MySQL(exec) => {
                RightJoinCollectFuture::MySQL(exec.collect::<C>())
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
    Result<Vec<(T, Option<J>)>, crate::Error>,
    std::marker::PhantomData
);

crate::impl_unified_join_collect_future!(
    InnerJoinCollectFuture,
    Result<Vec<(T, J)>, crate::Error>,
    std::marker::PhantomData
);

crate::impl_unified_join_collect_future!(
    RightJoinCollectFuture,
    Result<Vec<(Option<T>, J)>, crate::Error>,
    std::marker::PhantomData
);

/// 统一的 MappedSelectExecutor 枚举
pub enum MappedSelectExecutor<'a, T: Model, V> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::MappedSelectExecutor<T, V>,
        std::marker::PhantomData<&'a ()>,
    ),
    // TODO: 实现 postgresql 和 mysql 后端
    // #[cfg(feature = "postgresql")]
    // PostgreSQL(postgresql_backend::MappedSelectExecutor<'a, T, V>),
    // #[cfg(feature = "mysql")]
    // MySQL(mysql_backend::MappedSelectExecutor<'a, T, V>),
}

impl<'a, T: Model, V> Clone for MappedSelectExecutor<'a, T, V> {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "turso")]
            MappedSelectExecutor::Turso(exec, phantom) => {
                MappedSelectExecutor::Turso(exec.clone(), *phantom)
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!("MappedSelectExecutor only implemented for Turso backend"),
        }
    }
}

/// 统一的 MappedCollectFuture 枚举
pub enum MappedCollectFuture<'a, T: Model, V, C: FromIterator<V>> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::MappedCollectFuture<T, V, C>,
        std::marker::PhantomData<&'a ()>,
    ),
    // TODO: 实现 postgresql 和 mysql 后端
    // #[cfg(feature = "postgresql")]
    // PostgreSQL(postgresql_backend::MappedCollectFuture<'a, T, V, C>),
    // #[cfg(feature = "mysql")]
    // MySQL(mysql_backend::MappedCollectFuture<'a, T, V, C>),
}

/// 统一的 ModelCollectWithFuture 枚举
pub enum ModelCollectWithFuture<'a, T: Model, V, C, M, F> {
    #[cfg(feature = "turso")]
    Turso(
        turso_backend::ModelCollectWithFuture<T, V, C, M, F>,
        std::marker::PhantomData<&'a ()>,
    ),
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 字段投影 - 将查询结果映射到单个字段或元组
    /// 支持：
    /// - 单字段：map_to(|r| r.uid) -> MappedSelectExecutor<'a, T, i32>
    /// - 元组：map_to(|r| (r.uid, r.id)) -> MappedSelectExecutor<'a, T, (i32, i32)>
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(<T as Model>::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        match self {
            #[cfg(feature = "turso")]
            SelectExecutor::Turso(exec, _) => {
                MappedSelectExecutor::Turso(exec.map_to(f), std::marker::PhantomData)
            }
            // TODO: 实现 postgresql 和 mysql 后端
            #[allow(unreachable_patterns)]
            _ => unreachable!("MappedSelectExecutor only implemented for Turso backend"),
        }
    }
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<V> + 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            MappedSelectExecutor::Turso(exec, _) => {
                MappedCollectFuture::Turso(exec.collect::<C>(), std::marker::PhantomData)
            } // TODO: 实现 postgresql 和 mysql 后端
              // #[cfg(feature = "postgresql")]
              // MappedSelectExecutor::PostgreSQL(exec) => {
              //     MappedCollectFuture::PostgreSQL(exec.collect::<C>())
              // }
              // #[cfg(feature = "mysql")]
              // MappedSelectExecutor::MySQL(exec) => MappedCollectFuture::MySQL(exec.collect::<C>()),
        }
    }

    /// 执行查询并收集结果，同时应用转换函数
    /// 用于将查询结果转换为其他类型（如Model）
    /// 示例：collect_with(|v| Uids { id: v })
    pub fn collect_with<C, F, M>(&self, f: F) -> ModelCollectWithFuture<'a, T, V, C, M, F>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
        C: FromIterator<M> + 'static,
        F: Fn(V) -> M + Clone + 'static,
        M: 'static,
    {
        match self {
            #[cfg(feature = "turso")]
            MappedSelectExecutor::Turso(exec, _) => ModelCollectWithFuture::Turso(
                exec.collect_with::<C, F, M>(f),
                std::marker::PhantomData,
            ),
        }
    }
}

// 为 MappedSelectExecutor 实现 Subquery trait
impl<'a, T: Model, V> crate::query::filter::Subquery for MappedSelectExecutor<'a, T, V> {
    fn to_subquery_sql(&self) -> (String, Vec<crate::model::Value>) {
        match self {
            #[cfg(feature = "turso")]
            MappedSelectExecutor::Turso(exec, _) => exec.to_subquery_sql(),
            // TODO: 实现 postgresql 和 mysql 后端
            #[allow(unreachable_patterns)]
            _ => unreachable!("MappedSelectExecutor only implemented for Turso backend"),
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

impl<'a, T: Model + 'static, V: crate::model::FromRowValues + 'static, C: FromIterator<V> + 'static>
    std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "turso")]
            MappedCollectFuture::Turso(future, _) => Box::pin(future.into_future()),
            // TODO: 实现 postgresql 和 mysql 后端
            // #[cfg(feature = "postgresql")]
            // MappedCollectFuture::PostgreSQL(future) => Box::pin(future.into_future()),
            // #[cfg(feature = "mysql")]
            // MappedCollectFuture::MySQL(future) => Box::pin(future.into_future()),
        }
    }
}

impl<'a, T, V, C, M, F> std::future::IntoFuture for ModelCollectWithFuture<'a, T, V, C, M, F>
where
    T: Model + 'static,
    V: crate::model::FromRowValues + 'static,
    C: FromIterator<M> + 'static,
    M: 'static,
    F: Fn(V) -> M + Clone + 'static,
{
    type Output = Result<C, crate::Error>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            #[cfg(feature = "turso")]
            ModelCollectWithFuture::Turso(future, _) => Box::pin(future.into_future()),
        }
    }
}
