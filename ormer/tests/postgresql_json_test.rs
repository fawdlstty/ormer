#![cfg(feature = "postgresql")]

//! P0-19 回归测试：PG 下 JsonValue 字段的完整往返。
//! 绑定走 PgJsonParam（接受 JSONB），读取走 PgJsonText（剥离 jsonb 版本字节）。

pub mod _test_common;

use _test_common::postgresql_config;
use ormer::Model;

#[derive(Debug, Clone, PartialEq, Model)]
#[table = "test_postgresql_json_roundtrip"]
struct JsonDoc {
    #[primary(auto)]
    id: i32,
    doc: serde_json::Value,
    nullable_doc: Option<serde_json::Value>,
}

#[tokio::test]
async fn test_json_value_roundtrip_on_postgres() -> Result<(), Box<dyn std::error::Error>> {
    let config = postgresql_config();
    let db = ormer::Database::connect(config.0, config.1).await?;
    let _ = db.drop_table::<JsonDoc>().execute().await;

    let sql = ormer::generate_create_table_sql::<JsonDoc>(config.0)?;
    assert!(sql.contains("JSONB"), "doc column should be JSONB: {sql}");

    db.create_table::<JsonDoc>().execute().await?;
    db.validate_table::<JsonDoc>().await?;

    let doc = serde_json::json!({ "role": "admin", "level": 3 });
    db.insert(&[
        JsonDoc {
            id: 0,
            doc: doc.clone(),
            nullable_doc: Some(serde_json::json!([{ "tag": "a" }])),
        },
        JsonDoc {
            id: 0,
            doc: serde_json::json!({ "role": "viewer" }),
            nullable_doc: None,
        },
    ])
    .execute()
    .await?;

    let rows = db.select::<JsonDoc>().collect::<Vec<_>>().await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].doc, doc);
    assert_eq!(
        rows[0].nullable_doc,
        Some(serde_json::json!([{ "tag": "a" }]))
    );
    assert_eq!(rows[1].nullable_doc, None);

    db.drop_table::<JsonDoc>().execute().await?;
    Ok(())
}
