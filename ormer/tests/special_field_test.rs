#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, ormer::Model, Clone)]
#[table = "test_special_field_1"]
struct SpecialFieldModel {
    #[primary]
    id: i32,
    #[data_type(i64)]
    big_count: i32,
    name: String,
}

async fn test_data_type_override_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = ormer::generate_create_table_sql::<SpecialFieldModel>(config.0)?;
    println!("SQL: {}", sql);

    // data_type(i64) 应使 big_count 使用 i64 对应的 SQL 类型
    match config.0 {
        #[cfg(feature = "sqlite")]
        ormer::DbType::Sqlite => {
            assert!(sql.contains("big_count INTEGER NOT NULL"));
        }
        #[cfg(feature = "postgresql")]
        ormer::DbType::PostgreSQL => {
            assert!(sql.contains("big_count BIGINT NOT NULL"));
        }
        #[cfg(feature = "mysql")]
        ormer::DbType::MySQL => {
            assert!(sql.contains("big_count BIGINT NOT NULL"));
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }

    Ok(())
}

async fn test_data_type_crud_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<SpecialFieldModel>().execute().await?;
    #[cfg(feature = "postgresql")]
    if matches!(config.0, ormer::DbType::PostgreSQL) {
        db.validate_table::<SpecialFieldModel>().await?;
    }

    db.insert(&SpecialFieldModel {
        id: 1,
        big_count: 999,
        name: "test".to_string(),
    })
    .execute()
    .await?;

    let items: Vec<SpecialFieldModel> =
        db.select::<SpecialFieldModel>().collect::<Vec<_>>().await?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].big_count, 999);
    assert_eq!(items[0].name, "test");

    db.drop_table::<SpecialFieldModel>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_data_type_override_impl);
test_on_all_dbs_result!(test_data_type_crud_impl);
