//! 按块删除真实库集成测试。
//!
//! PostgreSQL：服务器装有 TimescaleDB 时验证 `drop_chunks` 整块语义，
//! 否则验证回退路径（对齐边界行删除，`rows_deleted` 可统计）。
//! QuestDB：验证 `ALTER TABLE ... DROP PARTITION` 与边界块保护。
//!
//! 连接地址通过 `ORMER_TEST_POSTGRES` / `ORMER_TEST_QUESTDB` 配置，
//! 未配置时使用默认本地地址；QuestDB 未部署时对应测试跳过。

#[derive(Debug, Clone, ormer::Model)]
#[table = "hypertable_block_delete_events"]
struct TsEvent {
    #[primary]
    id: i64,
    #[hypertable(std::time::Duration::from_secs(86400))]
    time: chrono::DateTime<chrono::Utc>,
}

fn day(text: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap();
    chrono::Utc.from_utc_datetime(&naive)
}

fn seed_events() -> Vec<TsEvent> {
    vec![
        TsEvent {
            id: 1,
            time: day("2024-01-15 08:00:00"),
        },
        TsEvent {
            id: 2,
            time: day("2024-01-16 08:00:00"),
        },
        TsEvent {
            id: 3,
            time: day("2024-01-17 08:00:00"),
        },
        TsEvent {
            id: 4,
            time: day("2024-01-17 23:59:00"),
        },
    ]
}

async fn remaining_ids(db: &ormer::Database) -> ormer::Result<Vec<i64>> {
    let mut ids: Vec<i64> = db
        .select::<TsEvent>()
        .collect::<Vec<TsEvent>>()
        .await?
        .into_iter()
        .map(|event| event.id)
        .collect();
    ids.sort();
    Ok(ids)
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn postgresql_block_delete_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let connection_string = option_env!("ORMER_TEST_POSTGRES")
        .filter(|value| !value.is_empty())
        .unwrap_or("postgres://postgres:postgres@localhost:5432/ormer_test");
    let db = ormer::Database::connect(ormer::DbType::PostgreSQL, connection_string).await?;

    let timescale = db
        .select_sql::<bool>("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')")
        .collect::<Vec<bool>>()
        .await?
        .into_iter()
        .next()
        .unwrap_or(false);

    if timescale {
        db.drop_table::<TsEvent>().execute().await.ok();
        db.create_table::<TsEvent>().execute().await?;
    } else {
        // 无 TimescaleDB：手动建普通表，验证对齐边界回退路径
        db.execute_sql("DROP TABLE IF EXISTS hypertable_block_delete_events")
            .await?;
        db.execute_sql(
            "CREATE TABLE hypertable_block_delete_events (id BIGINT PRIMARY KEY, time TIMESTAMPTZ)",
        )
        .await?;
    }

    db.insert(seed_events()).execute().await?;

    let result = db
        .delete_blocks::<TsEvent>()
        .before(day("2024-01-17 13:25:00"))
        .execute()
        .await?;

    // 对齐到 2024-01-17 00:00：01-15、01-16 两天的完整块被删，边界块保留
    if timescale {
        assert!(result.rows_deleted.is_none());
    } else {
        assert_eq!(result.rows_deleted, Some(2));
    }
    assert_eq!(remaining_ids(&db).await?, vec![3, 4]);

    Ok(())
}

#[cfg(feature = "questdb")]
#[tokio::test]
async fn questdb_block_delete_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let Some(connection_string) = option_env!("ORMER_TEST_QUESTDB") else {
        eprintln!("ORMER_TEST_QUESTDB not set; skipping QuestDB block delete test");
        return Ok(());
    };
    let db = ormer::Database::connect(ormer::DbType::QuestDB, connection_string).await?;

    db.drop_table::<TsEvent>().execute().await.ok();
    db.create_table::<TsEvent>().execute().await?;
    db.insert(seed_events()).execute().await?;

    let result = db
        .delete_blocks::<TsEvent>()
        .before(day("2024-01-17 13:25:00"))
        .execute()
        .await?;
    assert_eq!(result.blocks_dropped, 0);
    assert_eq!(result.rows_deleted, None);

    // 分区谓词对齐到日界：01-17 当天的边界块保留
    assert_eq!(remaining_ids(&db).await?, vec![3, 4]);

    Ok(())
}
