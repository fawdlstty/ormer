#![cfg(feature = "sqlite")]

use ormer::{Database, DbType, Model, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static TRACE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Model)]
#[table = "sql_trace_users"]
struct SqlTraceUser {
    #[primary]
    id: i32,
    name: String,
}

fn is_trace_test_sql(sql: &str) -> bool {
    sql.contains("sql_trace_users") || sql.contains("missing_sql_trace_table")
}

#[tokio::test]
async fn sql_trace_records_callbacks_and_parameter_view() -> ormer::Result<()> {
    let _guard = TRACE_TEST_LOCK.lock().await;

    let db = Database::connect(DbType::Sqlite, ":memory:").await?;
    let before_sql = Arc::new(Mutex::new(Vec::<String>::new()));
    let after_sql = Arc::new(Mutex::new(Vec::<String>::new()));
    let error_sql = Arc::new(Mutex::new(Vec::<String>::new()));
    let slow_sql = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_params = Arc::new(Mutex::new(Vec::<Vec<Value>>::new()));

    db.sql_trace()
        .clear()
        .redact_params(|params| {
            params
                .iter()
                .map(|_| Value::Text("***".to_string()))
                .collect()
        })
        .rewrite(|sql| {
            if is_trace_test_sql(sql) {
                format!("{sql} /* traced */")
            } else {
                sql.to_string()
            }
        })
        .before_with({
            let before_sql = Arc::clone(&before_sql);
            let seen_params = Arc::clone(&seen_params);
            move |event| {
                if is_trace_test_sql(event.sql()) {
                    before_sql.lock().unwrap().push(event.sql().to_string());
                    seen_params.lock().unwrap().push(event.params().to_vec());
                }
            }
        })
        .after({
            let after_sql = Arc::clone(&after_sql);
            move |sql, _elapsed| {
                if is_trace_test_sql(sql) {
                    after_sql.lock().unwrap().push(sql.to_string());
                }
            }
        })
        .on_error({
            let error_sql = Arc::clone(&error_sql);
            move |sql, _err| {
                if is_trace_test_sql(sql) {
                    error_sql.lock().unwrap().push(sql.to_string());
                }
            }
        })
        .slow_sql_threshold(Duration::ZERO)
        .slow({
            let slow_sql = Arc::clone(&slow_sql);
            move |sql, _elapsed| {
                if is_trace_test_sql(sql) {
                    slow_sql.lock().unwrap().push(sql.to_string());
                }
            }
        });

    db.create_table::<SqlTraceUser>().execute().await?;
    db.insert(&SqlTraceUser {
        id: 1,
        name: "Alice".to_string(),
    })
    .execute()
    .await?;
    let users: Vec<SqlTraceUser> = db
        .select::<SqlTraceUser>()
        .filter(|user| user.name.eq("Alice".to_string()))
        .collect()
        .await?;

    assert_eq!(users.len(), 1);

    let err = db
        .execute_sql("INSERT INTO missing_sql_trace_table (id) VALUES (1)")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("missing_sql_trace_table"));

    db.sql_trace().clear();

    let before_sql = before_sql.lock().unwrap();
    let after_sql = after_sql.lock().unwrap();
    let error_sql = error_sql.lock().unwrap();
    let slow_sql = slow_sql.lock().unwrap();
    let seen_params = seen_params.lock().unwrap();

    assert!(before_sql.iter().any(|sql| sql.contains("CREATE TABLE")));
    assert!(before_sql.iter().any(|sql| sql.contains("INSERT INTO")));
    assert!(before_sql.iter().any(|sql| sql.contains("SELECT")));
    assert!(before_sql.iter().all(|sql| sql.contains("/* traced */")));
    assert!(after_sql.iter().any(|sql| sql.contains("SELECT")));
    assert_eq!(error_sql.len(), 1);
    assert!(error_sql[0].contains("missing_sql_trace_table"));
    assert!(!slow_sql.is_empty());
    assert!(
        seen_params
            .iter()
            .flatten()
            .any(|value| matches!(value, Value::Text(value) if value == "***"))
    );

    Ok(())
}
