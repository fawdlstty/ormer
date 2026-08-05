#![cfg(feature = "sqlite")]

use ormer::{Database, DbType, Migration, MigrationStep};

#[derive(Debug, ormer::Model)]
#[table = "ormer_migration_users"]
struct MigrationUserV1 {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_migration_users"]
struct MigrationUserV2 {
    #[primary(auto)]
    id: i32,
    name: String,
    display_name: String,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_migration_type_values"]
struct MigrationIntegerValue {
    #[primary]
    id: i32,
    value: i32,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_migration_nullable_values"]
struct MigrationNonNullValue {
    #[primary]
    id: i32,
    value: String,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_migration_nullable_bad_values"]
struct MigrationNonNullBadValue {
    #[primary]
    id: i32,
    value: String,
}

struct CreateMigration;

impl Migration for CreateMigration {
    fn version(&self) -> u64 {
        1
    }

    fn name(&self) -> &str {
        "create_migration_users"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "CREATE TABLE migration_history_users (id INTEGER PRIMARY KEY)".to_string(),
        }]
    }
}

struct FailingMigration;

impl Migration for FailingMigration {
    fn version(&self) -> u64 {
        2
    }

    fn name(&self) -> &str {
        "failing_migration"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "CREATE TABLE migration_history_users (".to_string(),
        }]
    }
}

async fn database() -> anyhow::Result<Database> {
    Ok(Database::connect(DbType::Sqlite, ":memory:").await?)
}

#[tokio::test]
async fn table_plan_creates_and_adds_columns() -> anyhow::Result<()> {
    let db = database().await?;

    let initial = db.migrate_table::<MigrationUserV1>().plan().await?;
    assert_eq!(initial.table_name(), "ormer_migration_users");
    assert!(matches!(
        initial.steps().first(),
        Some(MigrationStep::CreateTable { .. })
    ));
    initial.to_sql()?;
    db.migrate_table::<MigrationUserV1>().execute().await?;
    db.validate_table::<MigrationUserV1>().await?;

    let additive = db.migrate_table::<MigrationUserV2>().plan().await?;
    assert!(additive.steps().iter().any(
        |step| matches!(step, MigrationStep::AddColumn { column, .. } if column == "display_name")
    ));
    db.migrate_table::<MigrationUserV2>().execute().await?;
    db.validate_table::<MigrationUserV2>().await?;
    assert!(
        db.migrate_table::<MigrationUserV2>()
            .plan()
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn table_plan_rejects_implicit_not_null_addition_on_populated_sqlite_table()
-> anyhow::Result<()> {
    let db = database().await?;
    db.migrate_table::<MigrationUserV1>().execute().await?;
    db.execute_sql("INSERT INTO ormer_migration_users (name) VALUES ('existing')")
        .await?;

    let error = db
        .migrate_table::<MigrationUserV2>()
        .plan()
        .await
        .expect_err("non-null column requires an explicit backfill");
    assert!(error.to_string().contains("explicit migration"));
    Ok(())
}

#[tokio::test]
async fn sqlite_migrates_text_to_integer_and_preserves_data() -> anyhow::Result<()> {
    let db = database().await?;
    db.execute_sql(
        "CREATE TABLE ormer_migration_type_values (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
    )
    .await?;
    db.execute_sql(
        "INSERT INTO ormer_migration_type_values (id, value) \
         VALUES (1, '0'), (2, '7'), (3, '-12')",
    )
    .await?;

    let plan = db.migrate_table::<MigrationIntegerValue>().plan().await?;
    assert!(!plan.is_empty());
    assert!(plan.warnings().is_empty());

    db.migrate_table::<MigrationIntegerValue>()
        .execute()
        .await?;
    db.validate_table::<MigrationIntegerValue>().await?;
    let values = db
        .select_sql::<(i32, i32)>("SELECT id, value FROM ormer_migration_type_values ORDER BY id")
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(values, vec![(1, 0), (2, 7), (3, -12)]);
    assert!(
        db.migrate_table::<MigrationIntegerValue>()
            .plan()
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_migrates_nullable_to_not_null_and_enforces_constraint() -> anyhow::Result<()> {
    let db = database().await?;
    db.execute_sql(
        "CREATE TABLE ormer_migration_nullable_values (id INTEGER PRIMARY KEY, value TEXT)",
    )
    .await?;
    db.execute_sql("INSERT INTO ormer_migration_nullable_values (id, value) VALUES (1, 'alice')")
        .await?;

    db.migrate_table::<MigrationNonNullValue>()
        .execute()
        .await?;
    db.validate_table::<MigrationNonNullValue>().await?;

    let column_info = db
        .select_sql::<(String, i64)>(
            "SELECT name, \"notnull\" FROM pragma_table_info('ormer_migration_nullable_values') \
             WHERE name = 'value'",
        )
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(column_info, vec![("value".to_string(), 1)]);
    let error = db
        .execute_sql("INSERT INTO ormer_migration_nullable_values (id, value) VALUES (2, NULL)")
        .await
        .expect_err("migrated NOT NULL column must reject NULL");
    assert!(!error.to_string().is_empty());
    Ok(())
}

#[tokio::test]
async fn sqlite_invalid_type_value_rolls_back() -> anyhow::Result<()> {
    let db = database().await?;
    db.execute_sql(
        "CREATE TABLE ormer_migration_type_values (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
    )
    .await?;
    db.execute_sql(
        "INSERT INTO ormer_migration_type_values (id, value) \
         VALUES (1, '12'), (2, 'not-an-integer')",
    )
    .await?;

    let error = db
        .migrate_table::<MigrationIntegerValue>()
        .execute()
        .await
        .expect_err("invalid integer text must abort migration");
    assert!(!error.to_string().is_empty());

    let rows = db
        .select_sql::<(i32, String)>(
            "SELECT id, value FROM ormer_migration_type_values ORDER BY id",
        )
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        rows,
        vec![(1, "12".to_string()), (2, "not-an-integer".to_string())]
    );
    let column_info = db
        .select_sql::<(String, String)>(
            "SELECT name, type FROM pragma_table_info('ormer_migration_type_values') \
             WHERE name = 'value'",
        )
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(column_info, vec![("value".to_string(), "TEXT".to_string())]);
    Ok(())
}

#[tokio::test]
async fn sqlite_not_null_migration_with_existing_null_rolls_back() -> anyhow::Result<()> {
    let db = database().await?;
    db.execute_sql(
        "CREATE TABLE ormer_migration_nullable_bad_values (id INTEGER PRIMARY KEY, value TEXT)",
    )
    .await?;
    db.execute_sql("INSERT INTO ormer_migration_nullable_bad_values (id, value) VALUES (1, NULL)")
        .await?;

    let error = db
        .migrate_table::<MigrationNonNullBadValue>()
        .execute()
        .await
        .expect_err("existing NULL must abort NOT NULL migration");
    assert!(!error.to_string().is_empty());

    let null_count = db
        .select_sql::<i64>(
            "SELECT COUNT(*) FROM ormer_migration_nullable_bad_values WHERE value IS NULL",
        )
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(null_count, vec![1]);
    let column_info = db
        .select_sql::<(String, i64)>(
            "SELECT name, \"notnull\" \
             FROM pragma_table_info('ormer_migration_nullable_bad_values') \
             WHERE name = 'value'",
        )
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(column_info, vec![("value".to_string(), 0)]);
    Ok(())
}

#[tokio::test]
async fn versioned_migrations_track_pending_and_rollback() -> anyhow::Result<()> {
    let db = database().await?;
    let create = CreateMigration;
    let failing = FailingMigration;
    let migrations: [&dyn Migration; 2] = [&create, &failing];

    let pending = db.pending_migrations(&migrations).await?;
    assert_eq!(
        pending
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let error = db
        .apply_migrations(&migrations)
        .await
        .expect_err("migration should fail");
    assert!(!error.to_string().is_empty());
    assert!(db.validate_table::<MigrationHistoryUser>().await.is_err());
    assert_eq!(db.pending_migrations(&migrations).await?.len(), 2);

    let create_only: [&dyn Migration; 1] = [&create];
    assert_eq!(db.apply_migrations(&create_only).await?, 1);
    let history = db.migration_history().await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].version, 1);
    assert_eq!(history[0].name, "create_migration_users");
    assert_ne!(history[0].checksum, 0);
    assert_eq!(db.apply_migrations(&create_only).await?, 0);
    Ok(())
}

#[derive(Debug, ormer::Model)]
#[table = "migration_history_users"]
struct MigrationHistoryUser {
    #[primary]
    id: i32,
}
