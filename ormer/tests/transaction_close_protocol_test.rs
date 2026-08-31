#![cfg(feature = "sqlite")]

pub mod _test_common;

#[derive(Debug, ormer::Model, Clone)]
#[table = "transaction_close_protocol_close"]
struct CloseUser {
    #[primary(auto)]
    id: Option<i64>,
    name: String,
}

#[derive(Debug, ormer::Model, Clone)]
#[table = "transaction_close_protocol_drop"]
struct DropUser {
    #[primary(auto)]
    id: Option<i64>,
    name: String,
}

#[tokio::test]
async fn explicit_close_rolls_back() -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(&_test_common::sqlite_config()).await?;
    _test_common::prepare_table::<CloseUser>(&db).await?;

    let mut txn = db.begin().await?;
    txn.insert(&CloseUser {
        id: None,
        name: "not persisted".to_string(),
    })
    .execute()
    .await?;
    txn.close().await?;

    let users: Vec<CloseUser> = db.select::<CloseUser>().collect::<Vec<CloseUser>>().await?;
    assert!(users.is_empty());

    _test_common::clean_table::<CloseUser>(&db).await?;
    Ok(())
}

#[tokio::test]
async fn drop_rolls_back_active_transaction() -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(&_test_common::sqlite_config()).await?;
    _test_common::prepare_table::<DropUser>(&db).await?;

    let mut txn = db.begin().await?;
    txn.insert(&DropUser {
        id: None,
        name: "not persisted".to_string(),
    })
    .execute()
    .await?;
    drop(txn);

    let users: Vec<DropUser> = db.select::<DropUser>().collect::<Vec<DropUser>>().await?;
    assert!(users.is_empty());

    _test_common::clean_table::<DropUser>(&db).await?;
    Ok(())
}
