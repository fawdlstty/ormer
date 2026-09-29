#![cfg(any(feature = "sqlite", feature = "postgresql"))]

//! 表迁移统一入口 `migrate_table` 的 SQL 形状与幂等行为测试。
//!
//! 覆盖（对应规格书 §4 各能力域）：
//! - migrate_table 建表/补列/收敛/幂等（sqlite e2e）
//! - 索引语义 diff：删除后 migrate 补建（sqlite e2e）
//! - CHECK 约束闭环（PG e2e）
//! - 推不动直接报 unmigratable_schema、不破坏存量表（PG e2e）
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

    /// V2 - note 列（模型删列后库里多余的 note 应被删除）
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
mod sqlite_migrate {
    use super::shared::*;
    use ormer::{Database, DbType, MigrationStep, TableDiagnosis};

    async fn database() -> ormer::Result<Database> {
        Database::connect(DbType::Sqlite, ":memory:").await
    }

    /// migrate_table：首建 → 补列 → 收敛 → 幂等（零步骤）。
    #[tokio::test]
    async fn migrate_creates_adds_and_converges() -> ormer::Result<()> {
        let db = database().await?;

        // 首次 migrate：建表
        let first = db.migrate_table::<ApplyItemV1>().await?;
        assert!(first.created_table, "first migrate should create the table");
        assert!(
            first.executed.iter().any(
                |step| matches!(step, MigrationStep::CreateTable { table, .. } if table == "ormer_apply_items")
            ),
            "executed should contain CreateTable, got {:?}",
            first.executed
        );
        assert!(matches!(
            db.migrate_table::<ApplyItemV1>().await?.diagnosis,
            TableDiagnosis::Ready
        ));

        // 模型加列：migrate 补列
        let second = db.migrate_table::<ApplyItemV2>().await?;
        assert!(!second.created_table);
        assert!(
            second.executed.iter().any(
                |step| matches!(step, MigrationStep::AddColumn { column, .. } if column == "note")
            ),
            "executed should contain AddColumn(note), got {:?}",
            second.executed
        );
        assert!(matches!(
            db.migrate_table::<ApplyItemV2>().await?.diagnosis,
            TableDiagnosis::Ready
        ));

        // 幂等：再次 migrate 零动作
        let third = db.migrate_table::<ApplyItemV2>().await?;
        assert!(!third.created_table);
        assert!(third.executed.is_empty(), "re-migrate must be a no-op");
        assert!(matches!(third.diagnosis, TableDiagnosis::Ready));

        // 多余列删除（固定语义）：模型删列后库里的 note 列被直接删除
        // （SQLite 通过整表重建实现，步骤为 Sql 形态而非 DropColumn 变体）
        let fourth = db.migrate_table::<ApplyItemV3>().await?;
        assert!(!fourth.created_table);
        assert!(
            !fourth.executed.is_empty(),
            "dropping an extra column must execute steps, got {:?}",
            fourth.executed
        );
        let table_sql = db
            .select_sql::<String>(ormer::sql!(
                "SELECT sql FROM sqlite_master \
                 WHERE type = 'table' AND name = 'ormer_apply_items'"
            ))
            .collect::<Vec<String>>()
            .await?;
        assert!(
            table_sql.iter().all(|sql| !sql.contains("note")),
            "extra column note must be dropped from the table, got {table_sql:?}"
        );
        assert!(matches!(
            db.migrate_table::<ApplyItemV3>().await?.diagnosis,
            TableDiagnosis::Ready
        ));
        Ok(())
    }

    /// 索引语义 diff：索引被删后 migrate 按语义签名补建（ormer 自动命名）。
    #[tokio::test]
    async fn migrate_rebuilds_dropped_index() -> ormer::Result<()> {
        let db = database().await?;
        db.migrate_table::<ApplyItemV1>().await?;

        let index_name = "idx_ormer_apply_items_name";
        db.execute_sql(ormer::sql!(format!("DROP INDEX {index_name}")))
            .await?;
        let before = index_names(&db).await?;
        assert!(
            !before.iter().any(|name| name == index_name),
            "index should be gone before migrate: {before:?}"
        );

        db.migrate_table::<ApplyItemV1>().await?;
        let after = index_names(&db).await?;
        assert!(
            after.iter().any(|name| name == index_name),
            "migrate must rebuild the missing index, got {after:?}"
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

    /// validate_table：只读校验——表缺失/结构漂移/多余列（严格口径）均报错，
    /// 与模型一致才通过；全程不执行任何 DDL。
    #[tokio::test]
    async fn validate_reports_missing_drift_and_ready() -> ormer::Result<()> {
        let db = database().await?;

        // 表不存在：报错并说明缺失
        let err = db.validate_table::<ApplyItemV1>().await.unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "missing table must be reported: {err}"
        );

        // 建齐后：通过
        db.migrate_table::<ApplyItemV1>().await?;
        db.validate_table::<ApplyItemV1>().await?;

        // 结构漂移（索引被删）：报错
        db.execute_sql(ormer::sql!("DROP INDEX idx_ormer_apply_items_name"))
            .await?;
        let err = db.validate_table::<ApplyItemV1>().await.unwrap_err();
        assert!(
            err.to_string().contains("schema mismatch"),
            "drift must be reported: {err}"
        );
        db.migrate_table::<ApplyItemV1>().await?;
        db.validate_table::<ApplyItemV1>().await?;

        // 多余列（严格口径）：模型删列后库里还在的列也算差异
        db.migrate_table::<ApplyItemV2>().await?;
        let err = db.validate_table::<ApplyItemV3>().await.unwrap_err();
        assert!(
            err.to_string().contains("schema mismatch"),
            "extra column must be reported in strict view: {err}"
        );
        Ok(())
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
async fn table_migrate_outcome_reports_steps() -> ormer::Result<()> {
    use shared::ApplyItemV1;
    use ormer::{TableDiagnosis, TableMigrateOutcome};
    let db = Database::connect(DbType::Sqlite, ":memory:").await?;
    let outcome = db.migrate_table::<ApplyItemV1>().await?;
    let TableMigrateOutcome {
        diagnosis,
        created_table,
        executed,
    } = outcome;
    assert!(matches!(diagnosis, TableDiagnosis::Migratable(_)));
    assert!(created_table);
    assert_eq!(executed.len(), 1);
    Ok(())
}

/// PostgreSQL 专属能力域：CHECK 闭环、推不动报错、咨询锁。
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

    /// CHECK 闭环：约束被外力删除后 migrate 自动补建（ormer 自动命名）。
    #[tokio::test]
    async fn migrate_restores_dropped_check_constraint() -> ormer::Result<()> {
        let db = pg_database().await;
        let _ = db.drop_table::<CheckTask>().execute().await;
        db.migrate_table::<CheckTask>().await?;

        let constraint_name = "ck_ormer_apply_check_tasks_name";
        db.execute_sql(ormer::sql!(format!(
            "ALTER TABLE ormer_apply_check_tasks DROP CONSTRAINT {constraint_name}"
        )))
        .await?;

        // 诊断与执行同源：outcome.diagnosis 是执行前结论，可审查补建计划
        let outcome = db.migrate_table::<CheckTask>().await?;
        let plan = match outcome.diagnosis {
            TableDiagnosis::Migratable(plan) => plan,
            other => panic!("expected Migratable, got {other:?}"),
        };
        let sql = plan.to_sql()?;
        assert!(
            sql.contains("ADD CONSTRAINT") && sql.contains(constraint_name),
            "plan should add the missing check constraint, got: {sql}"
        );
        assert!(matches!(
            db.migrate_table::<CheckTask>().await?.diagnosis,
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
        db.migrate_table::<PkItemV1>().await?;
        db.insert(&PkItemV1 {
            bucket_start: "2026-09-08 10:00:00".to_string(),
            project_id: "p1".to_string(),
            count: 1,
        })
        .execute()
        .await?;

        let outcome = db.migrate_table::<PkItemV2>().await?;
        let plan = match outcome.diagnosis {
            TableDiagnosis::Migratable(plan) => plan,
            other => panic!("pk change should be migratable in place, got {other:?}"),
        };
        assert!(
            plan.to_sql()?.contains("ADD PRIMARY KEY"),
            "plan should change the primary key in place"
        );

        assert!(matches!(
            db.migrate_table::<PkItemV2>().await?.diagnosis,
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

    /// 旧版建出的普通表形态（无超表声明，主键也不含分区列）。
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_hyp_events"]
    struct HypEventPlain {
        project_id: String,
        #[primary]
        agvid: i32,
        #[primary]
        event_time: chrono::NaiveDateTime,
        payload: String,
    }

    /// 现行超表模型（ynyz 的 collect.collect_exception_logs 同构：
    /// project_id 空间分区 + event_time 时间分区，主键不含分区列、
    /// 由 effective PK 机制补入）。
    #[derive(Debug, ormer::Model)]
    #[table = "ormer_apply_hyp_events"]
    struct HypEvent {
        #[hypertable]
        project_id: String,
        #[primary]
        agvid: i32,
        #[primary]
        #[hypertable(std::time::Duration::from_secs(86_400))]
        event_time: chrono::NaiveDateTime,
        payload: String,
    }

    /// 存量普通表 → 超表原地转换自愈（ynyz 形态）：旧版建出的普通表，
    /// migrate_table 先把主键对齐到含分区列，再 create_hypertable 原地
    /// 转换（migrate_data => TRUE，存量数据随迁），复验收敛、再跑幂等。
    #[tokio::test]
    async fn plain_table_converts_to_hypertable_in_place() -> ormer::Result<()> {
        const TABLE: &str = "ormer_apply_hyp_events";
        let db = pg_database().await;
        let _ = db.drop_table::<HypEvent>().execute().await;

        // 无 TimescaleDB 时跳过：原地转换依赖 create_hypertable，
        // 与 routed_columnstore_test 的守卫约定一致
        let timescale = db
            .select_sql::<bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')",
            )
            .collect::<Vec<bool>>()
            .await?
            .into_iter()
            .next()
            .unwrap_or(false);
        if !timescale {
            eprintln!(
                "TimescaleDB not installed; skipping plain->hypertable in-place conversion test"
            );
            return Ok(());
        }

        // 造出 ynyz 形态：按旧版模型建普通表并写入一行存量数据
        db.migrate_table::<HypEventPlain>().await?;
        db.insert(&HypEventPlain {
            project_id: "demo-project-001".to_string(),
            agvid: 1,
            event_time: chrono::NaiveDateTime::parse_from_str(
                "2026-09-24 20:00:00",
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap(),
            payload: "kept".to_string(),
        })
        .execute()
        .await?;

        // 修复前：Err(unmigratable_schema .. Hypertable mismatch) 死循环
        let outcome = db.migrate_table::<HypEvent>().await?;
        let plan = match outcome.diagnosis {
            TableDiagnosis::Migratable(plan) => plan,
            other => panic!("plain->hypertable should be migratable in place, got {other:?}"),
        };
        let sql = plan.to_sql()?;
        assert!(
            sql.contains("ADD PRIMARY KEY"),
            "plan should align the primary key with partition columns first: {sql}"
        );
        assert!(
            sql.contains("create_hypertable"),
            "plan should convert the regular table in place: {sql}"
        );

        let hypertables = db
            .select_sql::<i64>(ormer::sql!(format!(
                "SELECT count(*) FROM timescaledb_information.hypertables \
                 WHERE hypertable_name = '{TABLE}'"
            )))
            .collect::<Vec<i64>>()
            .await?;
        assert_eq!(
            hypertables.first().copied(),
            Some(1),
            "table must be a hypertable after migrate"
        );

        // migrate_data => TRUE：存量行随迁保留
        let rows = db
            .select_sql::<i64>(ormer::sql!(format!("SELECT count(*) FROM {TABLE}")))
            .collect::<Vec<i64>>()
            .await?;
        assert_eq!(rows.first().copied(), Some(1), "data must survive");

        assert!(matches!(
            db.migrate_table::<HypEvent>().await?.diagnosis,
            TableDiagnosis::Ready
        ));

        let _ = db.drop_table::<HypEvent>().execute().await;
        Ok(())
    }

    /// 推不动（无法回填的 NOT NULL 新列）：直接返回 unmigratable_schema 错误，
    /// 不删表重建，存量数据与业务触发器原样保留。
    #[tokio::test]
    async fn unmigratable_difference_reports_error_and_keeps_table() -> ormer::Result<()> {
        let db = pg_database().await;
        let _ = db.drop_table::<TriggerStatV1>().execute().await;
        let _ = db.execute_sql("DROP TABLE IF EXISTS ormer_apply_audit").await;
        let _ = db
            .execute_sql("DROP FUNCTION IF EXISTS ormer_apply_audit_fn()")
            .await;

        db.migrate_table::<TriggerStatV1>().await?;
        db.insert(&TriggerStatV1 {
            bucket_start: "2026-09-08 10:00:00".to_string(),
            project_id: "p1".to_string(),
            count: 1,
        })
        .execute()
        .await
        .unwrap();
        create_business_trigger(&db).await;

        // 无法回填的 NOT NULL 新列：推不动 → 直接报 unmigratable_schema
        let err = db.migrate_table::<TriggerStatV2>().await.unwrap_err();
        assert!(
            err.is_unmigratable_schema(),
            "unbackfillable not-null column must report unmigratable_schema, got {err}"
        );

        // 不删表重建：存量数据与业务触发器原样保留
        let rows = db
            .select_sql::<i64>(ormer::sql!(format!(
                "SELECT count(*) FROM {TRIGGER_TABLE}"
            )))
            .collect::<Vec<i64>>()
            .await?;
        assert_eq!(rows.first().copied(), Some(1), "data must survive");
        let triggers = db
            .select_sql::<String>(ormer::sql!(format!(
                "SELECT pg_get_triggerdef(t.oid) FROM pg_trigger t \
                 WHERE t.tgrelid = to_regclass('{TRIGGER_TABLE}') AND NOT t.tgisinternal"
            )))
            .collect::<Vec<String>>()
            .await
            .unwrap();
        assert_eq!(triggers.len(), 1, "trigger must be kept: {triggers:?}");
        assert!(triggers[0].contains(TRIGGER_NAME));

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
