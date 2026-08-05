# Migrations

Ormer supports two migration styles: inspectable schema plans derived from a model, and versioned migrations implemented with the `Migration` trait.

## Plan From A Model

`migrate_table::<T>()` builds an executable schema migration plan. A missing table produces a create-table step, and a missing non-primary-key column produces `ADD COLUMN`; existing-column type or nullability drift is converted into real migration steps where Ormer can do so safely. SQLite uses a table rebuild when needed. Conversions that cannot be inferred safely return an error.

```rust
let plan = db.migrate_table::<User>().plan().await?;

println!("{}", plan.to_sql()?);
for warning in plan.warnings() {
    eprintln!("{warning}");
}

db.migrate_table::<User>().execute().await?;
```

If a target conversion cannot be proven safe, `plan()` or `execute()` returns an error. SQLite complex changes are applied by rebuilding the table, and invalid legacy values fail the migration and roll back.

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

SQLite cannot add a foreign key after table creation; `MigrationStep::AddForeignKey` returns an error on SQLite.
