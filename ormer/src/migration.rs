//! Versioned schema migration primitives.
//!
//! The migration API deliberately keeps migration steps structured until the
//! final dialect rendering stage.  Applications can therefore inspect a plan
//! before executing it, while small hand-written migrations remain possible.

use crate::abstract_layer::DbType;
#[cfg(any(feature = "sqlite", feature = "duckdb"))]
use crate::abstract_layer::common::common_helpers;
use crate::abstract_layer::common::{Database, Transaction};
use crate::db_first::{DbFirstIndex, DbFirstTable};
#[cfg(any(feature = "postgresql", feature = "mysql"))]
use crate::model::CompressionAlgorithm;
use crate::model::{ColumnSchema, WritableModel};
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
fn is_schema_rebuild_error(err: &crate::OrmerError) -> bool {
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
        let columns = |names: &[String]| {
            names
                .iter()
                .map(|name| crate::model::quote_identifier(db_type, name))
                .collect::<Vec<_>>()
                .join(", ")
        };

        match self {
            Self::CreateType { definition, .. } | Self::AlterType { definition, .. } => {
                Ok(definition.clone())
            }
            Self::CreateTable { definition, .. } => Ok(definition.clone()),
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
            Self::CreateIndex {
                name,
                columns: index_columns,
                unique,
                ..
            } => {
                let unique = if *unique { " UNIQUE" } else { "" };
                let if_not_exists = match db_type {
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => "",
                    #[cfg(feature = "mssql")]
                    DbType::MSSQL => "",
                    #[cfg(feature = "sqlite")]
                    DbType::Sqlite => " IF NOT EXISTS",
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => " IF NOT EXISTS",
                    #[cfg(feature = "questdb")]
                    DbType::QuestDB => " IF NOT EXISTS",
                    #[cfg(any(feature = "duckdb", feature = "clickhouse", feature = "influxdb"))]
                    _ => " IF NOT EXISTS",
                };
                Ok(format!(
                    "CREATE{unique} INDEX{if_not_exists} {} ON {table} ({})",
                    crate::model::quote_identifier(db_type, name),
                    columns(index_columns)
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
                    // SQLite / PostgreSQL / DuckDB：索引名在库内唯一，支持 IF EXISTS
                    #[allow(unreachable_patterns)]
                    _ => Ok(format!("DROP INDEX IF EXISTS {name}")),
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

fn table_name(step: &MigrationStep) -> &str {
    match step {
        MigrationStep::AddColumn { table, .. }
        | MigrationStep::DropColumn { table, .. }
        | MigrationStep::RenameColumn { table, .. }
        | MigrationStep::BackfillColumn { table, .. }
        | MigrationStep::AlterColumn { table, .. }
        | MigrationStep::AddConstraint { table, .. }
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
    steps: Vec<MigrationStep>,
    warnings: Vec<String>,
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

    fn push(&mut self, step: MigrationStep) {
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

/// Builder returned by `Database::migrate_table`.
pub struct TableMigration<'a, T: WritableModel> {
    db: &'a Database,
    marker: PhantomData<T>,
    /// 用户显式标注的列重命名（old, new）。diff 阶段命中"删 old 列 + 加 new 列"
    /// 且存在对应标注时，生成 RenameColumn 而非删列加列，避免数据丢失。
    renames: Vec<(String, String)>,
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

    pub async fn plan(&self) -> crate::Result<MigrationPlan> {
        let db_type = self.db.db_type();
        // QuestDB 没有主键/NOT NULL 约束，自省结果不参与这两项比较
        #[cfg(feature = "questdb")]
        let questdb = matches!(db_type, DbType::QuestDB);
        #[cfg(not(feature = "questdb"))]
        let questdb = false;
        let table_name = T::table_name_for_db(db_type);
        let mut plan = MigrationPlan::new(table_name, db_type);
        let actual = self.db.schema_columns(table_name).await?;

        let Some(mut actual) = actual else {
            plan.push(MigrationStep::CreateTable {
                table: table_name.to_string(),
                definition: crate::generate_create_table_sql::<T>(db_type)?,
            });
            return Ok(plan);
        };

        self.db.validate_hypertable_for_migration::<T>().await?;

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

        // ---- 索引集合 diff：期望集合（模型声明）vs 实际集合（库内自省）----
        // 期望有实际没有 → CreateIndex；期望没有实际有 → DropIndex。
        // 覆盖"给已有列加 #[index]"与"从模型删除 #[index]"两种变更，
        // 不再局限于本次新增的列。
        //
        // QuestDB 例外：索引内联在建表语句（SYMBOL 列 INDEX），建表时已随列
        // 生成，视为已有索引；且无独立 CREATE/DROP INDEX DDL，跳过 diff。
        let mut available_columns = actual_names.clone();
        available_columns.extend(added_columns.iter().copied());
        let mut introspected_table = None;
        if !questdb {
            match self.db.db_first_table_for(table_name).await? {
                Some(db_first_table) => {
                    push_index_diff_steps::<T>(
                        db_type,
                        table_name,
                        &available_columns,
                        &db_first_table,
                        &mut plan,
                    )?;
                    introspected_table = Some(db_first_table);
                }
                None => {
                    plan.warnings.push(format!(
                        "metadata for table {table_name} was unavailable; \
                         index and default differences were not evaluated"
                    ));
                }
            }
        }

        // 外键只为本次新增的列补建（已有列的外键差异检测属后续收敛项）
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

        // 主键期望值使用有效主键列：TimescaleDB 空间分区超表的主键包含分区列
        let effective_primary_keys = crate::model::effective_primary_key_columns::<T>(db_type);
        for expected in T::COLUMN_SCHEMA {
            let Some(actual) = actual_by_name.get(expected.name) else {
                continue;
            };
            let expected_primary = effective_primary_keys.contains(&expected.name);
            if !questdb && actual.primary_key != expected_primary {
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

        // ---- #[default] 变更 diff：模型声明 vs 库内默认值 ----
        // CHECK 约束：自省结构（DbFirstTable）不包含 check 定义，模型表达式
        // 与库内约束无法可靠比对（PG 会以 `((expr))` 全限定形式存储），
        // 该项检测缺失，此处记录 warning 说明覆盖范围。
        if !questdb && T::COLUMN_SCHEMA.iter().any(|column| column.check.is_some()) {
            plan.warnings.push(
                "check-constraint differences are not detected by automatic migrations".to_string(),
            );
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
            steps.extend(plan.steps.drain(..));
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
    async fn execute_plan(&self, plan: &MigrationPlan) -> crate::Result<()> {
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

fn index_migration_step(
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

    let unique_sql = if unique { " UNIQUE" } else { "" };
    let if_not_exists = match db_type {
        #[cfg(feature = "mysql")]
        DbType::MySQL => "",
        #[cfg(feature = "mssql")]
        DbType::MSSQL => "",
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => " IF NOT EXISTS",
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => " IF NOT EXISTS",
        #[cfg(feature = "questdb")]
        DbType::QuestDB => " IF NOT EXISTS",
        #[cfg(any(feature = "duckdb", feature = "clickhouse", feature = "influxdb"))]
        _ => " IF NOT EXISTS",
    };
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
    Ok(MigrationStep::Sql {
        sql: format!(
            "CREATE{unique_sql} INDEX{if_not_exists} {} ON {} ({columns_sql}){predicate}",
            crate::model::quote_identifier(db_type, &name),
            crate::model::quote_qualified_identifier(db_type, table),
        ),
    })
}

/// 模型声明的一个索引/唯一约束（diff 的"期望集合"元素）。
struct ExpectedIndexDef<'a> {
    name: Option<&'a str>,
    columns: Vec<&'a ColumnSchema>,
    unique: bool,
}

impl<'a> ExpectedIndexDef<'a> {
    /// 声明了 method/expression/列清单覆盖（全文、GIN、函数索引等）时无法
    /// 按列集合与自省结果可靠比对，标记为 special：跳过比对与创建。
    fn special(&self) -> bool {
        self.columns.iter().any(|column| {
            column.index_method.is_some()
                || column.index_expression.is_some()
                || column.index_columns.is_some()
        })
    }

    fn special_columns(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.columns
            .iter()
            .filter(|column| {
                column.index_method.is_some()
                    || column.index_expression.is_some()
                    || column.index_columns.is_some()
            })
            .map(|column| column.name)
    }
}

/// 从模型列声明构建期望索引集合。
///
/// 分组口径与建表路径（`generate_indexes_with_name` + 内联 UNIQUE）及校验
/// 路径（`db_first::validate_model_constraints`）一致：未分组单列索引按列
/// 独立成组，`index_group`/`unique_group` 各自聚合。
fn expected_index_defs<T: WritableModel>() -> Vec<ExpectedIndexDef<'static>> {
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

/// 自省得到的索引列名规整：取首个空白分隔的 token 并去掉引号包裹
/// （SQLite/DuckDB 解析建表语句时列名可能带引号或 `DESC` 后缀）。
fn index_column_name(name: &str) -> &str {
    name.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(['"', '`', '[', ']'])
}

/// 期望索引定义与自省索引是否一致（列集合、顺序、唯一性；名称仅在模型
/// 显式声明时比对，与 validate_model_constraints 的口径一致）。
fn index_def_matches(db_type: DbType, expected: &ExpectedIndexDef<'_>, actual: &DbFirstIndex) -> bool {
    if expected.unique != actual.unique {
        return false;
    }
    if let Some(name) = expected.name {
        // SQLite 内联 UNIQUE 解析出的名称由建表语句决定，不可靠，跳过名称比对
        let sqlite_unique = {
            #[cfg(feature = "sqlite")]
            {
                expected.unique && matches!(db_type, DbType::Sqlite)
            }
            #[cfg(not(feature = "sqlite"))]
            {
                false
            }
        };
        if !sqlite_unique && name != index_column_name(&actual.name) {
            return false;
        }
    }
    if expected.columns.len() != actual.columns.len() {
        return false;
    }
    // SQLite/DuckDB 的索引解析器不识别 DESC 标志，降序比较仅对解析器
    // 能给出方向的后端生效
    let compares_descending = match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => true,
        #[cfg(feature = "mysql")]
        DbType::MySQL => true,
        #[cfg(feature = "mssql")]
        DbType::MSSQL => true,
        #[allow(unreachable_patterns)]
        _ => false,
    };
    expected.columns.iter().zip(&actual.columns).all(|(expected, actual)| {
        expected.name == index_column_name(&actual.name)
            && (!compares_descending
                || (expected.index_order == Some("DESC")) == actual.descending)
    })
}

/// 比较期望索引集合与自省索引集合，生成 CreateIndex / DropIndex 步骤。
///
/// - 期望有实际没有 → `CreateIndex`（复用 [`index_migration_step`] 渲染，
///   支持复合、降序与部分索引谓词）；
/// - 实际有期望没有 → `DropIndex`。外键后备索引（如 MySQL 为 FK 列自动
///   创建的索引）与特殊索引（全文/函数等）涉及列保留不动，记录 warning。
fn push_index_diff_steps<T: WritableModel>(
    db_type: DbType,
    table_name: &str,
    available_columns: &BTreeSet<&str>,
    db_first_table: &DbFirstTable,
    plan: &mut MigrationPlan,
) -> crate::Result<()> {
    let expected = expected_index_defs::<T>();
    let actual_indexes = &db_first_table.indexes;

    let mut consumed = vec![false; actual_indexes.len()];
    for expected in &expected {
        if expected.special() {
            plan.warnings.push(format!(
                "index on ({}) declares a method/expression override; \
                 its presence is not diffed automatically",
                expected
                    .columns
                    .iter()
                    .map(|column| column.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        }
        let matched = actual_indexes
            .iter()
            .enumerate()
            .find(|(position, actual)| {
                !consumed[*position] && index_def_matches(db_type, expected, actual)
            });
        if let Some((position, _)) = matched {
            consumed[position] = true;
            continue;
        }
        // 期望有实际没有 → 创建。索引列必须全部存在于表中（含本次新增列）。
        if !expected
            .columns
            .iter()
            .all(|column| available_columns.contains(column.name))
        {
            plan.warnings.push(format!(
                "skipping index on ({}) because some columns are missing from table {table_name}",
                expected
                    .columns
                    .iter()
                    .map(|column| column.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        }
        let name = expected
            .name
            .map(ToString::to_string)
            .unwrap_or_else(|| default_index_name(table_name, expected));
        plan.push(index_migration_step(
            db_type,
            name,
            table_name,
            &expected.columns,
            expected.unique,
        )?);
    }

    // 实际有期望没有 → 删除。以下两类保留不动：
    // 1. 外键后备索引（MySQL 会为 FK 列自动建索引，删除会破坏约束）；
    // 2. 涉及特殊索引声明（method/expression/列清单覆盖）列的索引。
    let foreign_key_columns: BTreeSet<&str> = db_first_table
        .foreign_keys
        .iter()
        .map(|foreign_key| foreign_key.column.as_str())
        .collect();
    let special_columns: BTreeSet<&str> = expected
        .iter()
        .flat_map(ExpectedIndexDef::special_columns)
        .collect();
    for (position, index) in actual_indexes.iter().enumerate() {
        if consumed[position] || index.name.is_empty() {
            continue;
        }
        let leading_column = index
            .columns
            .first()
            .map(|column| index_column_name(&column.name))
            .unwrap_or("");
        if !leading_column.is_empty() && foreign_key_columns.contains(leading_column) {
            plan.warnings.push(format!(
                "keeping index {} because it backs a foreign key on column {leading_column}",
                index.name
            ));
            continue;
        }
        if index.columns.iter().any(|column| {
            special_columns.contains(index_column_name(&column.name))
        }) {
            plan.warnings.push(format!(
                "keeping index {} because it may implement a method/expression index declaration",
                index.name
            ));
            continue;
        }
        plan.warnings
            .push(format!("dropping index {} because it is not declared in the model", index.name));
        plan.push(MigrationStep::DropIndex {
            name: index.name.clone(),
            table: table_name.to_string(),
        });
    }
    Ok(())
}

/// 未显式命名时的索引命名，与建表路径的默认命名保持一致：
/// 单列 `idx_{table}_{column}`、复合 `idx_{table}_{group}`、唯一 `uq_{table}_{group}`。
fn default_index_name(table_name: &str, expected: &ExpectedIndexDef<'_>) -> String {
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
fn split_qualified_table_name(table_name: &str) -> (Option<&str>, &str) {
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
        #[cfg(any(feature = "duckdb", feature = "clickhouse", feature = "influxdb"))]
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
        insert_columns.push(column.name);
        let target_type = column_type_definition(DbType::Sqlite, column);
        let expression = if types_equivalent(DbType::Sqlite, &actual.type_name, &target_type) {
            column.name.to_string()
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
            sqlite_conversion_expression(column.name, &actual.type_name, &target_type)?
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
            .skip(1)
            .map(|statement| statement),
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

async fn execute_steps(
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

async fn execute_steps_nontransactional(
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
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
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
            "END" => {
                if *trigger_depth > 0 {
                    *trigger_depth -= 1;
                }
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

    pub fn migrate_table<T: WritableModel>(&self) -> TableMigration<'_, T> {
        TableMigration {
            db: self,
            marker: PhantomData,
            renames: Vec::new(),
        }
    }

    /// 确保表结构与模型一致，返回实际执行的动作。
    ///
    /// 1. 表不存在时直接创建；
    /// 2. 存在结构差异时优先尝试增量迁移；
    /// 3. 默认拒绝破坏性步骤：数据库中存在而模型中没有的列不会被删除，
    ///    无法增量迁移（主键变更、超表分区约束冲突等）的差异也不会触发
    ///    删表重建，而是返回 [`crate::OrmerError::UnmigratableSchema`]（可用
    ///    `is_unmigratable_schema()` 编程判定）。确需删列或删表重建时，显式
    ///    调用 [`Database::ensure_table_permissive`]，或先通过
    ///    [`Database::migrate_table`] 的 [`MigrationPlan`] 预览再自行处理。
    pub async fn ensure_table<T: WritableModel>(
        &self,
    ) -> crate::Result<TableEnsureOutcome> {
        self.ensure_table_inner::<T>(false).await
    }

    /// 与 [`Database::ensure_table`] 相同，但显式允许破坏性步骤：
    /// 删除数据库中存在而模型中没有的列；差异无法增量迁移时删除整表并按
    /// 当前模型重建（返回 [`TableEnsureOutcome::Recreated`]）。
    pub async fn ensure_table_permissive<T: WritableModel>(
        &self,
    ) -> crate::Result<TableEnsureOutcome> {
        self.ensure_table_inner::<T>(true).await
    }

    async fn ensure_table_inner<T: WritableModel>(
        &self,
        allow_destructive: bool,
    ) -> crate::Result<TableEnsureOutcome> {
        let table_name = T::table_name_for_db(self.db_type());
        let actual = self.schema_columns(table_name).await?;
        let Some(actual) = actual else {
            self.create_table::<T>().execute().await?;
            return Ok(TableEnsureOutcome::Ready);
        };

        let validation_error = match self.validate_table::<T>().await {
            Ok(()) => return Ok(TableEnsureOutcome::Ready),
            Err(err) => err,
        };
        if !is_schema_rebuild_error(&validation_error) {
            return Err(validation_error);
        }

        // 默认策略：计划会删除"数据库有、模型没有"的列（非 SQLite 生成
        // DropColumn，SQLite 触发整表重建），执行前直接拒绝。
        if !allow_destructive {
            let expected_names: BTreeSet<&str> =
                T::COLUMN_SCHEMA.iter().map(|column| column.name).collect();
            let dropped_columns: Vec<&str> = actual
                .iter()
                .map(|column| column.name.as_str())
                .filter(|name| !expected_names.contains(name))
                .collect();
            if !dropped_columns.is_empty() {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!(
                        "columns [{}] exist in the database but not in the model; \
                         dropping them loses data. Call ensure_table_permissive() to opt in, \
                         or write an explicit migration",
                        dropped_columns.join(", ")
                    ),
                ));
            }
        }

        let migration = self.migrate_table::<T>();
        match migration.plan().await {
            Ok(plan) => {
                migration.execute_plan(&plan).await?;
                match self.validate_table::<T>().await {
                    Ok(()) => Ok(TableEnsureOutcome::Migrated),
                    Err(post_error) => {
                        if !allow_destructive {
                            return Err(crate::OrmerError::unmigratable_schema(
                                table_name,
                                format!(
                                    "incremental migration did not reconcile the schema: \
                                     {post_error}; fixing it requires dropping and recreating \
                                     the table. Call ensure_table_permissive() to opt in, \
                                     or write an explicit migration"
                                ),
                            ));
                        }
                        self.recreate_table_internal::<T>().await?;
                        Ok(TableEnsureOutcome::Recreated)
                    }
                }
            }
            Err(migration_error)
                if migration_error.is_unmigratable_schema()
                    || is_schema_rebuild_error(&migration_error) =>
            {
                if !allow_destructive {
                    return Err(crate::OrmerError::unmigratable_schema(
                        table_name,
                        format!(
                            "schema differences cannot be migrated incrementally: \
                             {migration_error}; fixing them requires dropping and recreating \
                             the table. Call ensure_table_permissive() to opt in, \
                             or write an explicit migration"
                        ),
                    ));
                }
                self.recreate_table_internal::<T>().await?;
                Ok(TableEnsureOutcome::Recreated)
            }
            Err(migration_error) => Err(migration_error),
        }
    }

    async fn recreate_table_internal<T: WritableModel>(&self) -> crate::Result<()> {
        self.drop_table::<T>().execute().await?;
        self.create_table::<T>().execute().await?;
        self.validate_table::<T>().await
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
        if let Database::ClickHouse(db) = self {
            return db.apply_migrations(migrations).await;
        }
        #[cfg(feature = "influxdb")]
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
        if let Database::ClickHouse(db) = self {
            return db.ensure_migration_table().await;
        }
        #[cfg(feature = "influxdb")]
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

    async fn schema_columns(
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
    async fn db_first_table_for(
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
                && schema.map_or(true, |schema| {
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

    /// QuestDB 表结构校验：基于 `table_columns` 自省结果逐列比对。
    ///
    /// QuestDB 没有主键/NOT NULL 约束，这两项不参与比较；类型不一致按
    /// UnmigratableSchema 报告，由 `ensure_table` 引导到增量迁移路径。
    #[cfg(feature = "questdb")]
    pub(crate) async fn validate_table_questdb<T: WritableModel>(&self) -> crate::Result<()> {
        let db_type = self.db_type();
        let table_name = T::table_name_for_db(db_type);
        let Some(actual) = self.schema_columns(table_name).await? else {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {table_name}, reason: Table does not exist"
            ));
        };
        let actual_by_name: BTreeMap<&str, &SchemaColumn> = actual
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();
        for expected in T::COLUMN_SCHEMA {
            let Some(actual) = actual_by_name.get(expected.name) else {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!("Column {} is missing", expected.name),
                ));
            };
            if actual.type_name.is_empty() {
                return Err(crate::ormer_error!(
                    "Cannot determine the database type of column {}",
                    expected.name
                ));
            }
            let expected_type = column_type_definition(db_type, expected);
            if !types_equivalent(db_type, &actual.type_name, &expected_type) {
                return Err(crate::OrmerError::unmigratable_schema(
                    table_name,
                    format!(
                        "Column {} has type {}, expected {}",
                        expected.name, actual.type_name, expected_type
                    ),
                ));
            }
        }
        Ok(())
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
