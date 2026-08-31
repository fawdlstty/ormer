#![cfg(feature = "questdb")]

use ormer::model::DbBackendTypeMapper;
use ormer::{Database, DbType, OrmerError, generate_create_table_sql_with_name};
use ormer::{Migration, MigrationStep};

#[derive(Debug, Clone, ormer::Model)]
#[table = "questdb_events"]
#[postgresql(storage = "main", fillfactor = 80)]
struct QuestDbEvent {
    #[primary]
    id: i64,
    name: String,
    #[hypertable(std::time::Duration::from_secs(3600))]
    created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "questdb_auto_events"]
struct QuestDbAutoEvent {
    #[primary(auto)]
    id: i32,
}

#[test]
fn questdb_type_mapper_uses_native_types() {
    assert_eq!(
        DbType::QuestDB.sql_type("i16", false, false, false, None),
        "SHORT"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("i32", false, false, false, None),
        "INT"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("i64", false, false, false, None),
        "LONG"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("f64", false, false, false, None),
        "DOUBLE"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("String", false, false, false, None),
        "STRING"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("Uuid", false, false, false, None),
        "UUID"
    );
    assert_eq!(
        DbType::QuestDB.sql_type("EventKind", false, false, false, Some(&["A", "B"])),
        "SYMBOL"
    );
    assert_eq!(
        <ormer::abstract_layer::questdb_backend::QuestDBTypeMapper as DbBackendTypeMapper>::sql_type(
            "bool", false, false, false, None,
        ),
        "BOOLEAN"
    );
}

#[test]
fn questdb_create_table_uses_designated_timestamp_without_constraints() {
    let sql = generate_create_table_sql_with_name::<QuestDbEvent>(DbType::QuestDB, None).unwrap();

    assert!(
        !sql.contains(" WITH ("),
        "unexpected PostgreSQL options: {sql}"
    );
    assert!(sql.contains("id LONG"), "actual SQL: {sql}");
    assert!(sql.contains("name STRING"));
    assert!(sql.contains("created_at TIMESTAMP"));
    assert!(sql.ends_with(") timestamp(created_at)"));
    for constraint in ["PRIMARY KEY", "FOREIGN KEY", "UNIQUE", "CHECK"] {
        assert!(!sql.contains(constraint), "unexpected {constraint}: {sql}");
    }
}

#[test]
fn questdb_rejects_auto_increment_columns() {
    let error =
        generate_create_table_sql_with_name::<QuestDbAutoEvent>(DbType::QuestDB, None).unwrap_err();

    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            feature: "auto-increment columns",
            ..
        }
    ));
}

struct QuestDbCreateMigration;

impl Migration for QuestDbCreateMigration {
    fn version(&self) -> u64 {
        918_273_645
    }

    fn name(&self) -> &str {
        "questdb_create_migration"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "CREATE TABLE questdb_migration_table (id LONG)".to_string(),
        }]
    }

    fn down(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "DROP TABLE IF EXISTS questdb_migration_table".to_string(),
        }]
    }
}

#[tokio::test]
async fn questdb_wire_protocol_supports_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let Some(connection_string) = option_env!("ORMER_TEST_QUESTDB") else {
        return Ok(());
    };
    let db = Database::connect(DbType::QuestDB, connection_string).await?;
    let _ = db.drop_table::<QuestDbEvent>().execute().await;
    db.create_table::<QuestDbEvent>().execute().await?;

    db.insert(&[QuestDbEvent {
        id: 1,
        name: "before".to_string(),
        created_at: chrono::Utc::now().naive_utc(),
    }])
    .execute()
    .await?;
    let events = db
        .select::<QuestDbEvent>()
        .filter(|event| event.id.eq(1i64))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "before");

    db.update::<QuestDbEvent>()
        .filter(|event| event.id.eq(1i64))
        .set(|event| event.name = event.name.set("after".to_string()))
        .execute()
        .await?;
    let events = db
        .select::<QuestDbEvent>()
        .filter(|event| event.id.eq(1i64))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(events[0].name, "after");
    let error = db
        .delete::<QuestDbEvent>()
        .filter(|event| event.id.eq(1i64))
        .execute()
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ormer::OrmerError::UnsupportedFeature { .. }
    ));

    let _ = db
        .execute_sql("DROP TABLE IF EXISTS questdb_migration_table")
        .await;
    let _ = db
        .execute_sql("DROP TABLE IF EXISTS __ormer_migrations")
        .await;
    let runner = ormer::MigrationRunner::new(&db, &[QuestDbCreateMigration]);
    assert_eq!(runner.execute().await?, 1);
    assert_eq!(runner.execution_status().await?.completed.len(), 1);
    runner.rollback_last().await?;
    db.drop_table::<QuestDbEvent>().execute().await?;

    Ok(())
}
