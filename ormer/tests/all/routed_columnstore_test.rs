//! 路由子表列存压缩端到端测试（PostgreSQL + TimescaleDB）。
//!
//! 拆表子表的路由列在子表内值固定，行存 heap 下逐行写盘；ormer 在建子表
//! 与 ensure_table 迁移时自动追加 TimescaleDB columnstore 与自动压缩策略。
//! 服务器未装 TimescaleDB 时跳过。
//!
//! 连接地址通过 `ORMER_TEST_POSTGRES` 配置，未配置时使用默认本地地址。
//! 两个阶段共用同一组表名，固定在单个测试函数内顺序执行以避免并行互扰。

#[derive(Debug, Clone, ormer::Model)]
#[table = "routed_cs_events"]
struct RoutedCsEvent {
    #[primary]
    #[hypertable(std::time::Duration::from_secs(86400))]
    time: chrono::DateTime<chrono::Utc>,
    #[primary]
    #[hypertable(route)]
    station: String,
    val: f64,
}

#[cfg_attr(
    not(any(feature = "postgresql", feature = "questdb")),
    allow(dead_code)
)]
const CHILD: &str = "routed_cs_events_s001";

#[cfg_attr(
    not(any(feature = "postgresql", feature = "questdb")),
    allow(dead_code)
)]
async fn has_timescaledb(db: &ormer::Database) -> bool {
    db.select_sql::<bool>("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')")
        .collect::<Vec<bool>>()
        .await
        .map(|rows| rows.into_iter().next().unwrap_or(false))
        .unwrap_or(false)
}

/// 子表存在且 compression_enabled（columnstore 已启用）。
#[cfg_attr(
    not(any(feature = "postgresql", feature = "questdb")),
    allow(dead_code)
)]
async fn child_compression_enabled(db: &ormer::Database) -> bool {
    db.select_sql::<bool>(&format!(
        "SELECT COALESCE((SELECT compression_enabled \
         FROM timescaledb_information.hypertables \
         WHERE hypertable_schema = 'public' AND hypertable_name = '{CHILD}'), false)"
    ))
    .collect::<Vec<bool>>()
    .await
    .map(|rows| rows.into_iter().next().unwrap_or(false))
    .unwrap_or(false)
}

/// 子表已挂自动压缩策略（policy_compression job）。
#[cfg_attr(
    not(any(feature = "postgresql", feature = "questdb")),
    allow(dead_code)
)]
async fn child_has_compression_policy(db: &ormer::Database) -> bool {
    db.select_sql::<bool>(&format!(
        "SELECT EXISTS (SELECT 1 FROM timescaledb_information.jobs \
         WHERE hypertable_name = '{CHILD}' AND proc_name = 'policy_compression')"
    ))
    .collect::<Vec<bool>>()
    .await
    .map(|rows| rows.into_iter().next().unwrap_or(false))
    .unwrap_or(false)
}

#[cfg_attr(
    not(any(feature = "postgresql", feature = "questdb")),
    allow(dead_code)
)]
async fn drop_child_and_base(db: &ormer::Database) -> ormer::Result<()> {
    db.execute_sql(format!("DROP TABLE IF EXISTS {CHILD} CASCADE"))
        .await?;
    db.execute_sql("DROP TABLE IF EXISTS routed_cs_events CASCADE")
        .await?;
    Ok(())
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn routed_child_columnstore_end_to_end() -> ormer::Result<()> {
    let connection_string = option_env!("ORMER_TEST_POSTGRES")
        .filter(|value| !value.is_empty())
        .unwrap_or("postgres://postgres:postgres@localhost:5432/ormer_test");
    let db = ormer::Database::connect(ormer::DbType::PostgreSQL, connection_string).await?;
    if !has_timescaledb(&db).await {
        eprintln!("TimescaleDB not installed; skipping routed columnstore test");
        return Ok(());
    }

    // 阶段一：带路由写入自动建子表，DDL 序列自带列存压缩与自动策略
    drop_child_and_base(&db).await?;
    db.insert(&RoutedCsEvent {
        time: chrono::Utc::now(),
        station: "s001".to_string(),
        val: 1.0,
    })
    .execute()
    .await?;
    assert!(
        child_compression_enabled(&db).await,
        "auto-created routed child table must have columnstore enabled"
    );
    assert!(
        child_has_compression_policy(&db).await,
        "auto-created routed child table must have a compression policy"
    );
    // 幂等：子表已存在时再次写入（触达自适应路径）不报错且状态不变
    db.insert(&RoutedCsEvent {
        time: chrono::Utc::now(),
        station: "s001".to_string(),
        val: 2.0,
    })
    .execute()
    .await?;
    assert!(child_compression_enabled(&db).await);

    // 阶段二：模拟旧版本 ormer 创建的子表（无压缩属性），ensure_table 自适应补挂
    drop_child_and_base(&db).await?;
    db.execute_sql(format!(
        "CREATE TABLE {CHILD} (time TIMESTAMPTZ NOT NULL, station TEXT NOT NULL, \
         val DOUBLE PRECISION, PRIMARY KEY (time, station))"
    ))
    .await?;
    db.execute_sql(format!(
        "SELECT create_hypertable('{CHILD}', 'time', chunk_time_interval => INTERVAL '1 day', \
         if_not_exists => TRUE, migrate_data => TRUE, create_default_indexes => FALSE)"
    ))
    .await?;
    assert!(!child_compression_enabled(&db).await);

    db.ensure_table::<RoutedCsEvent>().await?;
    assert!(
        child_compression_enabled(&db).await,
        "ensure_table must retrofit columnstore on legacy routed child tables"
    );
    assert!(child_has_compression_policy(&db).await);

    drop_child_and_base(&db).await?;
    Ok(())
}
