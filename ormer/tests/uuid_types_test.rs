#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

use ormer::query::builder::TypedColumn;
use ormer::query::filter::FilterExpr;
use ormer::{FromRowValues, FromValue, SqlExpr, Value, generate_create_table_sql};

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "uuid_types_test"]
struct UuidRecord {
    #[primary]
    id: uuid::Uuid,
    uid: uuid::Uuid,
    optional_uid: Option<uuid::Uuid>,
}

#[test]
fn uuid_value_and_row_conversions() {
    let id = uuid::Uuid::from_u128(1);
    let value = Value::from(id);
    assert!(matches!(value, Value::Uuid(value_id) if value_id == id));
    assert_eq!(uuid::Uuid::from_value(&value).unwrap(), id);
    assert_eq!(
        uuid::Uuid::from_row_values(&[Value::Text(id.to_string())]).unwrap(),
        id
    );
    assert_eq!(
        Option::<uuid::Uuid>::from_value(&Value::Null).unwrap(),
        None
    );
    assert_eq!(Option::<uuid::Uuid>::from_value(&value).unwrap(), Some(id));
}

#[test]
fn uuid_query_values_are_preserved() {
    let id = uuid::Uuid::from_u128(1);
    let other = uuid::Uuid::from_u128(2);

    let filter: FilterExpr = TypedColumn::<uuid::Uuid>::new("id").eq(id).into();
    match filter {
        FilterExpr::Comparison { value, .. } => {
            assert!(matches!(value, Value::Uuid(value_id) if value_id == id));
        }
        _ => panic!("expected UUID comparison"),
    }

    let filter: FilterExpr = TypedColumn::<uuid::Uuid>::new("id")
        .is_in([id, other])
        .into();
    match filter {
        FilterExpr::In { values, .. } => {
            assert!(matches!(
                values.as_slice(),
                [Value::Uuid(first), Value::Uuid(second)]
                    if *first == id && *second == other
            ));
        }
        _ => panic!("expected UUID IN expression"),
    }

    assert!(matches!(
        ormer::value(id).sql_expr(),
        SqlExpr::Value(Value::Uuid(value_id)) if value_id == id
    ));

    let db_type = {
        #[cfg(feature = "sqlite")]
        {
            ormer::DbType::Sqlite
        }
        #[cfg(all(not(feature = "sqlite"), feature = "postgresql"))]
        {
            ormer::DbType::PostgreSQL
        }
        #[cfg(all(
            not(feature = "sqlite"),
            not(feature = "postgresql"),
            feature = "mysql"
        ))]
        {
            ormer::DbType::MySQL
        }
        #[cfg(all(
            not(feature = "sqlite"),
            not(feature = "postgresql"),
            not(feature = "mysql"),
            feature = "mssql"
        ))]
        {
            ormer::DbType::MSSQL
        }
    };
    let (_, params) = ormer::sql("SELECT {id}")
        .bind_named("id", id)
        .render(db_type)
        .unwrap();
    assert!(matches!(
        params.as_slice(),
        [Value::Uuid(value_id)] if *value_id == id
    ));
}

#[test]
fn uuid_create_table_sql_uses_backend_types() {
    #[cfg(feature = "sqlite")]
    {
        let sql = generate_create_table_sql::<UuidRecord>(ormer::DbType::Sqlite).unwrap();
        assert!(sql.contains("id TEXT PRIMARY KEY"), "{sql}");
        assert!(sql.contains("uid TEXT NOT NULL"), "{sql}");
        assert!(sql.contains("optional_uid TEXT"), "{sql}");
    }

    #[cfg(feature = "postgresql")]
    {
        let sql = generate_create_table_sql::<UuidRecord>(ormer::DbType::PostgreSQL).unwrap();
        assert!(sql.contains("id UUID PRIMARY KEY"), "{sql}");
        assert!(sql.contains("uid UUID NOT NULL"), "{sql}");
        assert!(sql.contains("optional_uid UUID"), "{sql}");
    }

    #[cfg(feature = "mysql")]
    {
        let sql = generate_create_table_sql::<UuidRecord>(ormer::DbType::MySQL).unwrap();
        assert!(sql.contains("id CHAR(36) PRIMARY KEY"), "{sql}");
        assert!(sql.contains("uid CHAR(36) NOT NULL"), "{sql}");
        assert!(sql.contains("optional_uid CHAR(36)"), "{sql}");
    }

    #[cfg(feature = "mssql")]
    {
        let sql = generate_create_table_sql::<UuidRecord>(ormer::DbType::MSSQL).unwrap();
        assert!(sql.contains("id UNIQUEIDENTIFIER PRIMARY KEY"), "{sql}");
        assert!(sql.contains("uid UNIQUEIDENTIFIER NOT NULL"), "{sql}");
        assert!(sql.contains("optional_uid UNIQUEIDENTIFIER"), "{sql}");
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_uuid_round_trip_and_map_to() {
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:")
        .await
        .unwrap();
    let _ = db.drop_table::<UuidRecord>().execute().await;
    db.create_table::<UuidRecord>().execute().await.unwrap();

    let id = uuid::Uuid::from_u128(1);
    let uid = uuid::Uuid::from_u128(2);
    db.insert(&UuidRecord {
        id,
        uid,
        optional_uid: None,
    })
    .execute()
    .await
    .unwrap();

    let record = db.find_by_id::<UuidRecord>(id).await.unwrap().unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.uid, uid);
    assert_eq!(record.optional_uid, None);

    let ids = db
        .select::<UuidRecord>()
        .filter(|record| record.id.eq(id))
        .map_to(|record| record.id)
        .collect::<Vec<uuid::Uuid>>()
        .await
        .unwrap();
    assert_eq!(ids, vec![id]);
}
