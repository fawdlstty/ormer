#![cfg(any(feature = "sqlite", feature = "postgresql"))]

//! 表迁移统一入口 `plan_table` / `apply_table` 的 SQL 形状与幂等行为测试。
//!
//! 覆盖（对应规格书 §4 各能力域）：
//! - apply_table 建表/补列/收敛/幂等（sqlite e2e）
//! - ensure_table / ensure_table_permissive 薄糖映射（sqlite e2e）
//! - 索引语义 diff：删除后 apply 补建（sqlite e2e）
//! - CHECK 约束闭环（PG e2e）
//! - rebuild=Allow 的触发器托管（PG e2e）
//! - advisory_xact_lock（PG e2e roundtrip + 非 PG Unsupported）
//! - ping

use ormer::{Database, DbType};

#[cfg(any(feature = "sqlite", feature = "postgresql"))]
mod shared {
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_items"]
    pub struct ApplyItemV1 {
        #[primary]
        pub id: i32,
        #[index]
        pub name: String,
    }

    /// V1 + note 列（模拟模型演进加列）
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_items"]
    pub struct ApplyItemV2 {
        #[primary]
        pub id: i32,
        #[index]
        pub name: String,
        pub note: Option<String>,
    }

    /// V2 - note 列（Keep 策略下库里的 note 应保留）
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_items"]
    pub struct ApplyItemV3 {
        #[primary]
        pub id: i32,
        #[index]
        pub name: String,
    }
}

#[cfg(feature = "sqlite")]
mod sqlite_apply {
    use super::shared::*;
    use ormer::{Database, DbType, MigrationStep, TableDiagnosis, TableEnsureOutcome};

    async fn database() -> ormer::Result<Database> {
        Database::connect(DbType::Sqlite, ":memory:").await
    }

    /// apply_table：首建 → 补列 → 收敛 → 幂等（零步骤）。
    #[tokio::test]
    async fn apply_creates_adds_and_converges() -> ormer::Result<()> {
        let db = database().await?;

        // 首次 apply：建表
        let first = db.apply_table::<ApplyItemV1>(&Default::default()).await?;
        assert!(first.created_table, "first apply should create the table");
        assert!(
            first.executed.iter().any(
                |step| matches!(step, MigrationStep::CreateTable { table, .. } if table == "ormer_apply_items")
            ),
            "executed should contain CreateTable, got {:?}",
            first.executed
        );
        assert!(matches!(
            db.plan_table::<ApplyItemV1>().await?,
            TableDiagnosis::Ready
        ));

        // 模型加列：apply 补列
        let second = db.apply_table::<ApplyItemV2>(&Default::default()).await?;
        assert!(!second.created_table);
        assert!(
            second.executed.iter().any(
                |step| matches!(step, MigrationStep::AddColumn { column, .. } if column == "note")
            ),
            "executed should contain AddColumn(note), got {:?}",
            second.executed
        );
        assert!(matches!(
            db.plan_table::<ApplyItemV2>().await?,
            TableDiagnosis::Ready
        ));

        // 幂等：再次 apply 零动作
        let third = db.apply_table::<ApplyItemV2>(&Default::default()).await?;
        assert!(!third.created_table);
        assert!(third.executed.is_empty(), "re-apply must be a no-op");
        assert!(matches!(third.diagnosis, TableDiagnosis::Ready));

        // Keep 策略：模型删列后库里列保留，收敛为 Ready
        db.apply_table::<ApplyItemV3>(&Default::default()).await?;
        assert!(matches!(
            db.plan_table::<ApplyItemV3>().await?,
            TableDiagnosis::Ready
        ));
        Ok(())
    }

    /// ensure_table / ensure_table_permissive 是 apply_table 的策略预设薄糖。
    #[tokio::test]
    async fn ensure_table_maps_to_apply_outcome() -> ormer::Result<()> {
        let db = database().await?;

        let outcome = db.ensure_table::<ApplyItemV1>().await?;
        assert_eq!(outcome, TableEnsureOutcome::Ready);
        assert!(matches!(
            db.ensure_table::<ApplyItemV1>().await?,
            TableEnsureOutcome::Ready
        ));

        // permissive：模型缺列触发 SQLite 重建（Drop 多余列），保数据换表
        db.apply_table::<ApplyItemV2>(&Default::default()).await?;
        let outcome = db.ensure_table_permissive::<ApplyItemV3>().await?;
        assert_eq!(outcome, TableEnsureOutcome::Migrated);
        assert!(matches!(
            db.plan_table::<ApplyItemV3>().await?,
            TableDiagnosis::Ready
        ));
        Ok(())
    }

    /// 索引语义 diff：索引被删后 apply 按语义签名补建（ormer 自动命名）。
    #[tokio::test]
    async fn apply_rebuilds_dropped_index() -> ormer::Result<()> {
        let db = database().await?;
        db.apply_table::<ApplyItemV1>(&Default::default()).await?;

        let index_name = "idx_ormer_apply_items_name";
        db.execute_sql(ormer::sql!(format!("DROP INDEX {index_name}")))
            .await?;
        let before = index_names(&db).await?;
        assert!(
            !before.iter().any(|name| name == index_name),
            "index should be gone before apply: {before:?}"
        );

        db.apply_table::<ApplyItemV1>(&Default::default()).await?;
        let after = index_names(&db).await?;
        assert!(
            after.iter().any(|name| name == index_name),
            "apply must rebuild the missing index, got {after:?}"
        );
        Ok(())
    }

    async fn index_names(db: &Database) -> ormer::Result<Vec<String>> {
        db.select_sql::<String>(ormer::sql!(
            "SELECT name FROM sqlite_master WHERE type = 'index' \
             AND tbl_name = 'ormer_apply_items' AND name NOT LIKE 'sqlite_%'"
        ))
        .collect::<Vec<String>>()
        .await
    }

    /// ping：库健康检查。
    #[tokio::test]
    async fn ping_succeeds() -> ormer::Result<()> {
        let db = database().await?;
        db.ping().await?;
        Ok(())
    }

    /// advisory_xact_lock：非 PG 后端明确 Unsupported（不静默成功）。
    #[tokio::test]
    async fn advisory_lock_unsupported_on_sqlite() -> ormer::Result<()> {
        let db = database().await?;
        let tx = db.begin().await?;
        let err = tx.advisory_xact_lock(1).await.unwrap_err();
        assert!(
            matches!(err, ormer::OrmerError::UnsupportedFeature { .. }),
            "expected UnsupportedFeature, got {err}"
        );
        tx.rollback().await?;
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn table_apply_outcome_reports_steps() -> ormer::Result<()> {
    use shared::ApplyItemV1;
    use ormer::{TableApplyOutcome, TableDiagnosis};
    let db = Database::connect(DbType::Sqlite, ":memory:").await?;
    let outcome = db.apply_table::<ApplyItemV1>(&Default::default()).await?;
    let TableApplyOutcome {
        diagnosis,
        created_table,
        executed,
    } = outcome;
    assert!(matches!(diagnosis, TableDiagnosis::Migratable(_)));
    assert!(created_table);
    assert_eq!(executed.len(), 1);
    Ok(())
}

/// PostgreSQL 专属能力域：CHECK 闭环、触发器托管、咨询锁。
#[cfg(feature = "postgresql")]
mod postgresql_only {
    use ormer::{Database, DbType, TableDiagnosis};

    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_check_tasks"]
    struct CheckTask {
        #[primary]
        id: i32,
        #[check(expr = "length(name) > 0")]
        name: String,
    }

    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_pk_items"]
    struct PkItemV1 {
        #[primary]
        bucket_start: String,
        #[primary]
        project_id: String,
        count: i64,
    }

    /// project_id 移出主键：PG 上生成 ChangePrimaryKey 原地变更（保数据）。
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_pk_items"]
    struct PkItemV2 {
        #[primary]
        bucket_start: String,
        project_id: String,
        count: i64,
    }

    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_trigger_stats"]
    struct TriggerStatV1 {
        #[primary]
        bucket_start: String,
        #[primary]
        project_id: String,
        count: i64,
    }

    /// 新增 NOT NULL 列且表有数据、无回填来源 → NeedsRebuild。
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_trigger_stats"]
    struct TriggerStatV2 {
        #[primary]
        bucket_start: String,
        #[primary]
        project_id: String,
        count: i64,
        note: String,
    }

    const TRIGGER_TABLE: &str = "ormer_apply_trigger_stats";
    const TRIGGER_NAME: &str = "ormer_apply_audit_trg";

    async fn pg_database() -> Database {
        let url = option_env!("ORMER_TEST_POSTGRES")
            .unwrap_or("postgres://postgres:postgres@localhost:5432/ormer_test");
        Database::connect(DbType::PostgreSQL, url).await.unwrap()
    }

    async fn create_business_trigger(db: &Database) {
        db.execute_sql("CREATE TABLE IF NOT EXISTS ormer_apply_audit (note text)")
            .await
            .unwrap();
        db.execute_sql(
            r#"
            CREATE OR REPLACE FUNCTION ormer_apply_audit_fn() RETURNS trigger AS $$
            BEGIN
                INSERT INTO ormer_apply_audit (note) VALUES (TG_TABLE_NAME);
                RETURN NULL;
            END
            $$ LANGUAGE plpgsql
            "#,
        )
        .await
        .unwrap();
        db.execute_sql(ormer::sql!(format!(
            "DROP TRIGGER IF EXISTS {TRIGGER_NAME} ON {TRIGGER_TABLE}"
        )))
        .await
        .unwrap();
        db.execute_sql(ormer::sql!(format!(
            "CREATE TRIGGER {TRIGGER_NAME} AFTER INSERT ON {TRIGGER_TABLE} \
             FOR EACH STATEMENT EXECUTE FUNCTION ormer_apply_audit_fn()"
        )))
        .await
        .unwrap();
    }

    /// CHECK 闭环：约束被外力删除后 apply 自动补建（ormer 自动命名）。
    #[tokio::test]
    async fn apply_restores_dropped_check_constraint() -> ormer::Result<()> {
        let db = pg_database().await;
        let _ = db.drop_table::<CheckTask>().execute().await;
        db.apply_table::<CheckTask>(&Default::default()).await?;

        let constraint_name = "ck_ormer_apply_check_tasks_name";
        db.execute_sql(ormer::sql!(format!(
            "ALTER TABLE ormer_apply_check_tasks DROP CONSTRAINT {constraint_name}"
        )))
        .await?;

        let plan = match db.plan_table::<CheckTask>().await? {
            TableDiagnosis::Migratable(plan) => plan,
            other => panic!("expected Migratable, got {other:?}"),
        };
        let sql = plan.to_sql()?;
        assert!(
            sql.contains("ADD CONSTRAINT") && sql.contains(constraint_name),
            "plan should add the missing check constraint, got: {sql}"
        );

        db.apply_table::<CheckTask>(&Default::default()).await?;
        assert!(matches!(
            db.plan_table::<CheckTask>().await?,
            TableDiagnosis::Ready
        ));

        let restored = db
            .select_sql::<String>(ormer::sql!(
                "SELECT conname FROM pg_constraint \
                 WHERE conrelid = to_regclass('ormer_apply_check_tasks') AND contype = 'c'"
            ))
            .collect::<Vec<String>>()
            .await?;
        assert!(
            restored.iter().any(|name| name == constraint_name),
            "check constraint must be restored, got {restored:?}"
        );
        let _ = db.drop_table::<CheckTask>().execute().await;
        Ok(())
    }

    /// 主键原地变更：project_id 移出主键 → ChangePrimaryKey（保数据），
    /// 不走删表重建。
    #[tokio::test]
    async fn primary_key_change_is_inplace_on_postgresql() -> ormer::Result<()> {
        let db = pg_database().await;
        let _ = db.drop_table::<PkItemV1>().execute().await;
        db.apply_table::<PkItemV1>(&Default::default()).await?;
        db.insert(&PkItemV1 {
            bucket_start: "2026-09-08 10:00:00".to_string(),
            project_id: "p1".to_string(),
            count: 1,
        })
        .execute()
        .await?;

        let plan = match db.plan_table::<PkItemV2>().await? {
            TableDiagnosis::Migratable(plan) => plan,
            other => panic!("pk change should be migratable in place, got {other:?}"),
        };
        assert!(
            plan.to_sql()?.contains("ADD PRIMARY KEY"),
            "plan should change the primary key in place"
        );

        db.apply_table::<PkItemV2>(&Default::default()).await?;
        assert!(matches!(
            db.plan_table::<PkItemV2>().await?,
            TableDiagnosis::Ready
        ));

        // 数据保留
        let rows = db
            .select_sql::<i64>(ormer::sql!(
                "SELECT count(*) FROM ormer_apply_pk_items"
            ))
            .collect::<Vec<i64>>()
            .await?;
        assert_eq!(rows.first().copied(), Some(1), "data must survive");

        let _ = db.drop_table::<PkItemV2>().execute().await;
        Ok(())
    }

    /// rebuild=Allow：不可迁移差异 → 删表重建，业务触发器由 apply 托管备份/恢复。
    #[tokio::test]
    async fn rebuild_allow_restores_business_triggers() -> ormer::Result<()> {
        let db = pg_database().await;
        let _ = db.drop_table::<TriggerStatV1>().execute().await;
        let _ = db.execute_sql("DROP TABLE IF EXISTS ormer_apply_audit").await;
        let _ = db
            .execute_sql("DROP FUNCTION IF EXISTS ormer_apply_audit_fn()")
            .await;

        db.apply_table::<TriggerStatV1>(&Default::default()).await?;
        db.insert(&TriggerStatV1 {
            bucket_start: "2026-09-08 10:00:00".to_string(),
            project_id: "p1".to_string(),
            count: 1,
        })
        .execute()
        .await
        .unwrap();
        create_business_trigger(&db).await;

        let before = match db.plan_table::<TriggerStatV2>().await.unwrap() {
            TableDiagnosis::NeedsRebuild(cause) => cause,
            other => panic!("unbackfillable not-null column must require rebuild, got {other:?}"),
        };
        assert_eq!(before.table, TRIGGER_TABLE);

        let opts = ormer::ApplyOptions {
            rebuild: ormer::RebuildPolicy::Allow,
            ..Default::default()
        };
        let outcome = db.apply_table::<TriggerStatV2>(&opts).await.unwrap();
        assert!(outcome.created_table, "rebuild should recreate the table");
        assert!(matches!(
            db.plan_table::<TriggerStatV2>().await.unwrap(),
            TableDiagnosis::Ready
        ));

        // 触发器恢复且定义一致
        let restored = db
            .select_sql::<String>(ormer::sql!(format!(
                "SELECT pg_get_triggerdef(t.oid) FROM pg_trigger t \
                 WHERE t.tgrelid = to_regclass('{TRIGGER_TABLE}') AND NOT t.tgisinternal"
            )))
            .collect::<Vec<String>>()
            .await
            .unwrap();
        assert_eq!(restored.len(), 1, "trigger must be restored: {restored:?}");
        assert!(restored[0].contains(TRIGGER_NAME));

        let _ = db.drop_table::<TriggerStatV2>().execute().await;
        let _ = db
            .execute_sql("DROP TABLE IF EXISTS ormer_apply_audit")
            .await;
        let _ = db
            .execute_sql("DROP FUNCTION IF EXISTS ormer_apply_audit_fn()")
            .await;
        Ok(())
    }

    /// advisory_xact_lock：事务内可重入，事务结束自动释放。
    #[tokio::test]
    async fn advisory_xact_lock_roundtrip() -> ormer::Result<()> {
        let db = pg_database().await;
        let tx = db.begin().await?;
        tx.advisory_xact_lock(4242).await?;
        // 同事务可重入
        tx.advisory_xact_lock(4242).await?;
        tx.commit().await?;

        let other = db.begin().await?;
        other.advisory_xact_lock(4242).await?;
        other.rollback().await?;
        Ok(())
    }
}
