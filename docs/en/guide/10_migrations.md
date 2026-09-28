# Migrations

Ormer supports two migration styles: inspectable schema plans derived from a model, and versioned migrations implemented with the `Migration` trait.

## One-Shot Migration From A Model

`migrate_table::<T>()` is the unified entry point for aligning table schemas at application startup: a single call runs diagnose -> execute -> re-verify, and is idempotent so startup retry loops can call it repeatedly. A missing table is created in full (including enum/extension/hypertable companion steps), and a missing non-primary-key column produces `ADD COLUMN`; existing-column type or nullability drift is converted into real migration steps where Ormer can do so safely. SQLite uses a table rebuild when needed. Conversions that cannot be inferred safely return an error.

```rust
use ormer::{TableDiagnosis};

let outcome = db.migrate_table::<User>().await?;

// outcome.diagnosis is the pre-execution diagnosis and can be reviewed
// afterwards. On success it is always Ready / Migratable(plan).
// outcome.created_table tells whether this call created the table
// (only from a fresh create when the table did not exist).
// outcome.executed lists the steps run.
match &outcome.diagnosis {
    TableDiagnosis::Ready => {}
    TableDiagnosis::Migratable(plan) => {
        for warning in plan.warnings() {
            eprintln!("{warning}");
        }
    }
}
```

If a target conversion cannot be proven safe, the migration returns an error. SQLite complex changes are applied by rebuilding the table, and invalid legacy values fail the migration and roll back.

Schema evolution that cannot be pushed in place (type narrowing, hypertable partition conflicts, and so on) never drops and recreates the table: `migrate_table` returns `OrmerError::UnmigratableSchema` directly (check it with `err.is_unmigratable_schema()` to distinguish it from other migration failures), leaving existing data and table objects untouched for the caller to decide what to do next.

## Versioned Migrations

Implement `Migration` and pass migrations to `db.migrations()`:

```rust
use ormer::{Migration, MigrationRunner, MigrationStep};

struct AddUserEmail;

impl Migration for AddUserEmail {
    fn version(&self) -> u64 {
        1
    }

    fn name(&self) -> &str {
        "add_user_email"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::AddColumn {
            table: "users".into(),
            column: "email".into(),
            definition: "TEXT".into(),
        }]
    }

    fn down(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "DROP COLUMN email".into(),
        }]
    }
}

let migrations = [AddUserEmail];
let runner: MigrationRunner<'_, AddUserEmail> = db.migrations(&migrations);

let pending = runner.pending().await?;
println!("pending: {}", pending.len());
let applied = runner.execute().await?;
println!("applied: {applied}");
```

Preview pending work with `dry_run()`: it renders every pending statement and reports completed versions and warnings without touching the database.

```rust
let dry = db.migrations(&migrations).dry_run().await?;

for step in &dry.steps {
    println!("{} {} -> {}", step.version, step.migration_name, step.sql);
}
```

When a runner is not needed, call the database methods directly:

```rust
let pending = db.pending_migrations(&migrations).await?;
let applied = db.apply_migrations(&migrations).await?;
```

Applied migrations are recorded in `__ormer_migrations`. Migrations are sorted by version, run in a transaction, and store a checksum derived from the name and `up()` steps; changing an applied migration returns an error.

Available `MigrationStep` variants are `CreateType`, `AlterType`, `CreateTable`, `AddColumn`, `BackfillColumn`, `AlterColumn`, `AddConstraint`, `CreateIndex`, `AddForeignKey`, and `Sql`. Use `Sql` for complex or dialect-specific DDL.

```rust
let history = db.migration_history().await?;
for migration in history {
    println!("{} {} {}", migration.version, migration.name, migration.checksum);
}
```

SQLite cannot add a foreign key after table creation. `migrate_table` fails with
`UnmigratableSchema`; rebuild the table by hand if adding the key is required.

ClickHouse also uses the unified `Database` migration entry point:

```rust
let db = ormer::Database::connect(
    ormer::DbType::ClickHouse,
    "http://localhost:8123?database=default",
)
.await?;

let runner = db.migrations(&migrations);
let pending = runner.pending().await?;
let applied = runner.execute().await?;
```

ClickHouse stores migration history in a `MergeTree` table and does not provide
transactions or automatic rollback. Steps execute one at a time, so completed
steps remain applied if a later step fails. Use `MigrationStep::Sql` for
ClickHouse CREATE TABLE DDL that must specify an engine.

On populated DuckDB and ClickHouse tables, inferred column type changes are
rejected during planning; use an explicit staged migration for those tables.

InfluxDB uses the same migration entry point: there is no table-creation DDL
(the measurement is created by the first write), migration steps execute one at
a time without rollback, and history is recorded in the `__ormer_migrations`
measurement.

`migrate_table` includes column defaults for new columns and infers new regular, composite, and unique indexes when possible. A non-null column added to a populated table still requires an explicit backfill when it has no default.

Extra columns (present in the database but absent from the model) are dropped
outright, so keys, columns, and configuration always match the model exactly.
Indexes are diffed as expected-vs-actual sets: adding
`#[index]` to an existing column creates the index, removing it emits `DropIndex`.
Tightening a nullable column to `NOT NULL` fails first when existing rows contain
NULLs; backfill before migrating.

Model `#[compress(...)]` attributes are included in schema validation and migration. PostgreSQL uses column-level `SET COMPRESSION`; MySQL uses the table-level `COMPRESSION` option, so all compressed columns in one MySQL table must use the same algorithm.

## QuestDB migrations

QuestDB (8.3+ required) executes migrations one by one and keeps history in
`__ormer_migrations` (with a `rolled_back` flag):

- `AddColumn` and `DropColumn` work; `RenameColumn` renders
  `ALTER TABLE t RENAME COLUMN old TO new`.
- QuestDB cannot change column types, so `AlterColumn` and type-change
  migrations stay gated; hand-write a "new table + INSERT SELECT" rebuild.
- `migrate_table` introspects schemas via `table_columns('name')`. QuestDB has
  no primary-key or NOT NULL constraints, so those are not compared; the
  designated timestamp clause is generated at create-table time and existing
  tables are never rebuilt destructively by auto migration.
