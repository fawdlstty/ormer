use super::super::DbType;
use crate::model::Model;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, Semaphore};

#[cfg(feature = "postgresql")]
use bb8_postgres::PostgresConnectionManager;
#[cfg(feature = "postgresql")]
use tokio_postgres::NoTls;

// 导入统一的执行器类型
#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
use super::unified::{CreateTableExecutor, DropTableExecutor};

/// 连接池插入执行器
pub struct PooledInsertExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    _marker: PhantomData<I>,
}

impl<'a, I: crate::model::Insertable> PooledInsertExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        // 直接调用 PooledConnection 的 insert_impl 方法
        let refs = self.models.as_refs();
        match &self.pooled_conn.connection {
            #[cfg(feature = "sqlite")]
            Some(ConnectionWrapper::Sqlite(db)) => db.insert_impl::<I::Model>(&refs).await,
            #[cfg(feature = "postgresql")]
            Some(ConnectionWrapper::PostgreSQL(db)) => db.insert_impl::<I::Model>(&refs).await,
            #[cfg(feature = "mysql")]
            Some(ConnectionWrapper::MySQL(db)) => db.insert_impl::<I::Model>(&refs).await,
            #[cfg(feature = "mssql")]
            Some(ConnectionWrapper::MSSQL(db)) => db.insert_impl::<I::Model>(&refs).await,
            None => Err(anyhow::anyhow!("No connection available")),
        }
    }
}

/// 连接池插入或更新执行器
pub struct PooledInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    _marker: PhantomData<I>,
}

impl<'a, I: crate::model::Insertable> PooledInsertOrUpdateExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        // 直接调用 PooledConnection 的 insert_or_update_batch 方法
        let refs = self.models.as_refs();
        match &self.pooled_conn.connection {
            #[cfg(feature = "sqlite")]
            Some(ConnectionWrapper::Sqlite(db)) => {
                db.insert_or_update_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "postgresql")]
            Some(ConnectionWrapper::PostgreSQL(db)) => {
                db.insert_or_update_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "mysql")]
            Some(ConnectionWrapper::MySQL(db)) => {
                db.insert_or_update_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "mssql")]
            Some(ConnectionWrapper::MSSQL(db)) => db.insert_or_update_impl::<I::Model>(&refs).await,
            None => Err(anyhow::anyhow!("No connection available")),
        }
    }
}

/// 连接池插入或忽略执行器
pub struct PooledInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    pooled_conn: &'a PooledConnection<'a>,
    models: I,
    _marker: PhantomData<I>,
}

impl<'a, I: crate::model::Insertable> PooledInsertOrIgnoreExecutor<'a, I> {
    pub async fn execute(self) -> anyhow::Result<()> {
        // 直接调用 PooledConnection 的 insert_or_ignore_batch 方法
        let refs = self.models.as_refs();
        match &self.pooled_conn.connection {
            #[cfg(feature = "sqlite")]
            Some(ConnectionWrapper::Sqlite(db)) => {
                db.insert_or_ignore_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "postgresql")]
            Some(ConnectionWrapper::PostgreSQL(db)) => {
                db.insert_or_ignore_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "mysql")]
            Some(ConnectionWrapper::MySQL(db)) => {
                db.insert_or_ignore_batch::<I::Model>(&refs).await
            }
            #[cfg(feature = "mssql")]
            Some(ConnectionWrapper::MSSQL(db)) => db
                .insert_or_ignore_impl::<I::Model>(&refs)
                .await
                .map(|_| ()),
            None => Err(anyhow::anyhow!("No connection available")),
        }
    }
}

// 根据启用的 feature 导入后端实现
#[cfg(feature = "sqlite")]
use super::super::sqlite_backend;

#[cfg(feature = "postgresql")]
use super::super::postgresql_backend;

#[cfg(feature = "mysql")]
use super::super::mysql_backend;

#[cfg(feature = "mssql")]
use super::super::mssql_backend;

/// 连接包装器 - 包装各后端的 Database 实例
enum ConnectionWrapper {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_backend::Database),
    #[cfg(feature = "postgresql")]
    PostgreSQL(postgresql_backend::Database),
    #[cfg(feature = "mysql")]
    MySQL(mysql_backend::Database),
    #[cfg(feature = "mssql")]
    MSSQL(mssql_backend::Database),
}

impl ConnectionWrapper {
    /// 检查连接是否有效
    async fn is_valid(&self) -> bool {
        match self {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.is_valid().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.is_valid().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.is_valid().await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.is_valid(),
        }
    }
}

/// 手工连接池核心结构
///
/// 注意：对于 SQLite 后端，由于其嵌入式特性不支持多线程共享连接，
/// 建议设置 max_size=1。如需并发支持，可考虑启用 MVCC 模式。
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
    async fn create_connection(&self) -> anyhow::Result<ConnectionWrapper> {
        match self.db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => {
                let db = sqlite_backend::Database::connect(self.db_type, &self.connection_string)
                    .await?;
                Ok(ConnectionWrapper::Sqlite(db))
            }
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let db =
                    mssql_backend::Database::connect(self.db_type, &self.connection_string).await?;
                Ok(ConnectionWrapper::MSSQL(db))
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    /// 获取连接(异步)
    async fn get(&self) -> anyhow::Result<ConnectionWrapper> {
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
            // 连接失效，减少计数
            self.total_connections.fetch_sub(1, Ordering::SeqCst);
            // 连接失效时不放入空闲队列，会自动被丢弃
        }
        // 释放信号量 permit，允许新的连接请求
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
    pub async fn build(self) -> anyhow::Result<ConnectionPool> {
        // 注意：SQLite 后端建议设置 max_size=1，因为其嵌入式特性不支持多线程共享连接
        // 如需并发支持，可考虑启用 MVCC 模式（PRAGMA journal_mode = 'mvcc'）

        match self.db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::Sqlite(pool))
            }
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                let manager =
                    PostgresConnectionManager::new_from_stringlike(&self.connection_string, NoTls)?;
                let mut builder = bb8::Pool::builder();
                builder = builder.max_size(self.config.max_size as u32);
                if self.config.min_size > 0 {
                    builder = builder.min_idle(Some(self.config.min_size as u32));
                }
                let pool = builder.build(manager).await?;
                Ok(ConnectionPool::PostgreSQL(pool))
            }
            #[cfg(feature = "mysql")]
            DbType::MySQL => {
                let opts = mysql_async::Opts::from_url(&self.connection_string)?;
                let pool = mysql_async::Pool::new(opts);
                Ok(ConnectionPool::MySQL(pool))
            }
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let pool =
                    ManualPool::new(self.db_type, self.connection_string, self.config.clone());
                if self.config.min_size > 0 {
                    pool.maintain_min_connections().await;
                }
                Ok(ConnectionPool::MSSQL(pool))
            }
        }
    }
}

/// 统一的连接池枚举
pub enum ConnectionPool {
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL(bb8::Pool<PostgresConnectionManager<NoTls>>),
    #[cfg(feature = "mysql")]
    MySQL(mysql_async::Pool),
    #[cfg(feature = "mssql")]
    MSSQL(Arc<ManualPool>),
}

impl ConnectionPool {
    /// 从连接池异步获取连接
    ///
    /// 此方法会等待直到有可用连接或创建新连接
    /// 如果池中没有连接且未达到 max_size,会自动创建新连接
    pub async fn get(&self) -> anyhow::Result<PooledConnection<'_>> {
        match self {
            #[cfg(feature = "sqlite")]
            ConnectionPool::Sqlite(pool) => {
                // 获取信号量 permit
                let _permit = pool.semaphore.acquire().await?;
                let conn = pool.get().await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::Sqlite(pool.clone()),
                    connection: Some(conn),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "postgresql")]
            ConnectionPool::PostgreSQL(pool) => {
                let pooled = pool.get().await?;
                let db = postgresql_backend::Database::from_pooled_connection(pooled);
                Ok(PooledConnection {
                    inner: PooledConnectionInner::PostgreSQL,
                    connection: Some(ConnectionWrapper::PostgreSQL(db)),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mysql")]
            ConnectionPool::MySQL(pool) => {
                let db = mysql_backend::Database::from_pool(pool.clone());
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MySQL,
                    connection: Some(ConnectionWrapper::MySQL(db)),
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "mssql")]
            ConnectionPool::MSSQL(pool) => {
                let _permit = pool.semaphore.acquire().await?;
                let conn = pool.get().await?;
                Ok(PooledConnection {
                    inner: PooledConnectionInner::MSSQL(pool.clone()),
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
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<ManualPool>),
    #[cfg(feature = "postgresql")]
    PostgreSQL,
    #[cfg(feature = "mysql")]
    MySQL,
    #[cfg(feature = "mssql")]
    MSSQL(Arc<ManualPool>),
}

impl PooledConnectionInner {
    async fn return_connection(&self, conn: ConnectionWrapper) {
        match self {
            #[cfg(feature = "sqlite")]
            PooledConnectionInner::Sqlite(pool) => pool.return_connection(conn).await,
            #[cfg(feature = "postgresql")]
            PooledConnectionInner::PostgreSQL => {
                // bb8 自动管理连接生命周期，无需手动归还
                let _ = conn;
            }
            #[cfg(feature = "mysql")]
            PooledConnectionInner::MySQL => {
                // mysql_async::Pool 自动管理连接生命周期，无需手动归还
                let _ = conn;
            }
            #[cfg(feature = "mssql")]
            PooledConnectionInner::MSSQL(pool) => pool.return_connection(conn).await,
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
            // 尝试获取 tokio 运行时句柄
            // 如果成功，使用 spawn 异步归还连接
            // 如果失败（不在 tokio 运行时中），则阻塞执行
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    // 在 tokio 运行时中，异步归还连接
                    handle.spawn(async move {
                        inner.return_connection(conn).await;
                    });
                }
                Err(_) => {
                    // 不在 tokio 运行时中，这种情况不应该在正常使用中出现
                    // 记录警告信息，连接可能会被泄露
                    eprintln!(
                        "Warning: PooledConnection dropped outside tokio runtime, connection may be leaked"
                    );
                }
            }
        }
    }
}

impl<'a> PooledConnection<'a> {
    /// 获取底层连接的引用(内部使用)
    fn get_connection(&self) -> &ConnectionWrapper {
        self.connection.as_ref().expect("Connection already taken")
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: Model>(&self) -> CreateTableExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => CreateTableExecutor::Sqlite(db.create_table::<T>()),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                CreateTableExecutor::PostgreSQL(db.create_table::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => CreateTableExecutor::MySQL(db.create_table::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => CreateTableExecutor::MSSQL(db.create_table::<T>()),
        }
    }

    /// 验证表结构
    pub async fn validate_table<T: Model>(&self) -> anyhow::Result<()> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.validate_table::<T>().await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.validate_table::<T>().await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.validate_table::<T>().await,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> PooledInsertExecutor<'_, I> {
        PooledInsertExecutor {
            pooled_conn: self,
            models,
            _marker: PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> PooledInsertOrUpdateExecutor<'_, I> {
        PooledInsertOrUpdateExecutor {
            pooled_conn: self,
            models,
            _marker: PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> PooledInsertOrIgnoreExecutor<'_, I> {
        PooledInsertOrIgnoreExecutor {
            pooled_conn: self,
            models,
            _marker: PhantomData,
        }
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> super::unified::SelectExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::SelectExecutor::Sqlite(db.select::<T>())
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::SelectExecutor::PostgreSQL(db.select::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::SelectExecutor::MySQL(db.select::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::SelectExecutor::MSSQL(db.select::<T>()),
        }
    }

    /// 创建流式查询执行器
    pub fn stream<T: Model>(&self) -> super::unified::SelectStream<'_, T> {
        self.select::<T>().stream()
    }

    /// 创建 Delete 执行器
    pub fn delete<T: Model>(&self) -> super::unified::DeleteExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::DeleteExecutor::Sqlite(db.delete::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::DeleteExecutor::PostgreSQL(db.delete::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::DeleteExecutor::MySQL(db.delete::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::DeleteExecutor::MSSQL(db.delete::<T>()),
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: Model>(&self) -> super::unified::UpdateExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                super::unified::UpdateExecutor::Sqlite(db.update::<T>(), std::marker::PhantomData)
            }
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::UpdateExecutor::PostgreSQL(db.update::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => super::unified::UpdateExecutor::MySQL(db.update::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => super::unified::UpdateExecutor::MSSQL(db.update::<T>()),
        }
    }

    /// 创建 Related 查询执行器
    pub fn related<T: Model + 'static, R: Model>(
        &self,
    ) -> super::unified::RelatedSelectExecutor<'_, T, R> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => super::unified::RelatedSelectExecutor::Sqlite(
                db.related::<T, R>(),
                std::marker::PhantomData,
            ),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                super::unified::RelatedSelectExecutor::PostgreSQL(db.related::<T, R>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => {
                super::unified::RelatedSelectExecutor::MySQL(db.related::<T, R>())
            }
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                super::unified::RelatedSelectExecutor::MSSQL(db.related::<T, R>())
            }
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> anyhow::Result<super::unified::Transaction<'_>> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => {
                let txn = db.begin().await?;
                Ok(super::unified::Transaction::Sqlite(txn))
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
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => {
                let txn = db.begin().await?;
                Ok(super::unified::Transaction::MSSQL(txn))
            }
        }
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: Model>(&self) -> DropTableExecutor<'_, T> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => DropTableExecutor::Sqlite(db.drop_table::<T>()),
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => {
                DropTableExecutor::PostgreSQL(db.drop_table::<T>())
            }
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => DropTableExecutor::MySQL(db.drop_table::<T>()),
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => DropTableExecutor::MSSQL(db.drop_table::<T>()),
        }
    }

    /// 执行原生 SQL 查询并返回模型列表
    pub async fn execute<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.execute::<T>(sql).await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.execute::<T>(sql).await,
        }
    }

    /// 执行原生 SQL 查询并返回模型列表（向后兼容）
    #[deprecated(since = "0.1.0", note = "请使用 execute 方法")]
    pub async fn exec_table<T: Model>(&self, sql: &str) -> anyhow::Result<Vec<T>> {
        self.execute::<T>(sql).await
    }

    /// 执行原生非查询 SQL
    pub async fn exec_non_query(&self, sql: &str) -> anyhow::Result<u64> {
        match self.get_connection() {
            #[cfg(feature = "sqlite")]
            ConnectionWrapper::Sqlite(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "postgresql")]
            ConnectionWrapper::PostgreSQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mysql")]
            ConnectionWrapper::MySQL(db) => db.exec_non_query(sql).await,
            #[cfg(feature = "mssql")]
            ConnectionWrapper::MSSQL(db) => db.exec_non_query(sql).await,
        }
    }
}
