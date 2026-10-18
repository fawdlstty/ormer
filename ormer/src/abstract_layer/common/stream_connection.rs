use std::marker::PhantomData;
#[cfg(feature = "duckdb")]
use std::sync::Arc as DuckDbArc;

/// 统一的流式查询连接持有者
///
/// 该枚举统一管理各后端的数据库连接，确保连接在流式查询结束后正确释放。
/// 使用 RAII 模式管理连接生命周期，避免连接泄漏。
///
/// # 各后端的连接管理策略
///
/// ## SQLite/Turso
///
/// 使用 [`QueryConn`](super::super::sqlite_backend::QueryConn) 持有连接上下文
/// （排队共享或事务独占两态），当最后一个引用被 drop 时，连接会自动释放。
///
/// ## DuckDB
///
/// 使用 `Arc<duckcompat::Connection>` 共享所有权，语义同 SQLite；
/// 查询结果由后端在进入异步迭代器之前物化。
///
/// ## PostgreSQL
///
/// 使用 `&tokio_postgres::Client` 借用连接，真正的 `RowStream` 逐行拉取，
/// 连接生命周期由调用方（连接池/Database）管理。
///
/// ## MySQL/MSSQL
///
/// 不经此类型：MySQL 流式路径直接通过 `pool.get_conn()` 持有连接，
/// MSSQL 的 `SelectStream` 持有后端 `SelectExecutor`（结果物化语义，
/// 见 `mssql_backend::SelectStream` 文档）。
///
/// # 示例
///
/// 流式查询完成后，连接会自动释放：
///
/// ```text
/// let mut stream = db.select::<User>().stream().into_iter().await?;
/// while let Some(result) = stream.next().await {
///     let user = result?;
///     // 处理用户数据
/// }
/// // stream 在这里被 drop，连接自动释放
/// ```
pub enum StreamConnection<'a> {
    /// SQLite/Turso 连接上下文（排队路径 =
    /// [`QueryConn::Shared`](super::super::sqlite_backend::QueryConn)，事务
    /// 独占路径 = 同枚举 `Txn` 变体；流式逐行在 Shared 态经
    /// `SharedConn::next_row` 行级锁排队，Txn 态直用 raw 连接）
    #[cfg(feature = "sqlite")]
    Sqlite(super::super::sqlite_backend::QueryConn),

    /// DuckDB connection. Queries are materialized by the backend before
    /// entering the async iterator.
    #[cfg(feature = "duckdb")]
    DuckDB(DuckDbArc<super::super::duckdb_backend::duckcompat::Connection>),

    /// PostgreSQL 连接 - 使用Client引用
    #[cfg(feature = "postgresql")]
    PostgreSQL(&'a tokio_postgres::Client),

    #[doc(hidden)]
    __Lifetime(PhantomData<&'a ()>),
}

impl<'a> StreamConnection<'a> {
    /// 取 SQLite/Turso 连接；变体不匹配说明流式连接被错误地跨后端传递，
    /// 返回错误而非 panic。
    #[cfg(feature = "sqlite")]
    pub fn expect_sqlite(&self) -> crate::Result<&super::super::sqlite_backend::QueryConn> {
        match self {
            StreamConnection::Sqlite(conn) => Ok(conn),
            _ => Err(crate::ormer_error!(
                "expected a SQLite StreamConnection, got {:?}",
                self.db_kind()
            )),
        }
    }

    /// 取 PostgreSQL 连接；变体不匹配返回错误而非 panic。
    #[cfg(feature = "postgresql")]
    pub fn expect_postgresql(&self) -> crate::Result<&&'a tokio_postgres::Client> {
        match self {
            StreamConnection::PostgreSQL(client) => Ok(client),
            _ => Err(crate::ormer_error!(
                "expected a PostgreSQL StreamConnection, got {:?}",
                self.db_kind()
            )),
        }
    }

    /// 取 DuckDB 连接；变体不匹配返回错误而非 panic。
    #[cfg(feature = "duckdb")]
    pub fn expect_duckdb(
        &self,
    ) -> crate::Result<&DuckDbArc<super::super::duckdb_backend::duckcompat::Connection>> {
        match self {
            StreamConnection::DuckDB(conn) => Ok(conn),
            _ => Err(crate::ormer_error!(
                "expected a DuckDB StreamConnection, got {:?}",
                self.db_kind()
            )),
        }
    }

    #[cfg_attr(
        not(any(feature = "postgresql", feature = "duckdb", feature = "sqlite")),
        allow(dead_code)
    )]
    fn db_kind(&self) -> &'static str {
        match self {
            #[cfg(feature = "sqlite")]
            StreamConnection::Sqlite(_) => "sqlite",
            #[cfg(feature = "duckdb")]
            StreamConnection::DuckDB(_) => "duckdb",
            #[cfg(feature = "postgresql")]
            StreamConnection::PostgreSQL(_) => "postgresql",
            StreamConnection::__Lifetime(_) => "(lifetime placeholder)",
        }
    }
}
