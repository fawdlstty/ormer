#![cfg(feature = "postgresql")]

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_postgresql_null_binary_params"]
struct NullableBinary {
    #[primary]
    id: i32,
    payload: Option<Vec<u8>>,
}

#[tokio::test]
async fn test_insert_or_ignore_null_binary_param() -> Result<(), Box<dyn std::error::Error>> {
    let config = crate::_test_common::postgresql_config();
    let db = crate::_test_common::create_db_connection(&config).await?;
    let _ = db.drop_table::<NullableBinary>().execute().await;

    let sql = ormer::generate_create_table_sql::<NullableBinary>(config.0)?;
    assert!(sql.contains("payload BYTEA"));

    db.create_table::<NullableBinary>().execute().await?;
    assert!(
        matches!(db.plan_table::<NullableBinary>().await?, ormer::TableDiagnosis::Ready),
        "table should match model after create"
    );

    db.insert_or_ignore(&NullableBinary {
        id: 1,
        payload: None,
    })
    .execute()
    .await?;

    let items = db.select::<NullableBinary>().collect::<Vec<_>>().await?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].payload, None);

    db.drop_table::<NullableBinary>().execute().await?;
    Ok(())
}
