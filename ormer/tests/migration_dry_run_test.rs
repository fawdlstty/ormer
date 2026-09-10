#![cfg(any(feature = "sqlite", feature = "postgresql"))]

//! `MigrationRunner::dry_run`：pending 场景返回全部步骤与 SQL 且不落库，
//! 已应用场景只报告剩余步骤与已完成版本。

use ormer::{Database, DbType, Migration, MigrationStep};

struct CreateDryRunUsers;

impl Migration for CreateDryRunUsers {
    fn version(&self) -> u64 {
        1
    }

    fn name(&self) -> &str {
        "create_dry_run_users"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "CREATE TABLE ormer_dry_run_users (id INTEGER PRIMARY KEY, name TEXT)".to_string(),
        }]
    }
}

struct AddDryRunUserEmail;

impl Migration for AddDryRunUserEmail {
    fn version(&self) -> u64 {
        2
    }

    fn name(&self) -> &str {
        "add_dry_run_user_email"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "ALTER TABLE ormer_dry_run_users ADD COLUMN email TEXT".to_string(),
        }]
    }
}

async fn database() -> ormer::Result<Database> {
    #[cfg(feature = "sqlite")]
    {
        Ok(Database::connect(DbType::Sqlite, ":memory:").await?)
    }
    #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
    {
        let db = Database::connect(
            DbType::PostgreSQL,
            "postgres://postgres:postgres@localhost:5432/ormer_test",
        )
        .await?;
        // 清理历史运行的残留，保证测试可重复执行。
        let _ = db.execute_sql("DROP TABLE IF EXISTS __ormer_migrations").await;
        let _ = db.execute_sql("DROP TABLE IF EXISTS ormer_dry_run_users").await;
        Ok(db)
    }
}

/// PostgreSQL 上三个测试共享 `__ormer_migrations`/`ormer_dry_run_users`，
/// 并行的 DROP/CREATE 会互相触发唯一约束竞态，串行化整个测试体。
async fn pg_serial_guard() -> Option<tokio::sync::MutexGuard<'static, ()>> {
    #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
    {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        Some(LOCK.lock().await)
    }
    #[cfg(feature = "sqlite")]
    {
        None
    }
}

fn migrations() -> [&'static dyn Migration; 2] {
    [&CreateDryRunUsers, &AddDryRunUserEmail]
}

#[tokio::test]
async fn dry_run_returns_all_pending_steps_without_recording() -> ormer::Result<()> {
    let _serial = pg_serial_guard().await;
    let db = database().await?;
    let migrations = migrations();

    let dry = db.migrations(&migrations).dry_run().await?;

    #[cfg(feature = "sqlite")]
    let expected_backend = DbType::Sqlite;
    #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
    let expected_backend = DbType::PostgreSQL;
    assert_eq!(dry.backend, expected_backend);
    assert!(dry.transactional);
    assert!(dry.warnings.is_empty());
    assert!(dry.completed_versions.is_empty());

    assert_eq!(dry.steps.len(), 2);
    assert_eq!(dry.steps[0].version, 1);
    assert_eq!(dry.steps[0].migration_name, "create_dry_run_users");
    assert_eq!(dry.steps[0].migration_index, 0);
    assert_eq!(dry.steps[0].step_index, 0);
    assert!(dry.steps[0].sql.contains("CREATE TABLE ormer_dry_run_users"));
    assert_eq!(dry.steps[1].version, 2);
    assert_eq!(dry.steps[1].migration_name, "add_dry_run_user_email");
    assert_eq!(dry.steps[1].migration_index, 1);
    assert_eq!(dry.steps[1].step_index, 0);
    assert!(dry.steps[1].sql.contains("ADD COLUMN email TEXT"));

    // 预演不落库：历史仍为空，两个版本仍是 pending。
    assert!(db.migration_history().await?.is_empty());
    assert_eq!(db.pending_migrations(&migrations).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn dry_run_after_partial_apply_reports_remaining_steps() -> ormer::Result<()> {
    let _serial = pg_serial_guard().await;
    let db = database().await?;

    let first: [&'static dyn Migration; 1] = [&CreateDryRunUsers];
    assert_eq!(db.apply_migrations(&first).await?, 1);

    let migrations = migrations();
    let dry = db.migrations(&migrations).dry_run().await?;
    assert_eq!(dry.completed_versions, vec![1]);
    assert_eq!(dry.steps.len(), 1);
    assert_eq!(dry.steps[0].version, 2);
    assert!(dry.steps[0].sql.contains("ADD COLUMN email TEXT"));
    Ok(())
}

#[tokio::test]
async fn dry_run_and_status_after_full_apply() -> ormer::Result<()> {
    let _serial = pg_serial_guard().await;
    let db = database().await?;
    let migrations = migrations();
    assert_eq!(db.apply_migrations(&migrations).await?, 2);

    let dry = db.migrations(&migrations).dry_run().await?;
    assert!(dry.steps.is_empty());
    assert_eq!(dry.completed_versions, vec![1, 2]);

    let status = db.migrations(&migrations).execution_status().await?;
    assert!(status.pending.is_empty());
    assert_eq!(status.completed.len(), 2);
    assert_eq!(status.resume_version, None);
    assert_eq!(status.completed[0].version, 1);
    assert_ne!(status.completed[0].checksum, 0);
    Ok(())
}
