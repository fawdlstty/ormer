#![cfg(feature = "postgresql")]

pub mod _test_common;

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_postgresql_arrays_1"]
struct PgArrayModel {
    #[primary]
    id: i32,
    ints: Vec<i32>,
    bigints: Vec<i64>,
    nullable_bigints: Vec<Option<i64>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(i32)]
enum PgArrayRole {
    Admin = 1,
    Ops = 2,
}

impl TryFrom<i32> for PgArrayRole {
    type Error = &'static str;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Admin),
            2 => Ok(Self::Ops),
            _ => Err("unknown role"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_postgresql_arrays_2"]
struct PgEnumArrayModel {
    #[primary]
    id: i32,
    #[data_type(Vec<i32>)]
    roles: Vec<PgArrayRole>,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_postgresql_arrays_3"]
struct PgStringArrayModel {
    #[primary]
    id: i32,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_postgresql_text_to_array"]
struct PgTextToArrayMigrationModel {
    #[primary]
    id: i32,
    tags: Vec<String>,
}

#[tokio::test]
async fn test_postgresql_array_sql_and_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::postgresql_config();

    let sql = ormer::generate_create_table_sql::<PgArrayModel>(config.0)?;
    assert!(sql.contains("ints INTEGER[] NOT NULL"), "{sql}");
    assert!(sql.contains("bigints BIGINT[] NOT NULL"), "{sql}");
    assert!(sql.contains("nullable_bigints BIGINT[] NOT NULL"), "{sql}");

    let db = _test_common::create_db_connection(&config).await?;
    let _ = db.drop_table::<PgArrayModel>().execute().await;

    db.create_table::<PgArrayModel>().execute().await?;
    db.validate_table::<PgArrayModel>().await?;

    let model = PgArrayModel {
        id: 1,
        ints: vec![1, 2, 3],
        bigints: vec![10, 20, 30],
        nullable_bigints: vec![Some(100), None, Some(300)],
    };

    db.insert(&model).execute().await?;

    let items = db.select::<PgArrayModel>().collect::<Vec<_>>().await?;
    assert_eq!(items, vec![model]);

    db.drop_table::<PgArrayModel>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn test_postgresql_enum_array_data_type_roundtrip() -> Result<(), Box<dyn std::error::Error>>
{
    let config = _test_common::postgresql_config();

    let sql = ormer::generate_create_table_sql::<PgEnumArrayModel>(config.0)?;
    assert!(sql.contains("roles INTEGER[] NOT NULL"), "{sql}");

    let db = _test_common::create_db_connection(&config).await?;
    let _ = db.drop_table::<PgEnumArrayModel>().execute().await;

    db.create_table::<PgEnumArrayModel>().execute().await?;
    db.validate_table::<PgEnumArrayModel>().await?;

    let model = PgEnumArrayModel {
        id: 1,
        roles: vec![PgArrayRole::Admin, PgArrayRole::Ops],
    };

    db.insert(&model).execute().await?;

    let items = db.select::<PgEnumArrayModel>().collect::<Vec<_>>().await?;
    assert_eq!(items, vec![model]);

    db.drop_table::<PgEnumArrayModel>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn test_postgresql_string_array_uses_text_array_value()
-> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::postgresql_config();

    let sql = ormer::generate_create_table_sql::<PgStringArrayModel>(config.0)?;
    assert!(sql.contains("tags TEXT[] NOT NULL"), "{sql}");

    let model = PgStringArrayModel {
        id: 1,
        tags: vec!["alpha".to_string(), "beta".to_string()],
    };

    let values = <PgStringArrayModel as ormer::Model>::field_values(&model);
    match &values[1] {
        ormer::Value::TextArray(tags) => assert_eq!(tags, &model.tags),
        other => panic!("expected TextArray value, got {:?}", other),
    }

    let db = _test_common::create_db_connection(&config).await?;
    let _ = db.drop_table::<PgStringArrayModel>().execute().await;

    db.create_table::<PgStringArrayModel>().execute().await?;
    db.validate_table::<PgStringArrayModel>().await?;

    db.insert(&model).execute().await?;

    let items = db
        .select::<PgStringArrayModel>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(items, vec![model]);

    db.drop_table::<PgStringArrayModel>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn test_postgresql_text_to_string_array_migration() -> Result<(), Box<dyn std::error::Error>>
{
    let config = _test_common::postgresql_config();
    let db = _test_common::create_db_connection(&config).await?;

    let _ = db
        .drop_table::<PgTextToArrayMigrationModel>()
        .execute()
        .await;
    db.execute_sql(
        "CREATE TABLE test_postgresql_text_to_array (
            id INTEGER PRIMARY KEY,
            tags TEXT NOT NULL
        )",
    )
    .await?;
    db.execute_sql(
        "INSERT INTO test_postgresql_text_to_array (id, tags) VALUES
         (1, '[\"alpha,beta\",\"gamma\"]'),
         (2, 'legacy,value')",
    )
    .await?;

    let plan = db
        .migrate_table::<PgTextToArrayMigrationModel>()
        .plan()
        .await?;
    assert!(!plan.steps().is_empty());
    db.migrate_table::<PgTextToArrayMigrationModel>()
        .execute()
        .await?;
    assert!(
        db.migrate_table::<PgTextToArrayMigrationModel>()
            .plan()
            .await?
            .steps()
            .is_empty()
    );

    let items = db
        .select::<PgTextToArrayMigrationModel>()
        .order_by(|item| item.id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        items,
        vec![
            PgTextToArrayMigrationModel {
                id: 1,
                tags: vec!["alpha,beta".to_string(), "gamma".to_string()],
            },
            PgTextToArrayMigrationModel {
                id: 2,
                tags: vec!["legacy,value".to_string()],
            },
        ]
    );

    db.drop_table::<PgTextToArrayMigrationModel>()
        .execute()
        .await?;
    Ok(())
}
