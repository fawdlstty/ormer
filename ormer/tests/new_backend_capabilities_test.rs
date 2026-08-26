#[cfg(any(feature = "duckdb", feature = "clickhouse"))]
use ormer::DbType;
#[cfg(feature = "clickhouse")]
use ormer::OrmerError;

#[cfg(feature = "clickhouse")]
#[derive(Debug, ormer::Model)]
#[table = "clickhouse_capability_users"]
struct ClickHouseCapabilityUser {
    #[primary]
    id: i64,
    name: String,
    nickname: Option<String>,
}

#[cfg(feature = "clickhouse")]
#[derive(ormer::clickhouse::Row, serde::Serialize, serde::Deserialize)]
struct ClickHouseNativeRow {
    id: u64,
    name: String,
}

#[test]
#[cfg(feature = "duckdb")]
fn duckdb_exposes_dialect_type_mapping() {
    assert_eq!(
        DbType::DuckDB.sql_type("i64", false, false, false, None),
        "BIGINT NOT NULL"
    );
    assert_eq!(
        DbType::DuckDB.sql_type("serde_json::Value", false, false, true, None),
        "JSON"
    );
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_exposes_dialect_type_mapping() {
    assert_eq!(
        DbType::ClickHouse.sql_type("i64", false, false, false, None),
        "Int64"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("String", false, false, true, None),
        "Nullable(String)"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("i64", true, false, false, None),
        "Int64"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("Vec<i32>", false, false, false, None),
        "Array(Int32)"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("Vec<Option<i64>>", false, false, true, None),
        "Nullable(Array(Nullable(Int64)))"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("chrono::NaiveDateTime", false, false, false, None),
        "DateTime64(3)"
    );
    assert_eq!(
        DbType::ClickHouse.sql_type("u128", false, false, false, None),
        "UInt128"
    );
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_connects_with_an_in_memory_database() {
    ormer::Database::connect(DbType::DuckDB, ":memory:")
        .await
        .expect("DuckDB in-memory connection should be supported");
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_connection_pool_supports_crud_and_transactions()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = ormer::Database::create_pool(DbType::DuckDB, ":memory:")
        .range(0..1)
        .build()
        .await?;
    let connection = pool.get().await?;

    connection
        .create_table::<DuckDbPoolUser>()
        .execute()
        .await?;
    let id = connection
        .insert(&DuckDbPoolUser {
            id: 0,
            name: "Alice".to_string(),
        })
        .execute()
        .await?;
    assert_eq!(id, 1);

    connection
        .transaction(|txn| {
            Box::pin(async move {
                txn.update::<DuckDbPoolUser>()
                    .set(|user| user.name = user.name.set("Bob".to_string()))
                    .filter(|user| user.id.eq(1))
                    .execute()
                    .await?;
                Ok(())
            })
        })
        .await?;

    let users = connection
        .select::<DuckDbPoolUser>()
        .collect::<Vec<DuckDbPoolUser>>()
        .await?;
    assert_eq!(users[0].name, "Bob");
    Ok(())
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_create_table_requires_and_renders_engine() {
    let error = ormer::generate_create_table_sql::<ClickHouseCapabilityUser>(DbType::ClickHouse)
        .expect_err("ClickHouse tables require an explicit engine");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "CREATE TABLE without explicit ClickHouse engine settings",
        }
    ));

    let sql = ormer::generate_clickhouse_create_table_sql::<ClickHouseCapabilityUser>(
        "MergeTree ORDER BY (id)",
    )
    .unwrap();
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS clickhouse_capability_users"));
    assert!(sql.contains("id Int64"));
    assert!(sql.contains("name String"));
    assert!(sql.contains("nickname Nullable(String)"));
    assert!(sql.ends_with("ENGINE = MergeTree ORDER BY (id)"));

    let named_sql =
        ormer::generate_clickhouse_create_table_sql_with_name::<ClickHouseCapabilityUser>(
            "MergeTree ORDER BY id",
            Some("clickhouse_capability_users_archive"),
        )
        .unwrap();
    assert!(named_sql.contains("clickhouse_capability_users_archive"));
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_engine_clause_rejects_statement_terminators() {
    let error = ormer::generate_clickhouse_create_table_sql::<ClickHouseCapabilityUser>(
        "MergeTree ORDER BY (id); DROP TABLE users",
    )
    .expect_err("engine input must be a single clause");
    assert!(matches!(error, OrmerError::Other { .. }));
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_native_database_accepts_http_connection_string() {
    ormer::ClickHouseDatabase::connect("http://localhost:8123?database=default&compression=1")
        .expect("ClickHouse client construction should not require a live server");
}

#[test]
#[cfg(feature = "clickhouse")]
fn clickhouse_native_typed_api_accepts_owned_rows() {
    fn assert_api<
        T: ormer::clickhouse::RowOwned + ormer::clickhouse::RowRead + ormer::clickhouse::RowWrite,
    >() {
    }

    assert_api::<ClickHouseNativeRow>();
    let db = ormer::ClickHouseDatabase::connect("http://localhost:8123")
        .expect("ClickHouse client construction should not require a live server");
    let _ = db.select::<ClickHouseNativeRow>("SELECT 1 AS id, 'Alice' AS name");
    let _ = db.select_one::<ClickHouseNativeRow>("SELECT 1 AS id, 'Alice' AS name");
    let _ = db.select_optional::<ClickHouseNativeRow>("SELECT 1 AS id, 'Alice' AS name");
    let _ = db.select_stream::<ClickHouseNativeRow>("SELECT 1 AS id, 'Alice' AS name");
    let _ = db.select_json_stream("SELECT 1 AS id");
    let _ = db.insert_rows(
        "clickhouse_capability_users",
        [ClickHouseNativeRow {
            id: 1,
            name: "Alice".to_string(),
        }],
    );
}

#[tokio::test]
#[cfg(feature = "clickhouse")]
async fn clickhouse_connect_reports_unsupported_until_async_adapter_is_ready() {
    let error = match ormer::Database::connect(DbType::ClickHouse, "http://localhost:8123").await {
        Ok(_) => panic!("ClickHouse execution is intentionally capability-gated"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "Database::connect",
        }
    ));
}

#[cfg(feature = "duckdb")]
#[derive(Debug, ormer::Model)]
#[table = "duckdb_pool_users"]
struct DuckDbPoolUser {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[cfg(feature = "duckdb")]
#[derive(Debug, ormer::Model)]
#[table = "duckdb_extended_users"]
struct DuckDbExtendedUser {
    #[primary]
    id: i32,
    #[unique]
    email: String,
    name: String,
    tags: Vec<i32>,
    big_tags: Vec<i64>,
    nullable_tags: Vec<Option<i64>>,
    labels: Vec<String>,
}

#[cfg(feature = "duckdb")]
#[derive(Debug, ormer::Model)]
#[table = "duckdb_migration_users"]
struct DuckDbMigrationUserV1 {
    #[primary]
    id: i32,
    score: i32,
}

#[cfg(feature = "duckdb")]
#[derive(Debug, ormer::Model)]
#[table = "duckdb_migration_users"]
struct DuckDbMigrationUserV2 {
    #[primary]
    id: i32,
    score: i64,
}

#[cfg(feature = "duckdb")]
#[derive(Debug, ormer::Model)]
#[table = "duckdb_migration_users"]
struct DuckDbMigrationUserV3 {
    #[primary]
    id: i32,
    score: Option<i64>,
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_supports_conflicts_bulk_updates_and_arrays()
-> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.create_table::<DuckDbExtendedUser>().execute().await?;
    db.validate_table::<DuckDbExtendedUser>().await?;

    db.insert(&DuckDbExtendedUser {
        id: 1,
        email: "alice@example.com".to_string(),
        name: "Alice".to_string(),
        tags: vec![1, 2],
        big_tags: vec![10, 20],
        nullable_tags: vec![Some(30), None],
        labels: vec!["first".to_string(), "second".to_string()],
    })
    .execute()
    .await?;

    db.insert(&DuckDbExtendedUser {
        id: 2,
        email: "bob@example.com".to_string(),
        name: "Bob".to_string(),
        tags: vec![9],
        big_tags: vec![90],
        nullable_tags: vec![Some(91)],
        labels: vec!["bob".to_string()],
    })
    .execute()
    .await?;

    db.insert(&DuckDbExtendedUser {
        id: 3,
        email: "alice@example.com".to_string(),
        name: "Ignored".to_string(),
        tags: vec![9],
        big_tags: vec![90],
        nullable_tags: vec![Some(91)],
        labels: vec!["ignored".to_string()],
    })
    .on_conflict(|user| user.email)
    .do_nothing()
    .execute()
    .await?;

    db.update::<DuckDbExtendedUser>()
        .set_model(&DuckDbExtendedUser {
            id: 1,
            email: "alice@example.com".to_string(),
            name: "Alice updated".to_string(),
            tags: vec![3, 4, 5],
            big_tags: vec![30, 40],
            nullable_tags: vec![None, Some(50)],
            labels: vec!["updated".to_string()],
        })
        .set_model(&DuckDbExtendedUser {
            id: 2,
            email: "bob@example.com".to_string(),
            name: "Bob updated".to_string(),
            tags: vec![6],
            big_tags: vec![60],
            nullable_tags: vec![Some(61)],
            labels: vec!["updated".to_string(), "bob".to_string()],
        })
        .execute()
        .await?;

    let users = db
        .select::<DuckDbExtendedUser>()
        .order_by(|user| user.id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].name, "Alice updated");
    assert_eq!(users[0].tags, vec![3, 4, 5]);
    assert_eq!(users[0].big_tags, vec![30, 40]);
    assert_eq!(users[0].nullable_tags, vec![None, Some(50)]);
    assert_eq!(users[0].labels, vec!["updated"]);
    assert_eq!(users[1].name, "Bob updated");
    assert_eq!(users[1].tags, vec![6]);
    assert_eq!(users[1].big_tags, vec![60]);
    assert_eq!(users[1].nullable_tags, vec![Some(61)]);
    assert_eq!(users[1].labels, vec!["updated", "bob"]);
    Ok(())
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_preserves_iso8601_strings_and_empty_arrays()
-> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.create_table::<DuckDbExtendedUser>().execute().await?;
    db.insert(&DuckDbExtendedUser {
        id: 1,
        email: "iso@example.com".to_string(),
        name: "2024-01-01T00:00:00Z".to_string(),
        tags: Vec::new(),
        big_tags: Vec::new(),
        nullable_tags: Vec::new(),
        labels: Vec::new(),
    })
    .execute()
    .await?;

    let user = db
        .find_by_id::<DuckDbExtendedUser>(1)
        .await?
        .expect("inserted DuckDB row");
    assert_eq!(user.name, "2024-01-01T00:00:00Z");
    assert!(user.tags.is_empty());
    assert!(user.big_tags.is_empty());
    assert!(user.nullable_tags.is_empty());
    assert!(user.labels.is_empty());
    Ok(())
}

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_migrates_column_types_and_nullability() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;
    db.migrate_table::<DuckDbMigrationUserV1>()
        .execute()
        .await?;
    db.insert(&DuckDbMigrationUserV1 { id: 1, score: 7 })
        .execute()
        .await?;

    let type_plan = db.migrate_table::<DuckDbMigrationUserV2>().plan().await?;
    assert_eq!(
        type_plan.to_sql()?,
        "ALTER TABLE duckdb_migration_users ALTER COLUMN score SET DATA TYPE BIGINT"
    );
    db.migrate_table::<DuckDbMigrationUserV2>()
        .execute()
        .await?;

    let nullability_plan = db.migrate_table::<DuckDbMigrationUserV3>().plan().await?;
    assert_eq!(
        nullability_plan.to_sql()?,
        "ALTER TABLE duckdb_migration_users ALTER COLUMN score DROP NOT NULL"
    );
    db.migrate_table::<DuckDbMigrationUserV3>()
        .execute()
        .await?;

    db.execute_sql("UPDATE duckdb_migration_users SET score = NULL WHERE id = 1")
        .await?;
    let users = db
        .select::<DuckDbMigrationUserV3>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users[0].score, None);
    Ok(())
}
