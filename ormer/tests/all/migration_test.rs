#![cfg(feature = "sqlite")]

use ormer::{Database, DbType, Migration, MigrationStep};

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

async fn database() -> ormer::Result<Database> {
    Database::connect(DbType::Sqlite, ":memory:").await
}

#[tokio::test]
async fn versioned_migrations_track_pending_and_rollback() -> ormer::Result<()> {
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
    // 失败批次整体回滚：业务表未被建出（migration_history_users 不存在）
    let table_rows = db
        .select_sql::<i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = 'migration_history_users'",
        )
        .collect::<Vec<i64>>()
        .await?;
    assert_eq!(table_rows.first().copied(), Some(0));
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
