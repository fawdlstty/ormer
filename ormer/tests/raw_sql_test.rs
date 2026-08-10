#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

define_test_user_simple!(RawSqlUser, "raw_sql_users_1");

use ormer::model::{FromRowValues, Value};

#[derive(Debug, PartialEq)]
struct UserName {
    id: i32,
    name: String,
}

impl FromRowValues for UserName {
    fn from_row_values(values: &[Value]) -> ormer::Result<Self> {
        if values.len() < 2 {
            return Err(ormer::ormer_error!("Expected id and name"));
        }
        Ok(Self {
            id: i32::from_row_values(&values[0..1])?,
            name: String::from_row_values(&values[1..2])?,
        })
    }
}

#[test]
fn raw_sql_tokenizer_skips_sql_literal_regions() {
    let raw = ormer::sql(
        "SELECT ':skip', \"quoted:name\", [bracket:name], $$dollar:name$$ \
         FROM raw_sql_users_1 \
         WHERE name = :name AND id = {id} \
         -- :comment\n/* {comment} */",
    )
    .bind_named("name", "Alice".to_string())
    .bind_named("id", 1);

    #[cfg(feature = "sqlite")]
    {
        let (sql, params) = raw.render(ormer::DbType::Sqlite).unwrap();
        assert!(sql.contains("':skip'"));
        assert!(sql.contains("\"quoted:name\""));
        assert!(sql.contains("[bracket:name]"));
        assert!(sql.contains("$$dollar:name$$"));
        assert!(sql.contains("-- :comment"));
        assert!(sql.contains("/* {comment} */"));
        assert!(sql.contains("name = ? AND id = ?"));
        assert_eq!(params.len(), 2);
    }

    #[cfg(feature = "postgresql")]
    {
        let (sql, params) = raw.render(ormer::DbType::PostgreSQL).unwrap();
        assert!(sql.contains("name = $1 AND id = $2"));
        assert_eq!(params.len(), 2);
    }
}

async fn test_raw_sql_binding_and_mapping_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    let _ = db.drop_table::<RawSqlUser>().execute().await;
    db.create_table::<RawSqlUser>().execute().await?;

    db.execute_sql(ormer::sql!(
        "INSERT INTO raw_sql_users_1 (id, name, age) VALUES ({id}, {name}, {age})",
        id = 1,
        name = "Alice".to_string(),
        age = 20,
    ))
    .await?;
    db.execute_sql(
        ormer::sql("INSERT INTO raw_sql_users_1 (id, name, age) VALUES (:id, :name, :age)")
            .bind_named("id", 2)
            .bind_named("name", "Bob".to_string())
            .bind_named("age", 30),
    )
    .await?;

    let rows: Vec<UserName> = db
        .select_sql::<UserName>(
            ormer::sql(
                "SELECT id, name FROM raw_sql_users_1 \
                 WHERE name = :name AND ':name' = ':name' -- :ignored\n\
                 ORDER BY id",
            )
            .bind_named("name", "Alice".to_string()),
        )
        .collect()
        .await?;

    assert_eq!(
        rows,
        vec![UserName {
            id: 1,
            name: "Alice".to_string()
        }]
    );

    let mut txn = db.begin().await?;
    txn.execute_sql(
        ormer::sql("UPDATE raw_sql_users_1 SET name = :name WHERE id = :id")
            .bind_named("name", "Carol".to_string())
            .bind_named("id", 2),
    )
    .await?;
    let txn_rows: Vec<UserName> = txn
        .select_sql::<UserName>(
            ormer::sql("SELECT id, name FROM raw_sql_users_1 WHERE id = :id").bind_named("id", 2),
        )
        .collect()
        .await?;
    assert_eq!(
        txn_rows,
        vec![UserName {
            id: 2,
            name: "Carol".to_string()
        }]
    );
    txn.rollback().await?;

    let _ = db.drop_table::<RawSqlUser>().execute().await;
    Ok(())
}

test_on_all_dbs_result!(test_raw_sql_binding_and_mapping_impl);
