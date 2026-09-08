#![cfg(feature = "sqlite")]

//! 按块删除（Block Delete）测试：默认 sqlite 验证 OLTP 回退路径的 SQL 形态、
//! 对齐边界与端到端行为；PostgreSQL/TimescaleDB、QuestDB 的原生 SQL 形态
//! 在 `postgresql_backend.rs` 的 `block_delete_tests` 中覆盖。

use ormer::DbType;

#[derive(Debug, ormer::Model, Clone)]
#[table = "block_delete_cpu"]
struct CpuUsage {
    #[primary]
    id: i64,
    #[hypertable(std::time::Duration::from_secs(86400))] // 按天分块
    time: chrono::DateTime<chrono::Utc>,
    host: String,
}

#[derive(Debug, ormer::Model)]
#[table = "block_delete_plain"]
struct PlainRow {
    #[primary]
    id: i64,
    at: chrono::DateTime<chrono::Utc>,
}

fn day(text: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap();
    chrono::Utc.from_utc_datetime(&naive)
}

fn datetime_of(value: &ormer::model::Value) -> chrono::DateTime<chrono::Utc> {
    match value {
        ormer::model::Value::DateTime(t) => *t,
        other => panic!("expected DateTime, got {other:?}"),
    }
}

async fn connect() -> ormer::Database {
    ormer::Database::connect(DbType::Sqlite, ":memory:").await.unwrap()
}

#[tokio::test]
async fn fallback_before_generates_aligned_row_delete() {
    let db = connect().await;
    let sql = db
        .delete_blocks::<CpuUsage>()
        .before(day("2024-01-17 13:25:00"))
        .to_sql()
        .unwrap();

    assert_eq!(sql.db_type, DbType::Sqlite);
    let statement = &sql.statements[0];
    // cutoff 向下对齐到日界；SQL 带 block_delete_fallback 标注
    assert_eq!(
        statement.sql,
        "/* block_delete_fallback */ DELETE FROM block_delete_cpu WHERE time < ?"
    );
    assert_eq!(statement.params.len(), 1);
    assert_eq!(
        datetime_of(&statement.params[0]),
        day("2024-01-17 00:00:00")
    );
}

#[tokio::test]
async fn fallback_between_generates_aligned_window() {
    let db = connect().await;
    let sql = db
        .delete_blocks::<CpuUsage>()
        .between(day("2024-01-17 13:25:00"), day("2024-01-20 16:13:00"))
        .to_sql()
        .unwrap();

    let statement = &sql.statements[0];
    // 起点向上对齐、终点向下对齐，只删完整落在范围内的块
    assert_eq!(
        statement.sql,
        "/* block_delete_fallback */ DELETE FROM block_delete_cpu WHERE time >= ? AND time < ?"
    );
    assert_eq!(statement.params.len(), 2);
    assert_eq!(
        datetime_of(&statement.params[0]),
        day("2024-01-18 00:00:00")
    );
    assert_eq!(
        datetime_of(&statement.params[1]),
        day("2024-01-20 00:00:00")
    );
}

#[tokio::test]
async fn retain_matches_before_now_minus_duration() {
    let db = connect().await;
    let sql = db
        .delete_blocks::<CpuUsage>()
        .retain(std::time::Duration::from_secs(30 * 86_400))
        .to_sql()
        .unwrap();

    let statement = &sql.statements[0];
    assert_eq!(
        statement.sql,
        "/* block_delete_fallback */ DELETE FROM block_delete_cpu WHERE time < ?"
    );
    let Some(ormer::model::Value::DateTime(cutoff)) = statement.params.first().cloned() else {
        panic!("expected a datetime cutoff");
    };
    // cutoff = before(now - 30d)，且已对齐到日界（时、分、秒为 0）
    assert!(
        cutoff < chrono::Utc::now() - chrono::Duration::days(29),
        "cutoff too recent: {cutoff}"
    );
    assert_eq!(
        cutoff.format("%H:%M:%S").to_string(),
        "00:00:00",
        "cutoff must be aligned to the day boundary: {cutoff}"
    );
}

#[tokio::test]
async fn empty_aligned_range_is_safe_noop() {
    let db = connect().await;
    // 同一天内的区间对齐后为空 → 空语句，执行为 no-op
    let executor = db
        .delete_blocks::<CpuUsage>()
        .between(day("2024-01-17 10:00:00"), day("2024-01-17 18:00:00"));
    let sql = executor.to_sql().unwrap();
    assert!(sql.statements.is_empty());

    let result = db
        .delete_blocks::<CpuUsage>()
        .between(day("2024-01-17 10:00:00"), day("2024-01-17 18:00:00"))
        .execute()
        .await
        .unwrap();
    assert_eq!(
        result,
        ormer::BlockDeleteResult {
            blocks_dropped: 0,
            rows_deleted: Some(0),
        }
    );
}

#[tokio::test]
async fn between_reversed_range_reports_parameter_error() {
    let db = connect().await;
    let error = db
        .delete_blocks::<CpuUsage>()
        .between(day("2024-01-17 00:00:00"), day("2024-01-16 00:00:00"))
        .execute()
        .await
        .unwrap_err();
    assert!(matches!(error, ormer::OrmerError::InvalidOperation { .. }));
}

#[tokio::test]
async fn model_without_ts_declaration_is_rejected() {
    let db = connect().await;
    let error = db
        .delete_blocks::<PlainRow>()
        .before(day("2024-01-17 00:00:00"))
        .execute()
        .await
        .unwrap_err();
    match error {
        ormer::OrmerError::UnsupportedFeature { feature, .. } => {
            assert!(feature.contains("block delete"), "unexpected: {feature}");
            assert!(feature.contains("hypertable"), "unexpected: {feature}");
        }
        other => panic!("expected UnsupportedFeature, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_range_reports_operation_error() {
    let db = connect().await;
    let error = db
        .delete_blocks::<CpuUsage>()
        .execute()
        .await
        .unwrap_err();
    assert!(matches!(error, ormer::OrmerError::InvalidOperation { .. }));
}

#[tokio::test]
async fn fallback_execute_deletes_only_rows_before_boundary() {
    let db = connect().await;
    db.drop_table::<CpuUsage>().execute().await.ok();
    db.create_table::<CpuUsage>().execute().await.unwrap();
    db.insert(vec![
        CpuUsage {
            id: 1,
            time: day("2024-01-15 08:00:00"),
            host: "a".into(),
        },
        CpuUsage {
            id: 2,
            time: day("2024-01-16 08:00:00"),
            host: "a".into(),
        },
        CpuUsage {
            id: 3,
            time: day("2024-01-17 08:00:00"),
            host: "a".into(),
        },
        CpuUsage {
            id: 4,
            time: day("2024-01-17 23:59:00"),
            host: "b".into(),
        },
    ])
    .execute()
    .await
    .unwrap();

    // before(01-17 13:25) → 对齐到 01-17 00:00，删除 01-15、01-16 两天的完整块，
    // 边界所在块（01-17 当天）不受影响
    let result = db
        .delete_blocks::<CpuUsage>()
        .before(day("2024-01-17 13:25:00"))
        .execute()
        .await
        .unwrap();
    assert_eq!(
        result,
        ormer::BlockDeleteResult {
            blocks_dropped: 0,
            rows_deleted: Some(2),
        }
    );

    let remaining = db.select::<CpuUsage>().collect::<Vec<CpuUsage>>().await.unwrap();
    let mut ids: Vec<i64> = remaining.iter().map(|row| row.id).collect();
    ids.sort();
    assert_eq!(ids, vec![3, 4]);
}

#[tokio::test]
async fn pooled_connection_supports_block_delete() {
    let pool = ormer::Database::create_pool(DbType::Sqlite, ":memory:")
        .range(0..1)
        .build()
        .await
        .unwrap();
    let conn = pool.get().await.unwrap();
    let sql = conn
        .delete_blocks::<CpuUsage>()
        .before(day("2024-01-17 13:25:00"))
        .to_sql()
        .unwrap();
    assert_eq!(
        sql.statements[0].sql,
        "/* block_delete_fallback */ DELETE FROM block_delete_cpu WHERE time < ?"
    );
}
