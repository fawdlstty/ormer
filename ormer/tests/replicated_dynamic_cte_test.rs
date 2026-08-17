#![cfg(feature = "sqlite")]

pub mod _test_common;

use ormer::{ConnectionPool, Database, DbType};

define_test_user_simple!(ReplicatedUser, "replicated_users_test_1");
define_test_user_simple!(DynamicUser, "dynamic_users_test_1");

#[derive(Debug, ormer::Model, Clone)]
#[table = "cte_categories_test_1"]
struct Category {
    #[primary]
    id: i32,
    parent_id: Option<i32>,
    name: String,
}

fn sqlite_file(name: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "ormer_{}_{}_{}.db",
        name,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    path.to_string_lossy().into_owned()
}

async fn prepare_replica_db(path: &str, id: i32, name: &str) -> ormer::Result<()> {
    let db = Database::connect(DbType::Sqlite, path).await?;
    let _ = db.drop_table::<ReplicatedUser>().execute().await;
    db.create_table::<ReplicatedUser>().execute().await?;
    db.insert(&ReplicatedUser {
        id,
        name: name.to_string(),
        age: id,
    })
    .execute()
    .await?;
    Ok(())
}

#[tokio::test]
async fn replicated_database_routes_read_and_write() -> Result<(), Box<dyn std::error::Error>> {
    let primary = sqlite_file("replicated_primary");
    let replica = sqlite_file("replicated_read");

    prepare_replica_db(&primary, 1, "primary").await?;
    prepare_replica_db(&replica, 2, "replica").await?;

    let db = Database::replicated(DbType::Sqlite)
        .write(&primary)
        .read(&replica)
        .connect()
        .await?;

    let write_rows = db
        .write()
        .select::<ReplicatedUser>()
        .collect::<Vec<_>>()
        .await?;
    let read_rows = db
        .read()
        .select::<ReplicatedUser>()
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(write_rows[0].name, "primary");
    assert_eq!(read_rows[0].name, "replica");

    let _ = std::fs::remove_file(primary);
    let _ = std::fs::remove_file(replica);
    Ok(())
}

#[tokio::test]
async fn replicated_connection_pool_routes_read_and_write() -> Result<(), Box<dyn std::error::Error>>
{
    let primary = sqlite_file("replicated_pool_primary");
    let replica = sqlite_file("replicated_pool_read");

    prepare_replica_db(&primary, 10, "pool-primary").await?;
    prepare_replica_db(&replica, 20, "pool-replica").await?;

    let pool = ConnectionPool::replicated(DbType::Sqlite)
        .write(&primary)
        .read(&replica)
        .range(0..1)
        .connect()
        .await?;

    let writer = pool.write().get().await?;
    let reader = pool.read().get().await?;
    let write_rows = writer
        .select::<ReplicatedUser>()
        .collect::<Vec<_>>()
        .await?;
    let read_rows = reader
        .select::<ReplicatedUser>()
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(write_rows[0].name, "pool-primary");
    assert_eq!(read_rows[0].name, "pool-replica");

    let _ = std::fs::remove_file(primary);
    let _ = std::fs::remove_file(replica);
    Ok(())
}

#[tokio::test]
async fn dynamic_field_filter_and_order_validate_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let db = Database::connect(DbType::Sqlite, ":memory:").await?;
    db.create_table::<DynamicUser>().execute().await?;
    db.insert(&vec![
        DynamicUser {
            id: 1,
            name: "Alice".to_string(),
            age: 30,
        },
        DynamicUser {
            id: 2,
            name: "Bob".to_string(),
            age: 20,
        },
    ])
    .execute()
    .await?;

    let users = db
        .select::<DynamicUser>()
        .filter(|u| u.field("name").ne("Bob"))
        .order_by_dynamic(|u| u.field("age").desc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");

    let err = db
        .select::<DynamicUser>()
        .filter(|u| u.field("missing").eq(1))
        .collect::<Vec<_>>()
        .await
        .expect_err("missing dynamic field should fail before query execution");
    assert!(err.to_string().contains("missing"));

    Ok(())
}

#[tokio::test]
async fn recursive_cte_generates_descendants_and_ancestors_sql()
-> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Sqlite, ":memory:").await?;

    let descendants = db
        .select::<Category>()
        .descendants(|c| (c.id, c.parent_id), 1)
        .order_by(|c| c.id.asc())
        .to_sql()?;
    let descendants_sql = &descendants.statements[0].sql;
    assert!(descendants_sql.contains("WITH RECURSIVE"));
    assert!(descendants_sql.contains("UNION ALL"));
    assert!(descendants_sql.contains("child.parent_id = parent.id"));
    assert_eq!(descendants.statements[0].params.len(), 1);

    let ancestors = db
        .select::<Category>()
        .ancestors(|c| (c.id, c.parent_id), 3)
        .order_by(|c| c.id.asc())
        .to_sql()?;
    let ancestors_sql = &ancestors.statements[0].sql;
    assert!(ancestors_sql.contains("WITH RECURSIVE"));
    assert!(ancestors_sql.contains("UNION ALL"));
    assert!(ancestors_sql.contains("parent.id = child.parent_id"));
    assert_eq!(ancestors.statements[0].params.len(), 1);

    Ok(())
}
