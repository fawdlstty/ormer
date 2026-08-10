#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

use chrono::{NaiveDate, NaiveTime};
use ormer::model::FromValue;

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "temporal_types_test_1"]
struct TemporalModel {
    #[primary]
    id: i32,
    business_date: NaiveDate,
    business_time: NaiveTime,
    event_at: chrono::DateTime<chrono::Utc>,
    local_at: chrono::NaiveDateTime,
    optional_date: Option<NaiveDate>,
    optional_time: Option<NaiveTime>,
}

#[test]
fn temporal_sql_types_match_backend_mappings() {
    #[cfg(feature = "sqlite")]
    {
        let sql = ormer::generate_create_table_sql::<TemporalModel>(ormer::DbType::Sqlite).unwrap();
        assert!(sql.contains("business_date TEXT NOT NULL"));
        assert!(sql.contains("business_time TEXT NOT NULL"));
        assert!(sql.contains("event_at TEXT NOT NULL"));
        assert!(sql.contains("local_at TEXT NOT NULL"));
        assert!(sql.contains("optional_date TEXT"));
        assert!(sql.contains("optional_time TEXT"));
    }

    #[cfg(feature = "postgresql")]
    {
        let sql =
            ormer::generate_create_table_sql::<TemporalModel>(ormer::DbType::PostgreSQL).unwrap();
        assert!(sql.contains("business_date DATE NOT NULL"));
        assert!(sql.contains("business_time TIME NOT NULL"));
        assert!(sql.contains("event_at TIMESTAMPTZ NOT NULL"));
        assert!(sql.contains("local_at TIMESTAMPTZ NOT NULL"));
        assert!(sql.contains("optional_date DATE"));
        assert!(sql.contains("optional_time TIME"));
    }

    #[cfg(feature = "mysql")]
    {
        let sql = ormer::generate_create_table_sql::<TemporalModel>(ormer::DbType::MySQL).unwrap();
        assert!(sql.contains("business_date DATE NOT NULL"));
        assert!(sql.contains("business_time TIME NOT NULL"));
        assert!(sql.contains("event_at DATETIME NOT NULL"));
        assert!(sql.contains("local_at DATETIME NOT NULL"));
        assert!(sql.contains("optional_date DATE"));
        assert!(sql.contains("optional_time TIME"));
    }

    #[cfg(feature = "mssql")]
    {
        let sql = ormer::generate_create_table_sql::<TemporalModel>(ormer::DbType::MSSQL).unwrap();
        assert!(sql.contains("business_date DATE NOT NULL"));
        assert!(sql.contains("business_time TIME NOT NULL"));
        assert!(sql.contains("event_at DATETIME2 NOT NULL"));
        assert!(sql.contains("local_at DATETIME2 NOT NULL"));
        assert!(sql.contains("optional_date DATE"));
        assert!(sql.contains("optional_time TIME"));
    }
}

#[test]
fn date_value_converts_to_optional_datetimes_at_midnight() {
    let date = NaiveDate::from_ymd_opt(2026, 8, 6).unwrap();
    let value = ormer::Value::Date(date);

    let utc = Option::<chrono::DateTime<chrono::Utc>>::from_value(&value)
        .unwrap()
        .unwrap();
    let naive = Option::<chrono::NaiveDateTime>::from_value(&value)
        .unwrap()
        .unwrap();

    assert_eq!(utc.date_naive(), date);
    assert_eq!(utc.time(), NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    assert_eq!(naive, date.and_hms_opt(0, 0, 0).unwrap());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_temporal_types_roundtrip_and_filter() -> Result<(), Box<dyn std::error::Error>> {
    use chrono::{TimeZone, Utc};

    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<TemporalModel>().execute().await?;

    let date = NaiveDate::from_ymd_opt(2026, 8, 6).unwrap();
    let time = NaiveTime::from_hms_micro_opt(13, 14, 15, 123_456).unwrap();
    let event_at = Utc
        .with_ymd_and_hms(2026, 8, 6, 13, 14, 15)
        .single()
        .unwrap();
    let local_at = date.and_hms_micro_opt(9, 8, 7, 654_321).unwrap();

    let first = TemporalModel {
        id: 1,
        business_date: date,
        business_time: time,
        event_at,
        local_at,
        optional_date: Some(date),
        optional_time: Some(time),
    };
    let second = TemporalModel {
        id: 2,
        business_date: date,
        business_time: time,
        event_at,
        local_at,
        optional_date: None,
        optional_time: None,
    };

    db.insert(vec![first.clone(), second.clone()])
        .execute()
        .await?;

    let rows: Vec<TemporalModel> = db
        .select::<TemporalModel>()
        .filter(|model| model.business_date.eq(date))
        .filter(|model| model.business_time.eq(time))
        .collect()
        .await?;
    assert_eq!(rows.len(), 2);

    let first_row: Vec<TemporalModel> = db
        .select::<TemporalModel>()
        .filter(|model| model.id.eq(1))
        .collect()
        .await?;
    assert_eq!(first_row, vec![first]);

    let second_row: Vec<TemporalModel> = db
        .select::<TemporalModel>()
        .filter(|model| model.id.eq(2))
        .collect()
        .await?;
    assert_eq!(second_row, vec![second]);

    Ok(())
}
