use std::sync::Arc;

/// 统一的流式查询连接持有者
///
/// 该枚举统一管理各后端的数据库连接，确保连接在流式查询结束后正确释放。
/// 使用 RAII 模式管理连接生命周期，避免连接泄漏。
pub enum StreamConnection<'a> {
    /// SQLite/Turso 连接 - 使用 Arc 共享所有权
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<turso::Connection>),

    /// PostgreSQL 连接 - 借用 Client 引用
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
                // 借用引用，不需要显式释放
                // 生命周期由借用检查器保证
            }

            #[cfg(feature = "mysql")]
            StreamConnection::MySQL(conn) => {
                // mysql_async::Conn 在 Drop 时会自动返回连接池
                // 显式 drop 确保立即释放
                drop(conn);
            }

            // 处理 Phantom 变体
            StreamConnection::__Phantom(_) => {}
        }
    }
}
