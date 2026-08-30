//! Versioned schema migration primitives.
//!
//! The migration API deliberately keeps migration steps structured until the
//! final dialect rendering stage.  Applications can therefore inspect a plan
//! before executing it, while small hand-written migrations remain possible.

use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{Database, Transaction};
#[cfg(any(feature = "sqlite", feature = "duckdb"))]
use crate::abstract_layer::common::common_helpers;
#[cfg(any(feature = "postgresql", feature = "mysql"))]
use crate::model::CompressionAlgorithm;
use crate::model::{ColumnSchema, WritableModel};
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

pub const MIGRATION_TABLE_NAME: &str = "__ormer_migrations";

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
                    #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
                    _ => " IF NOT EXISTS",
                };
                Ok(format!(
                    "CREATE{unique} INDEX{if_not_exists} {} ON {table} ({})",
                    crate::model::quote_identifier(db_type, name),
                    columns(index_columns)
                ))
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
        | MigrationStep::BackfillColumn { table, .. }
        | MigrationStep::AlterColumn { table, .. }
        | MigrationStep::AddConstraint { table, .. }
        | MigrationStep::CreateIndex { table, .. }
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
}

impl<'a, T: WritableModel> TableMigration<'a, T> {
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
        let table_name = T::table_name_for_db(db_type);
        let mut plan = MigrationPlan::new(table_name, db_type);
        let actual = self.db.schema_columns(table_name).await?;

        let Some(actual) = actual else {
            plan.push(MigrationStep::CreateTable {
                table: table_name.to_string(),
                definition: crate::generate_create_table_sql::<T>(db_type)?,
            });
            return Ok(plan);
        };

        self.db.validate_hypertable_for_migration::<T>().await?;

        let actual_names: BTreeSet<&str> =
            actual.iter().map(|column| column.name.as_str()).collect();
        let expected_names: BTreeSet<&str> =
            T::COLUMN_SCHEMA.iter().map(|column| column.name).collect();
        let actual_by_name: BTreeMap<&str, &SchemaColumn> = actual
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();

        for column in &actual {
            if !expected_names.contains(column.name.as_str()) {
                return Err(crate::ormer_error!(
                    "Cannot migrate table {} because existing column {} is not present in the model",
                    table_name,
                    column.name
                ));
            }
        }

        // Additive changes are safe to infer when a non-null column can be
        // populated. A populated table without a model default needs an
        // explicit backfill because the ORM cannot invent its value.
        let mut added_columns = BTreeSet::new();
        for column in T::COLUMN_SCHEMA {
            if !actual_names.contains(column.name) {
                if column.is_primary {
                    return Err(crate::ormer_error!(
                        "Cannot infer adding primary key column {}; write an explicit migration",
                        column.name
                    ));
                }
                if !column.is_nullable
                    && column.default.is_none()
                    && self.db.table_row_count(table_name).await? > 0
                {
                    return Err(crate::ormer_error!(
                        "Cannot add NOT NULL column {} to populated table {}; \
                         write an explicit migration with a backfill",
                        column.name,
                        table_name
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

        if !added_columns.is_empty() {
            let available_columns =
                |name: &str| actual_names.contains(name) || added_columns.contains(name);

            let mut indexes = BTreeMap::<i32, Vec<&ColumnSchema>>::new();
            for column in T::COLUMN_SCHEMA {
                if !column.is_indexed {
                    continue;
                }
                if let Some(group) = column.index_group {
                    indexes.entry(group).or_default().push(column);
                } else if added_columns.contains(column.name) {
                    indexes.entry(i32::MIN).or_default().push(column);
                }
            }

            for (group, columns) in indexes {
                if group != i32::MIN
                    && !columns
                        .iter()
                        .any(|column| added_columns.contains(column.name))
                {
                    continue;
                }
                if !columns.iter().all(|column| available_columns(column.name)) {
                    continue;
                }
                let name = columns
                    .iter()
                    .find_map(|column| column.index_name)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        if group == i32::MIN {
                            format!("idx_{}_{}", table_name.replace('.', "_"), columns[0].name)
                        } else {
                            format!("idx_{}_{}", table_name.replace('.', "_"), group)
                        }
                    });
                plan.push(index_migration_step(
                    db_type, name, table_name, &columns, false,
                )?);
            }

            let mut unique_groups = BTreeMap::<i32, Vec<&ColumnSchema>>::new();
            for column in T::COLUMN_SCHEMA {
                if let Some(group) = column.unique_group {
                    unique_groups.entry(group).or_default().push(column);
                }
            }
            for (group, columns) in unique_groups {
                if !columns
                    .iter()
                    .any(|column| added_columns.contains(column.name))
                    || !columns.iter().all(|column| available_columns(column.name))
                {
                    continue;
                }
                let name = columns
                    .iter()
                    .find_map(|column| column.unique_name)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("uq_{}_{}", table_name.replace('.', "_"), group));
                plan.push(index_migration_step(
                    db_type, name, table_name, &columns, true,
                )?);
            }

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
                        return Err(crate::ormer_error!(
                            "Cannot add foreign key column {} to SQLite table {}; write an explicit table-rebuild migration",
                            column.name,
                            table_name
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

        #[cfg(feature = "sqlite")]
        let mut sqlite_rebuild_required = false;
        for expected in T::COLUMN_SCHEMA {
            let Some(actual) = actual_by_name.get(expected.name) else {
                continue;
            };
            if actual.primary_key != expected.is_primary {
                return Err(crate::ormer_error!(
                    "Cannot infer primary-key migration for column {}; write an explicit migration",
                    expected.name
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
            let nullable_changed = !expected.is_primary && actual.nullable != expected.is_nullable;
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
                return Err(crate::ormer_error!(
                    "Cannot infer primary-key type migration for column {}; write an explicit migration",
                    expected.name
                ));
            }

            #[cfg(feature = "sqlite")]
            if matches!(db_type, DbType::Sqlite) {
                sqlite_rebuild_required = true;
                continue;
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

        #[cfg(feature = "sqlite")]
        if sqlite_rebuild_required {
            plan.steps.clear();
            plan.push(MigrationStep::Sql {
                sql: sqlite_rebuild_sql::<T>(table_name, &actual_by_name)?,
            });
        }

        Ok(plan)
    }

    pub async fn execute(&self) -> crate::Result<()> {
        let plan = self.plan().await?;
        if plan.is_empty() {
            return Ok(());
        }

        let mut transaction = self.db.begin().await?;
        let result = execute_steps(&mut transaction, plan.db_type(), &plan.steps).await;
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
        #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
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
        #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
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
                 THEN ARRAY(SELECT jsonb_array_elements_text({column}::jsonb)) \
                 WHEN jsonb_typeof({column}::jsonb) = 'string' \
                 THEN ARRAY[({column}::jsonb #>> '{{}}')]::TEXT[] \
                 ELSE ARRAY[{column}::text]::TEXT[] END"
            ));
        }
        return Some(format!(
            "CASE WHEN {column} IS NULL THEN {null_value} \
             WHEN btrim({column}::text) = '' THEN ARRAY[]::TEXT[] \
                 WHEN left(btrim({column}::text), 1) = '[' \
             THEN ARRAY(SELECT jsonb_array_elements_text({column}::jsonb)) \
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
            Database::PostgreSQL(_) => DbType::PostgreSQL,
            #[cfg(feature = "mysql")]
            Database::MySQL(_) => DbType::MySQL,
            #[cfg(feature = "mssql")]
            Database::MSSQL(_) => DbType::MSSQL,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(_) => DbType::DuckDB,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(_) => DbType::ClickHouse,
        }
    }

    pub fn migrate_table<T: WritableModel>(&self) -> TableMigration<'_, T> {
        TableMigration {
            db: self,
            marker: PhantomData,
        }
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
        }
    }

    async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<crate::migration::SchemaColumn>>> {
        match self {
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
        }
    }

    async fn validate_hypertable_for_migration<T: WritableModel>(&self) -> crate::Result<()> {
        match self {
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
