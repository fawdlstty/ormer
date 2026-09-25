#![cfg(feature = "sqlite")]

use std::collections::HashMap;

use ormer::{Model, Value};

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "polymorphic_documents"]
struct Document {
    #[primary]
    id: i64,
    tenant_id: i64,
    title: String,
    body: DocumentBody,
}

#[derive(Debug, Clone, PartialEq, ormer::ModelEnum)]
#[db_type(String)]
enum DocumentBody {
    Article {
        article_body: String,
        article_word_count: i32,
    },
    Video {
        video_url: String,
        video_duration_seconds: i32,
    },
}

#[test]
fn polymorphic_enum_metadata_expands_flat_columns() {
    assert_eq!(
        Document::columns(),
        vec![
            "id",
            "tenant_id",
            "title",
            "body",
            "article_body",
            "article_word_count",
            "video_url",
            "video_duration_seconds",
        ]
    );

    let body = Document::column_schema()
        .into_iter()
        .find(|column| column.name == "body")
        .expect("body column");
    assert_eq!(body.rust_type, "String");
    assert!(!body.is_nullable);
    assert_eq!(body.enum_variants, None);

    let article_body = Document::column_schema()
        .into_iter()
        .find(|column| column.name == "article_body")
        .expect("article_body column");
    assert_eq!(article_body.rust_name, "body.article_body");
    assert!(article_body.is_nullable);
}

#[test]
fn polymorphic_enum_field_values_write_inactive_columns_as_null() {
    let document = Document {
        id: 1,
        tenant_id: 10,
        title: "Intro".to_string(),
        body: DocumentBody::Article {
            article_body: "Hello".to_string(),
            article_word_count: 1,
        },
    };

    let values = document.field_values();
    assert_eq!(values.len(), Document::columns().len());
    assert!(matches!(&values[3], Value::Text(value) if value == "article"));
    assert!(matches!(&values[4], Value::Text(value) if value == "Hello"));
    assert!(matches!(&values[5], Value::Integer(value) if *value == 1));
    assert!(matches!(&values[6], Value::Null));
    assert!(matches!(&values[7], Value::Null));

    assert!(matches!(
        document.column_value("body"),
        Some(Value::Text(value)) if value == "article"
    ));
    assert!(matches!(
        document.column_value("video_url"),
        Some(Value::Null)
    ));
}

#[test]
fn polymorphic_enum_from_row_values_dispatches_by_discriminator() -> ormer::Result<()> {
    let article = Document::from_row_values(&[
        Value::Integer(1),
        Value::Integer(10),
        Value::Text("Intro".to_string()),
        Value::Text("article".to_string()),
        Value::Text("Hello".to_string()),
        Value::Integer(1),
        Value::Null,
        Value::Null,
    ])?;
    assert_eq!(
        article.body,
        DocumentBody::Article {
            article_body: "Hello".to_string(),
            article_word_count: 1,
        }
    );

    let video = Document::from_row_values(&[
        Value::Integer(2),
        Value::Integer(10),
        Value::Text("Clip".to_string()),
        Value::Text("video".to_string()),
        Value::Null,
        Value::Null,
        Value::Text("https://example.test/video".to_string()),
        Value::Integer(30),
    ])?;
    assert_eq!(
        video.body,
        DocumentBody::Video {
            video_url: "https://example.test/video".to_string(),
            video_duration_seconds: 30,
        }
    );

    Ok(())
}

#[test]
fn polymorphic_enum_from_row_dispatches_by_discriminator() -> ormer::Result<()> {
    let row = ormer::Row::new(HashMap::from([
        ("id".to_string(), Value::Integer(1)),
        ("tenant_id".to_string(), Value::Integer(10)),
        ("title".to_string(), Value::Text("Intro".to_string())),
        ("body".to_string(), Value::Text("article".to_string())),
        ("article_body".to_string(), Value::Text("Hello".to_string())),
        ("article_word_count".to_string(), Value::Integer(1)),
        ("video_url".to_string(), Value::Null),
        ("video_duration_seconds".to_string(), Value::Null),
    ]));

    let document = Document::from_row(&row)?;
    assert_eq!(
        document.body,
        DocumentBody::Article {
            article_body: "Hello".to_string(),
            article_word_count: 1,
        }
    );

    Ok(())
}

#[test]
fn polymorphic_enum_rejects_unknown_discriminator() {
    let result = Document::from_row_values(&[
        Value::Integer(1),
        Value::Integer(10),
        Value::Text("Intro".to_string()),
        Value::Text("unknown".to_string()),
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
    ]);

    assert!(result.is_err());
}

#[tokio::test]
async fn polymorphic_enum_sqlite_roundtrip() -> ormer::Result<()> {
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<Document>().execute().await?;

    db.insert(&Document {
        id: 1,
        tenant_id: 10,
        title: "Intro".to_string(),
        body: DocumentBody::Article {
            article_body: "Hello".to_string(),
            article_word_count: 1,
        },
    })
    .execute()
    .await?;

    db.insert(&Document {
        id: 2,
        tenant_id: 10,
        title: "Clip".to_string(),
        body: DocumentBody::Video {
            video_url: "https://example.test/video".to_string(),
            video_duration_seconds: 30,
        },
    })
    .execute()
    .await?;

    let documents = db
        .select::<Document>()
        .order_by(|d| d.id.asc())
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(documents.len(), 2);
    assert!(matches!(documents[0].body, DocumentBody::Article { .. }));
    assert_eq!(
        documents[1].body,
        DocumentBody::Video {
            video_url: "https://example.test/video".to_string(),
            video_duration_seconds: 30,
        }
    );

    Ok(())
}
