#![cfg(feature = "postgresql")]

pub mod _test_common;

use ormer::Database;

/// 模拟线上旧表：project_id 在主键内（对应 ticket_hourly_stats 事故时的线上表）
#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_stats"]
struct RebuildStatV1 {
    #[primary]
    project_id: String,
    #[primary]
    bucket_start: String,
    count: i64,
}

/// 新模型：project_id 移出主键（对应被回滚的 WIP 改动）
#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_stats"]
struct RebuildStatV2 {
    #[primary]
    bucket_start: String,
    project_id: String,
    count: i64,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_new_pk"]
struct RebuildStatNewPkBase {
    #[primary]
    bucket_start: String,
    project_id: String,
    count: i64,
}

/// 新增主键列，旧表不可能就地补主键
#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_new_pk"]
struct RebuildStatNewPk {
    #[primary]
    bucket_start: String,
    #[primary]
    seq: i64,
    project_id: String,
    count: i64,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_not_null"]
struct RebuildStatNotNullBase {
    #[primary]
    bucket_start: String,
    project_id: String,
    count: i64,
}

/// 无默认值的非空新列，表里有数据时无法就地回填
#[derive(Debug, ormer::Model)]
#[table = "ormer_rebuild_not_null"]
struct RebuildStatNotNull {
    #[primary]
    bucket_start: String,
    project_id: String,
    count: i64,
    note: String,
}

const AUDIT_TABLE: &str = "ormer_rebuild_audit";
const TRIGGER_NAME: &str = "ormer_rebuild_audit_trg";

async fn create_audit_objects(db: &Database) {
    db.execute_sql(ormer::sql!(format!(
        "CREATE TABLE IF NOT EXISTS {AUDIT_TABLE} (note text)"
    )))
    .await
    .unwrap();
    db.execute_sql(ormer::sql!(
        r#"
        CREATE OR REPLACE FUNCTION ormer_rebuild_audit_fn() RETURNS trigger AS $$
        BEGIN
            INSERT INTO ormer_rebuild_audit (note) VALUES (TG_TABLE_NAME);
            RETURN NULL;
        END
        $$ LANGUAGE plpgsql
        "#
    ))
    .await
    .unwrap();
    db.execute_sql(ormer::sql!(format!(
        "DROP TRIGGER IF EXISTS {TRIGGER_NAME} ON ormer_rebuild_stats"
    )))
    .await
    .unwrap();
    db.execute_sql(ormer::sql!(format!(
        "CREATE TRIGGER {TRIGGER_NAME} AFTER INSERT ON ormer_rebuild_stats \
         FOR EACH STATEMENT EXECUTE FUNCTION ormer_rebuild_audit_fn()"
    )))
    .await
    .unwrap();
}

async fn snapshot_triggers(db: &Database, old_style: bool) -> ormer::Result<Vec<String>> {
    let condition = if old_style {
        "trigger_info.tgrelid = {table_name}::regclass"
    } else {
        "trigger_info.tgrelid = to_regclass({table_name})"
    };
    db.select_sql::<String>(ormer::sql!(
        format!(
            r#"
            SELECT pg_get_triggerdef(trigger_info.oid)
            FROM pg_trigger trigger_info
            WHERE {condition}
              AND NOT trigger_info.tgisinternal
            "#,
        ),
        table_name = "ormer_rebuild_stats".to_string(),
    ))
    .collect::<Vec<String>>()
    .await
}

async fn audit_rows(db: &Database) -> i64 {
    db.select_sql::<i64>(ormer::sql!(format!(
        "SELECT count(*) FROM {AUDIT_TABLE}"
    )))
    .collect::<Vec<i64>>()
    .await
    .unwrap()
    .first()
    .copied()
    .unwrap()
}

/// 复现 ticket_hourly_stats 事故：主键变更 → 迁移拒绝 → 触发器快照 → 删表重建 →
/// 触发器还原且仍然生效。老写法 `绑定参数::regclass` 在驱动层必败，必须用 to_regclass。
#[tokio::test]
async fn unmigratable_primary_key_change_rebuild_flow() {
    let db = Database::connect(
        ormer::DbType::PostgreSQL,
        _test_common::postgresql_config().1,
    )
    .await
    .unwrap();

    let _ = db.drop_table::<RebuildStatV1>().execute().await;
    db.execute_sql(ormer::sql!(format!("DROP TABLE IF EXISTS {AUDIT_TABLE}")))
        .await
        .unwrap();
    db.execute_sql(ormer::sql!("DROP FUNCTION IF EXISTS ormer_rebuild_audit_fn()"))
        .await
        .unwrap();

    db.create_table::<RebuildStatV1>().execute().await.unwrap();
    db.insert(&RebuildStatV1 {
        project_id: "p1".to_string(),
        bucket_start: "2026-09-08 10:00:00".to_string(),
        count: 1,
    })
    .execute()
    .await
    .unwrap();
    create_audit_objects(&db).await;

    // 1. validate_table 必须报出 schema mismatch（ensure_table 的进入条件）
    let validate_err = db.validate_table::<RebuildStatV2>().await.unwrap_err();
    assert!(
        validate_err.to_string().starts_with("Schema mismatch"),
        "unexpected validate error: {validate_err}"
    );

    // 2. 迁移必须失败，且失败类型可编程判定（替代脆弱的字符串匹配）
    let migrate_err = db
        .migrate_table::<RebuildStatV2>()
        .execute()
        .await
        .unwrap_err();
    assert!(
        migrate_err.is_unmigratable_schema(),
        "expected unmigratable schema error, got: {migrate_err}"
    );

    // 3. 老的快照写法把表名绑成 String 参数再 ::regclass，驱动层必败（事故根因）
    let old_snapshot = snapshot_triggers(&db, true).await;
    assert!(
        old_snapshot.is_err(),
        "legacy ::regclass param binding must fail, got {:?}",
        old_snapshot.unwrap()
    );

    // 4. to_regclass 写法能拿到触发器定义
    let snapshot = snapshot_triggers(&db, false).await.unwrap();
    assert_eq!(snapshot.len(), 1, "snapshot: {snapshot:?}");
    assert!(snapshot[0].contains(TRIGGER_NAME), "snapshot: {snapshot:?}");

    // 5. 删表重建并还原触发器
    db.drop_table::<RebuildStatV2>().execute().await.unwrap();
    db.create_table::<RebuildStatV2>().execute().await.unwrap();
    for definition in &snapshot {
        db.execute_sql(ormer::sql!(definition.clone()))
            .await
            .unwrap();
    }

    // 6. 重建后新模型校验通过
    db.validate_table::<RebuildStatV2>().await.unwrap();

    // 7. 还原的触发器仍然生效
    db.insert(&RebuildStatV2 {
        project_id: "p1".to_string(),
        bucket_start: "2026-09-08 11:00:00".to_string(),
        count: 2,
    })
    .execute()
    .await
    .unwrap();
    assert_eq!(audit_rows(&db).await, 1);

    let _ = db.drop_table::<RebuildStatV2>().execute().await;
    let _ = db
        .execute_sql(ormer::sql!(format!("DROP TABLE IF EXISTS {AUDIT_TABLE}")))
        .await;
    let _ = db
        .execute_sql(ormer::sql!("DROP FUNCTION IF EXISTS ormer_rebuild_audit_fn()"))
        .await;
}

#[tokio::test]
async fn new_primary_key_column_is_unmigratable() {
    let db = Database::connect(
        ormer::DbType::PostgreSQL,
        _test_common::postgresql_config().1,
    )
    .await
    .unwrap();

    let _ = db.drop_table::<RebuildStatNewPk>().execute().await;
    db.create_table::<RebuildStatNewPkBase>()
        .execute()
        .await
        .unwrap();

    let err = db
        .migrate_table::<RebuildStatNewPk>()
        .plan()
        .await
        .unwrap_err();
    assert!(
        err.is_unmigratable_schema(),
        "expected unmigratable schema error, got: {err}"
    );

    let _ = db.drop_table::<RebuildStatNewPkBase>().execute().await;
}

#[tokio::test]
async fn not_null_column_without_backfill_is_unmigratable() {
    let db = Database::connect(
        ormer::DbType::PostgreSQL,
        _test_common::postgresql_config().1,
    )
    .await
    .unwrap();

    let _ = db.drop_table::<RebuildStatNotNull>().execute().await;
    db.create_table::<RebuildStatNotNullBase>()
        .execute()
        .await
        .unwrap();
    db.insert(&RebuildStatNotNullBase {
        bucket_start: "2026-09-08 10:00:00".to_string(),
        project_id: "p1".to_string(),
        count: 1,
    })
    .execute()
    .await
    .unwrap();

    let err = db
        .migrate_table::<RebuildStatNotNull>()
        .plan()
        .await
        .unwrap_err();
    assert!(
        err.is_unmigratable_schema(),
        "expected unmigratable schema error, got: {err}"
    );

    let _ = db.drop_table::<RebuildStatNotNullBase>().execute().await;
}
