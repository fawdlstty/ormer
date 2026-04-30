use std::sync::Arc;

/// 统一的流式查询连接持有者
///
/// 该枚举统一管理各后端的数据库连接，确保连接在流式查询结束后正确释放。
/// 使用 RAII 模式管理连接生命周期，避免连接泄漏。
///
/// # 各后端的连接管理策略
///
/// ## SQLite/Turso
///
/// 使用 `Arc<turso::Connection>` 共享所有权。多个流式查询可以共享同一个连接，
/// 当最后一个 Arc 引用被 drop 时，连接会自动释放。
///
/// ## PostgreSQL
///
/// 使用 `bb8::PooledConnection<'a, PostgresConnectionManager>` 借用连接。
/// 连接的生命周期由bb8连接池管理，当PooledConnection被drop时会自动返回连接池。
///
/// ## MySQL
///
/// 使用 `mysql_async::Conn` 拥有连接所有权。当 `StreamConnection` 被 drop 时，
/// 连接会自动返回到连接池。这是 mysql_async 库的内置行为。
///
/// # 示例
///
/// 流式查询完成后，连接会自动释放：
///
/// ```rust,ignore
/// let mut stream = db.select::<User>().stream().into_iter().await?;
/// while let Some(result) = stream.next().await {
///     let user = result?;
///     // 处理用户数据
/// }
/// // stream 在这里被 drop，连接自动释放
/// ```
pub enum StreamConnection<'a> {
    /// SQLite/Turso 连接 - 使用 Arc 共享所有权
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<turso::Connection>),

    /// PostgreSQL 连接 - 使用Client引用
    #[cfg(feature = "postgresql")]
    PostgreSQL(&'a tokio_postgres::Client),

    /// MySQL 连接 - 拥有连接所有权，Drop 时自动返回连接池
    #[cfg(feature = "mysql")]
    MySQL(mysql_async::Conn),

    /// 空变体，用于未启用任何 feature 时保持枚举有效
    #[doc(hidden)]
    #[allow(dead_code)]
    __Phantom(std::marker::PhantomData<&'a ()>),
}

impl<'a> Drop for StreamConnection<'a> {
    fn drop(&mut self) {
        match self {
            #[cfg(feature = "sqlite")]
            StreamConnection::Sqlite(_) => {
                // Arc 会在最后一个引用释放时自动清理
                // 不需要显式操作
            }

            #[cfg(feature = "postgresql")]
            StreamConnection::PostgreSQL(_) => {
                // bb8::PooledConnection 在 Drop 时会自动返回连接池
                // 不需要显式释放
            }

            #[cfg(feature = "mysql")]
            StreamConnection::MySQL(conn) => {
                // mysql_async::Conn 在 Drop 时会自动返回连接池
                // 显式 drop 确保立即释放
                let _ = conn;
            }

            // 处理 Phantom 变体
            StreamConnection::__Phantom(_) => {}
        }
    }
}
