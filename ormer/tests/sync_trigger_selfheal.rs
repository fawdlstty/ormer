#![cfg(feature = "postgresql")]

//! sync 游标复位事件触发器链路自愈的端到端实测：在真实 PG 里复刻
//! data_synchronization 的库级事件触发器 + 缺序列/默认值的
//! data_sync_table_reset_events（2026-10 dghq 事故形态），验证
//! migrate_table 首轮被触发器炸掉后自适应修复并重跑成功。
//!
//! 需要超级用户连接串（CREATE EVENT TRIGGER 是库级特权），未设置环境变量
//! `ORMER_PG_TEST_URL` 时整个测试静默跳过：
//!
//! ```bash
//! ORMER_PG_TEST_URL=postgres://postgres:xxx@127.0.0.1:5432/postgres \
//!     cargo test -p ormer --features postgresql --test sync_trigger_selfheal
//! ```

use ormer::raw_sql::RawSql;

const EVENT_TRIGGER: &str = "data_sync_clear_cursors_on_create_table";
const RESET_EVENTS: &str = "data_sync_table_reset_events";
const RESET_EVENTS_SEQ: &str = "data_sync_table_reset_events_reset_id_seq";
const PROBE_TABLE: &str = "ormer_selfheal_probe";
const PROBE_TABLE_2: &str = "ormer_selfheal_probe2";

#[derive(Debug, ormer::Model)]
#[table = "ormer_selfheal_probe"]
struct SelfHealProbe {
    #[primary(auto)]
    id: i32,
    name: String,
}

async fn exec(db: &ormer::Database, sql: &str) {
    db.execute_sql(RawSql::plain(sql))
        .await
        .unwrap_or_else(|err| panic!("exec failed: {err} (sql: {sql})"));
}

/// 单值查询（无行返回 None）
async fn scalar(db: &ormer::Database, sql: &str) -> Option<String> {
    db.select_sql::<Option<String>>(RawSql::plain(sql))
        .collect::<Vec<Option<String>>>()
        .await
        .unwrap_or_else(|err| panic!("select failed: {err} (sql: {sql})"))
        .into_iter()
        .flatten()
        .next()
}

#[tokio::test]
async fn sync_cursor_reset_trigger_self_heal_end_to_end() {
    let Ok(url) = std::env::var("ORMER_PG_TEST_URL") else {
        eprintln!("ORMER_PG_TEST_URL not set; skipping live-PG self-heal test");
        return;
    };
    let db = ormer::Database::connect(ormer::DbType::PostgreSQL, &url)
        .await
        .expect("connect test PG");
    cleanup(&db).await;

    // 复刻事故形态：reset_events 表存在但 reset_id 无序列、无默认值，
    // 库级事件触发器在 CREATE TABLE DDL 末尾 INSERT 本表（不含 reset_id）
    exec(
        &db,
        "CREATE TABLE data_sync_table_reset_events (\
             reset_id bigint NOT NULL, \
             table_name text NOT NULL, \
             created_at timestamptz NOT NULL DEFAULT now())",
    )
    .await;
    exec(
        &db,
        "CREATE OR REPLACE FUNCTION data_sync_clear_cursors_on_create_table() \
         RETURNS event_trigger AS $$ \
         BEGIN \
             INSERT INTO data_sync_table_reset_events (table_name) \
             VALUES ('ormer-selfheal-test'); \
         END $$ LANGUAGE plpgsql",
    )
    .await;
    exec(
        &db,
        "CREATE EVENT TRIGGER data_sync_clear_cursors_on_create_table \
         ON ddl_command_end \
         WHEN TAG IN ('CREATE TABLE') \
         EXECUTE FUNCTION data_sync_clear_cursors_on_create_table()",
    )
    .await;

    // 预检：事故形态下裸建表确实被触发器连带炸掉（23502）
    let bare_create = db
        .execute_sql(RawSql::plain(format!(
            "CREATE TABLE {PROBE_TABLE_2} (id bigint primary key)"
        )))
        .await;
    assert!(
        bare_create.is_err(),
        "broken-state bare CREATE TABLE should fail via the event trigger"
    );
    exec(&db, &format!("DROP TABLE IF EXISTS {PROBE_TABLE_2}")).await;

    // migrate_table：首轮被触发器炸掉 → 自适应修复 → 重跑成功
    let outcome = db
        .migrate_table::<SelfHealProbe>()
        .await
        .expect("migrate_table should self-heal and succeed");
    assert!(outcome.created_table, "probe table should be created");

    // 修复落位验证：序列存在 + reset_id 默认值指回序列
    assert_eq!(
        scalar(&db, &format!("SELECT to_regclass('{RESET_EVENTS_SEQ}')::text")).await,
        Some(RESET_EVENTS_SEQ.to_string()),
        "reset_id sequence should exist after self-heal"
    );
    let column_default = scalar(
        &db,
        &format!(
            "SELECT column_default FROM information_schema.columns \
             WHERE table_name = '{RESET_EVENTS}' AND column_name = 'reset_id'"
        ),
    )
    .await
    .expect("reset_id column row");
    assert!(
        column_default.contains(RESET_EVENTS_SEQ),
        "reset_id default should point at the sequence, got: {column_default}"
    );

    // 触发器链路复通验证：再建一张表，reset_events 正常收到带 reset_id 的行
    exec(
        &db,
        &format!("CREATE TABLE {PROBE_TABLE_2} (id bigint primary key)"),
    )
    .await;
    let reset_rows = scalar(&db, &format!("SELECT count(*)::text FROM {RESET_EVENTS}"))
        .await
        .expect("reset_events count");
    assert!(
        reset_rows.parse::<i64>().unwrap_or(0) >= 1,
        "event trigger should have inserted reset rows after the repair"
    );

    cleanup(&db).await;
}

/// 清理本轮与前序遗留（幂等，供重复运行）
async fn cleanup(db: &ormer::Database) {
    for sql in [
        format!("DROP EVENT TRIGGER IF EXISTS {EVENT_TRIGGER}"),
        format!("DROP FUNCTION IF EXISTS {EVENT_TRIGGER}()"),
        format!("DROP TABLE IF EXISTS {RESET_EVENTS}"),
        format!("DROP SEQUENCE IF EXISTS {RESET_EVENTS_SEQ}"),
        format!("DROP TABLE IF EXISTS {PROBE_TABLE}"),
        format!("DROP TABLE IF EXISTS {PROBE_TABLE_2}"),
    ] {
        let _ = db.execute_sql(RawSql::plain(&sql)).await;
    }
}
