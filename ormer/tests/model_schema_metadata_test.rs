#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

pub mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table(schema = "auth", name = "schema_meta_parents_1")]
struct SchemaMetaParent {
    #[primary]
    id: i32,
}

#[derive(Debug, Clone, ormer::Model)]
#[table(schema = "auth", name = "schema_meta_children_1")]
struct SchemaMetaChild {
    #[primary]
    id: i32,

    #[column(name = "display_name")]
    #[default("")]
    #[check(expr = "length(display_name) > 0")]
    #[unique(name = "uq_schema_meta_children_1_display_name")]
    #[index(
        name = "idx_schema_meta_children_1_display_name",
        order = "DESC",
        where = "deleted_at IS NULL"
    )]
    name: String,

    #[default(expr = "CURRENT_TIMESTAMP")]
    created_at: chrono::NaiveDateTime,

    deleted_at: Option<chrono::NaiveDateTime>,

    #[foreign(
        SchemaMetaParent.id,
        name = "fk_schema_meta_children_1_parent_id",
        on_delete = Cascade,
        on_update = Restrict
    )]
    parent_id: i32,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "quoted_column_models_1"]
struct QuotedColumnModel {
    #[primary]
    id: i32,

    #[column(name = "select")]
    select_value: String,

    #[column(name = "display-name")]
    display_name: String,
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_create_table_sql_rejects_partial_index_metadata() {
    let error = ormer::generate_create_table_sql::<SchemaMetaChild>(ormer::DbType::Sqlite)
        .expect_err("SQLite must reject partial index metadata");
    assert!(matches!(
        error,
        ormer::OrmerError::UnsupportedFeature {
            backend: ormer::DbType::Sqlite,
            feature: "partial index WHERE clauses",
        }
    ));

    let name_column = <SchemaMetaChild as ormer::Model>::column_schema()
        .into_iter()
        .find(|column| column.name == "display_name")
        .expect("display_name column");
    assert_eq!(name_column.index_where, Some("deleted_at IS NULL"));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_quoted_column_crud_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<QuotedColumnModel>().execute().await.ok();
    db.create_table::<QuotedColumnModel>().execute().await?;

    db.insert(&QuotedColumnModel {
        id: 1,
        select_value: "alpha".to_string(),
        display_name: "Alpha".to_string(),
    })
    .execute()
    .await?;

    let rows: Vec<QuotedColumnModel> = db
        .select::<QuotedColumnModel>()
        .filter(|p| p.select_value.eq("alpha".to_string()))
        .collect()
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "Alpha");

    let updated = QuotedColumnModel {
        id: 1,
        select_value: "beta".to_string(),
        display_name: "Beta".to_string(),
    };
    db.update::<QuotedColumnModel>()
        .set_model_fields(&updated, |p| (p.select_value, p.display_name))
        .execute()
        .await?;

    let rows: Vec<QuotedColumnModel> = db
        .select::<QuotedColumnModel>()
        .filter(|p| p.display_name.eq("Beta".to_string()))
        .collect()
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].select_value, "beta");

    let ordered: Vec<QuotedColumnModel> = db
        .select::<QuotedColumnModel>()
        .order_by_desc(|p| p.display_name)
        .collect()
        .await?;
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].display_name, "Beta");

    db.drop_table::<QuotedColumnModel>().execute().await?;

    Ok(())
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_metadata_roundtrip_rejects_partial_index() -> Result<(), Box<dyn std::error::Error>>
{
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<SchemaMetaChild>().execute().await.ok();
    db.drop_table::<SchemaMetaParent>().execute().await.ok();

    db.create_table::<SchemaMetaParent>().execute().await?;
    let error = db
        .create_table::<SchemaMetaChild>()
        .execute()
        .await
        .expect_err("SQLite create_table must reject partial index metadata");
    assert!(matches!(
        error,
        ormer::OrmerError::UnsupportedFeature {
            backend: ormer::DbType::Sqlite,
            feature: "partial index WHERE clauses",
        }
    ));

    db.drop_table::<SchemaMetaParent>().execute().await?;

    Ok(())
}
