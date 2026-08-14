#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[version(u64)]
#[table = "optimistic_lock_orders"]
struct OptimisticLockOrder {
    #[primary]
    id: i32,
    status: String,
}

async fn test_optimistic_lock_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    _test_common::prepare_table::<OptimisticLockOrder>(&db).await?;

    let inserted = OptimisticLockOrder {
        id: 1,
        status: "draft".to_string(),
    };
    assert_eq!(inserted.version(), 1);
    db.insert(&inserted).execute().await?;

    let mut order = db
        .select::<OptimisticLockOrder>()
        .filter(|o| o.id.eq(1))
        .first()
        .await?
        .expect("inserted order should exist");
    assert_eq!(order.version(), 1);

    order.status = "approved".to_string();
    let affected = db
        .update::<OptimisticLockOrder>()
        .set_model(&order)
        .execute()
        .await?;
    assert_eq!(affected, 1);
    assert_eq!(order.version(), 2);

    let stale = OptimisticLockOrder {
        id: 1,
        status: "stale".to_string(),
    };
    assert_eq!(stale.version(), 1);

    let err = db
        .update::<OptimisticLockOrder>()
        .set_model(&stale)
        .execute()
        .await
        .expect_err("stale update should fail");
    assert!(matches!(err, ormer::OrmerError::OptimisticLock { .. }));

    let err = db
        .delete::<OptimisticLockOrder>()
        .model(&stale)
        .execute()
        .await
        .expect_err("stale delete should fail");
    assert!(matches!(err, ormer::OrmerError::OptimisticLock { .. }));

    let fresh = db
        .select::<OptimisticLockOrder>()
        .filter(|o| o.id.eq(1))
        .first()
        .await?
        .expect("order should still exist");
    assert_eq!(fresh.version(), 2);

    let deleted = db
        .delete::<OptimisticLockOrder>()
        .model(&fresh)
        .execute()
        .await?;
    assert_eq!(deleted, 1);

    _test_common::clean_table::<OptimisticLockOrder>(&db).await?;
    Ok(())
}

test_on_all_dbs_result!(test_optimistic_lock_impl);
