#![cfg(feature = "postgresql")]

//! `migrate_table` 保数据重建路径测试（方案正本：roboupdater 仓 `migrate.md` §3/§7）。
//!
//! 场景 → 测试函数对照（场景 8 为既有测试回归确认，本文件不改写它们，见文件尾说明）：
//! 1. 端到端正例                 → `rebuild_preserves_data_end_to_end`
//! 2. 前置冲突拒绝               → `duplicate_new_pk_rows_rejected_with_diagnostic_sql`
//!                               → `null_in_new_pk_column_rejected`
//! 3. 非空新列无来源拒绝         → `not_null_new_column_without_source_rejected`
//! 4. 遗留备份自动清理           → `legacy_backup_table_auto_cleaned`
//! 5. 阶段一失败回滚零残留       → `stage_one_failure_rolls_back_zero_residue`
//! 6. 跨 schema 不干预（拍板 C） → `cross_schema_public_table_untouched`
//! 7. 改名列映射（拍板 D）       → `renamed_time_column_maps_old_values`
//!                               → `unannotated_column_change_drops_old_and_nulls_new`
//!
//! 约定：
//! - 全部只走公开入口 `db.migrate_table::<T>()`，不直接调用内部函数；
//! - "中间版老表"用裸 SQL 建成真超表（`create_hypertable` / `add_dimension`），
//!   使诊断确实判 NeedsRebuild 而非其他差异；
//! - 测试间表名互不冲突（`ormer_rb_*` 前缀），每个测试先 DROP IF EXISTS 自清理；
//! - 无 TimescaleDB 时跳过（与既有超表测试同一守卫）。
//!
//! 依赖实现契约（并行实现中；未落地前本文件编译失败属预期）：
//! - `TableMigrateOutcome` 新增 `migrated_rows: Option<i64>`、`backup_table: Option<String>`；
//! - 列改名标注 `#[column(old_name = "老列名")]`；
//! - NeedsRebuild（超表时间/空间分区列变更、主键换轨）且表有数据 → 保数据重建；
//!   新主键冲突 / 新主键列 NULL / 非空新列无来源 → 拒绝且原表原样，错误含诊断 SQL。

use ormer::{Database, TableDiagnosis};

// ---------------------------------------------------------------------------
// 公共辅助
// ---------------------------------------------------------------------------

fn ts(value: &str) -> chrono::NaiveDateTime {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").unwrap()
}

async fn pg_database() -> Database {
    Database::connect(
        ormer::DbType::PostgreSQL,
        crate::_test_common::postgresql_config().1,
    )
    .await
    .unwrap()
}

/// 无 TimescaleDB 时跳过：保数据重建只发生在超表维度不匹配场景，
/// 与既有超表测试（`table_migrate_test.rs` 等）同一守卫约定。
async fn require_timescale(db: &Database) -> bool {
    let installed = db
        .select_sql::<bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')",
        )
        .collect::<Vec<bool>>()
        .await
        .expect("query pg_extension")
        .into_iter()
        .next()
        .unwrap_or(false);
    if !installed {
        eprintln!("TimescaleDB not installed; skipping migrate-rebuild test");
    }
    installed
}

async fn scalar_i64(db: &Database, sql: String) -> i64 {
    db.select_sql::<i64>(sql)
        .collect::<Vec<i64>>()
        .await
        .expect("scalar query must succeed")
        .first()
        .copied()
        .unwrap_or(0)
}

async fn scalar_bool(db: &Database, sql: String) -> bool {
    db.select_sql::<bool>(sql)
        .collect::<Vec<bool>>()
        .await
        .expect("scalar query must succeed")
        .into_iter()
        .next()
        .unwrap_or(false)
}

async fn string_rows(db: &Database, sql: String) -> Vec<String> {
    db.select_sql::<String>(sql)
        .collect::<Vec<String>>()
        .await
        .expect("query must succeed")
}

async fn drop_quietly(db: &Database, name: &str) {
    let _ = db.execute_sql(format!("DROP TABLE IF EXISTS {name}")).await;
}

/// 测试开始/结束清理：主表 + 潜在备份表名。
async fn cleanup_tables(db: &Database, names: &[&str]) {
    for name in names {
        drop_quietly(db, name).await;
        drop_quietly(db, &format!("{name}_migrate_old")).await;
    }
}

/// 断言没有任何 `{table}_migrate_old` 残留（拍板 B）。
async fn assert_no_backup_residue(db: &Database, table: &str) {
    let residue = scalar_i64(
        db,
        format!(
            "SELECT count(*) FROM pg_tables \
             WHERE schemaname || '.' || tablename = 'public.{table}_migrate_old' \
             OR tablename = '{table}_migrate_old'"
        ),
    )
    .await;
    assert_eq!(
        residue, 0,
        "no {table}_migrate_old residue expected (decision B)"
    );
}

/// 时间维断言：兼容 time_interval 为 INTERVAL（'1 day'）或整型微秒
/// （86400000000）两种 information view 形态。
async fn assert_time_dimension(db: &Database, table: &str, column: &str, schema: &str) {
    let rows = string_rows(
        db,
        format!(
            "SELECT time_interval::text FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{table}' AND hypertable_schema = '{schema}' \
             AND column_name = '{column}' \
             AND time_interval::text IN ('1 day', '86400000000')"
        ),
    )
    .await;
    assert_eq!(
        rows.len(),
        1,
        "time dimension must be {schema}.{table}({column}) with 1-day interval"
    );
}

/// 主键列集合断言（pg_constraint，列名排序后比对）。
async fn assert_primary_key_columns(db: &Database, table: &str, expected: &[&str]) {
    let mut columns = string_rows(
        db,
        format!(
            "SELECT a.attname FROM pg_constraint c \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey) \
             WHERE c.contype = 'p' AND c.conrelid = to_regclass('{table}') \
               AND NOT a.attisdropped ORDER BY a.attname"
        ),
    )
    .await;
    columns.sort();
    let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(columns, expected, "primary key columns of {table}");
}

// ---------------------------------------------------------------------------
// 场景 1：端到端正例
// ---------------------------------------------------------------------------

/// 新 model：时间分区 update_time → event_time、空间分区 project_id ×4、
/// 主键换轨为 (project_id, zone_id, agvid, event_time)、索引增删
/// （新增 update_time 索引；老 payload 索引随模型删除）。
/// 全部列同名同类型直取（对应中间版→最终版"字段名零修改"）。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_e2e_events"]
struct RbE2eEvent {
    #[hypertable]
    #[primary]
    project_id: String,
    #[primary]
    zone_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    #[index]
    update_time: chrono::NaiveDateTime,
    payload: String,
}

/// 端到端：裸 SQL 建中间版超表（时间维 update_time 1 天 + 空间维 project_id ×4、
/// 主键含 update_time、带 payload 旧索引）灌 5 行 → migrate_table → 断言
/// 行数/维度/主键/索引/无备份残留/outcome.migrated_rows，再跑一遍幂等 Ready。
#[tokio::test]
async fn rebuild_preserves_data_end_to_end() {
    const TABLE: &str = "ormer_rb_e2e_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    // 中间版老表：真超表（时间维 update_time + 空间维 project_id ×4）
    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             zone_id TEXT NOT NULL, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT add_dimension('{TABLE}', 'project_id', \
         number_partitions => 4, if_not_exists => TRUE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, zone_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    // 旧索引：新 model 已删除（payload 不再建索引）
    db.execute_sql(format!(
        "CREATE INDEX idx_{TABLE}_payload ON {TABLE} (payload)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, zone_id, agvid, event_time, update_time, payload) VALUES \
         ('p1', 'z1', 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', 'a'), \
         ('p1', 'z1', 2, '2026-10-02 08:00:00', '2026-10-12 09:05:00', 'b'), \
         ('p1', 'z2', 3, '2026-10-03 08:00:00', '2026-10-12 09:10:00', 'c'), \
         ('p2', 'z1', 4, '2026-10-04 08:00:00', '2026-10-12 09:15:00', 'd'), \
         ('p2', 'z2', 5, '2026-10-05 08:00:00', '2026-10-12 09:20:00', 'e')"
    ))
    .await
    .unwrap();

    let outcome = db.migrate_table::<RbE2eEvent>().await.unwrap();

    // 回灌行数上报（实现契约）
    assert_eq!(
        outcome.migrated_rows,
        Some(5),
        "migrated_rows must report backfilled row count, got {outcome:?}"
    );
    assert!(
        outcome.backup_table.is_none(),
        "no legacy backup was pre-seeded, backup_table must be None"
    );

    // 行数对账
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 5, "all rows must survive the rebuild");

    // 超表维度：时间维 event_time 1 天、空间维 project_id ×4、共 2 维
    assert_time_dimension(&db, TABLE, "event_time", "public").await;
    let space_dim = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND hypertable_schema = 'public' \
             AND column_name = 'project_id' AND num_partitions = 4"
        ),
    )
    .await;
    assert_eq!(space_dim, 1, "space dimension project_id x4 expected");
    let total_dims = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND hypertable_schema = 'public'"
        ),
    )
    .await;
    assert_eq!(total_dims, 2, "exactly time+space dimensions expected");
    let old_time_dim = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND hypertable_schema = 'public' \
             AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(old_time_dim, 0, "old time dimension must be gone");

    // 主键列集合 == model 声明
    assert_primary_key_columns(
        &db,
        TABLE,
        &["project_id", "zone_id", "agvid", "event_time"],
    )
    .await;

    // 索引集合语义 == model 声明：恰好 PK + update_time 两个，
    // payload 旧索引删除、无 TimescaleDB 默认索引污染
    let total_indexes = scalar_i64(
        &db,
        format!("SELECT count(*) FROM pg_indexes WHERE tablename = '{TABLE}'"),
    )
    .await;
    assert_eq!(
        total_indexes, 2,
        "parent-table indexes must be exactly PK + model update_time index"
    );
    let payload_indexes = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM pg_indexes \
             WHERE tablename = '{TABLE}' AND indexdef ILIKE '%payload%'"
        ),
    )
    .await;
    assert_eq!(payload_indexes, 0, "dropped payload index must be gone");
    let update_time_indexes = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM pg_indexes \
             WHERE tablename = '{TABLE}' AND indexdef ILIKE '%update_time%'"
        ),
    )
    .await;
    assert_eq!(
        update_time_indexes, 1,
        "model-declared update_time index must exist"
    );

    // 逐行数据保真：时间列与业务列各自与灌入值一致（证明分区列值
    // 来自正确的源列）。select_sql 行元组最多支持 3 元，拆两条查询，
    // agvid 唯一保证 ORDER BY 稳定。
    let time_rows = db
        .select_sql::<(i32, chrono::NaiveDateTime, chrono::NaiveDateTime)>(format!(
            "SELECT agvid, event_time, update_time FROM {TABLE} ORDER BY agvid"
        ))
        .collect::<Vec<(i32, chrono::NaiveDateTime, chrono::NaiveDateTime)>>()
        .await
        .unwrap();
    let expected_times = [
        (1, "2026-10-01 08:00:00", "2026-10-12 09:00:00"),
        (2, "2026-10-02 08:00:00", "2026-10-12 09:05:00"),
        (3, "2026-10-03 08:00:00", "2026-10-12 09:10:00"),
        (4, "2026-10-04 08:00:00", "2026-10-12 09:15:00"),
        (5, "2026-10-05 08:00:00", "2026-10-12 09:20:00"),
    ];
    assert_eq!(time_rows.len(), expected_times.len());
    for (got, want) in time_rows.iter().zip(expected_times.iter()) {
        assert_eq!(
            (got.0, got.1, got.2),
            (want.0, ts(want.1), ts(want.2)),
            "event_time/update_time must be preserved verbatim"
        );
    }
    let payload_rows = db
        .select_sql::<(i32, String, String)>(format!(
            "SELECT agvid, project_id, payload FROM {TABLE} ORDER BY agvid"
        ))
        .collect::<Vec<(i32, String, String)>>()
        .await
        .unwrap();
    let expected_payloads = [
        (1, "p1", "a"),
        (2, "p1", "b"),
        (3, "p1", "c"),
        (4, "p2", "d"),
        (5, "p2", "e"),
    ];
    assert_eq!(payload_rows.len(), expected_payloads.len());
    for (got, want) in payload_rows.iter().zip(expected_payloads.iter()) {
        assert_eq!(
            (got.0, got.1.as_str(), got.2.as_str()),
            (want.0, want.1, want.2),
            "project_id/payload must be preserved verbatim"
        );
    }

    // 拍板 B：无备份表残留
    assert_no_backup_residue(&db, TABLE).await;

    // 幂等：第二次 migrate_table 应为 Ready 语义、不重复迁移
    let second = db.migrate_table::<RbE2eEvent>().await.unwrap();
    assert!(
        matches!(second.diagnosis, TableDiagnosis::Ready),
        "second migrate must be Ready, got {:?}",
        second.diagnosis
    );

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 2：前置冲突拒绝（拍板 A）
// ---------------------------------------------------------------------------

/// 新 model：主键换轨为 (project_id, zone_id, agvid, event_time)，
/// zone_id 升入主键；时间分区列 update_time → event_time。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_conflict_events"]
struct RbConflictEvent {
    #[primary]
    project_id: String,
    #[primary]
    zone_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    payload: String,
}

/// 前置冲突（拍板 A）：老数据存在同 (project_id, zone_id, agvid, event_time)
/// 两行（ds 重发形态）→ 迁移前拒绝，报 unmigratable_schema，错误信息含
/// 诊断 SQL 字样；原表行数/结构原样，无备份残留。
#[tokio::test]
async fn duplicate_new_pk_rows_rejected_with_diagnostic_sql() {
    const TABLE: &str = "ormer_rb_conflict_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             zone_id TEXT, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, zone_id, agvid, event_time, update_time, payload) VALUES \
         ('p1', 'z1', 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', 'first'), \
         ('p1', 'z1', 1, '2026-10-01 08:00:00', '2026-10-12 10:00:00', 'resend'), \
         ('p1', 'z2', 2, '2026-10-02 08:00:00', '2026-10-12 10:00:00', 'clean')"
    ))
    .await
    .unwrap();

    let err = db
        .migrate_table::<RbConflictEvent>()
        .await
        .expect_err("duplicate rows for the new PK must be rejected");
    assert!(
        err.is_unmigratable_schema(),
        "expected unmigratable_schema, got: {err}"
    );
    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("group by"),
        "error must embed the diagnostic SQL, got: {err}"
    );

    // 原表原样：行数不变、老时间维仍在、无备份残留
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 3, "original data must be untouched");
    let dims = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(dims, 1, "original time dimension must be untouched");
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

/// NULL 场景独立模型（表名 `ormer_rb_null_events`，与冲突场景表分开）：
/// 主键换轨为 (project_id, zone_id, agvid, event_time)，zone_id 升入主键。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_null_events"]
struct RbNullEvent {
    #[primary]
    project_id: String,
    #[primary]
    zone_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    payload: String,
}

/// 前置冲突（拍板 A）：新主键列（zone_id）在老数据中含 NULL →
/// 回灌 INSERT 必然违反 NOT NULL，迁移前拒绝；原表原样。
#[tokio::test]
async fn null_in_new_pk_column_rejected() {
    const TABLE: &str = "ormer_rb_null_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             zone_id TEXT, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, zone_id, agvid, event_time, update_time, payload) VALUES \
         ('p1', NULL, 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', 'null-zone'), \
         ('p1', 'z1', 2, '2026-10-02 08:00:00', '2026-10-12 09:05:00', 'clean')"
    ))
    .await
    .unwrap();

    let err = db
        .migrate_table::<RbNullEvent>()
        .await
        .expect_err("NULL in a new-PK column must be rejected");
    assert!(
        err.is_unmigratable_schema(),
        "expected unmigratable_schema, got: {err}"
    );

    // 原表原样：行数不变、NULL 行仍在、老时间维仍在
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 2, "original data must be untouched");
    let null_rows = scalar_i64(
        &db,
        format!("SELECT count(*) FROM {TABLE} WHERE zone_id IS NULL"),
    )
    .await;
    assert_eq!(null_rows, 1, "the NULL zone_id row must be untouched");
    let dims = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(dims, 1, "original time dimension must be untouched");
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 3：非空新列无来源拒绝
// ---------------------------------------------------------------------------

/// 新 model：`operator_name` 为非 Option、无默认值、老表没有的列 →
/// 前置检查 c 拒绝；event_time 通过改名标注可解析（隔离变量为
/// operator_name 无来源）。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_nonnull_events"]
struct RbNonNullEvent {
    #[primary]
    project_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    #[column(old_name = "update_time")]
    event_time: chrono::NaiveDateTime,
    payload: String,
    /// 非 Option 且无默认值、老表没有 → 无填充来源，必须拒绝保数据。
    operator_name: String,
}

/// 非空新列无来源：拒绝保数据重建，原表结构（列集合）与数据原样。
#[tokio::test]
async fn not_null_new_column_without_source_rejected() {
    const TABLE: &str = "ormer_rb_nonnull_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, agvid, event_time, update_time, payload) VALUES \
         ('p1', 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', 'a'), \
         ('p1', 2, '2026-10-02 08:00:00', '2026-10-12 09:05:00', 'b')"
    ))
    .await
    .unwrap();

    let err = db
        .migrate_table::<RbNonNullEvent>()
        .await
        .expect_err("not-null new column without source must be rejected");
    assert!(
        err.is_unmigratable_schema(),
        "expected unmigratable_schema, got: {err}"
    );

    // 原表原样：行数不变、列集合不变（无 operator_name、update_time 还在）
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 2, "original data must be untouched");
    let operator_col = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'operator_name'"
        ),
    )
    .await;
    assert_eq!(operator_col, 0, "new sourceless column must not appear");
    let update_time_col = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(update_time_col, 1, "original columns must be untouched");
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 4：遗留备份表自动清理
// ---------------------------------------------------------------------------

/// 新 model：时间分区列 update_time → event_time（同名直取，老表已有该列）。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_legacy_events"]
struct RbLegacyEvent {
    #[primary]
    project_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    payload: String,
}

/// 遗留备份清理：预置含数据的 `{table}_migrate_old` → 前置检查 a 直接 DROP
/// 后继续，迁移正常完成，`outcome.backup_table` 记录被清理的备份表名，
/// 结束后无任何 `{table}_migrate_old` 残留（拍板 B）。
#[tokio::test]
async fn legacy_backup_table_auto_cleaned() {
    const TABLE: &str = "ormer_rb_legacy_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, agvid, event_time, update_time, payload) VALUES \
         ('p1', 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', 'a'), \
         ('p1', 2, '2026-10-02 08:00:00', '2026-10-12 09:05:00', 'b'), \
         ('p2', 1, '2026-10-03 08:00:00', '2026-10-12 09:10:00', 'c')"
    ))
    .await
    .unwrap();

    // 遗留备份表：含数据的普通表（历史版本/异常现场残留形态）
    db.execute_sql(format!("CREATE TABLE {TABLE}_migrate_old (note TEXT)"))
        .await
        .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE}_migrate_old (note) VALUES ('stale-legacy-backup')"
    ))
    .await
    .unwrap();

    let outcome = db.migrate_table::<RbLegacyEvent>().await.unwrap();
    assert_eq!(outcome.migrated_rows, Some(3));
    assert!(
        matches!(&outcome.backup_table, Some(name) if name.ends_with("_migrate_old")),
        "backup_table must record the cleaned legacy backup name, got {outcome:?}"
    );

    // 迁移正常完成：行数对账 + 新时间维生效
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 3);
    assert_time_dimension(&db, TABLE, "event_time", "public").await;

    // 无任何 {TABLE}_migrate_old 残留（遗留备份与阶段一备份都已不在）
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 5：阶段一失败回滚零残留
// ---------------------------------------------------------------------------

/// 新 model：stage 声明为 i64 而老表 stage 是 TIMESTAMP——PG 不存在
/// timestamp→integer 的任何转换（赋值/隐式皆无），回灌
/// `INSERT ... SELECT` 必败，且该失败发生在阶段一事务内
/// （前置检查 a/b/c 均不校验列类型）。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_rollback_events"]
struct RbRollbackEvent {
    #[primary]
    project_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    stage: i64,
}

/// 阶段一失败（回灌中途报错）→ ROLLBACK：老表原名/结构/数据完全复原，
/// 无新表、无备份表残留。
#[tokio::test]
async fn stage_one_failure_rolls_back_zero_residue() {
    const TABLE: &str = "ormer_rb_rollback_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    // stage 为 TIMESTAMP，新 model 声明 i64 → 回灌必败
    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             agvid INTEGER NOT NULL, \
             event_time TIMESTAMP NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             stage TIMESTAMP NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, agvid, event_time, update_time, stage) VALUES \
         ('p1', 1, '2026-10-01 08:00:00', '2026-10-12 09:00:00', '2026-10-01 10:00:00'), \
         ('p1', 2, '2026-10-02 08:00:00', '2026-10-12 09:05:00', '2026-10-02 10:00:00'), \
         ('p2', 3, '2026-10-03 08:00:00', '2026-10-12 09:10:00', '2026-10-03 10:00:00')"
    ))
    .await
    .unwrap();

    let result = db.migrate_table::<RbRollbackEvent>().await;
    assert!(
        result.is_err(),
        "timestamp->integer backfill cannot succeed; expected an error"
    );

    // 老表原名还在（ROLLBACK 使 RENAME 复原），且仍是原形态超表
    let table_exists = scalar_bool(
        &db,
        format!(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = '{TABLE}')"
        ),
    )
    .await;
    assert!(table_exists, "original table must be restored by ROLLBACK");
    let dims = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_name = '{TABLE}' AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(dims, 1, "original time dimension must be intact");

    // 结构复原：stage 仍是 timestamp
    let stage_type = string_rows(
        &db,
        format!(
            "SELECT data_type FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'stage'"
        ),
    )
    .await;
    assert_eq!(
        stage_type,
        vec!["timestamp without time zone".to_string()],
        "stage column type must be restored"
    );

    // 数据复原：行数与 stage 值逐行原样。
    // 用 to_char 文本比对：裸 timestamp 列经 ormer 读取会被时区归一
    // （按 UTC 瞬间转本地 naive），文本读出才是存储原值。
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 3, "all original rows must survive");
    let stages = string_rows(
        &db,
        format!("SELECT to_char(stage, 'YYYY-MM-DD HH24:MI:SS') FROM {TABLE} ORDER BY agvid"),
    )
    .await;
    assert_eq!(
        stages,
        vec![
            "2026-10-01 10:00:00".to_string(),
            "2026-10-02 10:00:00".to_string(),
            "2026-10-03 10:00:00".to_string(),
        ],
        "stage values must be intact"
    );

    // 零残留：无备份表
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 6：跨 schema 不干预（拍板 C）
// ---------------------------------------------------------------------------

/// 新 model：声明在 collect schema；诊断只看 collect.{TABLE}，
/// public.{TABLE}（HEAD 老基线形态）不读、不迁、不 WARN。
#[derive(Debug, ormer::Model)]
#[table = "collect.ormer_rb_schema_events"]
struct RbSchemaEvent {
    #[hypertable]
    #[primary]
    project_id: String,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    event_time: chrono::NaiveDateTime,
    agvid: i32,
    payload: String,
}

/// 跨 schema 不干预（拍板 C）：预置 public.{TABLE} 老结构表与数据 →
/// migrate_table 后 collect.{TABLE} 按新表新建（含 schema 自动补建）、
/// public.{TABLE} 原样未动。
#[tokio::test]
async fn cross_schema_public_table_untouched() {
    const TABLE: &str = "ormer_rb_schema_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    // cleanup 需 schema 先存在（DROP TABLE IF EXISTS 在 schema 缺失时报错）
    let _ = db
        .execute_sql("CREATE SCHEMA IF NOT EXISTS collect".to_string())
        .await;
    drop_quietly(&db, &format!("collect.{TABLE}")).await;
    drop_quietly(&db, TABLE).await;
    drop_quietly(&db, &format!("{TABLE}_migrate_old")).await;

    // public 老结构：普通表（非超表），带 model 已不含的 update_time 列
    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT, \
             agvid INTEGER, \
             update_time TIMESTAMP, \
             payload TEXT)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, agvid, update_time, payload) VALUES \
         ('p1', 1, '2026-09-01 08:00:00', 'old-1'), \
         ('p2', 2, '2026-09-02 08:00:00', 'old-2')"
    ))
    .await
    .unwrap();

    let outcome = db.migrate_table::<RbSchemaEvent>().await.unwrap();
    assert!(
        outcome.created_table,
        "collect.{TABLE} must be freshly bootstrapped"
    );

    // collect.{TABLE} 按新表建：存在且是真超表（时间维 event_time）
    let collect_table_exists = scalar_bool(
        &db,
        format!(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'collect' AND table_name = '{TABLE}')"
        ),
    )
    .await;
    assert!(collect_table_exists, "collect.{TABLE} must be created");
    let collect_dims = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.dimensions \
             WHERE hypertable_schema = 'collect' AND hypertable_name = '{TABLE}' \
             AND column_name = 'event_time'"
        ),
    )
    .await;
    assert_eq!(collect_dims, 1, "collect.{TABLE} must be a hypertable");
    let collect_count = scalar_i64(&db, format!("SELECT count(*) FROM collect.{TABLE}")).await;
    assert_eq!(
        collect_count, 0,
        "collect.{TABLE} starts empty (decision C)"
    );

    // public.{TABLE} 原样未动：行数、列集合、非超表
    let public_count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(public_count, 2, "public.{TABLE} rows must be untouched");
    let public_update_time = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = '{TABLE}' \
             AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(
        public_update_time, 1,
        "public.{TABLE} columns must be untouched"
    );
    let public_event_time = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = '{TABLE}' \
             AND column_name = 'event_time'"
        ),
    )
    .await;
    assert_eq!(public_event_time, 0, "public.{TABLE} must not be migrated");
    let public_is_hypertable = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM timescaledb_information.hypertables \
             WHERE hypertable_schema = 'public' AND hypertable_name = '{TABLE}'"
        ),
    )
    .await;
    assert_eq!(
        public_is_hypertable, 0,
        "public.{TABLE} must stay a plain table"
    );

    // 清理：collect 表随模型 drop，public 表裸删（schema 留给其他用例无碍）
    let _ = db.drop_table::<RbSchemaEvent>().execute().await;
    drop_quietly(&db, TABLE).await;
}

// ---------------------------------------------------------------------------
// 场景 7：改名列映射（拍板 D）
// ---------------------------------------------------------------------------

/// 改名映射（拍板 D）：老表时间分区列 update_time → 新列 event_time，
/// 标注 `#[column(old_name = "update_time")]` 后迁移按老列值回灌。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_rename_events"]
struct RbRenameEvent {
    #[primary]
    project_id: String,
    #[primary]
    agvid: i32,
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86_400))]
    #[column(old_name = "update_time")]
    event_time: chrono::NaiveDateTime,
    payload: String,
}

/// 改名列映射：新列值逐行等于老列值，老列删除，行数对账。
#[tokio::test]
async fn renamed_time_column_maps_old_values() {
    const TABLE: &str = "ormer_rb_rename_events";
    let db = pg_database().await;
    if !require_timescale(&db).await {
        return;
    }
    cleanup_tables(&db, &[TABLE]).await;

    db.execute_sql(format!(
        "CREATE TABLE {TABLE} (\
             project_id TEXT NOT NULL, \
             agvid INTEGER NOT NULL, \
             update_time TIMESTAMP NOT NULL, \
             payload TEXT NOT NULL)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "SELECT create_hypertable('{TABLE}', 'update_time', \
         chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE, \
         migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "ALTER TABLE {TABLE} ADD PRIMARY KEY (project_id, agvid, update_time)"
    ))
    .await
    .unwrap();
    db.execute_sql(format!(
        "INSERT INTO {TABLE} (project_id, agvid, update_time, payload) VALUES \
         ('p1', 1, '2026-10-01 08:00:00', 'a'), \
         ('p1', 2, '2026-10-02 09:30:00', 'b'), \
         ('p2', 3, '2026-10-03 12:45:00', 'c')"
    ))
    .await
    .unwrap();

    let outcome = db.migrate_table::<RbRenameEvent>().await.unwrap();
    assert_eq!(outcome.migrated_rows, Some(3));

    // 行数对账
    let count = scalar_i64(&db, format!("SELECT count(*) FROM {TABLE}")).await;
    assert_eq!(count, 3);

    // 新列值 == 老列值（逐行）
    let rows = db
        .select_sql::<(i32, chrono::NaiveDateTime)>(format!(
            "SELECT agvid, event_time FROM {TABLE} ORDER BY agvid"
        ))
        .collect::<Vec<(i32, chrono::NaiveDateTime)>>()
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, ts("2026-10-01 08:00:00")),
            (2, ts("2026-10-02 09:30:00")),
            (3, ts("2026-10-03 12:45:00")),
        ],
        "event_time must carry the old update_time values row by row"
    );

    // 老列删除、列集合 == model 声明（project_id/agvid/event_time/payload）
    let update_time_col = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'update_time'"
        ),
    )
    .await;
    assert_eq!(update_time_col, 0, "old update_time column must be gone");
    assert_time_dimension(&db, TABLE, "event_time", "public").await;
    assert_no_backup_residue(&db, TABLE).await;

    cleanup_tables(&db, &[TABLE]).await;
}

/// 对照组（拍板 D 无标注语义）：station_id 从模型删除、新列
/// start_station_id 无标注 → 老列删除、新列取 NULL/默认值（就地路径：
/// 删列 + 加可空列），其余数据保留。
#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_unrename_events"]
struct RbUnrenameV1 {
    #[primary]
    agvid: i32,
    #[primary]
    seq: i64,
    station_id: Option<i32>,
    payload: String,
}

#[derive(Debug, ormer::Model)]
#[table = "ormer_rb_unrename_events"]
struct RbUnrenameV2 {
    #[primary]
    agvid: i32,
    #[primary]
    seq: i64,
    /// 无 `old_name` 标注：与被删的 station_id 无映射关系。
    start_station_id: Option<i32>,
    payload: String,
}

#[tokio::test]
async fn unannotated_column_change_drops_old_and_nulls_new() {
    const TABLE: &str = "ormer_rb_unrename_events";
    let db = pg_database().await;
    cleanup_tables(&db, &[TABLE]).await;

    let _ = db.drop_table::<RbUnrenameV1>().execute().await;
    db.migrate_table::<RbUnrenameV1>().await.unwrap();
    db.insert(&RbUnrenameV1 {
        agvid: 1,
        seq: 1,
        station_id: Some(7),
        payload: "p1".to_string(),
    })
    .execute()
    .await
    .unwrap();
    db.insert(&RbUnrenameV1 {
        agvid: 2,
        seq: 2,
        station_id: Some(9),
        payload: "p2".to_string(),
    })
    .execute()
    .await
    .unwrap();

    db.migrate_table::<RbUnrenameV2>().await.unwrap();

    // 老列删除
    let station_col = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'station_id'"
        ),
    )
    .await;
    assert_eq!(station_col, 0, "old station_id column must be dropped");

    // 新列存在且全为 NULL（无标注不保数据）
    let start_col = scalar_i64(
        &db,
        format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = '{TABLE}' AND column_name = 'start_station_id'"
        ),
    )
    .await;
    assert_eq!(start_col, 1, "start_station_id column must be added");
    let null_count = scalar_i64(
        &db,
        format!("SELECT count(*) FROM {TABLE} WHERE start_station_id IS NULL"),
    )
    .await;
    assert_eq!(
        null_count, 2,
        "unannotated new column must take NULL for all rows"
    );

    // 其余数据保留
    let rows = db
        .select_sql::<(i64, String)>(format!("SELECT seq, payload FROM {TABLE} ORDER BY seq"))
        .collect::<Vec<(i64, String)>>()
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![(1, "p1".to_string()), (2, "p2".to_string())],
        "untouched columns must keep their data"
    );
    assert!(matches!(
        db.migrate_table::<RbUnrenameV2>().await.unwrap().diagnosis,
        TableDiagnosis::Ready
    ));

    cleanup_tables(&db, &[TABLE]).await;
}

// ---------------------------------------------------------------------------
// 场景 8：回归不破坏（仅确认，不改写）
// ---------------------------------------------------------------------------
// 既有测试（本仓 tests/all/table_migrate_test.rs）继续覆盖且本文件未触碰：
// - `plain_table_converts_to_hypertable_in_place`：普通表 → 超表原地转换
//   （migrate_data => TRUE 存量随迁）；
// - `wrong_time_dimension_empty_table_rebuilds_data_table_refuses`：
//   空表兜底自动重建 + 有数据仍拒绝（旧语义回归锚点）；
// - `unmigratable_difference_reports_error_and_keeps_table`、
//   `new_primary_key_column_is_unmigratable`、
//   `not_null_column_without_backfill_is_unmigratable`（rebuild_unmigratable_test.rs）：
//   推不动拒绝语义。保数据重建落地后它们的"有数据拒绝"断言若按方案语义
//   收敛为重建路径，由实现侧同步改写；本文件不代改。
