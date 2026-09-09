use super::DbType;

/// Public capability boundary for database backends.
///
/// 统一层在分派处消费该矩阵，`Capabilities::of(db_type)` 是唯一取矩阵入口，
/// 各消费点通过 [`Capabilities::ensure`] 做“矩阵优先”的前置校验并返回
/// `UnsupportedFeature`。QuestDB 复用 `Database::PostgreSQL` 连接，所有消费点
/// 都必须按运行时 `db_type`（`Database::db_type()` / 后端持有的 `db_type`）判定，
/// 不能按 `Database` 枚举变体判定。新增能力项时需同步消费点，避免声明与实现漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// 是否支持显式事务（BEGIN/COMMIT/ROLLBACK）。
    ///
    /// 消费点：`Database::begin()`；PostgreSQL 后端 `Database::begin()` 按运行时
    /// `db_type` 对 QuestDB 连接做同一判定。
    pub transactions: bool,
    /// 是否支持自增主键列。
    ///
    /// 消费点：QuestDB 建表 SQL 生成（`generate_questdb_create_table_sql_with_name`）
    /// 与 PostgreSQL 后端插入路径（QuestDB 连接拒绝自增列 INSERT/RETURNING）。
    /// ClickHouse 建表把自增列按普通列映射、InfluxDB 在 Line Protocol 写入前由
    /// `validate_influx_model` 拒绝，两处语义各异，仍由各后端自行处理。
    pub auto_increment: bool,
    /// 是否支持 DML `RETURNING`（或等价的 MSSQL `OUTPUT inserted./deleted.`）。
    ///
    /// 消费点：统一层 `InsertExecutor::returning` / `DeleteExecutor::returning` /
    /// `UpdateExecutor::returning`，以及 PostgreSQL 后端 `InsertExecutor::returning`
    /// （QuestDB 连接）。
    pub dml_returning: bool,
    /// 是否支持插入冲突处理（`insert_or_update` / `upsert` / `on_conflict`）。
    ///
    /// 消费点：统一层 `Database::insert_or_update`、连接池同名入口、
    /// `common_helpers::append_insert_conflict_clause`，以及 PostgreSQL 后端
    /// `InsertExecutor::to_sql` / `InsertOrUpdateExecutor::to_sql`（QuestDB 连接）。
    /// MSSQL 为 true（`insert_or_update` 走 MERGE），但可配置 conflict 子句的细粒度
    /// 限制仍由 `append_insert_conflict_clause` 单独拒绝。
    pub insert_conflict: bool,
    /// 是否支持插入忽略（`insert_or_ignore`）。
    ///
    /// 消费点：统一层 `Database::insert_or_ignore`、连接池同名入口，以及
    /// PostgreSQL 后端 `InsertOrIgnoreExecutor::to_sql`（QuestDB 连接）。
    pub insert_ignore: bool,
    /// 是否支持行级 `DELETE`。
    ///
    /// 消费点：统一层 `Database::delete`、连接池 `PooledConnection::delete`，以及
    /// PostgreSQL 后端 `DeleteExecutor::to_sql`（QuestDB 连接）。
    pub row_delete: bool,
    /// 是否支持按块删除（`delete_blocks`）。
    ///
    /// 当前所有后端均为 true：PostgreSQL/TimescaleDB `drop_chunks`、QuestDB/ClickHouse
    /// `DROP PARTITION`、InfluxDB HTTP delete API，其余 OLTP 后端回退为对齐边界的
    /// 行删除。统一层 `Database::delete_blocks` 不做矩阵拒绝，该字段保留为未来新增
    /// 后端的统一门控入口。
    pub block_delete: bool,
    /// 是否提供 `COPY ... FROM STDIN` 批量导入通道。
    ///
    /// 消费点：PostgreSQL 后端 `InsertExecutor::execute` 的 `use_copy` 判定
    /// （QuestDB 连接不走 COPY）。DuckDB 的 COPY INTO 尚未实现，保持 false。
    pub copy: bool,
    /// 是否支持行锁（`FOR UPDATE` / `FOR SHARE` / `WITH (UPDLOCK)`）。
    ///
    /// 消费点：`query::builder::validate_row_lock`（粗粒度门控）；MSSQL 的 `NOWAIT`
    /// 细粒度限制在矩阵之后单独校验。
    pub row_lock: bool,
    /// 是否在建表时生成 PRIMARY KEY/UNIQUE/CHECK/FOREIGN KEY 约束。
    ///
    /// 消费点：`model::generate_create_table_sql_with_engine`。QuestDB 由专用建表
    /// 函数生成无约束 DDL，与此声明一致。
    pub constraints: bool,
    /// 是否支持通过系统表校验表结构（`validate_table`）。
    ///
    /// 消费点：统一层 `Database::validate_table`、连接池 `PooledConnection::validate_table`。
    /// QuestDB 走 `table_columns()` 专用校验路径，因此为 true。该字段不覆盖 db-first
    /// 实体生成（`generate_entities`）：ClickHouse 可用、QuestDB/InfluxDB 拒绝，
    /// 该维度由各自路径单独硬编码。
    pub schema_introspection: bool,
    /// 是否支持高级分组（GROUP BY 聚合投影、HAVING、GROUPING SETS/CUBE/
    /// ROLLUP）。该标志当前仅驱动统一层 ClickHouse/InfluxDB 的
    /// GroupedSelectExecutor 分支；MySQL 虽然 GROUP BY/HAVING/ROLLUP 可用，
    /// 但 GROUPING SETS/CUBE 由 `validate_grouping_clause` 按子句精确拒绝，
    /// 故此处保持 false 不影响普通聚合查询。
    pub advanced_grouping: bool,
    /// 是否提供 `truncate_table` 执行器（当前为 PostgreSQL/QuestDB）。
    ///
    /// 消费点：统一层 `Database::truncate_table`。QuestDB 走 `Database::PostgreSQL`
    /// 分支，`TruncateTableExecutor` 携带运行时 `db_type`。
    pub truncate: bool,
}

impl Capabilities {
    pub const fn of(db_type: DbType) -> Self {
        match db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => Self {
                copy: false,
                row_lock: false,
                advanced_grouping: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => Self {
                truncate: true,
                ..Self::full_oltp()
            },
            #[cfg(feature = "questdb")]
            DbType::QuestDB => Self {
                // QuestDB 无事务：统一层 Database::begin() 依此声明直接报错，
                // 而不是透传 PostgreSQL 协议的伪事务。
                transactions: false,
                auto_increment: false,
                dml_returning: false,
                insert_conflict: false,
                insert_ignore: false,
                row_delete: false,
                block_delete: true,
                copy: false,
                row_lock: false,
                constraints: false,
                // validate_table 走 table_columns() 专用路径；db-first 实体生成
                // 不在本字段覆盖范围内（postgresql_backend::db_first_tables 单独拒绝）。
                schema_introspection: true,
                advanced_grouping: false,
                truncate: true,
            },
            #[cfg(feature = "mysql")]
            DbType::MySQL => Self {
                dml_returning: false,
                copy: false,
                advanced_grouping: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "mssql")]
            DbType::MSSQL => Self {
                // MSSQL 通过 OUTPUT inserted./deleted. 实现 insert/update/delete
                // returning（mssql_backend::*Executor::returning 均可用），此前
                // 声明为 false 与实现矛盾，以实现为准修正。
                dml_returning: true,
                copy: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => Self {
                // COPY INTO 批量导入尚未实现，待 duckdb_backend 接入后再放开。
                copy: false,
                row_lock: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => Self {
                transactions: false,
                auto_increment: false,
                dml_returning: false,
                insert_conflict: false,
                insert_ignore: false,
                row_delete: false,
                block_delete: true,
                row_lock: false,
                constraints: false,
                schema_introspection: false,
                // ClickHouse 支持 GROUP BY/HAVING/GROUPING SETS/CUBE/ROLLUP，
                // 统一层的分组聚合执行分支据此放行。
                advanced_grouping: true,
                ..Self::full_oltp()
            },
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => Self {
                transactions: false,
                auto_increment: false,
                dml_returning: false,
                insert_conflict: false,
                insert_ignore: false,
                row_delete: false,
                block_delete: true,
                copy: false,
                row_lock: false,
                constraints: false,
                schema_introspection: false,
                advanced_grouping: false,
                truncate: false,
            },
        }
    }

    /// 矩阵优先的能力门控：`supported` 对 `db_type` 取 false 时返回
    /// `UnsupportedFeature { backend: db_type, feature }`。
    ///
    /// 统一层 / 公共 helper / 后端的分派前置校验统一经由此处构造错误，
    /// 保证错误结构集中维护。
    pub fn ensure(
        db_type: DbType,
        supported: impl FnOnce(Self) -> bool,
        feature: &'static str,
    ) -> crate::Result<()> {
        if supported(Self::of(db_type)) {
            Ok(())
        } else {
            Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature,
            })
        }
    }

    const fn full_oltp() -> Self {
        Self {
            transactions: true,
            auto_increment: true,
            dml_returning: true,
            insert_conflict: true,
            insert_ignore: true,
            row_delete: true,
            block_delete: true,
            copy: true,
            row_lock: true,
            constraints: true,
            schema_introspection: true,
            advanced_grouping: true,
            truncate: false,
        }
    }
}
