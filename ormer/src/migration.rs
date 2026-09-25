//! Versioned schema migration primitives.
//!
//! The migration API deliberately keeps migration steps structured until the
//! final dialect rendering stage.  Applications can therefore inspect a plan
//! before executing it, while small hand-written migrations remain possible.

use crate::abstract_layer::DbType;
#[cfg(any(feature = "sqlite", feature = "duckdb"))]
use crate::abstract_layer::common::common_helpers;
use crate::abstract_layer::common::{Database, Transaction};
use crate::db_first::{DbFirstForeignKey, DbFirstTable};
#[cfg(any(feature = "postgresql", feature = "mysql"))]
use crate::model::CompressionAlgorithm;
#[cfg(feature = "postgresql")]
use crate::model::DurationToInterval;
use crate::model::{ColumnSchema, WritableModel};
use crate::table_migrate::ExpectedCheck;
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

pub const MIGRATION_TABLE_NAME: &str = "__ormer_migrations";

/// [`Database::ensure_table`] 的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableEnsureOutcome {
    /// 表原本不存在并已创建，或已存在且结构校验通过。
    Ready,
    /// 存在结构差异，已通过增量迁移对齐。
    Migrated,
    /// 存在无法增量迁移的结构差异（如主键变更、超表分区约束冲突），
    /// 已按约定删除表并按当前模型重建。
    ///
    /// 仅由 [`Database::ensure_table_permissive`] 返回：默认的
    /// [`Database::ensure_table`] 会拒绝删表重建并直接返回错误。
    Recreated,
}

/// 判断错误是否属于"应当删除重建"的结构性差异。
///
/// 包括：ormer 自身报告的不可迁移 schema / 结构不匹配，以及 TimescaleDB
/// 在超表分区列与唯一索引（主键）冲突时抛出的建表错误。后者的典型场景是
/// 旧版本建出的表结构与当前模型的主键定义不一致，导致 create_hypertable
/// 无法完成，此时删除重建是唯一出路。
pub(crate) fn is_schema_rebuild_error(err: &crate::OrmerError) -> bool {
    match err {
        crate::OrmerError::UnmigratableSchema { .. } => true,
        crate::OrmerError::Other { message } => message.contains("Schema mismatch"),
        crate::OrmerError::Database { message, .. } => {
            message.contains("cannot create a unique index without the column")
                && message.contains("used in partitioning")
        }
        _ => false,
    }
}

/// 路由子表的 `LIKE` 前缀匹配模式：`{base}_%`。基础表名中的 `LIKE` 通配符
/// （`%`、`_`）与转义符本身按 PostgreSQL `LIKE` 默认转义规则（反斜杠）转义，
/// 保证前缀按字面匹配，不会把 `aaaxval` 之类同前缀表误判为子表。
#[cfg(feature = "postgresql")]
fn routed_child_table_like_pattern(base_name: &str) -> String {
    let mut pattern = String::with_capacity(base_name.len() + 8);
    for character in base_name.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push_str("\\_%");
    pattern
}

/// 组装路由子表的 `pg_class` 前缀查询：枚举 `schema` 下形如
/// `{base}_<路由值>` 的已存在子表，排除基础表名本身。schema 处理与既有
/// 表存在性校验（`check_table_exists` 等）一致：模型表名可带 schema 前缀，
/// 无前缀时按 `public` 处理。字面量双写单引号做防御性转义（表名与模式
/// 均来自模型常量，正常不含引号）。路由值无法枚举，因此只列出已存在的
/// 子表，不预建。
#[cfg(feature = "postgresql")]
fn routed_child_tables_sql(schema: &str, pattern: &str, base_name: &str) -> String {
    fn quote_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    format!(
        "SELECT c.relname \
         FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = {} \
           AND c.relname LIKE {} \
           AND c.relname <> {} \
           AND c.relkind IN ('r', 'p') \
         ORDER BY c.relname",
        quote_literal(schema),
        quote_literal(pattern),
        quote_literal(base_name),
    )
}

/// A single migration operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationStep {
    CreateType {
        name: String,
        definition: String,
    },
    AlterType {
        name: String,
        definition: String,
    },
    CreateTable {
        table: String,
        definition: String,
    },
    AddColumn {
        table: String,
        column: String,
        definition: String,
    },
    DropColumn {
        table: String,
        column: String,
    },
    RenameColumn {
        table: String,
        old_name: String,
        new_name: String,
    },
    BackfillColumn {
        table: String,
        column: String,
        expression: String,
    },
    AlterColumn {
        table: String,
        column: String,
        definition: String,
        using: Option<String>,
    },
    AddConstraint {
        table: String,
        definition: String,
    },
    /// 主键原地变更（仅 PostgreSQL 生成）：`DROP CONSTRAINT` 旧主键约束 +
    /// `ADD PRIMARY KEY` 新主键列，保数据。其他后端的 sql() 渲染返回错误
    /// ——不支持的后端在计划阶段即转 `NeedsRebuild`，不会生成该步骤。
    ChangePrimaryKey {
        table: String,
        columns: Vec<String>,
        /// 现有主键约束名（自省所得）；None 时不生成 DROP 子动作。
        drop_constraint: Option<String>,
    },
    CreateIndex {
        name: String,
        table: String,
        columns: Vec<String>,
        unique: bool,
    },
    DropIndex {
        name: String,
        table: String,
    },
    AddForeignKey {
        table: String,
        column: String,
        ref_table: String,
        ref_column: String,
    },
    Sql {
        sql: String,
    },
}

impl MigrationStep {
    pub fn sql(&self, db_type: DbType) -> crate::Result<String> {
        let table = crate::model::quote_qualified_identifier(db_type, table_name(self));

        match self {
            Self::CreateType { definition, .. } | Self::AlterType { definition, .. } => {
                Ok(definition.clone())
            }
            Self::CreateTable { table, definition } => {
                // schema 限定表的建表步骤与 CreateTableExecutor 同源：
                // 目标 schema 缺失时由幂等 CREATE SCHEMA 前置语句补齐
                // （仅 PG 有该语义，函数内部对其余后端返回 None）。
                #[cfg(feature = "postgresql")]
                {
                    if let Some(create_schema_sql) = crate::abstract_layer::postgresql_backend::create_schema_sql_if_qualified(db_type, table)
                    {
                        return Ok(format!("{create_schema_sql}; {definition}"));
                    }
                }
                #[cfg(not(feature = "postgresql"))]
                {
                    let _ = table;
                }
                Ok(definition.clone())
            }
            Self::AddColumn {
                column, definition, ..
            } => {
                let column = crate::model::quote_identifier(db_type, column);
                Ok(format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {definition}"
                ))
            }
            Self::DropColumn { column, .. } => {
                let column = crate::model::quote_identifier(db_type, column);
                match db_type {
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => Err(crate::ormer_error!(
                        "SQLite does not support DROP COLUMN; use a hand-written table rebuild"
                    )),
                    // 仅启用部分 feature 时通配分支可能不可达
                    #[allow(unreachable_patterns)]
                    _ => Ok(format!("ALTER TABLE {table} DROP COLUMN {column}")),
                }
            }
            Self::RenameColumn {
                old_name,
                new_name,
                ..
            } => {
                let old_name = crate::model::quote_identifier(db_type, old_name);
                let new_name = crate::model::quote_identifier(db_type, new_name);
                match db_type {
                    // MSSQL 无 ALTER TABLE RENAME COLUMN 语法，使用 sp_rename 过程
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => Ok(format!(
                        "EXEC sp_rename N'{table}.{old_name}', N'{new_name}', N'COLUMN'"
                    )),
                    #[cfg(feature = "influxdb")]
                    DbType::InfluxDB => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "RENAME COLUMN migrations",
                    }),
                    // PostgreSQL / QuestDB / MySQL 8+ / SQLite 3.25+ / DuckDB / ClickHouse
                    // 均支持 `ALTER TABLE t RENAME COLUMN old TO new`
                    #[allow(unreachable_patterns)]
                    _ => Ok(format!(
                        "ALTER TABLE {table} RENAME COLUMN {old_name} TO {new_name}"
                    )),
                }
            }
            Self::BackfillColumn {
                column, expression, ..
            } => {
                let column = crate::model::quote_identifier(db_type, column);
                Ok(format!("UPDATE {table} SET {column} = {expression}"))
            }
            Self::AlterColumn {
                column,
                definition,
                using,
                ..
            } => {
                let _ = (definition, using);
                let column = crate::model::quote_identifier(db_type, column);
                let _ = column;
                match db_type {
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => Err(crate::ormer_error!(
                        "SQLite does not support ALTER COLUMN; use a hand-written table rebuild"
                    )),
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        let using = using
                            .as_deref()
                            .map(|expression| format!(" USING {expression}"))
                            .unwrap_or_default();
                        Ok(format!(
                            "ALTER TABLE {table} ALTER COLUMN {column} {definition}{using}"
                        ))
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => Ok(format!(
                        "ALTER TABLE {table} MODIFY COLUMN {column} {definition}"
                    )),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => Ok(format!(
                        "ALTER TABLE {table} ALTER COLUMN {column} {definition}"
                    )),
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => {
                        if using.is_some() {
                            return Err(crate::ormer_error!(
                                "DuckDB does not support USING expressions in ALTER COLUMN"
                            ));
                        }
                        Ok(format!(
                            "ALTER TABLE {table} ALTER COLUMN {column} {definition}"
                        ))
                    }
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => {
                        if using.is_some() {
                            return Err(crate::ormer_error!(
                                "ClickHouse does not support USING expressions in ALTER COLUMN"
                            ));
                        }
                        Ok(format!(
                            "ALTER TABLE {table} MODIFY COLUMN {column} {definition}"
                        ))
                    }
                    #[cfg(feature = "questdb")]
                    DbType::QuestDB => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "ALTER COLUMN migrations (QuestDB cannot change column types; rebuild the table via a new table + INSERT SELECT in a hand-written migration)",
                    }),
                    #[cfg(feature = "influxdb")]
                    DbType::InfluxDB => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "ALTER COLUMN migrations",
                    }),
                }
            }
            Self::AddConstraint { definition, .. } => {
                Ok(format!("ALTER TABLE {table} ADD {definition}"))
            }
            Self::ChangePrimaryKey {
                columns: pk_columns,
                drop_constraint,
                ..
            } => {
                #[cfg(feature = "postgresql")]
                {
                    let columns_sql = pk_columns
                        .iter()
                        .map(|column| crate::model::quote_identifier(db_type, column))
                        .collect::<Vec<_>>()
                        .join(", ");
                    // 仅启用 postgresql 时 DbType 只剩 PG 变体，通配臂不可达
                    #[allow(unreachable_patterns)]
                    match db_type {
                        DbType::PostgreSQL => {
                            // 单条 ALTER 的多个子动作原子生效，DROP 与 ADD 之间
                            // 无窗口期；ADD 不带约束名，由 PG 按默认规则命名
                            // （{table}_pkey），与命名唯一化原则一致。
                            let drop_action = drop_constraint
                                .as_deref()
                                .map(|constraint| {
                                    format!(
                                        "DROP CONSTRAINT {}, ",
                                        crate::model::quote_identifier(db_type, constraint)
                                    )
                                })
                                .unwrap_or_default();
                            Ok(format!(
                                "ALTER TABLE {table} {drop_action}ADD PRIMARY KEY ({columns_sql})"
                            ))
                        }
                        _ => Err(crate::OrmerError::UnsupportedFeature {
                            backend: db_type,
                            feature: "in-place primary key change (ChangePrimaryKey is \
                                      PostgreSQL-only; other backends surface NeedsRebuild)",
                        }),
                    }
                }
                #[cfg(not(feature = "postgresql"))]
                {
                    let _ = (pk_columns, drop_constraint);
                    Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "in-place primary key change (ChangePrimaryKey is \
                                  PostgreSQL-only; other backends surface NeedsRebuild)",
                    })
                }
            }
            Self::CreateIndex {
                name,
                columns: index_columns,
                unique,
                ..
            } => {
                let columns_sql = index_columns
                    .iter()
                    .map(|column| crate::model::quote_identifier(db_type, column))
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(crate::model::render_create_index(
                    db_type,
                    name,
                    table_name(self),
                    &columns_sql,
                    *unique,
                ))
            }
            Self::DropIndex { name, .. } => {
                let name = crate::model::quote_identifier(db_type, name);
                match db_type {
                    // QuestDB 索引内联在建表语句中，无法单独删除；
                    // ClickHouse 数据跳数索引需 ALTER TABLE ... DROP INDEX；
                    // InfluxDB 无 DDL 索引概念
                    #[cfg(feature = "questdb")]
                    DbType::QuestDB => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "DROP INDEX migrations (QuestDB indexes are inline table definitions)",
                    }),
                    #[cfg(feature = "clickhouse")]
                    DbType::ClickHouse => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "DROP INDEX migrations (write the step as Sql { .. } with ALTER TABLE ... DROP INDEX)",
                    }),
                    #[cfg(feature = "influxdb")]
                    DbType::InfluxDB => Err(crate::OrmerError::UnsupportedFeature {
                        backend: db_type,
                        feature: "DROP INDEX migrations",
                    }),
                    // MySQL / MSSQL 要求 DROP INDEX 显式指定所属表
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => Ok(format!("DROP INDEX {name} ON {table}")),
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => Ok(format!("DROP INDEX {name} ON {table}")),
                    // SQLite / PostgreSQL / DuckDB：索引名在库内唯一，支持 IF EXISTS。
                    // PG 的索引名只在 schema 内唯一，必须以表所在 schema 限定：
                    // 业务表多落在非默认 schema（search_path 不含它），裸索引名
                    // 会被 IF EXISTS 静默吞掉——删除没生效而计划视为已执行，
                    // 复验不收敛就会误触发删表重建。
                    #[allow(unreachable_patterns)]
                    _ => {
                        let index_name = match split_qualified_table_name(&table) {
                            (Some(schema), _) => format!(
                                "{}.{name}",
                                crate::model::quote_identifier(db_type, schema)
                            ),
                            (None, _) => name,
                        };
                        Ok(format!("DROP INDEX IF EXISTS {index_name}"))
                    }
                }
            }
            Self::AddForeignKey {
                column,
                ref_table,
                ref_column,
                ..
            } => {
                #[cfg(feature = "sqlite")]
                if matches!(db_type, DbType::Sqlite) {
                    return Err(crate::ormer_error!(
                        "SQLite does not support adding a foreign key after table creation"
                    ));
                }
                Ok(format!(
                    "ALTER TABLE {table} ADD FOREIGN KEY ({}) REFERENCES {} ({})",
                    crate::model::quote_identifier(db_type, column),
                    crate::model::quote_qualified_identifier(db_type, ref_table),
                    crate::model::quote_identifier(db_type, ref_column)
                ))
            }
            Self::Sql { sql } => Ok(sql.clone()),
        }
    }
}

pub(crate) fn table_name(step: &MigrationStep) -> &str {
    match step {
        MigrationStep::AddColumn { table, .. }
        | MigrationStep::DropColumn { table, .. }
        | MigrationStep::RenameColumn { table, .. }
        | MigrationStep::BackfillColumn { table, .. }
        | MigrationStep::AlterColumn { table, .. }
        | MigrationStep::AddConstraint { table, .. }
        | MigrationStep::ChangePrimaryKey { table, .. }
        | MigrationStep::CreateIndex { table, .. }
        | MigrationStep::DropIndex { table, .. }
        | MigrationStep::AddForeignKey { table, .. } => table,
        _ => "",
    }
}

/// A deterministic, inspectable migration plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    table_name: String,
    db_type: DbType,
    pub(crate) steps: Vec<MigrationStep>,
    pub(crate) warnings: Vec<String>,
}

impl MigrationPlan {
    pub fn new(table_name: impl Into<String>, db_type: DbType) -> Self {
        Self {
            table_name: table_name.into(),
            db_type,
            steps: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    pub fn db_type(&self) -> DbType {
        self.db_type
    }

    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn to_sql(&self) -> crate::Result<String> {
        let mut sql = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            sql.push(step.sql(self.db_type)?);
        }
        Ok(sql.join(";\n"))
    }

    pub(crate) fn push(&mut self, step: MigrationStep) {
        self.steps.push(step);
    }
}

/// A migration file/definition with a stable version identifier.
pub trait Migration: Send + Sync {
    fn version(&self) -> u64;
    fn name(&self) -> &str;
    fn up(&self) -> Vec<MigrationStep>;

    fn down(&self) -> Vec<MigrationStep> {
        Vec::new()
    }

    fn checksum(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in self
            .name()
            .bytes()
            .chain(format!("{:?}", self.up()).bytes())
            .chain(format!("{:?}", self.down()).bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }
}

impl<T: Migration + ?Sized> Migration for &T {
    fn version(&self) -> u64 {
        (**self).version()
    }

    fn name(&self) -> &str {
        (**self).name()
    }

    fn up(&self) -> Vec<MigrationStep> {
        (**self).up()
    }

    fn down(&self) -> Vec<MigrationStep> {
        (**self).down()
    }

    fn checksum(&self) -> u64 {
        (**self).checksum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationInfo {
    pub version: u64,
    pub name: String,
    pub checksum: u64,
}

/// One executable statement in a migration dry-run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDryRunStep {
    pub version: u64,
    pub migration_name: String,
    pub migration_index: usize,
    pub step_index: usize,
    pub sql: String,
}

/// An inspectable execution plan for pending versioned migrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDryRun {
    pub backend: DbType,
    pub transactional: bool,
    pub completed_versions: Vec<u64>,
    pub steps: Vec<MigrationDryRunStep>,
    pub warnings: Vec<String>,
}

/// The precise resume point after a migration was interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationExecutionStatus {
    pub backend: DbType,
    pub completed: Vec<MigrationInfo>,
    pub pending: Vec<MigrationInfo>,
    pub resume_version: Option<u64>,
    pub transactional: bool,
}

impl MigrationInfo {
    pub fn new(version: u64, name: impl Into<String>) -> Self {
        Self {
            version,
            name: name.into(),
            checksum: 0,
        }
    }

    fn with_checksum(version: u64, name: impl Into<String>, checksum: u64) -> Self {
        Self {
            version,
            name: name.into(),
            checksum,
        }
    }
}

/// A reusable migration runner over a statically owned migration list.
pub struct MigrationRunner<'a, M: Migration> {
    db: &'a Database,
    migrations: &'a [M],
}

impl<'a, M: Migration> MigrationRunner<'a, M> {
    pub fn new(db: &'a Database, migrations: &'a [M]) -> Self {
        Self { db, migrations }
    }

    pub async fn pending(&self) -> crate::Result<Vec<MigrationInfo>> {
        self.db.pending_migrations(self.migrations).await
    }

    pub async fn execute(&self) -> crate::Result<usize> {
        self.db.apply_migrations(self.migrations).await
    }

    /// Render every pending statement without executing it.
    ///
    /// For non-transactional backends each item is one statement, so a failed
    /// statement identifies the exact recovery point.
    pub async fn dry_run(&self) -> crate::Result<MigrationDryRun> {
        let backend = self.db.db_type();
        let transactional = backend.is_transactional();
        let pending = self.pending().await?;
        let applied: BTreeSet<u64> = self
            .db
            .migration_history()
            .await?
            .into_iter()
            .map(|migration| migration.version)
            .collect();
        let mut warnings = Vec::new();
        if !transactional {
            warnings.push(
                "backend migrations are not transactional; resume from the first pending version"
                    .to_string(),
            );
        }

        let mut by_version = self
            .migrations
            .iter()
            .map(|migration| (migration.version(), migration))
            .collect::<BTreeMap<_, _>>();
        let mut steps = Vec::new();
        for (migration_index, info) in pending.iter().enumerate() {
            let migration = by_version
                .remove(&info.version)
                .ok_or_else(|| crate::OrmerError::migration("pending migration disappeared"))?;
            for (step_index, step) in migration.up().into_iter().enumerate() {
                let sql = step.sql(backend)?;
                if !transactional && sql.contains(';') {
                    return Err(crate::OrmerError::migration(format!(
                        "non-transactional migration {} step {} must contain one statement",
                        info.version, step_index
                    )));
                }
                steps.push(MigrationDryRunStep {
                    version: info.version,
                    migration_name: info.name.clone(),
                    migration_index,
                    step_index,
                    sql,
                });
            }
        }

        Ok(MigrationDryRun {
            backend,
            transactional,
            completed_versions: applied.into_iter().collect(),
            steps,
            warnings,
        })
    }

    /// Report completed work and the next safe resume version.
    pub async fn execution_status(&self) -> crate::Result<MigrationExecutionStatus> {
        let backend = self.db.db_type();
        let completed = self.db.migration_history().await?;
        let pending = self.pending().await?;
        let resume_version = pending.first().map(|migration| migration.version);
        Ok(MigrationExecutionStatus {
            backend,
            transactional: backend.is_transactional(),
            completed,
            pending,
            resume_version,
        })
    }

    pub async fn rollback_last(&self) -> crate::Result<()> {
        let history = self.db.migration_history().await?;
        let Some(last) = history.last() else {
            return Err(crate::OrmerError::migration(
                "no applied migrations to roll back",
            ));
        };
        let migration = self
            .migrations
            .iter()
            .find(|migration| migration.version() == last.version)
            .ok_or_else(|| {
                crate::OrmerError::migration(format!(
                    "migration {} is not present in the configured migration list",
                    last.version
                ))
            })?;
        let steps = migration.down();
        if steps.is_empty() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "migration rollback without down steps",
            });
        }

        if !self.db.db_type().is_transactional() {
            execute_steps_nontransactional(self.db, self.db.db_type(), &steps).await?;
            let sql = if self.db.db_type().is_questdb() {
                format!(
                    "UPDATE {MIGRATION_TABLE_NAME} SET rolled_back = TRUE WHERE version = {}",
                    last.version
                )
            } else {
                format!(
                    "DELETE FROM {MIGRATION_TABLE_NAME} WHERE version = {}",
                    last.version
                )
            };
            self.db.execute_sql(sql).await?;
            return Ok(());
        }

        let mut transaction = self.db.begin().await?;
        let result = async {
            execute_steps(&mut transaction, self.db.db_type(), &steps).await?;
            transaction
                .execute_sql(format!(
                    "DELETE FROM {MIGRATION_TABLE_NAME} WHERE version = {}",
                    last.version
                ))
                .await?;
            Ok::<(), crate::OrmerError>(())
        }
        .await;
        match result {
            Ok(()) => transaction.commit().await,
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}

/// 把 [`crate::table_migrate::TableApplyOutcome`] 映射回
/// [`TableEnsureOutcome`]（薄糖入口的返回形态）。
fn ensure_outcome_from_apply(
    outcome: &crate::table_migrate::TableApplyOutcome,
) -> TableEnsureOutcome {
    match &outcome.diagnosis {
        crate::table_migrate::TableDiagnosis::Ready => TableEnsureOutcome::Ready,
        crate::table_migrate::TableDiagnosis::Migratable(_) => {
            if outcome.created_table || outcome.executed.is_empty() {
                TableEnsureOutcome::Ready
            } else {
                TableEnsureOutcome::Migrated
            }
        }
        crate::table_migrate::TableDiagnosis::NeedsRebuild(_) => TableEnsureOutcome::Recreated,
    }
}

/// Builder returned by `Database::migrate_table`.
pub struct TableMigration<'a, T: WritableModel> {
    db: &'a Database,
    marker: PhantomData<T>,
    /// 用户显式标注的列重命名（old, new）。diff 阶段命中"删 old 列 + 加 new 列"
    /// 且存在对应标注时，生成 RenameColumn 而非删列加列，避免数据丢失。
    renames: Vec<(String, String)>,
    /// 目标表名覆盖：迁移 PG 路由子表时按子表名参数化（对齐
    /// `CreateTableExecutor` 的 `table_name: Some(..)` 先例）。`None` 时
    /// 使用模型基础表名。
    table_name: Option<String>,
    /// 主键差异处理模式：false（默认，`migrate_table` 公开面）保持既有语义
    /// ——主键变更报 `UnmigratableSchema`；true（plan_table/apply_table 内部）
    /// 在 PostgreSQL 上生成 `ChangePrimaryKey` 原地变更步骤，其余后端转重建。
    pub(crate) allow_inplace_pk: bool,
    /// 多余列处理：Drop（默认，`migrate_table` 公开面的既有语义——计划含
    /// DropColumn，由调用方决定是否执行）或 Keep（保留，不生成 DropColumn）。
    pub(crate) extra_columns: crate::table_migrate::ExtraPolicy,
}

impl<'a, T: WritableModel> TableMigration<'a, T> {
    /// 标注一次列重命名：数据库中的 `old_name` 列对应模型中的 `new_name` 字段。
    ///
    /// 未标注时，列重命名会被 diff 识别为"删旧列 + 加新列"，默认的
    /// [`Database::ensure_table`] 会拒绝（数据丢失）；标注后迁移计划生成
    /// `ALTER TABLE ... RENAME COLUMN`（MSSQL 为 `sp_rename`），数据原地保留。
    /// 可链式标注多次，随后调用 [`TableMigration::plan`] / `execute`：
    ///
    /// ```ignore
    /// db.migrate_table::<User>()
    ///     .rename_column("name", "full_name")
    ///     .execute()
    ///     .await?;
    /// ```
    pub fn rename_column(
        mut self,
        old_name: impl Into<String>,
        new_name: impl Into<String>,
    ) -> Self {
        self.renames.push((old_name.into(), new_name.into()));
        self
    }

    /// 内部构造：plan_table / apply_table 的诊断路径使用（主键原地变更
    /// 开启、多余列策略由 ApplyOptions 决定）。
    #[allow(dead_code)]
    pub(crate) fn for_diagnosis(
        db: &'a Database,
        table_name: Option<&str>,
        extra_columns: crate::table_migrate::ExtraPolicy,
    ) -> Self {
        Self {
            db,
            marker: PhantomData,
            renames: Vec::new(),
            table_name: table_name.map(str::to_string),
            allow_inplace_pk: true,
            extra_columns,
        }
    }

    #[cfg(feature = "sqlite")]
    pub async fn sqlite_rebuild_plan(&self) -> crate::Result<MigrationPlan> {
        let db_type = self.db.db_type();
        if !matches!(db_type, DbType::Sqlite) {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "SQLite table rebuild",
            });
        }

        let table_name = T::table_name_for_db(db_type);
        let actual = self.db.schema_columns(table_name).await?.ok_or_else(|| {
            crate::ormer_error!(
                "SQLite table {} does not exist; use create_table instead of a rebuild",
                table_name
            )
        })?;
        let actual_by_name = actual
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect::<BTreeMap<_, _>>();
        let mut plan = MigrationPlan::new(table_name, db_type);
        plan.warnings.push(
            "SQLite rebuild replaces the table in one transaction; review copy rules before execution"
                .to_string(),
        );
        plan.push(MigrationStep::Sql {
            sql: sqlite_rebuild_sql::<T>(table_name, &actual_by_name)?,
        });
        Ok(plan)
    }

    #[cfg(feature = "sqlite")]
    pub async fn sqlite_rebuild_sql(&self) -> crate::Result<String> {
        self.sqlite_rebuild_plan().await?.to_sql()
    }

    /// 迁移计划预览：基础表动作 + PG 路由子表（已存在的）动作。
    ///
    /// 模型声明 hypertable route key 时（仅 PostgreSQL），写入路径会把数据
    /// 落到 `{基础表名}_{路由值}` 子表；本方法在基础表计划之后，用 `pg_class`
    /// 前缀查询枚举已存在的子表，把每个子表的差异动作并入同一份计划。路由
    /// 值无法枚举，不存在的子表不预建（首次写入时仍按当时 DDL 自动创建）。
    pub async fn plan(&self) -> crate::Result<MigrationPlan> {
        #[cfg_attr(not(feature = "postgresql"), allow(unused_mut))]
        let mut plan = self.plan_base_only().await?;
        #[cfg(feature = "postgresql")]
        if self.table_name.is_none() {
            self.append_routed_child_steps(&mut plan).await?;
        }
        Ok(plan)
    }

    /// 仅目标表的计划（不含路由子表扩散）。[`Database::ensure_table`] 的
    /// 增量分支使用它：子表动作由专门的子表迁移步骤逐表套用与基础表相同
    /// 的破坏性预检后再执行，避免严格模式下绕过删列拒绝。
    async fn plan_base_only(&self) -> crate::Result<MigrationPlan> {
        let db_type = self.db.db_type();
        let table_name = self
            .table_name
            .as_deref()
            .unwrap_or(T::table_name_for_db(db_type));
        self.plan_for_table(table_name).await
    }

    /// 表名参数化的计划核心：基础表与路由子表共用同一套 schema diff 逻辑
    /// （对齐 `CreateTableExecutor` 的 `table_name: Some(..)` 参数化先例）。
    pub(crate) async fn plan_for_table(&self, table_name: &str) -> crate::Result<MigrationPlan> {
        let db_type = self.db.db_type();
        // QuestDB 没有主键/NOT NULL 约束，自省结果不参与这两项比较
        #[cfg(feature = "questdb")]
        let questdb = matches!(db_type, DbType::QuestDB);
        #[cfg(not(feature = "questdb"))]
        let questdb = false;
        let mut plan = MigrationPlan::new(table_name, db_type);
        let actual = self.db.schema_columns(table_name).await?;

        let Some(mut actual) = actual else {
            // 路由子表不预建：不存在时计划为空，由调用方跳过（写入路径会在
            // 首次写入时按当前 DDL 自动创建子表）。
            if table_name != T::table_name_for_db(db_type) {
                return Ok(plan);
            }
            // 建表配套步骤对齐 CreateTableExecutor 的 DDL 序列：
            // 枚举/EXTENSION 前置，create_hypertable 后置（缺了前者建表
            // 直接报缺类型，缺了后者表永远不是超表、复验必不收敛）。
            let (pre, post) = create_table_bootstrap_steps::<T>(db_type, table_name);
            for step in pre {
                plan.push(step);
            }
            plan.push(MigrationStep::CreateTable {
                table: table_name.to_string(),
                definition: crate::generate_create_table_sql::<T>(db_type)?,
            });
            for step in post {
                plan.push(step);
            }
            return Ok(plan);
        };

        // TimescaleDB 超表元数据按模型表名校验，仅对基础表执行；子表的超表
        // 声明与基础表同源（同一份 #[hypertable] 声明随建表 DDL 下发）。
        if table_name == T::table_name_for_db(db_type) {
            self.db.validate_hypertable_for_migration::<T>().await?;
        }

        // 先应用用户显式标注的列重命名：把自省结果中的旧列名改写为新列名，
        // 后续 drop/add/type diff 基于改写后的列集合进行，避免"删旧列 + 加
        // 新列"丢数据。RenameColumn 步骤置于计划最前，重命名之后才执行该列
        // 上的类型/约束变更。
        //
        // SQLite 特例：若本次迁移还触发了整表重建，重建按新列名复制数据，
        // 无需单独的重命名步骤，因此先暂存、在重建判定后再决定是否下发。
        #[cfg(feature = "sqlite")]
        let mut sqlite_rename_steps = Vec::new();
        for (old_name, new_name) in &self.renames {
            if !T::COLUMN_SCHEMA
                .iter()
                .any(|column| column.name == new_name.as_str())
            {
                return Err(crate::ormer_error!(
                    "rename_column target {new_name} is not a column of model table {table_name}"
                ));
            }
            let old_exists = actual.iter().any(|column| column.name == *old_name);
            let new_exists = actual.iter().any(|column| column.name == *new_name);
            if !old_exists && new_exists {
                continue; // 重命名已生效，幂等跳过
            }
            if !old_exists || new_exists {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!(
                        "rename_column {old_name} -> {new_name} does not match table {table_name}: \
                         source column is {}, target column {}",
                        if old_exists {
                            "present".to_string()
                        } else {
                            "missing".to_string()
                        },
                        if new_exists {
                            "already present".to_string()
                        } else {
                            "missing".to_string()
                        },
                    ),
                ));
            }
            for column in &mut actual {
                if column.name == *old_name {
                    column.name = new_name.clone();
                }
            }
            let step = MigrationStep::RenameColumn {
                table: table_name.to_string(),
                old_name: old_name.clone(),
                new_name: new_name.clone(),
            };
            #[cfg(feature = "sqlite")]
            if matches!(db_type, DbType::Sqlite) {
                sqlite_rename_steps.push(step);
                continue;
            }
            plan.push(step);
        }

        let actual_names: BTreeSet<&str> =
            actual.iter().map(|column| column.name.as_str()).collect();
        let expected_names: BTreeSet<&str> =
            T::COLUMN_SCHEMA.iter().map(|column| column.name).collect();
        let actual_by_name: BTreeMap<&str, &SchemaColumn> = actual
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();

        #[cfg(feature = "sqlite")]
        let mut sqlite_rebuild_required = false;

        for column in &actual {
            if expected_names.contains(column.name.as_str()) {
                continue;
            }
            if self.extra_columns == crate::table_migrate::ExtraPolicy::Keep {
                // Keep（apply_table 默认）：多余列保留在库内，不参与 DDL。
                // 必须先于 SQLite 重建判定，否则 Keep 语义在 SQLite 上被绕过。
                plan.warnings.push(format!(
                    "keeping column {} because it is not present in the model (extra_columns = Keep)",
                    column.name
                ));
                continue;
            }
            #[cfg(feature = "sqlite")]
            if matches!(db_type, DbType::Sqlite) {
                sqlite_rebuild_required = true;
                continue;
            }
            plan.warnings.push(format!(
                "dropping column {} because it is not present in the model",
                column.name
            ));
            plan.push(MigrationStep::DropColumn {
                table: table_name.to_string(),
                column: column.name.clone(),
            });
        }

        // Additive changes are safe to infer when a non-null column can be
        // populated. A populated table without a model default needs an
        // explicit backfill because the ORM cannot invent its value.
        let mut added_columns = BTreeSet::new();
        for column in T::COLUMN_SCHEMA {
            if !actual_names.contains(column.name) {
                if column.is_primary {
                    return Err(crate::OrmerError::unmigratable_schema(
                        table_name,
                        format!(
                            "Cannot infer adding primary key column {}; write an explicit migration",
                            column.name
                        ),
                    ));
                }
                if !column.is_nullable
                    && column.default.is_none()
                    && self.db.table_row_count(table_name).await? > 0
                {
                    return Err(crate::OrmerError::unmigratable_schema(
                        table_name,
                        format!(
                            "Cannot add NOT NULL column {} to populated table {}; \
                             write an explicit migration with a backfill",
                            column.name,
                            table_name
                        ),
                    ));
                }
                plan.push(MigrationStep::AddColumn {
                    table: table_name.to_string(),
                    column: column.name.to_string(),
                    definition: column_definition(db_type, column)?,
                });
                added_columns.insert(column.name);
            }
        }

        // ---- 索引语义 diff：期望集合（模型声明）vs 实际集合（库内自省）----
        // 按 列集合+唯一性+method/expression/where 定义文本 比对而非按名字：
        // 期望有实际没有 → 建（ormer 自动命名）；语义相等但名字漂移
        // （旧命名事故 idx_{裸表名}_{列}）或列序/定义漂移 → 删旧按 ormer
        // 命名重建；INVALID（indisvalid=false）残留 → 清除。
        // 覆盖"给已有列加 #[index]"与"从模型删除 #[index]"两种变更。
        //
        // QuestDB 例外：索引内联在建表语句（SYMBOL 列 INDEX），建表时已随列
        // 生成，视为已有索引；且无独立 CREATE/DROP INDEX DDL，跳过 diff。
        let mut available_columns = actual_names.clone();
        available_columns.extend(added_columns.iter().copied());
        let mut introspected_table = None;
        if !questdb {
            match self.db.db_first_table_for(table_name).await? {
                Some(db_first_table) => {
                    introspected_table = Some(db_first_table);
                }
                None => {
                    plan.warnings.push(format!(
                        "metadata for table {table_name} was unavailable; \
                         index and default differences were not evaluated"
                    ));
                }
            }
            if let Some(db_first_table) = &introspected_table {
                let actual_facts =
                    self.db.actual_index_facts(table_name, &db_first_table.indexes).await?;
                let foreign_key_columns: BTreeSet<&str> = db_first_table
                    .foreign_keys
                    .iter()
                    .map(|foreign_key| foreign_key.column.as_str())
                    .collect();
                let (steps, warnings) = crate::table_migrate::plan_index_semantic_diff(
                    db_type,
                    table_name,
                    &expected_index_defs::<T>(),
                    &actual_facts,
                    &available_columns,
                    &foreign_key_columns,
                    index_names_reliable(db_type),
                )?;
                plan.warnings.extend(warnings);
                for step in steps {
                    plan.push(step);
                }
            }
        }

        // ---- 外键集合管理 ----
        // 新增列的外键随 AddColumn 后补 AddForeignKey；已有列上的 `#[foreign]`
        // 增删通过与 db_first 自省结果做集合 diff 检测，库内外键不再与模型
        // 静默漂移（不支持相应 DDL 的后端记录 warning 或拒绝，见
        // push_foreign_key_diff_steps）。
        if !added_columns.is_empty() {
            for column in T::COLUMN_SCHEMA {
                if !added_columns.contains(column.name) {
                    continue;
                }
                if let Some(foreign_key) = &column.foreign_key {
                    let sqlite_backend = {
                        #[cfg(feature = "sqlite")]
                        {
                            matches!(db_type, DbType::Sqlite)
                        }
                        #[cfg(not(feature = "sqlite"))]
                        {
                            false
                        }
                    };
                    if sqlite_backend {
                        return Err(crate::OrmerError::unmigratable_schema(
                            table_name,
                            format!(
                                "Cannot add foreign key column {} to SQLite table {}; write an explicit table-rebuild migration",
                                column.name,
                                table_name
                            ),
                        ));
                    }
                    plan.push(MigrationStep::AddForeignKey {
                        table: table_name.to_string(),
                        column: column.name.to_string(),
                        ref_table: crate::model::normalize_table_name_for_db(
                            db_type,
                            foreign_key.ref_table,
                        )
                        .to_string(),
                        ref_column: foreign_key.get_ref_column().to_string(),
                    });
                }
            }
        }
        if let Some(db_first_table) = &introspected_table {
            push_foreign_key_diff_steps::<T>(db_type, table_name, db_first_table, &mut plan)?;
        }

        // ---- PostgreSQL 枚举变体追加 ----
        // 模型为枚举列新增了库内枚举类型尚不存在的变体时，生成 ALTER TYPE ...
        // ADD VALUE 步骤让增量迁移收敛。没有这一步时，列类型 diff（只比类型名，
        // 枚举列名不变则视为未变更）与 db_first 校验（比变体列表）之间出现空洞：
        // 校验报 Enum variants mismatch → 计划为空 → 复验仍失败 → permissive
        // 路径删表重建，而重建时的 CREATE TYPE 因同名类型已存在被跳过，新表
        // 仍引用缺变体的旧类型 → 校验永远失败，ensure_table 死循环。
        // 变体删除/重排是 PostgreSQL 不支持的变更，仍交由重建路径处理。
        #[cfg(feature = "postgresql")]
        {
            if matches!(db_type, DbType::PostgreSQL) {
                if let Some(db_first_table) = &introspected_table {
                    push_enum_add_value_steps::<T>(db_first_table, &mut plan)?;
                }
            }
        }

        // 主键期望值使用有效主键列：TimescaleDB 空间分区超表的主键包含分区列
        let effective_primary_keys = crate::model::effective_primary_key_columns::<T>(db_type);

        // ---- 主键集合 diff：PG 原地变更（ChangePrimaryKey）或转重建 ----
        // 集合级比较先于逐列循环：主键列成员变化不再逐列报
        // UnmigratableSchema，而是按 allow_inplace_pk 生成单条
        // ALTER TABLE .. DROP CONSTRAINT .., ADD PRIMARY KEY ..（保数据），
        // 或把差异上抛为重建原因。步骤在列循环之后下发（新主键列的
        // SET NOT NULL 等 AlterColumn 必须先行）。
        let actual_pk_set: BTreeSet<String> = actual
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.clone())
            .collect();
        let mut pk_change_step: Option<MigrationStep> = None;
        if !questdb {
            let expected_pk_set: BTreeSet<&str> =
                effective_primary_keys.iter().copied().collect();
            if actual_pk_set.len() != expected_pk_set.len()
                || actual_pk_set.iter().any(|name| !expected_pk_set.contains(name.as_str()))
            {
                let partition_columns: Vec<&str> = {
                    #[cfg(feature = "postgresql")]
                    {
                        let mut columns = Vec::new();
                        if let Some((time_column, _)) = T::hypertable_info() {
                            columns.push(time_column);
                        }
                        if let Some((space_column, _)) = T::hypertable_space_info() {
                            columns.push(space_column);
                        }
                        columns
                    }
                    #[cfg(not(feature = "postgresql"))]
                    {
                        Vec::new()
                    }
                };
                let is_hypertable = {
                    #[cfg(feature = "postgresql")]
                    {
                        T::hypertable_info().is_some()
                            && matches!(db_type, DbType::PostgreSQL)
                    }
                    #[cfg(not(feature = "postgresql"))]
                    {
                        false
                    }
                };
                let pk_constraint_name = if self.allow_inplace_pk {
                    self.db.primary_key_constraint_name(table_name).await?
                } else {
                    None
                };
                match crate::table_migrate::plan_primary_key_change(
                    db_type,
                    table_name,
                    &actual_pk_set,
                    &effective_primary_keys,
                    pk_constraint_name,
                    is_hypertable,
                    &partition_columns,
                ) {
                    Ok(Some(step)) if self.allow_inplace_pk => {
                        pk_change_step = Some(step);
                    }
                    Ok(_) => {
                        return Err(crate::OrmerError::unmigratable_schema(
                            table_name,
                            format!(
                                "primary key change (actual {{{}}}, expected {{{}}}); \
                                 write an explicit migration",
                                actual_pk_set
                                    .iter()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                effective_primary_keys.join(", "),
                            ),
                        ));
                    }
                    Err(reason) => {
                        return Err(crate::OrmerError::unmigratable_schema(
                            table_name,
                            format!(
                                "primary key change (actual {{{}}}, expected {{{}}}): {}; \
                                 write an explicit migration",
                                actual_pk_set
                                    .iter()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                effective_primary_keys.join(", "),
                                reason
                            ),
                        ));
                    }
                }
            }
        }

        for expected in T::COLUMN_SCHEMA {
            let Some(actual) = actual_by_name.get(expected.name) else {
                continue;
            };
            let expected_primary = effective_primary_keys.contains(&expected.name);
            if !questdb && !pk_change_step.is_some() && actual.primary_key != expected_primary {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!(
                        "Cannot infer primary-key migration for column {}; write an explicit migration",
                        expected.name
                    ),
                ));
            }

            if actual.type_name.is_empty() {
                return Err(crate::ormer_error!(
                    "Cannot determine the database type of column {}",
                    expected.name
                ));
            }

            let expected_type = column_type_definition(db_type, expected);
            let type_changed = !types_equivalent(db_type, &actual.type_name, &expected_type);
            let nullable_changed =
                !expected.is_primary && !questdb && actual.nullable != expected.is_nullable;
            let compression_changed = {
                #[cfg(feature = "postgresql")]
                if matches!(db_type, DbType::PostgreSQL) {
                    let expected_compression = crate::model::column_compression_algorithm(expected);
                    actual.compression.as_deref()
                        != expected_compression.map(CompressionAlgorithm::as_str)
                } else {
                    false
                }
                #[cfg(not(feature = "postgresql"))]
                {
                    false
                }
            };

            if !type_changed && !nullable_changed && !compression_changed {
                continue;
            }

            if expected.is_primary && type_changed {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!(
                        "Cannot infer primary-key type migration for column {}; write an explicit migration",
                        expected.name
                    ),
                ));
            }

            // nullable → NOT NULL：先检查存量 NULL。无论步骤走 SET/DROP
            // NOT NULL 风格的 AlterColumn，还是 MySQL/MSSQL 全列定义的
            // MODIFY/ALTER COLUMN（类型与约束一并重写），存在 NULL 时执行都
            // 必然失败，这里提前给出带表名列名的可诊断错误。修复方式：先跑
            // BackfillColumn/UPDATE 清理存量，再重新生成迁移计划。
            if nullable_changed && !expected.is_nullable {
                let null_count = self
                    .db
                    .null_count(db_type, table_name, expected.name)
                    .await?;
                if null_count > 0 {
                    return Err(crate::OrmerError::unmigratable_schema(
                        table_name,
                        format!(
                            "column {} contains {null_count} NULL value(s); \
                             backfill them before setting NOT NULL",
                            expected.name
                        ),
                    ));
                }
            }

            #[cfg(feature = "sqlite")]
            if matches!(db_type, DbType::Sqlite) {
                sqlite_rebuild_required = true;
                continue;
            }
            #[cfg(feature = "questdb")]
            if matches!(db_type, DbType::QuestDB) {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: db_type,
                    feature: "column type migration (QuestDB cannot ALTER COLUMN types; rebuild the table via a new table + INSERT SELECT in a hand-written migration)",
                });
            }
            #[cfg(feature = "influxdb")]
            if matches!(db_type, DbType::InfluxDB) {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: db_type,
                    feature: "column metadata migrations",
                });
            }

            #[cfg(any(
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql",
                feature = "duckdb"
            ))]
            {
                if type_changed {
                    let (definition, using) = match db_type {
                        #[cfg(feature = "postgresql")]
                        DbType::PostgreSQL => (
                            format!("TYPE {expected_type}"),
                            postgresql_using_expression(db_type, actual, expected, &expected_type),
                        ),
                        #[cfg(feature = "mysql")]
                        DbType::MySQL => (column_definition(db_type, expected)?, None),
                        #[cfg(feature = "mssql")]
                        DbType::MSSQL => (column_definition(db_type, expected)?, None),
                        #[cfg(feature = "duckdb")]
                        DbType::DuckDB => (format!("SET DATA TYPE {expected_type}"), None),
                        #[cfg(feature = "sqlite")]
                        DbType::Sqlite => unreachable!("SQLite uses table rebuilds"),
                        #[cfg(feature = "clickhouse")]
                        DbType::ClickHouse => (column_definition(db_type, expected)?, None),
                        #[cfg(feature = "questdb")]
                        DbType::QuestDB => (String::new(), None),
                        // 不可达：InfluxDB 已提前报错，仅为 match 穷尽性保留
                        #[cfg(feature = "influxdb")]
                        DbType::InfluxDB => (String::new(), None),
                    };
                    plan.push(MigrationStep::AlterColumn {
                        table: table_name.to_string(),
                        column: expected.name.to_string(),
                        definition,
                        using,
                    });
                }

                if nullable_changed && !type_changed {
                    let definition = match db_type {
                        #[cfg(feature = "postgresql")]
                        DbType::PostgreSQL => {
                            if expected.is_nullable {
                                "DROP NOT NULL".to_string()
                            } else {
                                "SET NOT NULL".to_string()
                            }
                        }
                        #[cfg(feature = "mysql")]
                        DbType::MySQL => column_definition(db_type, expected)?,
                        #[cfg(feature = "mssql")]
                        DbType::MSSQL => column_definition(db_type, expected)?,
                        #[cfg(feature = "sqlite")]
                        DbType::Sqlite => unreachable!("SQLite uses table rebuilds"),
                        #[cfg(feature = "duckdb")]
                        DbType::DuckDB => {
                            if expected.is_nullable {
                                "DROP NOT NULL".to_string()
                            } else {
                                "SET NOT NULL".to_string()
                            }
                        }
                        #[cfg(feature = "clickhouse")]
                        DbType::ClickHouse => column_definition(db_type, expected)?,
                        #[cfg(feature = "questdb")]
                        DbType::QuestDB => String::new(),
                        // 不可达：InfluxDB 已提前报错，仅为 match 穷尽性保留
                        #[cfg(feature = "influxdb")]
                        DbType::InfluxDB => String::new(),
                    };
                    plan.push(MigrationStep::AlterColumn {
                        table: table_name.to_string(),
                        column: expected.name.to_string(),
                        definition,
                        using: None,
                    });
                }

                #[cfg(feature = "postgresql")]
                if compression_changed {
                    let expected_compression = crate::model::column_compression_algorithm(expected);
                    if let Some(step) = column_compression_migration_step(
                        db_type,
                        table_name,
                        expected.name,
                        expected_compression,
                    )? {
                        plan.push(step);
                    }
                }
            }
        }

        #[cfg(feature = "mysql")]
        if matches!(db_type, DbType::MySQL) {
            let expected_compression = crate::model::table_compression_algorithm::<T>()?;
            let actual_compression = actual
                .iter()
                .find_map(|column| column.compression.as_deref())
                .and_then(parse_compression_algorithm);
            if actual_compression != expected_compression {
                if let Some(step) = column_compression_migration_step(
                    db_type,
                    table_name,
                    "",
                    expected_compression,
                )? {
                    plan.push(step);
                }
            }
        }

        // 主键原地变更步骤在列对齐（AddColumn/SET NOT NULL）之后下发
        if let Some(step) = pk_change_step {
            plan.push(step);
        }

        // ---- CHECK 约束闭环：模型声明 vs 库内存量（PG）----
        // 模型声明的 check 进比对集：缺失 → AddConstraint；同名但定义
        // （归一化表达式）不一致 → Drop + Add；表达式相同但名字不同的
        // 存量约束视为同一约束不重复加；模型未声明的存量 CHECK 不动。
        // 非 PG 后端无可靠的 check 定义自省，保守跳过（维持旧行为）。
        #[cfg(feature = "postgresql")]
        {
            if matches!(db_type, DbType::PostgreSQL) {
                let expected_checks = expected_check_defs::<T>(table_name);
                if !expected_checks.is_empty() {
                    let (schema, bare) = split_qualified_table_name(table_name);
                    let schema = schema.unwrap_or("public");
                    let actual_checks = self.db.check_constraint_facts(schema, bare).await?;
                    for step in crate::table_migrate::plan_check_diff(
                        table_name,
                        &expected_checks,
                        &actual_checks,
                        db_type,
                    ) {
                        plan.push(step);
                    }
                }
            }
        }
        if let Some(db_first_table) = &introspected_table {
            for (column, expected_default, actual_had_default) in
                column_default_changes::<T>(db_type, db_first_table)
            {
                // 仅启用 sqlite 等部分 feature 时，这些变量可能没有下游使用
                let _ = (column, &expected_default, actual_had_default);
                #[cfg(feature = "sqlite")]
                if matches!(db_type, DbType::Sqlite) {
                    // SQLite 无法 ALTER COLUMN，默认值变更并入整表重建
                    sqlite_rebuild_required = true;
                    continue;
                }
                match db_type {
                    // PostgreSQL / MySQL / DuckDB 均支持
                    // ALTER TABLE ... ALTER COLUMN ... SET/DROP DEFAULT
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        plan.push(alter_column_default_step(
                            db_type,
                            table_name,
                            column,
                            expected_default.as_deref(),
                        ));
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        plan.push(alter_column_default_step(
                            db_type,
                            table_name,
                            column,
                            expected_default.as_deref(),
                        ));
                    }
                    #[cfg(feature = "duckdb")]
                    DbType::DuckDB => {
                        plan.push(alter_column_default_step(
                            db_type,
                            table_name,
                            column,
                            expected_default.as_deref(),
                        ));
                    }
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => {
                        if actual_had_default {
                            // MSSQL 默认值是具名约束，改/删都需先 DROP CONSTRAINT，
                            // 而约束名不在自省结果中，无法安全推断
                            return Err(crate::OrmerError::unmigratable_schema(
                                table_name,
                                format!(
                                    "cannot change the default of column {column} on MSSQL: \
                                     dropping the existing default constraint requires its name; \
                                     write an explicit migration"
                                ),
                            ));
                        }
                        let constraint = crate::model::quote_identifier(
                            db_type,
                            &format!("DF_{}_{}", table_name.replace('.', "_"), column),
                        );
                        plan.push(MigrationStep::Sql {
                            sql: format!(
                                "ALTER TABLE {} ADD CONSTRAINT {constraint} DEFAULT {} FOR {}",
                                crate::model::quote_qualified_identifier(db_type, table_name),
                                expected_default.as_deref().unwrap_or_default(),
                                crate::model::quote_identifier(db_type, column)
                            ),
                        });
                    }
                    // ClickHouse/QuestDB/InfluxDB 走不到这里：schema 自省在
                    // plan() 开头即返回 UnsupportedFeature
                    #[allow(unreachable_patterns)]
                    _ => {}
                }
            }
        }

        #[cfg(feature = "sqlite")]
        if sqlite_rebuild_required {
            // 整表重建按模型列名复制数据（重命名已改写自省结果），并以最终
            // 建表语句重建索引，此前累积的增量步骤全部作废。
            plan.steps.clear();
            plan.push(MigrationStep::Sql {
                sql: sqlite_rebuild_sql::<T>(table_name, &actual_by_name)?,
            });
        } else if !sqlite_rename_steps.is_empty() {
            // 未触发重建时补发重命名；SQLite 3.25+ 支持 RENAME COLUMN，
            // 且重命名必须先于同列上的其他变更执行
            let mut steps = sqlite_rename_steps;
            steps.append(&mut plan.steps);
            plan.steps = steps;
        }

        Ok(plan)
    }

    pub async fn execute(&self) -> crate::Result<()> {
        let plan = self.plan().await?;
        self.execute_plan(&plan).await
    }

    /// 执行一个已生成的迁移计划。
    ///
    /// 与 `execute` 共用尾部逻辑，供 `ensure_table` 在执行前审查计划（拒绝
    /// 破坏性步骤）后复用，避免两处各写一份事务/非事务执行分支。
    pub(crate) async fn execute_plan(&self, plan: &MigrationPlan) -> crate::Result<()> {
        if plan.is_empty() {
            return Ok(());
        }

        if !plan.db_type().is_transactional() {
            return execute_steps_nontransactional(self.db, plan.db_type(), plan.steps()).await;
        }

        let mut transaction = self.db.begin().await?;
        let result = execute_steps(&mut transaction, plan.db_type(), plan.steps()).await;
        match result {
            Ok(()) => transaction.commit().await,
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    /// 把已存在的 PG 路由子表的差异动作并入预览计划。
    ///
    /// 仅 PostgreSQL 且模型声明 route key 时生效；每个子表复用
    /// [`TableMigration::plan_for_table`] 生成子计划，步骤的目标表名即子表
    /// 名（`AddColumn { table: "aaa_val" }` 等），与基础表动作合并在同一份
    /// [`MigrationPlan`] 中。子表已不存在（枚举后被并发删除）时跳过，不预建。
    #[cfg(feature = "postgresql")]
    async fn append_routed_child_steps(&self, plan: &mut MigrationPlan) -> crate::Result<()> {
        let db_type = self.db.db_type();
        if !matches!(db_type, DbType::PostgreSQL) || T::hypertable_route_key().is_none() {
            return Ok(());
        }
        let mut included = Vec::new();
        for child in self.db.existing_routed_child_tables::<T>().await? {
            if self.db.schema_columns(&child).await?.is_none() {
                continue;
            }
            let child_plan = self.plan_for_table(&child).await?;
            if !child_plan.is_empty() {
                included.push((child, child_plan));
            }
        }
        if included.is_empty() {
            return Ok(());
        }
        plan.warnings.push(format!(
            "routed child tables included in this plan: {}",
            included
                .iter()
                .map(|(child, _)| child.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (_, child_plan) in included {
            plan.steps.extend(child_plan.steps);
            plan.warnings.extend(child_plan.warnings);
        }
        Ok(())
    }
}

fn column_definition(db_type: DbType, column: &ColumnSchema) -> crate::Result<String> {
    let mut definition = db_type.sql_type(
        column.data_type.unwrap_or(column.rust_type),
        false,
        column.is_auto_increment,
        column.is_nullable,
        column.enum_variants,
    );

    #[cfg(feature = "postgresql")]
    if let Some(compression) = crate::model::column_compression_algorithm(column) {
        if matches!(db_type, DbType::PostgreSQL) {
            if !matches!(
                compression,
                CompressionAlgorithm::Pglz | CompressionAlgorithm::Lz4
            ) {
                return Err(crate::ormer_error!(
                    "PostgreSQL does not support compression algorithm {}",
                    compression.as_str()
                ));
            }
            let suffix = " NOT NULL";
            let nullable = definition.ends_with(suffix);
            if nullable {
                definition.truncate(definition.len() - suffix.len());
            }
            definition.push_str(" COMPRESSION ");
            definition.push_str(compression.as_str());
            if nullable {
                definition.push_str(suffix);
            }
        }
    }

    if let Some(default) = column.default {
        definition.push_str(" DEFAULT ");
        definition.push_str(&default.to_sql(db_type));
    }

    validate_compression(db_type, column)?;

    if let Some(check) = column.check {
        definition.push_str(" CHECK (");
        definition.push_str(check.expr);
        definition.push(')');
    }

    Ok(definition)
}

fn validate_compression(db_type: DbType, column: &ColumnSchema) -> crate::Result<()> {
    let Some(algorithm) = crate::model::column_compression_algorithm(column) else {
        return Ok(());
    };
    #[cfg(not(any(feature = "postgresql", feature = "mysql")))]
    let _ = &algorithm;

    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            if !matches!(
                algorithm,
                CompressionAlgorithm::Pglz | CompressionAlgorithm::Lz4
            ) {
                return Err(crate::ormer_error!(
                    "PostgreSQL does not support compression algorithm {} for column {}",
                    algorithm.as_str(),
                    column.name
                ));
            }
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => {
            if !matches!(
                algorithm,
                CompressionAlgorithm::Lz4 | CompressionAlgorithm::Zlib
            ) {
                return Err(crate::ormer_error!(
                    "MySQL does not support compression algorithm {} for column {}",
                    algorithm.as_str(),
                    column.name
                ));
            }
        }
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
        #[cfg(feature = "questdb")]
        DbType::QuestDB => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
        #[cfg(feature = "influxdb")]
        DbType::InfluxDB => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "column compression",
            });
        }
    }
    #[cfg(any(feature = "postgresql", feature = "mysql"))]
    Ok(())
}

#[cfg(any(feature = "postgresql", feature = "mysql"))]
fn column_compression_migration_step(
    db_type: DbType,
    table_name: &str,
    column_name: &str,
    compression: Option<CompressionAlgorithm>,
) -> crate::Result<Option<MigrationStep>> {
    #[cfg(feature = "postgresql")]
    if matches!(db_type, DbType::PostgreSQL) {
        let method = compression
            .map(CompressionAlgorithm::as_str)
            .unwrap_or("default");
        return Ok(Some(MigrationStep::Sql {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} SET COMPRESSION {}",
                crate::model::quote_qualified_identifier(DbType::PostgreSQL, table_name),
                crate::model::quote_identifier(DbType::PostgreSQL, column_name),
                method,
            ),
        }));
    }

    #[cfg(feature = "mysql")]
    if matches!(db_type, DbType::MySQL) {
        let method = compression
            .map(CompressionAlgorithm::as_upper_str)
            .unwrap_or("NONE");
        return Ok(Some(MigrationStep::Sql {
            sql: format!(
                "ALTER TABLE {} COMPRESSION='{}'",
                crate::model::quote_qualified_identifier(DbType::MySQL, table_name),
                method,
            ),
        }));
    }

    let _ = (db_type, table_name, column_name, compression);
    Ok(None)
}

#[cfg(feature = "mysql")]
fn parse_compression_algorithm(value: &str) -> Option<CompressionAlgorithm> {
    match value.to_ascii_lowercase().as_str() {
        "pglz" => Some(CompressionAlgorithm::Pglz),
        "lz4" => Some(CompressionAlgorithm::Lz4),
        "zlib" => Some(CompressionAlgorithm::Zlib),
        "zstd" => Some(CompressionAlgorithm::Zstd),
        _ => None,
    }
}

pub(crate) fn index_migration_step(
    db_type: DbType,
    name: String,
    table: &str,
    columns: &[&ColumnSchema],
    unique: bool,
) -> crate::Result<MigrationStep> {
    let has_order_or_predicate = columns
        .iter()
        .any(|column| column.index_order.is_some() || column.index_where.is_some());
    if !has_order_or_predicate {
        return Ok(MigrationStep::CreateIndex {
            name,
            table: table.to_string(),
            columns: columns
                .iter()
                .map(|column| column.name.to_string())
                .collect(),
            unique,
        });
    }
    let has_predicate = columns.iter().any(|column| column.index_where.is_some());
    if has_predicate {
        #[cfg(feature = "sqlite")]
        if matches!(db_type, DbType::Sqlite) {
            return Err(common_helpers::unsupported_partial_index_where(db_type));
        }
        #[cfg(feature = "duckdb")]
        if matches!(db_type, DbType::DuckDB) {
            return Err(common_helpers::unsupported_partial_index_where(db_type));
        }
    }

    let columns_sql = columns
        .iter()
        .map(|column| {
            let mut value = crate::model::quote_identifier(db_type, column.name);
            if let Some(order) = column.index_order {
                value.push(' ');
                value.push_str(order);
            }
            value
        })
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = columns
        .iter()
        .find_map(|column| column.index_where)
        .map(|where_clause| format!(" WHERE {where_clause}"))
        .unwrap_or_default();
    // IF NOT EXISTS 后端分派与标识符引用统一走建表路径的公共入口
    let sql = crate::model::render_create_index(db_type, &name, table, &columns_sql, unique);
    Ok(MigrationStep::Sql {
        sql: format!("{sql}{predicate}"),
    })
}

/// 模型声明的一个索引/唯一约束（diff 的"期望集合"元素）。
pub(crate) struct ExpectedIndexDef<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) columns: Vec<&'a ColumnSchema>,
    pub(crate) unique: bool,
}

/// 从模型列声明构建期望索引集合。
///
/// 分组口径与建表路径（`generate_indexes_with_name` + 内联 UNIQUE）及校验
/// 路径（`db_first::validate_model_constraints`）一致：未分组单列索引按列
/// 独立成组，`index_group`/`unique_group` 各自聚合。
/// 建表配套步骤：与 `CreateTableExecutor` 的 DDL 序列对齐，把建表 DDL
/// 不覆盖的部分拆成前置（EXTENSION、枚举类型）与后置（create_hypertable）
/// 两组。`MigrationStep::CreateTable` 的渲染只含 CREATE TABLE/索引 DDL；
/// 建表计划若不带这些步骤，增量路径（含 ensure 对缺失表的自动建表）建出
/// 的表要么缺枚举类型直接报错，要么永远是普通表、复验必报 Hypertable
/// mismatch 而卡死或触发删表重建。
/// 返回 `(前置步骤, 后置步骤)`：前置须在 CreateTable 之前执行，后置在其后。
#[allow(unused_variables)]
pub(crate) fn create_table_bootstrap_steps<T: WritableModel>(
    db_type: DbType,
    table_name: &str,
) -> (Vec<MigrationStep>, Vec<MigrationStep>) {
    #[cfg(feature = "postgresql")]
    {
        if matches!(db_type, DbType::PostgreSQL) {
            // 枚举类型前置：DO 块幂等创建（与 CreateTableExecutor 的模板一致）。
            let mut pre: Vec<MigrationStep> = T::column_schema()
                .iter()
                .filter_map(|column| {
                    column.enum_variants.map(|variants| {
                        let enum_name =
                            crate::abstract_layer::postgresql_backend::to_snake_case(
                                column.rust_type,
                            );
                        let variants_str = variants
                            .iter()
                            .map(|v| format!("'{v}'"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        MigrationStep::Sql {
                            sql: format!(
                                "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = '{enum_name}') THEN CREATE TYPE {enum_name} AS ENUM ({variants_str}); END IF; END $$"
                            ),
                        }
                    })
                })
                .collect();
            if let Some((time_column, chunk_interval)) = T::hypertable_info() {
                // 超表模型：EXTENSION 前置、create_hypertable 后置。
                // create_default_indexes => FALSE：与 CreateTableExecutor 一致，
                // 避免 TimescaleDB 默认索引使实际索引集合偏离模型声明。
                pre.push(MigrationStep::Sql {
                    sql: "CREATE EXTENSION IF NOT EXISTS timescaledb".to_string(),
                });
                let interval_str = chunk_interval.to_interval_string();
                let hypertable_sql =
                    if let Some((space_column, partitions)) = T::hypertable_space_info() {
                        format!(
                            "SELECT create_hypertable('{table_name}', '{time_column}', chunk_time_interval => INTERVAL '{interval_str}', partitioning_column => '{space_column}', number_partitions => {partitions}, if_not_exists => TRUE, migrate_data => TRUE, create_default_indexes => FALSE)"
                        )
                    } else {
                        format!(
                            "SELECT create_hypertable('{table_name}', '{time_column}', chunk_time_interval => INTERVAL '{interval_str}', if_not_exists => TRUE, migrate_data => TRUE, create_default_indexes => FALSE)"
                        )
                    };
                let post = vec![MigrationStep::Sql {
                    sql: hypertable_sql,
                }];
                return (pre, post);
            }
            return (pre, Vec::new());
        }
    }
    (Vec::new(), Vec::new())
}

pub(crate) fn expected_index_defs<T: WritableModel>() -> Vec<ExpectedIndexDef<'static>> {
    let mut defs = Vec::new();

    let mut grouped: BTreeMap<i32, Vec<&'static ColumnSchema>> = BTreeMap::new();
    for column in T::COLUMN_SCHEMA {
        if !column.is_indexed {
            continue;
        }
        match column.index_group {
            Some(group) => {
                grouped.entry(group).or_default().push(column);
            }
            None => defs.push(ExpectedIndexDef {
                name: column.index_name,
                columns: vec![column],
                unique: false,
            }),
        }
    }
    for columns in grouped.into_values() {
        defs.push(ExpectedIndexDef {
            name: columns.iter().find_map(|column| column.index_name),
            columns,
            unique: false,
        });
    }

    let mut unique_groups: BTreeMap<i32, Vec<&'static ColumnSchema>> = BTreeMap::new();
    for column in T::COLUMN_SCHEMA {
        if let Some(group) = column.unique_group {
            unique_groups.entry(group).or_default().push(column);
        }
    }
    for columns in unique_groups.into_values() {
        defs.push(ExpectedIndexDef {
            name: columns.iter().find_map(|column| column.unique_name),
            columns,
            unique: true,
        });
    }

    defs
}

/// 名字比对的可靠性：SQLite 与 DuckDB（同为 turso 引擎）内联 UNIQUE
/// 解析出的名称由建表语句决定且可能为空串，不可靠，此类组合跳过名字
/// 归一（其余后端可靠）。
pub(crate) fn index_names_reliable(db_type: DbType) -> bool {
    #[cfg(feature = "sqlite")]
    if matches!(db_type, DbType::Sqlite) {
        return false;
    }
    #[cfg(feature = "duckdb")]
    if matches!(db_type, DbType::DuckDB) {
        return false;
    }
    let _ = db_type;
    true
}

/// 从模型列声明提取期望 CHECK 约束集合（显式名或 ormer 自动名
/// `ck_{table}_{column}`）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
pub(crate) fn expected_check_defs<T: WritableModel>(table_name: &str) -> Vec<ExpectedCheck> {
    T::COLUMN_SCHEMA
        .iter()
        .filter_map(|column| {
            column.check.map(|check| ExpectedCheck {
                name: check
                    .name
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        crate::table_migrate::default_check_name(table_name, column.name)
                    }),
                expr: check.expr.to_string(),
            })
        })
        .collect()
}

/// 未显式命名时的索引命名，与建表路径的默认命名保持一致：
/// 单列 `idx_{table}_{column}`、复合 `idx_{table}_{group}`、唯一 `uq_{table}_{group}`。
pub(crate) fn default_index_name(table_name: &str, expected: &ExpectedIndexDef<'_>) -> String {
    let table = table_name.replace('.', "_");
    if expected.unique {
        format!(
            "uq_{table}_{}",
            expected
                .columns
                .first()
                .and_then(|column| column.unique_group)
                .map(|group| group.to_string())
                .unwrap_or_else(|| expected.columns[0].name.to_string())
        )
    } else if expected.columns.len() == 1 {
        format!("idx_{table}_{}", expected.columns[0].name)
    } else {
        expected
            .columns
            .first()
            .and_then(|column| column.index_group)
            .map(|group| format!("idx_{table}_{group}"))
            .unwrap_or_else(|| format!("idx_{table}_{}", expected.columns[0].name))
    }
}

/// 比较外键引用目标是否一致（忽略 schema 前缀差异）。
fn foreign_key_target_matches(actual: &DbFirstForeignKey, ref_table: &str, ref_column: &str) -> bool {
    actual.ref_column == ref_column
        && crate::model::table_name_without_schema(&actual.ref_table)
            == crate::model::table_name_without_schema(ref_table)
}

/// 为已存在的外键约束生成 DROP 步骤。
///
/// 返回 `Ok(true)` 表示已生成步骤；返回 `Ok(false)` 表示该后端无法/无法安全
/// 删除（SQLite 无相应 DDL、自省拿不到约束名），此时已在计划中记录 warning。
fn push_drop_foreign_key_step(
    db_type: DbType,
    table_name: &str,
    foreign_key: &DbFirstForeignKey,
    plan: &mut MigrationPlan,
) -> crate::Result<bool> {
    #[cfg(feature = "sqlite")]
    if matches!(db_type, DbType::Sqlite) {
        plan.warnings.push(format!(
            "keeping foreign key on column {} because SQLite cannot drop constraints; \
             rebuild the table with a hand-written migration",
            foreign_key.column
        ));
        return Ok(false);
    }
    let Some(constraint_name) = foreign_key.name.as_deref() else {
        plan.warnings.push(format!(
            "keeping foreign key on column {} because its constraint name is unknown \
             to introspection; drop it with a hand-written migration",
            foreign_key.column
        ));
        return Ok(false);
    };
    // MySQL 删除外键需要 FOREIGN KEY 关键字（DROP CONSTRAINT 仅 8.0.19+ 支持）
    #[cfg(feature = "mysql")]
    let keyword = if matches!(db_type, DbType::MySQL) {
        "FOREIGN KEY"
    } else {
        "CONSTRAINT"
    };
    #[cfg(not(feature = "mysql"))]
    let keyword = "CONSTRAINT";
    plan.warnings.push(format!(
        "dropping foreign key constraint {constraint_name} because it is not declared in the model"
    ));
    plan.push(MigrationStep::Sql {
        sql: format!(
            "ALTER TABLE {} DROP {keyword} {}",
            crate::model::quote_qualified_identifier(db_type, table_name),
            crate::model::quote_identifier(db_type, constraint_name)
        ),
    });
    Ok(true)
}

/// 已有列的外键集合 diff：期望集合（模型 `#[foreign]`）vs 实际集合（自省）。
///
/// - 期望有实际没有 → `AddForeignKey`；
/// - 实际有期望没有（或引用目标变化，按列名先 drop 再 add）→ `DROP` 步骤；
/// - SQLite 无法给已有表补外键，直接报 UnmigratableSchema 引导显式重建；
/// - QuestDB/ClickHouse/InfluxDB 不进入此函数（自省在 plan() 更早处已拦截）。
fn push_foreign_key_diff_steps<T: WritableModel>(
    db_type: DbType,
    table_name: &str,
    db_first_table: &DbFirstTable,
    plan: &mut MigrationPlan,
) -> crate::Result<()> {
    let actual_by_column: BTreeMap<&str, &DbFirstForeignKey> = db_first_table
        .foreign_keys
        .iter()
        .map(|foreign_key| (foreign_key.column.as_str(), foreign_key))
        .collect();

    for column in T::COLUMN_SCHEMA {
        let Some(expected) = &column.foreign_key else {
            continue;
        };
        let ref_table =
            crate::model::normalize_table_name_for_db(db_type, expected.ref_table).to_string();
        let ref_column = expected.get_ref_column();
        if let Some(actual) = actual_by_column.get(column.name) {
            if foreign_key_target_matches(actual, &ref_table, ref_column) {
                continue;
            }
            // 引用目标变化：先删旧约束；删不掉时跳过重建，避免留下半成品状态
            if !push_drop_foreign_key_step(db_type, table_name, actual, plan)? {
                continue;
            }
        }
        #[cfg(feature = "sqlite")]
        if matches!(db_type, DbType::Sqlite) {
            return Err(crate::OrmerError::unmigratable_schema(
                table_name,
                format!(
                    "Cannot add foreign key for existing column {}; \
                     SQLite requires an explicit table-rebuild migration",
                    column.name
                ),
            ));
        }
        plan.push(MigrationStep::AddForeignKey {
            table: table_name.to_string(),
            column: column.name.to_string(),
            ref_table,
            ref_column: ref_column.to_string(),
        });
    }

    // 实际有期望没有 → 删除
    let expected_fk_columns: BTreeSet<&str> = T::COLUMN_SCHEMA
        .iter()
        .filter(|column| column.foreign_key.is_some())
        .map(|column| column.name)
        .collect();
    for foreign_key in &db_first_table.foreign_keys {
        if expected_fk_columns.contains(foreign_key.column.as_str()) {
            continue;
        }
        push_drop_foreign_key_step(db_type, table_name, foreign_key, plan)?;
    }
    Ok(())
}

/// PostgreSQL 枚举变体 diff：模型声明了库内枚举类型尚不存在的变体时，为每个
/// 缺失变体生成一条 `ALTER TYPE ... ADD VALUE IF NOT EXISTS` 步骤（按模型声明
/// 顺序；ADD VALUE 只能追加到类型末尾，`IF NOT EXISTS` 保证重放幂等）。
///
/// 只处理"实际列类型就是同名枚举类型"的列：
/// - 列在库内不存在 → 由 AddColumn 建列 DDL 处理；
/// - 列类型本身不一致（如被改成 TEXT）→ 由列类型 diff 的 AlterColumn 处理；
/// - 库内变体比模型多（删除/重排变体）→ PostgreSQL 不支持，此处无能为力，
///   交由 ensure_table 的重建路径。
///
/// 注意：`ALTER TYPE ... ADD VALUE` 在 PostgreSQL 12+ 才允许在事务块内执行
/// （且新值须等事务提交后可用——本计划的后续步骤不会用到新值，复验在
/// 提交后进行），TimescaleDB 2.x 要求的 PostgreSQL 版本均满足。
#[cfg(feature = "postgresql")]
fn push_enum_add_value_steps<T: WritableModel>(
    db_first_table: &DbFirstTable,
    plan: &mut MigrationPlan,
) -> crate::Result<()> {
    for column in T::COLUMN_SCHEMA {
        let Some(expected_variants) = column.enum_variants else {
            continue;
        };
        let Some(actual_column) = db_first_table
            .columns
            .iter()
            .find(|actual| actual.name == column.name)
        else {
            continue;
        };
        let enum_name = column_type_definition(DbType::PostgreSQL, column);
        if !types_equivalent(DbType::PostgreSQL, &actual_column.type_name, &enum_name) {
            continue;
        }
        for variant in expected_variants {
            if actual_column
                .enum_variants
                .iter()
                .any(|actual| actual == variant)
            {
                continue;
            }
            plan.push(MigrationStep::AlterType {
                name: enum_name.clone(),
                definition: format!(
                    "ALTER TYPE {enum_name} ADD VALUE IF NOT EXISTS '{}'",
                    variant.replace('\'', "''")
                ),
            });
        }
    }
    Ok(())
}

/// 渲染 `ALTER TABLE ... ALTER COLUMN ... SET/DROP DEFAULT` 步骤
/// （PostgreSQL / MySQL / DuckDB 语法一致）。
#[allow(dead_code)] // 仅在涉及 ALTER DEFAULT 的后端 feature 组合下被调用
fn alter_column_default_step(
    db_type: DbType,
    table_name: &str,
    column: &str,
    expected_default: Option<&str>,
) -> MigrationStep {
    let action = match expected_default {
        Some(expression) => format!("SET DEFAULT {expression}"),
        None => "DROP DEFAULT".to_string(),
    };
    MigrationStep::Sql {
        sql: format!(
            "ALTER TABLE {} ALTER COLUMN {} {action}",
            crate::model::quote_qualified_identifier(db_type, table_name),
            crate::model::quote_identifier(db_type, column)
        ),
    }
}

/// 规整默认值表达式用于比对：统一大小写、去括号/类型转换后缀/引号，
/// 与 db_first 校验路径（`validate_model_constraints`）口径一致。
fn normalize_default_expr(value: &str) -> String {
    let mut value = value.trim().to_ascii_uppercase();
    while value.starts_with('(') && value.ends_with(')') && value.len() >= 2 {
        value = value[1..value.len() - 1].trim().to_string();
    }
    if let Some((expression, _type_name)) = value.split_once("::") {
        value = expression.trim().to_string();
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value = value[1..value.len() - 1].replace("''", "'");
    }
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 找出默认值与模型声明不一致的列，返回 `(列名, 期望默认值 SQL, 实际是否有默认值)`。
///
/// 自增列跳过（PG 的 `nextval(...)` / SQLite 的 autoincrement 属实现细节）；
/// 新增列跳过（其默认值已包含在 AddColumn 定义中）。
fn column_default_changes<T: WritableModel>(
    db_type: DbType,
    db_first_table: &DbFirstTable,
) -> Vec<(&'static str, Option<String>, bool)> {
    let mut changes = Vec::new();
    for expected in T::COLUMN_SCHEMA {
        if expected.is_auto_increment {
            continue;
        }
        let Some(actual) = db_first_table
            .columns
            .iter()
            .find(|actual| actual.name == expected.name)
        else {
            continue;
        };
        if actual.auto_increment {
            continue;
        }
        let expected_default = expected
            .default
            .map(|default| normalize_default_expr(&default.to_sql(db_type)));
        let actual_had_default = actual.default.is_some();
        let actual_default = actual
            .default
            .as_deref()
            .map(normalize_default_expr);
        if expected_default != actual_default {
            changes.push((
                expected.name,
                expected
                    .default
                    .map(|default| default.to_sql(db_type)),
                actual_had_default,
            ));
        }
    }
    changes
}

/// 拆分可能带 schema 前缀的表名（如 PG/MSSQL 的 `public.users`）。
pub(crate) fn split_qualified_table_name(table_name: &str) -> (Option<&str>, &str) {
    match table_name.rsplit_once('.') {
        Some((schema, name)) if !schema.is_empty() && !name.is_empty() => (Some(schema), name),
        _ => (None, table_name),
    }
}

fn column_type_definition(db_type: DbType, column: &ColumnSchema) -> String {
    db_type.sql_type(
        column.data_type.unwrap_or(column.rust_type),
        false,
        false,
        true,
        column.enum_variants,
    )
}

fn types_equivalent(db_type: DbType, actual: &str, expected: &str) -> bool {
    normalize_type(db_type, actual) == normalize_type(db_type, expected)
}

fn normalize_type(db_type: DbType, type_name: &str) -> String {
    let upper = type_name
        .trim()
        .to_ascii_uppercase()
        .replace(" NOT NULL", "");
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => {
            if upper.contains("INT") {
                "INTEGER".to_string()
            } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
                "TEXT".to_string()
            } else if upper.contains("REAL")
                || upper.contains("FLOA")
                || upper.contains("DOUB")
                || upper.contains("NUM")
            {
                "REAL".to_string()
            } else if upper.contains("BLOB") || upper.is_empty() {
                "BLOB".to_string()
            } else {
                upper
            }
        }
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let compact = upper.replace(' ', "");
            match compact.as_str() {
                "_TEXT" | "TEXT[]" | "_VARCHAR" | "VARCHAR[]" | "_BPCHAR" | "CHAR[]" => {
                    "TEXT[]".to_string()
                }
                "_INT2" | "INT2[]" | "SMALLINT[]" => "SMALLINT[]".to_string(),
                "_INT4" | "INT4[]" | "INTEGER[]" => "INTEGER[]".to_string(),
                "_INT8" | "INT8[]" | "BIGINT[]" => "BIGINT[]".to_string(),
                "INT2" | "SMALLINT" => "SMALLINT".to_string(),
                "INT4" | "INT" | "INTEGER" | "SERIAL" => "INTEGER".to_string(),
                "INT8" | "BIGINT" | "BIGSERIAL" => "BIGINT".to_string(),
                "BOOL" | "BOOLEAN" => "BOOLEAN".to_string(),
                "FLOAT4" | "REAL" => "REAL".to_string(),
                "FLOAT8" | "DOUBLEPRECISION" | "FLOAT" => "DOUBLE PRECISION".to_string(),
                "TIMESTAMPTZ" | "TIMESTAMPWITHTIMEZONE" => "TIMESTAMPTZ".to_string(),
                "TIMESTAMP" | "TIMESTAMPWITHOUTTIMEZONE" => "TIMESTAMP".to_string(),
                "UUID" => "UUID".to_string(),
                "CHARACTERVARYING" | "VARCHAR" | "CHAR" | "BPCHAR" | "TEXT" => "TEXT".to_string(),
                _ => upper,
            }
        }
        #[cfg(feature = "questdb")]
        DbType::QuestDB => upper,
        #[cfg(feature = "mysql")]
        DbType::MySQL => {
            let base = upper.split('(').next().unwrap_or(&upper);
            match base {
                "CHAR" | "VARCHAR" => upper,
                "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" => "TEXT".to_string(),
                "TINYINT" if upper.contains("(1)") => "BOOLEAN".to_string(),
                "TINYINT" => "TINYINT".to_string(),
                "INTEGER" | "INT" => "INT".to_string(),
                "BIGINT" => "BIGINT".to_string(),
                "SMALLINT" => "SMALLINT".to_string(),
                "FLOAT" => "FLOAT".to_string(),
                "DOUBLE" | "DOUBLE PRECISION" => "DOUBLE".to_string(),
                "DATETIME" | "TIMESTAMP" => "DATETIME".to_string(),
                "JSON" => "JSON".to_string(),
                "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" => "BLOB".to_string(),
                _ => base.to_string(),
            }
        }
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            let base = upper.split('(').next().unwrap_or(&upper);
            match base {
                "NCHAR" | "NVARCHAR" | "CHAR" | "VARCHAR" | "NTEXT" | "TEXT" => "TEXT".to_string(),
                "BIT" => "BOOLEAN".to_string(),
                "TINYINT" => "TINYINT".to_string(),
                "SMALLINT" => "SMALLINT".to_string(),
                "INT" | "INTEGER" => "INT".to_string(),
                "BIGINT" => "BIGINT".to_string(),
                "REAL" => "REAL".to_string(),
                "FLOAT" => "FLOAT".to_string(),
                "DATETIME" | "DATETIME2" => "DATETIME".to_string(),
                "DATE" => "DATE".to_string(),
                "TIME" => "TIME".to_string(),
                "VARBINARY" | "BINARY" | "IMAGE" => "BLOB".to_string(),
                "UNIQUEIDENTIFIER" => "UUID".to_string(),
                _ => base.to_string(),
            }
        }
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => {
            // DuckDB 兼容层（turso）会把 TEXT 声明回读为 VARCHAR：
            // 标量字符串类型两侧必须归一到同一形态，否则 diff 永远报 AlterColumn。
            if upper.ends_with("[]") {
                upper
            } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
                "TEXT".to_string()
            } else {
                upper
            }
        }
        #[cfg(any(feature = "clickhouse", feature = "influxdb"))]
        _ => upper,
    }
}

#[cfg(feature = "postgresql")]
fn postgresql_using_expression(
    db_type: DbType,
    actual: &SchemaColumn,
    expected: &ColumnSchema,
    expected_type: &str,
) -> Option<String> {
    if !matches!(db_type, DbType::PostgreSQL) {
        return None;
    }

    let column = expected.name;
    if expected_type.eq_ignore_ascii_case("TEXT[]")
        && matches!(
            normalize_type(DbType::PostgreSQL, &actual.type_name).as_str(),
            "TEXT" | "JSONB" | "JSON"
        )
    {
        let null_value = if expected.is_nullable {
            "NULL::TEXT[]"
        } else {
            "ARRAY[]::TEXT[]"
        };
        if matches!(
            normalize_type(DbType::PostgreSQL, &actual.type_name).as_str(),
            "JSONB" | "JSON"
        ) {
            return Some(format!(
                "CASE WHEN {column} IS NULL THEN {null_value} \
                 WHEN jsonb_typeof({column}::jsonb) = 'array' \
                 THEN CONCAT( \
                     '{{', btrim({column}::text, '[]'), '}}' \
                 )::TEXT[] \
                 WHEN jsonb_typeof({column}::jsonb) = 'string' \
                 THEN ARRAY[({column}::jsonb #>> '{{}}')]::TEXT[] \
                 ELSE ARRAY[{column}::text]::TEXT[] END"
            ));
        }
        return Some(format!(
            "CASE WHEN {column} IS NULL THEN {null_value} \
             WHEN btrim({column}::text) = '' THEN ARRAY[]::TEXT[] \
                 WHEN left(btrim({column}::text), 1) = '[' \
             THEN CONCAT( \
                 '{{', btrim({column}::text, '[]'), '}}' \
             )::TEXT[] \
             ELSE ARRAY[{column}::text]::TEXT[] END"
        ));
    }
    Some(format!("{column}::{expected_type}"))
}

#[cfg(feature = "sqlite")]
fn sqlite_rebuild_sql<T: WritableModel>(
    table_name: &str,
    actual_by_name: &BTreeMap<&str, &SchemaColumn>,
) -> crate::Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    table_name.hash(&mut hasher);
    for column in T::COLUMN_SCHEMA {
        column.name.hash(&mut hasher);
        column.rust_type.hash(&mut hasher);
        column.is_nullable.hash(&mut hasher);
    }
    let temporary_table = format!("__ormer_migrate_{}", hasher.finish());
    let check_table = format!("__ormer_migrate_check_{}", hasher.finish());

    let temporary_create =
        crate::generate_create_table_sql_with_name::<T>(DbType::Sqlite, Some(&temporary_table))?;
    let mut temporary_statements = split_sql_statements(&temporary_create);
    let create_table = temporary_statements
        .drain(..1)
        .next()
        .ok_or_else(|| crate::ormer_error!("Generated SQLite migration table SQL is empty"))?;

    let mut statements = Vec::new();
    let mut validation_statements = Vec::new();

    let mut insert_columns = Vec::new();
    let mut select_expressions = Vec::new();
    for column in T::COLUMN_SCHEMA {
        let Some(actual) = actual_by_name.get(column.name) else {
            continue;
        };
        // 重建语句里的列名一律引用：保留字清单补全后，未引用的保留字列
        // 会在这里拼出非法 SQL，与建表路径的引号化口径保持一致。
        let quoted_column = crate::model::quote_identifier(DbType::Sqlite, column.name);
        insert_columns.push(quoted_column.clone());
        let target_type = column_type_definition(DbType::Sqlite, column);
        let expression = if types_equivalent(DbType::Sqlite, &actual.type_name, &target_type) {
            quoted_column
        } else {
            if let Some(validation) = sqlite_conversion_validation_sql(
                &check_table,
                table_name,
                column.name,
                &actual.type_name,
                &target_type,
            ) {
                validation_statements.push(validation.clone());
                validation_statements.push(validation);
            }
            sqlite_conversion_expression(&quoted_column, &actual.type_name, &target_type)?
        };
        select_expressions.push(expression);
    }

    if !validation_statements.is_empty() {
        statements.push(format!("DROP TABLE IF EXISTS {check_table}"));
        statements.push(format!(
            "CREATE TABLE {check_table} (ok INTEGER PRIMARY KEY)"
        ));
        statements.extend(validation_statements);
        statements.push(format!("DROP TABLE {check_table}"));
    }
    statements.push(format!("DROP TABLE IF EXISTS {temporary_table}"));
    statements.push(create_table.to_string());

    if !insert_columns.is_empty() {
        statements.push(format!(
            "INSERT INTO {temporary_table} ({}) SELECT {} FROM {table_name}",
            insert_columns.join(", "),
            select_expressions.join(", ")
        ));
    }
    statements.push(format!("DROP TABLE {table_name}"));
    statements.push(format!(
        "ALTER TABLE {temporary_table} RENAME TO {table_name}"
    ));

    let original_create = crate::generate_create_table_sql::<T>(DbType::Sqlite)?;
    statements.extend(
        split_sql_statements(&original_create)
            .into_iter()
            .skip(1),
    );
    Ok(statements.join(";\n"))
}

#[cfg(feature = "sqlite")]
fn sqlite_conversion_expression(
    column: &str,
    actual_type: &str,
    target_type: &str,
) -> crate::Result<String> {
    let actual = normalize_type(DbType::Sqlite, actual_type);
    let target = normalize_type(DbType::Sqlite, target_type);
    match (actual.as_str(), target.as_str()) {
        ("TEXT", "INTEGER") => Ok(format!("CAST(trim({column}) AS INTEGER)")),
        ("INTEGER", "REAL") => Ok(format!("CAST({column} AS REAL)")),
        (_, "TEXT") => Ok(format!("CAST({column} AS TEXT)")),
        _ => Err(crate::ormer_error!(
            "Cannot safely infer SQLite type migration for column {column}: {actual_type} -> {target_type}"
        )),
    }
}

#[cfg(feature = "sqlite")]
fn sqlite_conversion_validation_sql(
    check_table: &str,
    table_name: &str,
    column: &str,
    actual_type: &str,
    target_type: &str,
) -> Option<String> {
    let actual = normalize_type(DbType::Sqlite, actual_type);
    let target = normalize_type(DbType::Sqlite, target_type);
    if actual != "TEXT" || target != "INTEGER" {
        return None;
    }

    let value = format!("trim({column})");
    let unsigned_digits = format!("({value} GLOB '[0-9]*' AND {value} NOT GLOB '*[^0-9]*')");
    let signed_digits = format!(
        "(({value} GLOB '+[0-9]*' OR {value} GLOB '-[0-9]*') \
         AND length(substr({value}, 2)) > 0 \
         AND substr({value}, 2) NOT GLOB '*[^0-9]*')"
    );
    let valid = format!("(length({value}) > 0 AND ({unsigned_digits} OR {signed_digits}))");

    Some(format!(
        "INSERT INTO {check_table} (ok) \
         SELECT 1 FROM {table_name} \
         WHERE {column} IS NOT NULL AND NOT ({valid}) \
         LIMIT 1"
    ))
}

pub(crate) async fn execute_steps(
    transaction: &mut Transaction<'_>,
    db_type: DbType,
    steps: &[MigrationStep],
) -> crate::Result<()> {
    for step in steps {
        let sql = step.sql(db_type)?;
        for statement in split_sql_statements(&sql) {
            transaction.execute_sql(statement).await?;
        }
    }
    Ok(())
}

pub(crate) async fn execute_steps_nontransactional(
    db: &Database,
    db_type: DbType,
    steps: &[MigrationStep],
) -> crate::Result<()> {
    for step in steps {
        let sql = step.sql(db_type)?;
        for statement in split_sql_statements(&sql) {
            db.execute_sql(statement).await?;
        }
    }
    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut word = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_bracket = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut trigger_mode = false;
    let mut trigger_depth = 0usize;
    // PG dollar-quote（$$...$$ / $tag$...$tag$）：体内分号/引号一律不参与
    // 拆分。DO $$ ... END $$ 块（枚举类型创建等）依赖此状态才能整条送达。
    let mut in_dollar = false;
    let mut dollar_tag = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if in_dollar {
            current.push(c);
            if c == '$' {
                // 匹配定界符剩余部分（dollar_tag 首字符 '$' 已消费）。
                let rest = &dollar_tag[1..];
                let mut matched = String::new();
                for expected in rest.chars() {
                    if chars.peek() == Some(&expected) {
                        matched.push(chars.next().expect("peeked char"));
                    } else {
                        break;
                    }
                }
                current.push_str(&matched);
                if matched == rest {
                    in_dollar = false;
                }
            }
            continue;
        }

        if in_line_comment {
            current.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }

        if in_block_comment {
            current.push(c);
            if c == '*' && matches!(chars.peek(), Some('/')) {
                current.push(chars.next().expect("peeked slash"));
                in_block_comment = false;
            }
            continue;
        }

        if in_single {
            current.push(c);
            if c == '\'' {
                if matches!(chars.peek(), Some('\'')) {
                    current.push(chars.next().expect("peeked quote"));
                } else {
                    in_single = false;
                }
            }
            continue;
        }

        if in_double {
            current.push(c);
            if c == '"' {
                if matches!(chars.peek(), Some('"')) {
                    current.push(chars.next().expect("peeked quote"));
                } else {
                    in_double = false;
                }
            }
            continue;
        }

        if in_backtick {
            current.push(c);
            if c == '`' {
                if matches!(chars.peek(), Some('`')) {
                    current.push(chars.next().expect("peeked backtick"));
                } else {
                    in_backtick = false;
                }
            }
            continue;
        }

        if in_bracket {
            current.push(c);
            if c == ']' {
                if matches!(chars.peek(), Some(']')) {
                    current.push(chars.next().expect("peeked bracket"));
                } else {
                    in_bracket = false;
                }
            }
            continue;
        }

        if c == '$' {
            // 尝试解析 dollar-quote 开定界符：$$ 或 $ident$；其余（如 PG
            // 参数占位符 $1）按普通字符处理。
            let mut probe = chars.clone();
            let mut ident = String::new();
            loop {
                match probe.next() {
                    Some('$') => {
                        dollar_tag = format!("${ident}$");
                        let rest = dollar_tag[1..].to_string();
                        for _ in 0..rest.chars().count() {
                            chars.next();
                        }
                        flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
                        current.push(c);
                        current.push_str(&rest);
                        in_dollar = true;
                        break;
                    }
                    Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => {
                        ident.push(ch);
                    }
                    _ => {
                        flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
                        current.push(c);
                        break;
                    }
                }
            }
            continue;
        }

        if c == '-' && matches!(chars.peek(), Some('-')) {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            current.push(chars.next().expect("peeked comment dash"));
            in_line_comment = true;
            continue;
        }

        if c == '/' && matches!(chars.peek(), Some('*')) {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            current.push(chars.next().expect("peeked comment star"));
            in_block_comment = true;
            continue;
        }

        if c == '\'' {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            in_single = true;
            continue;
        }

        if c == '"' {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            in_double = true;
            continue;
        }

        if c == '`' {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            in_backtick = true;
            continue;
        }

        if c == '[' {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            in_bracket = true;
            continue;
        }

        if c == ';' {
            flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
            current.push(c);
            if trigger_mode && trigger_depth > 0 {
                continue;
            }

            let statement = current.trim().trim_end_matches(';').trim();
            if !statement.is_empty() {
                statements.push(statement.to_string());
            }
            current.clear();
            trigger_mode = false;
            trigger_depth = 0;
            continue;
        }

        if c.is_ascii_alphanumeric() || c == '_' {
            current.push(c);
            word.push(c);
            if !trigger_mode
                && current
                    .trim_start()
                    .to_ascii_uppercase()
                    .starts_with("CREATE TRIGGER")
            {
                trigger_mode = true;
                trigger_depth = 0;
            }
            continue;
        }

        flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
        current.push(c);
    }

    flush_sql_word(&mut word, trigger_mode, &mut trigger_depth);
    let statement = current.trim().trim_end_matches(';').trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }

    statements
}

fn flush_sql_word(word: &mut String, trigger_mode: bool, trigger_depth: &mut usize) {
    if trigger_mode {
        match word.to_ascii_uppercase().as_str() {
            "BEGIN" => *trigger_depth += 1,
            "END" if *trigger_depth > 0 => {
                *trigger_depth -= 1;
            }
            _ => {}
        }
    }
    word.clear();
}

fn validate_migrations<M: Migration>(migrations: &[M]) -> crate::Result<Vec<&M>> {
    let mut sorted: Vec<&M> = migrations.iter().collect();
    sorted.sort_by_key(|migration| migration.version());
    let mut versions = BTreeSet::new();
    for migration in &sorted {
        if !versions.insert(migration.version()) {
            return Err(crate::ormer_error!(
                "Duplicate migration version {}",
                migration.version()
            ));
        }
    }
    Ok(sorted)
}

impl Database {
    pub fn db_type(&self) -> DbType {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(_) => DbType::Sqlite,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.db_type(),
            #[cfg(feature = "mysql")]
            Database::MySQL(_) => DbType::MySQL,
            #[cfg(feature = "mssql")]
            Database::MSSQL(_) => DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(_) => DbType::DuckDB,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => DbType::ClickHouse,
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => DbType::InfluxDB,
        }
    }

    /// 版本化迁移的表级 diff 入口（内部能力，服务 `MigrationStep` 生成与
    /// 版本化迁移编排）。应用启动时的表结构对齐请改用
    /// [`Database::apply_table`]：诊断与执行同源、幂等、策略可控。
    pub fn migrate_table<T: WritableModel>(&self) -> TableMigration<'_, T> {
        TableMigration {
            db: self,
            marker: PhantomData,
            renames: Vec::new(),
            table_name: None,
            allow_inplace_pk: false,
            extra_columns: crate::table_migrate::ExtraPolicy::Drop,
        }
    }

    /// 删除并按当前模型重建一个路由子表。表名参数化复用
    /// `create_table::<T>().with_table_name()` 的完整 DDL 序列（枚举类型 +
    /// CREATE TABLE + create_hypertable + 索引 + 列存压缩），与写入路径
    /// 自动建子表一致。
    #[cfg(feature = "postgresql")]
    pub(crate) async fn recreate_routed_child_table<T: WritableModel>(
        &self,
        child: &str,
    ) -> crate::Result<()> {
        let sql = format!(
            "DROP TABLE IF EXISTS {}",
            crate::model::quote_qualified_identifier(self.db_type(), child)
        );
        self.execute_sql(sql).await?;
        self.create_table::<T>()
            .with_table_name(child)
            .with_route_columnstore()
            .execute()
            .await?;
        Ok(())
    }

    /// 用 `pg_class` 前缀查询枚举 PG 上已存在的路由子表，返回带 schema
    /// 前缀的完整表名（schema 解析与 `check_table_exists` 等既有逻辑一致：
    /// 模型表名可带前缀，无前缀按 `public` 处理）。结果按表名排序，保证
    /// 迁移计划与执行顺序确定性。
    #[cfg(feature = "postgresql")]
    pub(crate) async fn existing_routed_child_tables<T: WritableModel>(
        &self,
    ) -> crate::Result<Vec<String>> {
        let base = T::table_name_for_db(self.db_type());
        let (schema, base_name) = crate::model::split_schema_table_name(base, "public");
        let pattern = routed_child_table_like_pattern(base_name);
        let sql = routed_child_tables_sql(schema, &pattern, base_name);
        let rows = self
            .select_sql::<String>(sql)
            .collect::<Vec<String>>()
            .await?;
        Ok(rows.into_iter().map(|name| format!("{schema}.{name}")).collect())
    }

    /// [`Database::apply_table`] 的策略预设薄糖：Keep 多余列 + Refuse 重建 +
    /// 非并发建索引（与改造前 ensure_table 的执行形态一致）。
    ///
    /// 语义差异说明：改造前遇到"库中有而模型没有"的列时整体报错拒绝；
    /// 现按 Keep 策略保留多余列并继续迁移其余差异（规格书 §4.1 拍板：
    /// Keep = 保留，最保守）。
    pub async fn ensure_table<T: WritableModel>(
        &self,
    ) -> crate::Result<TableEnsureOutcome> {
        let outcome = self
            .apply_table::<T>(&crate::table_migrate::ApplyOptions::strict())
            .await?;
        Ok(ensure_outcome_from_apply(&outcome))
    }

    /// [`Database::apply_table`] 的策略预设薄糖：Drop 多余列 + Allow 重建
    /// （对齐改造前 ensure_table_permissive 语义）。
    pub async fn ensure_table_permissive<T: WritableModel>(
        &self,
    ) -> crate::Result<TableEnsureOutcome> {
        let outcome = self
            .apply_table::<T>(&crate::table_migrate::ApplyOptions::permissive())
            .await?;
        Ok(ensure_outcome_from_apply(&outcome))
    }

    pub fn migrations<'a, M: Migration>(&'a self, migrations: &'a [M]) -> MigrationRunner<'a, M> {
        MigrationRunner::new(self, migrations)
    }

    pub async fn migration_history(&self) -> crate::Result<Vec<MigrationInfo>> {
        #[cfg(feature = "clickhouse")]
        if let Database::ClickHouse(db) = self {
            db.ensure_migration_table().await?;
        }
        self.ensure_migration_table().await?;
        let rows = self.migration_history_rows().await?;
        Ok(rows
            .into_iter()
            .map(|(version, name, checksum)| MigrationInfo::with_checksum(version, name, checksum))
            .collect())
    }

    pub async fn pending_migrations<M: Migration>(
        &self,
        migrations: &[M],
    ) -> crate::Result<Vec<MigrationInfo>> {
        self.ensure_migration_table().await?;
        let applied: BTreeMap<u64, u64> = self
            .migration_history_rows()
            .await?
            .into_iter()
            .map(|(version, _, checksum)| (version, checksum))
            .collect();
        let sorted = validate_migrations(migrations)?;
        let mut pending = Vec::new();
        for migration in sorted {
            if let Some(checksum) = applied.get(&migration.version()) {
                if *checksum != migration.checksum() {
                    return Err(crate::ormer_error!(
                        "Migration {} checksum changed after it was applied",
                        migration.version()
                    ));
                }
                continue;
            }
            pending.push(MigrationInfo::with_checksum(
                migration.version(),
                migration.name(),
                migration.checksum(),
            ));
        }
        Ok(pending)
    }

    pub async fn apply_migrations<M: Migration>(&self, migrations: &[M]) -> crate::Result<usize> {
        #[cfg(feature = "clickhouse")]
        #[allow(irrefutable_let_patterns)]
        if let Database::ClickHouse(db) = self {
            return db.apply_migrations(migrations).await;
        }
        #[cfg(feature = "influxdb")]
        #[allow(irrefutable_let_patterns)]
        if let Database::InfluxDB(db) = self {
            return db.apply_migrations(migrations).await;
        }
        self.ensure_migration_table().await?;
        let applied: BTreeMap<u64, u64> = self
            .migration_history_rows()
            .await?
            .into_iter()
            .map(|(version, _, checksum)| (version, checksum))
            .collect();
        let sorted = validate_migrations(migrations)?;
        let mut pending = Vec::new();
        for migration in sorted {
            if let Some(checksum) = applied.get(&migration.version()) {
                if *checksum != migration.checksum() {
                    return Err(crate::ormer_error!(
                        "Migration {} checksum changed after it was applied",
                        migration.version()
                    ));
                }
                continue;
            }
            pending.push(migration);
        }
        if pending.is_empty() {
            return Ok(0);
        }

        let db_type = self.db_type();
        if !db_type.is_transactional() {
            for migration in &pending {
                execute_steps_nontransactional(self, db_type, &migration.up()).await?;
                let name = migration.name().replace('\'', "''");
                let sql = format!(
                    "INSERT INTO {MIGRATION_TABLE_NAME} (version, name, checksum, rolled_back) VALUES ({}, '{}', '{}', FALSE)",
                    migration.version(),
                    name,
                    migration.checksum()
                );
                self.execute_sql(sql).await?;
            }
            return Ok(pending.len());
        }

        let mut transaction = self.begin().await?;
        let result = async {
            for migration in &pending {
                execute_steps(&mut transaction, db_type, &migration.up()).await?;
                let name = migration.name().replace('\'', "''");
                let sql = format!(
                    "INSERT INTO {MIGRATION_TABLE_NAME} (version, name, checksum) VALUES ({}, '{}', '{}')",
                    migration.version(),
                    name,
                    migration.checksum()
                );
                transaction.execute_sql(&sql).await?;
            }
            Ok::<(), crate::OrmerError>(())
        }
        .await;

        match result {
            Ok(()) => {
                transaction.commit().await?;
                Ok(pending.len())
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    async fn ensure_migration_table(&self) -> crate::Result<()> {
        #[cfg(feature = "clickhouse")]
        #[allow(irrefutable_let_patterns)]
        if let Database::ClickHouse(db) = self {
            return db.ensure_migration_table().await;
        }
        #[cfg(feature = "influxdb")]
        #[allow(irrefutable_let_patterns)]
        if let Database::InfluxDB(_) = self {
            // measurement 由首条写入自动创建
            return Ok(());
        }
        let sql = match self.db_type() {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE_NAME} \
                 (version INTEGER PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ),
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE_NAME} \
                 (version BIGINT PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ),
            #[cfg(feature = "mysql")]
            DbType::MySQL => format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE_NAME} \
                 (version BIGINT PRIMARY KEY, name VARCHAR(255) NOT NULL, checksum VARCHAR(32) NOT NULL, applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => format!(
                "IF OBJECT_ID(N'{MIGRATION_TABLE_NAME}', N'U') IS NULL \
                 CREATE TABLE {MIGRATION_TABLE_NAME} \
                 (version BIGINT NOT NULL PRIMARY KEY, name NVARCHAR(255) NOT NULL, checksum NVARCHAR(32) NOT NULL, applied_at DATETIME2 NOT NULL DEFAULT SYSUTCDATETIME())"
            ),
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE_NAME} \
                 (version BIGINT PRIMARY KEY, name VARCHAR NOT NULL, checksum VARCHAR NOT NULL, applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP)"
            ),
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => unreachable!("handled by the ClickHouse backend"),
            #[cfg(feature = "questdb")]
            DbType::QuestDB => format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATION_TABLE_NAME} \
                 (version LONG, name STRING, checksum STRING, rolled_back BOOLEAN)"
            ),
            // 不可达：InfluxDB 已在函数开头提前报错，仅为 match 穷尽性保留
            #[cfg(feature = "influxdb")]
            DbType::InfluxDB => String::new(),
        };
        self.execute_sql(&sql).await?;
        Ok(())
    }

    async fn migration_history_rows(&self) -> crate::Result<Vec<(u64, String, u64)>> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.migration_history().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.migration_history().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.migration_history().await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.migration_history().await,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.migration_history().await,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => Ok(db
                .migration_history()
                .await?
                .into_iter()
                .map(|migration| (migration.version, migration.name, migration.checksum))
                .collect()),
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => Ok(db
                .migration_history()
                .await?
                .into_iter()
                .map(|migration| (migration.version, migration.name, migration.checksum))
                .collect()),
        }
    }

    // 仅启用 clickhouse/influxdb 时所有分支都 diverge，table_name 不被使用属预期
    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unused_variables)
    )]
    pub(crate) async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<crate::migration::SchemaColumn>>> {
        match self {
            #[cfg(feature = "questdb")]
            Database::PostgreSQL(db) if db.db_type().is_questdb() => {
                db.questdb_schema_columns(table_name).await
            }
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.schema_columns(table_name).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.schema_columns(table_name).await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.schema_columns(table_name).await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.schema_columns(table_name).await,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.schema_columns(table_name).await,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::ClickHouse,
                feature: "migrate_table schema introspection",
            }),
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => Err(crate::OrmerError::UnsupportedFeature {
                backend: DbType::InfluxDB,
                feature: "migrate_table schema introspection",
            }),
        }
    }

    /// 迁移用的表级自省（索引/默认值/外键）。复用各后端 db_first 的既有
    /// 自省结构，不自建第二套抽象；找不到目标表时返回 None。
    ///
    /// 仅启用 clickhouse/influxdb 时所有分支都 diverge，尾部代码不可达，
    /// 这是 cfg 组合下的预期形态。
    #[cfg_attr(
        not(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "mssql",
            feature = "duckdb"
        )),
        allow(unreachable_code, unused_variables)
    )]
    pub(crate) async fn db_first_table_for(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<DbFirstTable>> {
        let (schema, name) = split_qualified_table_name(table_name);
        // 仅启用 clickhouse/influxdb 时所有分支都 diverge，需显式标注类型
        let tables: Vec<DbFirstTable> = match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.db_first_tables(None).await?,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.db_first_tables(None).await?,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.db_first_tables(schema).await?,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.db_first_tables(None).await?,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: DbType::ClickHouse,
                    feature: "migrate_table table introspection",
                });
            }
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => {
                return Err(crate::OrmerError::UnsupportedFeature {
                    backend: DbType::InfluxDB,
                    feature: "migrate_table table introspection",
                });
            }
        };
        Ok(tables.into_iter().find(|table| {
            table.name == name
                && schema.is_none_or(|schema| {
                    table.schema.as_deref().is_some_and(|actual| actual == schema)
                })
        }))
    }

    /// 统计表中某列的存量 NULL 行数，用于 NOT NULL 收紧前的预检。
    async fn null_count(
        &self,
        db_type: DbType,
        table_name: &str,
        column_name: &str,
    ) -> crate::Result<u64> {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} IS NULL",
            crate::model::quote_qualified_identifier(db_type, table_name),
            crate::model::quote_identifier(db_type, column_name)
        );
        let rows = self
            .select_sql::<i64>(sql)
            .collect::<Vec<i64>>()
            .await?;
        Ok(rows.into_iter().next().unwrap_or(0).max(0) as u64)
    }

    async fn validate_hypertable_for_migration<T: WritableModel>(&self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "questdb")]
            Database::PostgreSQL(db) if db.db_type().is_questdb() => {
                // QuestDB 无 TimescaleDB 元数据，designated timestamp 随建表生成，无需校验
                let _ = db;
                Ok(())
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.validate_hypertable_for_migration::<T>().await,
            #[cfg(feature = "sqlite")]
            Database::Sqlite(_) => Ok(()),
            #[cfg(feature = "mysql")]
            Database::MySQL(_) => Ok(()),
            #[cfg(feature = "mssql")]
            Database::MSSQL(_) => Ok(()),
            #[cfg(feature = "duckdb")]
            Database::DuckDB(_) => Ok(()),
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => Ok(()),
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(_) => Ok(()),
        }
    }
}

/// Backend-independent table-column metadata used by schema planning.
#[derive(Debug, Clone)]
pub(crate) struct SchemaColumn {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) nullable: bool,
    pub(crate) primary_key: bool,
    #[allow(dead_code)]
    pub(crate) compression: Option<String>,
}

/// Keep a deterministic map available to backend implementations without
/// exposing driver-specific row types through the public API.
#[allow(dead_code)]
pub(crate) fn schema_column(
    name: impl Into<String>,
    type_name: impl Into<String>,
    nullable: bool,
    primary_key: bool,
) -> SchemaColumn {
    schema_column_with_compression(name, type_name, nullable, primary_key, None)
}

pub(crate) fn schema_column_with_compression(
    name: impl Into<String>,
    type_name: impl Into<String>,
    nullable: bool,
    primary_key: bool,
    compression: Option<String>,
) -> SchemaColumn {
    SchemaColumn {
        name: name.into(),
        type_name: type_name.into(),
        nullable,
        primary_key,
        compression,
    }
}

#[cfg(all(test, feature = "postgresql"))]
mod compression_tests {
    use super::{CompressionAlgorithm, MigrationStep, column_compression_migration_step};
    use crate::abstract_layer::DbType;

    #[test]
    fn postgres_compression_migration_renders_column_step() {
        let step = column_compression_migration_step(
            DbType::PostgreSQL,
            "public.documents",
            "payload",
            Some(CompressionAlgorithm::Lz4),
        )
        .expect("compression migration step")
        .expect("postgres compression migration step");
        assert_eq!(
            step.sql(DbType::PostgreSQL).unwrap(),
            "ALTER TABLE public.documents ALTER COLUMN payload SET COMPRESSION lz4"
        );
        assert!(matches!(step, MigrationStep::Sql { .. }));
    }
}

#[cfg(all(test, feature = "duckdb"))]
mod duckdb_tests {
    use super::MigrationStep;
    use crate::abstract_layer::DbType;

    #[test]
    fn duckdb_alter_column_renders_type_and_nullability_changes() {
        let type_change = MigrationStep::AlterColumn {
            table: "events".to_string(),
            column: "kind".to_string(),
            definition: "SET DATA TYPE BIGINT".to_string(),
            using: None,
        };
        assert_eq!(
            type_change.sql(DbType::DuckDB).unwrap(),
            "ALTER TABLE events ALTER COLUMN kind SET DATA TYPE BIGINT"
        );

        let nullable_change = MigrationStep::AlterColumn {
            table: "events".to_string(),
            column: "kind".to_string(),
            definition: "SET NOT NULL".to_string(),
            using: None,
        };
        assert_eq!(
            nullable_change.sql(DbType::DuckDB).unwrap(),
            "ALTER TABLE events ALTER COLUMN kind SET NOT NULL"
        );
    }
}

#[cfg(all(test, feature = "clickhouse"))]
mod clickhouse_tests {
    use super::MigrationStep;
    use crate::abstract_layer::DbType;

    #[test]
    fn clickhouse_alter_column_renders_modify_column() {
        let step = MigrationStep::AlterColumn {
            table: "events".to_string(),
            column: "kind".to_string(),
            definition: "Nullable(String)".to_string(),
            using: None,
        };
        assert_eq!(
            step.sql(DbType::ClickHouse).unwrap(),
            "ALTER TABLE events MODIFY COLUMN kind Nullable(String)"
        );
    }
}

#[cfg(all(test, feature = "mysql"))]
mod mysql_compression_tests {
    use super::{CompressionAlgorithm, column_compression_migration_step};
    use crate::abstract_layer::DbType;

    #[test]
    fn mysql_compression_migration_renders_table_step() {
        let step = column_compression_migration_step(
            DbType::MySQL,
            "documents",
            "",
            Some(CompressionAlgorithm::Lz4),
        )
        .expect("compression migration step")
        .expect("mysql compression migration step");
        assert_eq!(
            step.sql(DbType::MySQL).unwrap(),
            "ALTER TABLE documents COMPRESSION='LZ4'"
        );
    }
}

#[cfg(all(test, feature = "postgresql"))]
mod postgres_enum_add_value_tests {
    use super::{DbType, MigrationPlan, MigrationStep, push_enum_add_value_steps};
    use crate::db_first::{DbFirstColumn, DbFirstTable};

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ormer::ModelEnum)]
    enum PlanTaskKind {
        Charge,
        #[default]
        Load,
        Unload,
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "collect.plan_enum_tasks"]
    struct PlanEnumTask {
        #[primary]
        id: i64,
        kind: PlanTaskKind,
    }

    fn db_first_table(column: Option<DbFirstColumn>) -> DbFirstTable {
        DbFirstTable {
            schema: Some("collect".to_string()),
            name: "plan_enum_tasks".to_string(),
            columns: column.into_iter().collect(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }

    fn actual_column(type_name: &str, variants: &[&str]) -> DbFirstColumn {
        DbFirstColumn {
            name: "kind".to_string(),
            type_name: type_name.to_string(),
            nullable: false,
            primary_key: false,
            auto_increment: false,
            enum_variants: variants.iter().map(|variant| variant.to_string()).collect(),
            default: None,
        }
    }

    fn alter_type_definitions(table: &DbFirstTable) -> Vec<String> {
        let mut plan = MigrationPlan::new("collect.plan_enum_tasks", DbType::PostgreSQL);
        push_enum_add_value_steps::<PlanEnumTask>(table, &mut plan).expect("enum diff steps");
        plan.steps()
            .iter()
            .filter_map(|step| match step {
                MigrationStep::AlterType { definition, .. } => Some(definition.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn missing_variant_becomes_add_value_step() {
        let definitions = alter_type_definitions(&db_first_table(Some(actual_column(
            "plan_task_kind",
            &["Charge", "Load"],
        ))));
        assert_eq!(
            definitions,
            vec!["ALTER TYPE plan_task_kind ADD VALUE IF NOT EXISTS 'Unload'"]
        );
    }

    #[test]
    fn matching_variants_produce_no_steps() {
        let definitions = alter_type_definitions(&db_first_table(Some(actual_column(
            "plan_task_kind",
            &["Charge", "Load", "Unload"],
        ))));
        assert!(definitions.is_empty());
    }

    #[test]
    fn non_enum_column_is_left_to_type_diff() {
        let definitions =
            alter_type_definitions(&db_first_table(Some(actual_column("text", &[]))));
        assert!(definitions.is_empty());
    }

    #[test]
    fn absent_column_is_left_to_add_column() {
        let definitions = alter_type_definitions(&db_first_table(None));
        assert!(definitions.is_empty());
    }
}

#[cfg(all(test, feature = "postgresql"))]
mod routed_child_migration_tests {
    use super::{routed_child_table_like_pattern, routed_child_tables_sql};
    use crate::abstract_layer::DbType;
    use crate::model::{routed_model_table_name_for_db, TableRoute};

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "routed_child_events"]
    struct RoutedChildEvent {
        #[primary]
        id: i64,
        #[hypertable(std::time::Duration::from_secs(86_400))]
        recorded_at: chrono::DateTime<chrono::Utc>,
        #[hypertable(route)]
        tenant: String,
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "collect.routed_child_events"]
    struct CollectRoutedChildEvent {
        #[primary]
        id: i64,
        #[hypertable(std::time::Duration::from_secs(86_400))]
        recorded_at: chrono::DateTime<chrono::Utc>,
        #[hypertable(route)]
        tenant: String,
    }

    fn route(tenant: &str) -> TableRoute {
        TableRoute::new().with("tenant", tenant.to_string())
    }

    /// 路由子表命名约定与枚举模式的衔接：模型带 route key 时写入路径落到
    /// `{基础表名}_{路由值}`，迁移侧的 `pg_class` 前缀查询按同一约定匹配。
    #[test]
    fn route_key_naming_convention_matches_enumeration_pattern() {
        assert_eq!(
            <RoutedChildEvent as ormer::Model>::hypertable_route_key(),
            Some("tenant")
        );
        assert_eq!(
            routed_model_table_name_for_db::<RoutedChildEvent>(DbType::PostgreSQL, &route("val"))
                .unwrap(),
            "routed_child_events_val"
        );
        // schema 前缀保留在子表名里；枚举侧按 schema 分段后对表名做前缀匹配
        assert_eq!(
            routed_model_table_name_for_db::<CollectRoutedChildEvent>(
                DbType::PostgreSQL,
                &route("val")
            )
            .unwrap(),
            "collect.routed_child_events_val"
        );
        let (schema, base_name) =
            crate::model::split_schema_table_name("collect.routed_child_events", "public");
        assert_eq!((schema, base_name), ("collect", "routed_child_events"));
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "plain_no_route_events"]
    struct PlainNoRouteEvent {
        #[primary]
        id: i64,
        name: String,
    }

    #[test]
    fn route_free_models_do_not_opt_into_child_migration() {
        assert_eq!(
            <PlainNoRouteEvent as ormer::Model>::hypertable_route_key(),
            None
        );
    }

    /// LIKE 模式转义：基础表名中的 `_`、`%`、`\` 按字面匹配，避免
    /// `aaaxval` 这类同前缀表被 `{base}_%` 误判为路由子表。
    #[test]
    fn child_table_like_pattern_escapes_like_wildcards() {
        assert_eq!(
            routed_child_table_like_pattern("routed_child_events"),
            r"routed\_child\_events\_%"
        );
        assert_eq!(routed_child_table_like_pattern("events"), r"events\_%");
        assert_eq!(
            routed_child_table_like_pattern("we%ird\\name"),
            r"we\%ird\\name\_%"
        );
    }

    /// 枚举 SQL 锁定：固定目标 schema、`{base}_%` 前缀匹配、排除基础表
    /// 本身、只取表对象并按表名排序（计划与执行顺序确定）。
    #[test]
    fn child_table_enumeration_sql_pins_schema_and_excludes_base() {
        let pattern = routed_child_table_like_pattern("routed_child_events");
        let sql = routed_child_tables_sql("public", &pattern, "routed_child_events");
        assert_eq!(
            sql,
            "SELECT c.relname \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'public' \
               AND c.relname LIKE 'routed\\_child\\_events\\_%' \
               AND c.relname <> 'routed_child_events' \
               AND c.relkind IN ('r', 'p') \
             ORDER BY c.relname"
        );
    }

    /// 模型表名带 schema 前缀时，枚举 SQL 钉住目标 schema 而不是依赖
    /// search_path，与 `check_table_exists` 等既有 schema 处理一致。
    #[test]
    fn child_table_enumeration_sql_uses_schema_qualified_base() {
        let pattern = routed_child_table_like_pattern("events");
        let sql = routed_child_tables_sql("collect", &pattern, "events");
        assert!(sql.contains("nspname = 'collect'"));
        assert!(sql.contains(r"LIKE 'events\_%'"));
        assert!(sql.contains("AND c.relname <> 'events'"));
    }
}

/// DropIndex 渲染必须以表所在 schema 限定索引名：业务表多在非默认 schema，
/// 裸索引名叠加 IF EXISTS 会在 search_path 不含该 schema 时静默空转——
/// 删除未生效而计划视为已执行，复验不收敛就会误触发删表重建。
#[cfg(all(test, feature = "postgresql"))]
mod drop_index_render_tests {
    use super::{DbType, MigrationStep};

    #[test]
    fn drop_index_renders_schema_qualified_name() {
        let step = MigrationStep::DropIndex {
            table: "collect.collect_exception_logs".to_string(),
            name: "collect_exception_logs_event_time_idx".to_string(),
        };
        let sql = step.sql(DbType::PostgreSQL).expect("render drop index");
        assert_eq!(
            sql,
            "DROP INDEX IF EXISTS collect.collect_exception_logs_event_time_idx"
        );
    }

    #[test]
    fn drop_index_without_schema_keeps_bare_name() {
        let step = MigrationStep::DropIndex {
            table: "plain_events".to_string(),
            name: "plain_events_time_idx".to_string(),
        };
        let sql = step.sql(DbType::PostgreSQL).expect("render drop index");
        assert_eq!(sql, "DROP INDEX IF EXISTS plain_events_time_idx");
    }
}

/// 建表计划必须随带配套步骤：`CreateTable` 渲染只含 CREATE TABLE/索引 DDL，
/// 计划缺前置枚举类型会直接报缺类型，缺后置 create_hypertable 则表永远不是
/// 超表、复验必报 Hypertable mismatch。
#[cfg(all(test, feature = "postgresql"))]
mod create_table_bootstrap_tests {
    use super::{DbType, MigrationStep, create_table_bootstrap_steps};
    use crate::model::{ColumnSchema, WritableModel};

    /// 手写列元数据：时间列（7 天分片）+ 空间分区列 + 一个枚举列（task_kind）。
    fn mock_columns() -> Vec<ColumnSchema> {
        vec![
            ColumnSchema {
                rust_name: "event_time",
                name: "event_time",
                rust_type: "DateTimeUtc",
                is_primary: true,
                is_auto_increment: false,
                is_nullable: false,
                unique_group: None,
                unique_name: None,
                is_indexed: false,
                index_group: None,
                index_name: None,
                index_order: None,
                index_where: None,
                foreign_key: None,
                enum_variants: None,
                data_type: None,
                db_value_type: None,
                default: None,
                check: None,
                hypertable: Some(std::time::Duration::from_secs(7 * 86_400)),
                hypertable_space: None,
                compress: false,
                compression: None,
                index_method: None,
                index_expression: None,
                index_columns: None,
            },
            ColumnSchema {
                rust_name: "project_id",
                name: "project_id",
                rust_type: "String",
                is_primary: true,
                is_auto_increment: false,
                is_nullable: false,
                unique_group: None,
                unique_name: None,
                is_indexed: false,
                index_group: None,
                index_name: None,
                index_order: None,
                index_where: None,
                foreign_key: None,
                enum_variants: None,
                data_type: None,
                db_value_type: None,
                default: None,
                check: None,
                hypertable: None,
                hypertable_space: Some(4),
                compress: false,
                compression: None,
                index_method: None,
                index_expression: None,
                index_columns: None,
            },
            ColumnSchema {
                rust_name: "task_kind",
                name: "task_kind",
                rust_type: "TaskKind",
                is_primary: false,
                is_auto_increment: false,
                is_nullable: false,
                unique_group: None,
                unique_name: None,
                is_indexed: false,
                index_group: None,
                index_name: None,
                index_order: None,
                index_where: None,
                foreign_key: None,
                enum_variants: Some(&["loading", "unloading"]),
                data_type: None,
                db_value_type: None,
                default: None,
                check: None,
                hypertable: None,
                hypertable_space: None,
                compress: false,
                compression: None,
                index_method: None,
                index_expression: None,
                index_columns: None,
            },
        ]
    }

    struct MockHypertableEnumModel;

    impl crate::Model for MockHypertableEnumModel {
        const TABLE_NAME: &'static str = "mock_hypertable_enum_models";
        const COLUMNS: &'static [&'static str] = &["event_time", "project_id", "task_kind"];
        const COLUMN_SCHEMA: &'static [ColumnSchema] = &[];

        type AutoIncrementKeyType = ();
        type QueryBuilder = ();
        type Where = ();
        type Update = ();

        fn query() -> Self::QueryBuilder {}

        fn select() -> Self::QueryBuilder {}

        fn from_row(_row: &crate::Row) -> crate::Result<Self> {
            unreachable!()
        }

        fn from_row_values(_values: &[crate::Value]) -> crate::Result<Self> {
            unreachable!()
        }

        fn field_values(&self) -> Vec<crate::Value> {
            Vec::new()
        }

        fn primary_key_values(&self) -> Vec<crate::Value> {
            Vec::new()
        }

        fn column_schema() -> Vec<ColumnSchema> {
            mock_columns()
        }
    }

    impl WritableModel for MockHypertableEnumModel {}

    #[test]
    fn bootstrap_steps_cover_enum_extension_and_hypertable() {
        let (pre, post) = create_table_bootstrap_steps::<MockHypertableEnumModel>(
            DbType::PostgreSQL,
            "collect.mock_hypertable_enum_models",
        );
        // 前置：枚举类型 DO 块 + timescaledb 扩展
        assert_eq!(pre.len(), 2);
        let MigrationStep::Sql { sql: enum_sql } = &pre[0] else {
            panic!("expected Sql step");
        };
        assert_eq!(
            enum_sql,
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'task_kind') THEN CREATE TYPE task_kind AS ENUM ('loading', 'unloading'); END IF; END $$"
        );
        let MigrationStep::Sql { sql: extension } = &pre[1] else {
            panic!("expected Sql step");
        };
        assert_eq!(extension, "CREATE EXTENSION IF NOT EXISTS timescaledb");
        // 后置：create_hypertable 转换
        assert_eq!(post.len(), 1);
        let MigrationStep::Sql { sql: hypertable } = &post[0] else {
            panic!("expected Sql step");
        };
        assert_eq!(
            hypertable,
            "SELECT create_hypertable('collect.mock_hypertable_enum_models', 'event_time', \
             chunk_time_interval => INTERVAL '7 days', partitioning_column => 'project_id', \
             number_partitions => 4, if_not_exists => TRUE, migrate_data => TRUE, \
             create_default_indexes => FALSE)"
        );
    }

    #[test]
    fn plain_model_gets_no_bootstrap_steps() {
        struct MockPlainModel;

        impl crate::Model for MockPlainModel {
            const TABLE_NAME: &'static str = "mock_plain_models";
            const COLUMNS: &'static [&'static str] = &["id", "name"];
            const COLUMN_SCHEMA: &'static [ColumnSchema] = &[];

            type AutoIncrementKeyType = ();
            type QueryBuilder = ();
            type Where = ();
            type Update = ();

            fn query() -> Self::QueryBuilder {}

            fn select() -> Self::QueryBuilder {}

            fn from_row(_row: &crate::Row) -> crate::Result<Self> {
                unreachable!()
            }

            fn from_row_values(_values: &[crate::Value]) -> crate::Result<Self> {
                unreachable!()
            }

            fn field_values(&self) -> Vec<crate::Value> {
                Vec::new()
            }

            fn primary_key_values(&self) -> Vec<crate::Value> {
                Vec::new()
            }
        }

        impl WritableModel for MockPlainModel {}

        let (pre, post) =
            create_table_bootstrap_steps::<MockPlainModel>(DbType::PostgreSQL, "mock_plain_models");
        assert!(pre.is_empty());
        assert!(post.is_empty());
    }
}

/// 拆分器必须把 PG dollar-quote 体（DO 块等）当单条语句：体内分号、引号
/// 不参与拆分，否则枚举类型创建会被切断成不完整 SQL。
#[cfg(all(test, feature = "postgresql"))]
mod split_sql_dollar_tests {
    use super::split_sql_statements;

    #[test]
    fn dollar_quoted_do_block_stays_intact() {
        let sql = "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'task_kind') \
                   THEN CREATE TYPE task_kind AS ENUM ('a', 'b'); END IF; END $$; SELECT 1; SELECT 2";
        let statements = split_sql_statements(sql);
        assert_eq!(statements.len(), 3);
        assert!(statements[0].starts_with("DO $$"));
        assert!(statements[0].ends_with("END $$"));
        assert!(statements[0].contains("AS ENUM ('a', 'b');"));
        assert_eq!(statements[1], "SELECT 1");
        assert_eq!(statements[2], "SELECT 2");
    }

    #[test]
    fn tagged_dollar_quote_and_placeholders() {
        // $fn$ 定界的体内分号不拆分；$1 是参数占位符不是 dollar quote。
        let sql = "$fn$ body; with $1 maybe $fn$; SELECT $1::int";
        let statements = split_sql_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("$fn$"));
        assert!(statements[0].ends_with("$fn$"));
        assert_eq!(statements[1], "SELECT $1::int");
    }
}
