#![cfg(feature = "sqlite")]

//! P3-7 回归测试：SQLite 模型路径解码不再对所有文本做时间/数值嗅探。
//! 普通文本列存时间样式或数字样式字符串必须原样解码回 String。

pub mod _test_common;

use _test_common::sqlite_config;
use ormer::Model;

#[derive(Debug, Clone, PartialEq, Model)]
#[table = "no_sniff_texts"]
struct SniffProofText {
    #[primary]
    id: i32,
    label: String,
    // 时间样式、数字样式与含前导零的文本
    timestamp_like: String,
    number_like: String,
    zero_padded: String,
}

#[derive(Debug, Clone, PartialEq, Model)]
#[table = "no_sniff_events"]
struct SniffProofEvent {
    #[primary]
    id: i32,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn text_columns_preserve_datetime_and_numeric_strings() -> Result<(), Box<dyn std::error::Error>>
{
    let db = ormer::Database::connect(sqlite_config().0, sqlite_config().1).await?;
    db.create_table::<SniffProofText>().execute().await?;

    let row = SniffProofText {
        id: 1,
        label: "plain".to_string(),
        timestamp_like: "2024-01-01T00:00:00+00:00".to_string(),
        number_like: "3.14".to_string(),
        zero_padded: "007".to_string(),
    };
    db.insert(&row).execute().await?;

    let rows = db.select::<SniffProofText>().collect::<Vec<_>>().await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], row);

    db.drop_table::<SniffProofText>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn datetime_columns_roundtrip_through_text_storage() -> Result<(), Box<dyn std::error::Error>>
{
    let db = ormer::Database::connect(sqlite_config().0, sqlite_config().1).await?;
    db.create_table::<SniffProofEvent>().execute().await?;

    let event = SniffProofEvent {
        id: 1,
        occurred_at: chrono::DateTime::from_timestamp_millis(1_725_854_400_123).unwrap(),
    };
    db.insert(&event).execute().await?;

    let rows = db.select::<SniffProofEvent>().collect::<Vec<_>>().await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].occurred_at, event.occurred_at);

    db.drop_table::<SniffProofEvent>().execute().await?;
    Ok(())
}
