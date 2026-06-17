#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_duration_field_1"]
struct DurationFieldModel {
    #[primary]
    id: i32,
    duration: std::time::Duration,
}

async fn test_duration_sql_type_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = ormer::generate_create_table_sql::<DurationFieldModel>(config.0)?;
    println!("SQL: {}", sql);

    match config.0 {
        #[cfg(feature = "sqlite")]
        ormer::DbType::Sqlite => {
            assert!(sql.contains("duration INTEGER NOT NULL"));
        }
        #[cfg(feature = "postgresql")]
        ormer::DbType::PostgreSQL => {
            assert!(sql.contains("duration INTERVAL NOT NULL"));
        }
        #[cfg(feature = "mysql")]
        ormer::DbType::MySQL => {
            assert!(sql.contains("duration BIGINT NOT NULL"));
        }
        #[cfg(feature = "mssql")]
        ormer::DbType::MSSQL => {
            assert!(sql.contains("duration BIGINT NOT NULL"));
        }
    }

    Ok(())
}

#[test]
fn duration_value_roundtrip() {
    let duration = std::time::Duration::from_millis(1234);
    let value = ormer::Value::from(duration);
    let decoded = <std::time::Duration as ormer::FromValue>::from_value(&value).unwrap();
    assert_eq!(decoded, duration);
}

test_on_all_dbs_result!(test_duration_sql_type_impl);
