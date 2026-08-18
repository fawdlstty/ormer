#![cfg(feature = "sqlite")]

pub mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "db_first_parent"]
struct DbFirstParentSource {
    #[primary(auto)]
    id: i32,

    #[index(name = "idx_db_first_parent_name")]
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "db_first_child"]
struct DbFirstChildSource {
    #[primary(auto)]
    id: i32,

    #[foreign(DbFirstParentSource.id, on_delete = Cascade)]
    parent_id: i32,

    #[unique(name = "uq_db_first_child_code")]
    code: String,

    note: Option<String>,
}

#[tokio::test]
async fn sqlite_generate_entities_from_schema() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::sqlite_config();
    let db = _test_common::create_db_connection(&config).await?;

    db.drop_table::<DbFirstChildSource>().execute().await.ok();
    db.drop_table::<DbFirstParentSource>().execute().await.ok();

    db.create_table::<DbFirstParentSource>().execute().await?;
    db.create_table::<DbFirstChildSource>().execute().await?;

    let code = db.generate_entities(None).await?;

    assert!(code.contains("#[table = \"db_first_parent\"]"), "{code}");
    assert!(code.contains("pub struct DbFirstParent"), "{code}");
    assert!(code.contains("#[primary(auto)]"), "{code}");
    assert!(
        code.contains("#[index(name = \"idx_db_first_parent_name\")]"),
        "{code}"
    );
    assert!(
        code.contains("#[unique(name = \"uq_db_first_child_code\")]"),
        "{code}"
    );
    assert!(code.contains("#[foreign(DbFirstParent.id"), "{code}");
    assert!(code.contains("#[belongs_to(parent_id)]"), "{code}");
    assert!(code.contains("pub parent: Option<DbFirstParent>"), "{code}");
    assert!(
        code.contains("#[has_many(DbFirstChild.parent_id)]"),
        "{code}"
    );
    assert!(code.contains("pub note: Option<String>"), "{code}");

    db.drop_table::<DbFirstChildSource>().execute().await?;
    db.drop_table::<DbFirstParentSource>().execute().await?;

    Ok(())
}
