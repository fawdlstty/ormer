#![cfg(feature = "postgresql")]

//! schema 限定表（`#[table = "schema.table"]`）的建表端到端验证：
//! 目标 schema 不存在时，`migrate_table` 必须先自动补建 schema 再建表，
//! 且重复执行幂等。

use ormer::{Database, DbType};

#[derive(Debug, Clone, ormer::Model)]
#[table = "ormer_schema_e2e.schema_users"]
struct SchemaUser {
    #[primary]
    id: i32,
    #[index]
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "ormer_schema_e2e_p.schema_users"]
struct PermissiveSchemaUser {
    #[primary]
    id: i32,
    #[index]
    name: String,
}

async fn connect() -> Result<Database, Box<dyn std::error::Error>> {
    let connection_string = option_env!("ORMER_TEST_POSTGRES")
        .filter(|value| !value.is_empty())
        .unwrap_or("postgres://postgres:postgres@localhost:5432/ormer_test");
    Ok(Database::connect(DbType::PostgreSQL, connection_string).await?)
}

async fn schema_exists(db: &Database, schema: &str) -> bool {
    db.select_sql::<bool>(&format!(
        "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = '{schema}')"
    ))
    .collect::<Vec<bool>>()
    .await
    .expect("query schemata")
    .into_iter()
    .next()
    .unwrap_or(false)
}

async fn table_exists(db: &Database, schema: &str, table: &str) -> bool {
    db.select_sql::<bool>(&format!(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = '{schema}' AND table_name = '{table}')"
    ))
    .collect::<Vec<bool>>()
    .await
    .expect("query tables")
    .into_iter()
    .next()
    .unwrap_or(false)
}

#[tokio::test]
async fn migrate_table_creates_missing_schema() -> Result<(), Box<dyn std::error::Error>> {
    let db = connect().await?;

    // 从干净状态开始：schema 与表都不存在
    db.execute_sql("DROP TABLE IF EXISTS ormer_schema_e2e.schema_users")
        .await?;
    db.execute_sql("DROP SCHEMA IF EXISTS ormer_schema_e2e CASCADE")
        .await?;
    assert!(!schema_exists(&db, "ormer_schema_e2e").await);

    // 首次 migrate_table：应自动建 schema + 表
    let outcome = db.migrate_table::<SchemaUser>().await?;
    assert!(outcome.created_table, "first migrate should create the table");
    assert!(
        matches!(outcome.diagnosis, ormer::TableDiagnosis::Migratable(_)),
        "first migrate should diagnose a create plan, got {:?}",
        outcome.diagnosis
    );
    assert!(schema_exists(&db, "ormer_schema_e2e").await);
    assert!(table_exists(&db, "ormer_schema_e2e", "schema_users").await);

    // 幂等：二次执行不报错，且写入可用
    let outcome = db.migrate_table::<SchemaUser>().await?;
    assert!(
        matches!(outcome.diagnosis, ormer::TableDiagnosis::Ready),
        "second migrate must converge to Ready, got {:?}",
        outcome.diagnosis
    );
    db.insert(&SchemaUser { id: 1, name: "alice".into() })
        .execute()
        .await?;

    // 删表不删 schema：migrate_table 只补表
    db.drop_table::<SchemaUser>().execute().await?;
    assert!(!table_exists(&db, "ormer_schema_e2e", "schema_users").await);
    db.migrate_table::<SchemaUser>().await?;
    assert!(table_exists(&db, "ormer_schema_e2e", "schema_users").await);

    Ok(())
}

#[tokio::test]
async fn migrate_table_creates_missing_schema_in_custom_schema() -> Result<(), Box<dyn std::error::Error>> {
    let db = connect().await?;

    db.execute_sql("DROP SCHEMA IF EXISTS ormer_schema_e2e_p CASCADE")
        .await?;
    db.migrate_table::<PermissiveSchemaUser>().await?;
    assert!(
        table_exists(&db, "ormer_schema_e2e_p", "schema_users").await,
        "migrate_table should create missing schema and table"
    );

    Ok(())
}
