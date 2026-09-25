#![cfg(feature = "sqlite")]

use std::collections::HashMap;

use ormer::{DbType, Model, Value, generate_create_table_sql};

#[derive(Debug, Clone, PartialEq, ormer::DbValue)]
#[db_type(
    sqlite = "TEXT",
    postgresql = "CIDR",
    mysql = "VARCHAR(64)",
    mssql = "NVARCHAR(64)"
)]
struct Cidr(String);

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "db_value_networks"]
struct Network {
    #[primary]
    id: i32,
    cidr: Cidr,
}

#[test]
fn db_value_schema_and_value_conversions_work() -> ormer::Result<()> {
    let cidr_column = <Network as Model>::COLUMN_SCHEMA
        .iter()
        .find(|column| column.name == "cidr")
        .expect("cidr column exists");
    let db_value_type = cidr_column.db_value_type.expect("DbValue type metadata");
    assert_eq!(db_value_type(DbType::Sqlite), "TEXT");

    let sql = generate_create_table_sql::<Network>(DbType::Sqlite)?;
    assert!(sql.contains("cidr TEXT NOT NULL"), "SQL: {sql}");

    let model = Network {
        id: 1,
        cidr: Cidr("10.0.0.0/8".to_string()),
    };
    let values = model.field_values();
    assert_text_param(&values[1], "10.0.0.0/8");

    let raw: Value = Cidr("192.168.0.0/16".to_string()).into();
    assert_eq!(
        <Cidr as ormer::FromValue>::from_value(&raw)?,
        Cidr("192.168.0.0/16".to_string())
    );

    let from_values = <Network as Model>::from_row_values(&[
        Value::Integer(2),
        Value::Text("10.1.0.0/16".to_string()),
    ])?;
    assert_eq!(from_values.cidr, Cidr("10.1.0.0/16".to_string()));

    let row = ormer::Row::new(HashMap::from([
        ("id".to_string(), Value::Integer(3)),
        ("cidr".to_string(), Value::Text("10.2.0.0/16".to_string())),
    ]));
    let from_row = <Network as Model>::from_row(&row)?;
    assert_eq!(from_row.cidr, Cidr("10.2.0.0/16".to_string()));

    Ok(())
}

#[test]
fn db_value_can_be_used_in_typed_filters_and_raw_params() {
    let (sql, params) = ormer::Select::<Network>::new()
        .filter(|n| n.cidr.eq(Cidr("10.0.0.0/8".to_string())))
        .to_sql_with_params(DbType::Sqlite);

    assert!(sql.contains("cidr = ?"), "SQL: {sql}");
    assert_text_param(&params[0], "10.0.0.0/8");

    let (raw_sql, raw_params) = ormer::sql("SELECT * FROM networks WHERE cidr = {}")
        .bind(Cidr("10.0.0.0/8".to_string()))
        .render(DbType::Sqlite)
        .expect("raw SQL renders");
    assert_eq!(raw_sql, "SELECT * FROM networks WHERE cidr = ?");
    assert_text_param(&raw_params[0], "10.0.0.0/8");
}

fn assert_text_param(value: &Value, expected: &str) {
    match value {
        Value::Text(value) => assert_eq!(value, expected),
        other => panic!("expected text parameter, got {other:?}"),
    }
}
