#![cfg(feature = "sqlite")]

//! sqlite 后端并发串行化回归（2026-10-17 WorkRecorder 服务器偶发 500 调查
//! 沉淀）：turso local 模式单连接，`ConcurrentGuard`（AtomicU32 CAS）对并发
//! step 一律报 `Misuse("concurrent use forbidden")`——而调用方形态（HTTP
//! 服务器每连接一个任务 + 后台 ticker）天然并发共享同一 `Database`。
//! ormer sqlite 后端须把并发访问**排队化**而非报错：语句在库级互斥锁内
//! 执行（prepare + 全量拉干），事务独占连接至 commit/rollback。
//!
//! 本文件三个用例在修复前均以 `concurrent use forbidden` 失败：
//! 1. 多任务并发全表扫描（多行多次 step，CAS 窗口密集）；
//! 2. 并发读写交错（查询与插入并发竞争）；
//! 3. 事务独占（事务提交前并发查询排队等待，提交后可见新行）。

define_test_user_simple!(ConcUser, "wr_conc_serialize_1");

use ormer::Database;
use std::sync::Arc;

const SEED_ROWS: i32 = 600;
const TASKS: usize = 8;
const ROUNDS: usize = 3;

async fn prepare_seeded_db() -> Arc<Database> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await.unwrap();
    let _ = db.drop_table::<ConcUser>().execute().await;
    db.create_table::<ConcUser>().execute().await.unwrap();
    let rows: Vec<ConcUser> = (0..SEED_ROWS)
        .map(|i| ConcUser {
            id: i + 1,
            name: format!("u{i}"),
            age: i % 97,
        })
        .collect();
    db.insert(&rows).execute().await.unwrap();
    Arc::new(db)
}

/// 多任务并发全表扫描 + 单行查询 + 并发插入：全部成功、结果一致。
/// （turso step 窗口微秒级但多行扫描 step 密集，8 任务真并发下修复前
/// 必现 CAS 撞车。）
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_sqlite_concurrent_queries_and_writes_serialize() {
    let db = prepare_seeded_db().await;
    let mut tasks = Vec::new();
    for t in 0..TASKS {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            for round in 0..ROUNDS {
                // 全表扫描：修复前在多任务交错 step 下撞 concurrent guard。
                // 断言用下界（自己前 round 轮已插入的 round 行必然可见）；
                // 其余任务的并发插入何时可见取决于调度，不假设精确值——
                // 总量一致性由末尾断言兜底。
                let all: Vec<ConcUser> = db.select::<ConcUser>().collect().await?;
                assert!(
                    all.len() >= SEED_ROWS as usize + round,
                    "并发插入逐轮可见（含自己已插入的行）"
                );
                // 单行查询与并发插入交替，制造读写竞争窗口。
                let one: Option<ConcUser> = db
                    .select::<ConcUser>()
                    .filter(|u| u.id.eq(round + 1))
                    .first()
                    .await?;
                assert!(one.is_some());
                db.insert(&ConcUser {
                    id: SEED_ROWS + (t * ROUNDS + round) as i32 + 1,
                    name: format!("w{t}-{round}"),
                    age: t as i32,
                })
                .execute()
                .await?;
            }
            Ok::<(), ormer::OrmerError>(())
        }));
    }
    for task in tasks {
        task.await.unwrap().expect("并发任务必须全部成功（排队化后无 concurrent use forbidden）");
    }
    let total: Vec<ConcUser> = db.select::<ConcUser>().collect().await.unwrap();
    assert_eq!(total.len(), SEED_ROWS as usize + TASKS * ROUNDS);
}

/// 事务独占：事务提交前并发查询排队等待（不报错、不看到未提交行），
/// 提交后并发查询可见新行。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_sqlite_transaction_excludes_concurrent_reads_until_commit() {
    let db = prepare_seeded_db().await;

    let mut txn = db.begin().await.unwrap();
    txn.insert(&ConcUser {
        id: SEED_ROWS + 1,
        name: "in-txn".into(),
        age: 1,
    })
    .execute()
    .await
    .unwrap();

    // 事务持有连接期间发起并发查询：修复前 concurrent use forbidden /
    // 事务语义被破坏；修复后查询排队，commit 后才真正执行。
    let reader_db = db.clone();
    let reader = tokio::spawn(async move {
        let rows: Vec<ConcUser> = reader_db.select::<ConcUser>().collect().await.unwrap();
        rows.len()
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!reader.is_finished(), "并发查询应阻塞在连接锁上等待事务提交");
    txn.commit().await.unwrap();

    let count = reader.await.unwrap();
    assert_eq!(count, SEED_ROWS as usize + 1, "提交后排队查询可见新行");
}
