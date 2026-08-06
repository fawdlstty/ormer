#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

mod _test_common;

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
fn sqlite_create_table_sql_includes_metadata() {
    let sql = ormer::generate_create_table_sql::<SchemaMetaChild>(ormer::DbType::Sqlite).unwrap();

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS schema_meta_children_1"));
    assert!(!sql.contains("auth.schema_meta_children_1"));
    assert!(sql.contains("display_name TEXT NOT NULL DEFAULT ''"));
    assert!(
        sql.contains("CONSTRAINT uq_schema_meta_children_1_display_name UNIQUE (display_name)")
    );
    assert!(sql.contains("CONSTRAINT fk_schema_meta_children_1_parent_id FOREIGN KEY (parent_id) REFERENCES schema_meta_parents_1 (id) ON DELETE CASCADE ON UPDATE RESTRICT"));
    assert!(sql.contains("CONSTRAINT"));
    assert!(sql.contains("CHECK (length(display_name) > 0)"));
    assert!(sql.contains("DEFAULT CURRENT_TIMESTAMP"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS idx_schema_meta_children_1_display_name ON schema_meta_children_1 (display_name DESC) WHERE deleted_at IS NULL"));
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
async fn sqlite_metadata_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<SchemaMetaChild>().execute().await.ok();
    db.drop_table::<SchemaMetaParent>().execute().await.ok();

    db.create_table::<SchemaMetaParent>().execute().await?;
    db.create_table::<SchemaMetaChild>().execute().await?;
    db.validate_table::<SchemaMetaChild>().await?;

    db.drop_table::<SchemaMetaChild>().execute().await?;
    db.drop_table::<SchemaMetaParent>().execute().await?;

    Ok(())
}
