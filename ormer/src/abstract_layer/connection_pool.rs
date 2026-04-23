use crate::abstract_layer::DbType;
use crate::model::Model;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, Semaphore};

// 根据启用的 feature 导入后端实现
#[cfg(feature = "turso")]
use super::turso_backend;

#[cfg(feature = "postgresql")]
use super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::mysql_backend;

/// 连接包装器 - 包装各后端的 Database 实例
enum ConnectionWrapper {
    #[cfg(feature = "turso")]
    Turso(turso_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
}

impl ConnectionWrapper {
    /// 检查连接是否有效
    async fn is_valid(&self) -> bool {
        match self {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.is_valid().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.is_valid().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.is_valid().await,
        }
    }
}

/// 手工连接池核心结构
pub struct ManualPool {
    /// 空闲连接队列
    idle_connections: Mutex<VecDeque<ConnectionWrapper>>,
    /// 控制最大连接数的信号量
    semaphore: Semaphore,
    /// 当前连接总数(包括使用中和空闲的)
    total_connections: AtomicU32,
    /// 连接池配置
    config: PoolConfig,
    /// 数据库类型
    db_type: DbType,
    /// 连接字符串
    connection_string: String,
}

impl ManualPool {
    /// 创建新的连接池
    fn new(db_type: DbType, connection_string: String, config: PoolConfig) -> Arc<Self> {
        let max_size = config.max_size;
        Arc::new(Self {
            idle_connections: Mutex::new(VecDeque::new()),
            semaphore: Semaphore::new(max_size as usize),
            total_connections: AtomicU32::new(0),
            config,
            db_type,
            connection_string,
        })
    }

    /// 创建新的数据库连接
    async fn create_connection(&self) -> Result<ConnectionWrapper, crate::Error> {
        match self.db_type {
            #[cfg(feature = "turso")]
            DbType::Turso => {
                let db =
                    turso_backend::Database::connect(self.db_type, &self.connection_string).await?;
                Ok(ConnectionWrapper::Turso(db))
            }
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                let db =
                    postgresql_backend::Database::connect(self.db_type, &self.connection_string)
                        .await?;
                Ok(ConnectionWrapper::PostgreSQL(db))
            }
            #[cfg(feature = "mysql")]
            DbType::MySQL => {
                let db =
                    mysql_backend::Database::connect(self.db_type, &self.connection_string).await?;
                Ok(ConnectionWrapper::MySQL(db))
            }
        }
    }

    /// 获取连接(异步)
    async fn get(&self) -> Result<ConnectionWrapper, crate::Error> {
        // 尝试从空闲队列获取
        {
            let mut idle = self.idle_connections.lock().await;
            if let Some(conn) = idle.pop_front() {
                // 检查连接是否有效
                if conn.is_valid().await {
                    return Ok(conn);
                }
                // 连接失效,减少计数
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
            }
        }

        // 空闲队列没有可用连接,尝试创建新连接
        let current_total = self.total_connections.load(Ordering::SeqCst);
        if current_total < self.config.max_size {
            // 可以增加连接数
            let conn = self.create_connection().await?;
            self.total_connections.fetch_add(1, Ordering::SeqCst);
            return Ok(conn);
        }

        // 已达到最大连接数,等待信号量(会有其他连接归还)
        // 注意:这里需要先释放 semaphore permit,然后等待
        // 实际上我们应该等待空闲队列中有连接
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            let mut idle = self.idle_connections.lock().await;
            if let Some(conn) = idle.pop_front() {
                if conn.is_valid().await {
                    return Ok(conn);
                }
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// 归还连接到池
    async fn return_connection(&self, conn: ConnectionWrapper) {
        // 检查连接是否有效
        if conn.is_valid().await {
            let mut idle = self.idle_connections.lock().await;
            idle.push_back(conn);
        } else {
            // 连接失效,减少计数
            self.total_connections.fetch_sub(1, Ordering::SeqCst);
        }
        // 释放信号量 permit
        self.semaphore.add_permits(1);
    }

    /// 维护最小连接数
    async fn maintain_min_connections(&self) {
        let current_total = self.total_connections.load(Ordering::SeqCst);
        let target = self.config.min_size;

        if current_total < target {
            let to_create = target - current_total;
            for _ in 0..to_create {
                if let Ok(conn) = self.create_connection().await {
                    self.total_connections.fetch_add(1, Ordering::SeqCst);
                    let mut idle = self.idle_connections.lock().await;
                    idle.push_back(conn);
                }
            }
        }
    }
}

/// 连接池配置
#[derive(Clone)]
pub struct PoolConfig {
    min_size: u32,
    max_size: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 0,
            max_size: 10,
        }
    }
}

/// 连接池构建器
pub struct PoolBuilder {
    db_type: DbType,
    connection_string: String,
    config: PoolConfig,
}

impl PoolBuilder {
    pub fn new(db_type: DbType, connection_string: &str) -> Self {
        Self {
            db_type,
            connection_string: connection_string.to_string(),
            config: PoolConfig::default(),
        }
    }

    /// 设置连接池大小范围
    pub fn range(mut self, range: std::ops::Range<u32>) -> Self {
        self.config.min_size = range.start;
        self.config.max_size = range.end;
        self
    }

    /// 构建连接池
    pub async fn build(self) -> Result<ConnectionPool, crate::Error> {
        let pool = ManualPool::new(self.db_type, self.connection_string, self.config.clone());

        // 如果 min_size > 0,预先创建最小连接数
        if self.config.min_size > 0 {
            pool.maintain_min_connections().await;
        }

        match self.db_type {
            #[cfg(feature = "turso")]
            DbType::Turso => Ok(ConnectionPool::Turso(pool)),
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => Ok(ConnectionPool::PostgreSQL(pool)),
            #[cfg(feature = "mysql")]
            DbType::MySQL => Ok(ConnectionPool::MySQL(pool)),
        }
    }
}

/// 统一的连接池枚举
pub enum ConnectionPool {
    #[cfg(feature = "turso")]
    Turso(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(Arc<ManualPool>),
    #[cfg(feature = "mysql")]
    MySQL(Arc<ManualPool>),
}

impl ConnectionPool {
    /// 从连接池异步获取连接
    ///
    /// 此方法会等待直到有可用连接或创建新连接
    /// 如果池中没有连接且未达到 max_size,会自动创建新连接
    pub async fn get(&self) -> Result<PooledConnection<'_>, crate::Error> {
        match self {
            #[cfg(feature = "turso")]
            ConnectionPool::Turso(pool) => {
                // 获取信号量 permit
                let _permit = pool.semaphore.acquire().await.map_err(|e| {
                    crate::Error::Database(format!("Failed to acquire connection: {}", e))
                })?;
                let conn = pool.get().await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::Turso(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "postgresql")]
            ConnectionPool::PostgreSQL(pool) => {
                let _permit = pool.semaphore.acquire().await.map_err(|e| {
                    crate::Error::Database(format!("Failed to acquire connection: {}", e))
                })?;
                let conn = pool.get().await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::PostgreSQL(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mysql")]
            ConnectionPool::MySQL(pool) => {
                let _permit = pool.semaphore.acquire().await.map_err(|e| {
                    crate::Error::Database(format!("Failed to acquire connection: {}", e))
                })?;
                let conn = pool.get().await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MySQL(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
        }
    }
}

/// 连接池内部类型
#[derive(Clone)]
enum PooledConnectionInner {
    #[cfg(feature = "turso")]
    Turso(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(Arc<ManualPool>),
    #[cfg(feature = "mysql")]
    MySQL(Arc<ManualPool>),
}

impl PooledConnectionInner {
    async fn return_connection(&self, conn: ConnectionWrapper) {
        match self {
            #[cfg(feature = "turso")]
            PooledConnectionInner::Turso(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL(pool) => pool.return_connection(conn).await,
        }
    }
}

/// 统一的 PooledConnection
/// 包装连接,实现 Database 的所有方法,Drop 时自动归还到池
pub struct PooledConnection<'a> {
    inner: PooledConnectionInner,
    connection: Option<ConnectionWrapper>,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Drop for PooledConnection<'a> {
    fn drop(&mut self) {
        if let Some(conn) = self.connection.take() {
            let inner = self.inner.clone();
            // 使用 tokio::spawn 异步归还连接并释放信号量
            tokio::spawn(async move {
                inner.return_connection(conn).await;
            });
        }
    }
}

impl<'a> PooledConnection<'a> {
    /// 获取底层连接的引用(内部使用)
    fn get_connection(&self) -> &ConnectionWrapper {
        self.connection.as_ref().expect("Connection already taken")
    }

    /// 创建表
    pub async fn create_table<T: Model>(&self) -> Result<(), crate::Error> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.create_table::<T>().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.create_table::<T>().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.create_table::<T>().await,
        }
    }

    /// 插入记录
    pub async fn insert<I: crate::model::Insertable>(&self, models: I) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.insert_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.insert_batch::<I::Model>(&refs).await,
        }
    }

    /// 插入或更新记录
    pub async fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> Result<(), crate::Error> {
        let refs = models.as_refs();
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.insert_or_update_batch::<I::Model>(&refs).await,
        }
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> super::unified::SelectExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => {
                super::unified::SelectExecutor::Turso(db.select::<T>(), PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::SelectExecutor::PostgreSQL(db.select::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::SelectExecutor::MySQL(db.select::<T>()),
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> super::unified::DeleteExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => {
                super::unified::DeleteExecutor::Turso(db.delete::<T>(), PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::DeleteExecutor::PostgreSQL(db.delete::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::DeleteExecutor::MySQL(db.delete::<T>()),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> super::unified::UpdateExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => {
                super::unified::UpdateExecutor::Turso(db.update::<T>(), PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::UpdateExecutor::PostgreSQL(db.update::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::UpdateExecutor::MySQL(db.update::<T>()),
        }
    }

    /// 创建 Related 查询执行器
    pub fn related<T: Model + 'static, R: Model>(
        &self,
    ) -> super::unified::RelatedSelectExecutor<'_, T, R> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => {
                super::unified::RelatedSelectExecutor::Turso(db.related::<T, R>(), PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::RelatedSelectExecutor::PostgreSQL(db.related::<T, R>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                super::unified::RelatedSelectExecutor::MySQL(db.related::<T, R>())
            }
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> Result<super::unified::Transaction<'_>, crate::Error> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => {
                let txn = db.begin().await?;
                Ok(super::unified::Transaction::Turso(txn, PhantomData))
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                let txn = db.begin().await?;
                Ok(super::unified::Transaction::PostgreSQL(txn))
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                let txn = db.begin().await?;
                Ok(super::unified::Transaction::MySQL(txn))
            }
        }
    }

    /// 删除表
    pub async fn drop_table<T: Model>(&self) -> Result<(), crate::Error> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.drop_table::<T>().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.drop_table::<T>().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.drop_table::<T>().await,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn exec_table<T: Model>(&self, sql: &str) -> Result<Vec<T>, crate::Error> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.exec_table::<T>(sql).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.exec_table::<T>(sql).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.exec_table::<T>(sql).await,
        }
    }

    /// 执行原生非查询 SQL
    pub async fn exec_non_query(&self, sql: &str) -> Result<u64, crate::Error> {
        match self.get_connection() {
            #[cfg(feature = "turso")]
            ConnectionWrapper::Turso(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.exec_non_query(sql).await,
        }
    }
}
