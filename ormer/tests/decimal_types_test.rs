#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

use bigdecimal::BigDecimal;
use ormer::FromValue;
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "decimal_types_test_1"]
struct DecimalModel {
    #[primary]
    id: i32,
    price: Decimal,
    ratio: BigDecimal,
    optional_price: Option<Decimal>,
    optional_ratio: Option<BigDecimal>,
}

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

fn bigdecimal(value: &str) -> BigDecimal {
    BigDecimal::from_str(value).unwrap()
}

#[test]
fn decimal_sql_types_match_backend_mappings() {
    #[cfg(feature = "sqlite")]
    {
        let sql = ormer::generate_create_table_sql::<DecimalModel>(ormer::DbType::Sqlite).unwrap();
        assert!(sql.contains("price TEXT NOT NULL"));
        assert!(sql.contains("ratio TEXT NOT NULL"));
        assert!(sql.contains("optional_price TEXT"));
        assert!(sql.contains("optional_ratio TEXT"));
    }

    #[cfg(feature = "postgresql")]
    {
        let sql =
            ormer::generate_create_table_sql::<DecimalModel>(ormer::DbType::PostgreSQL).unwrap();
        assert!(sql.contains("price NUMERIC NOT NULL"));
        assert!(sql.contains("ratio NUMERIC NOT NULL"));
        assert!(sql.contains("optional_price NUMERIC"));
        assert!(sql.contains("optional_ratio NUMERIC"));
    }

    #[cfg(feature = "mysql")]
    {
        let sql = ormer::generate_create_table_sql::<DecimalModel>(ormer::DbType::MySQL).unwrap();
        assert!(sql.contains("price DECIMAL(65,30) NOT NULL"));
        assert!(sql.contains("ratio DECIMAL(65,30) NOT NULL"));
        assert!(sql.contains("optional_price DECIMAL(65,30)"));
        assert!(sql.contains("optional_ratio DECIMAL(65,30)"));
    }

    #[cfg(feature = "mssql")]
    {
        let sql = ormer::generate_create_table_sql::<DecimalModel>(ormer::DbType::MSSQL).unwrap();
        assert!(sql.contains("price DECIMAL(38,18) NOT NULL"));
        assert!(sql.contains("ratio DECIMAL(38,18) NOT NULL"));
        assert!(sql.contains("optional_price DECIMAL(38,18)"));
        assert!(sql.contains("optional_ratio DECIMAL(38,18)"));
    }
}

#[test]
fn decimal_values_roundtrip_without_float_loss() {
    let price = decimal("1234567890.123456789012345678");
    let price_value = ormer::Value::from(price);
    let decoded_price = Decimal::from_value(&price_value).unwrap();
    assert_eq!(decoded_price, price);

    let ratio = bigdecimal("0.123456789012345678901234567890123456789");
    let ratio_value = ormer::Value::from(ratio.clone());
    let decoded_ratio = BigDecimal::from_value(&ratio_value).unwrap();
    assert_eq!(decoded_ratio, ratio);
}

#[test]
fn decimal_filter_builds_typed_params() {
    let (sql, params) = ormer::Select::<DecimalModel>::new()
        .filter(|m| m.price.ge(decimal("10.00")))
        .filter(|m| m.ratio.le(bigdecimal("99.99")))
        .to_sql_with_params(ormer::DbType::Sqlite);

    assert!(sql.contains("price"));
    assert!(sql.contains("ratio"));
    assert!(matches!(
        params.as_slice(),
        [
            ormer::Value::Decimal(price),
            ormer::Value::BigDecimal(ratio),
        ] if price == "10.00" && ratio == "99.99"
    ));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_decimal_roundtrip_and_filter() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<DecimalModel>().execute().await?;

    let first = DecimalModel {
        id: 1,
        price: decimal("10.01"),
        ratio: bigdecimal("0.333333333333333333"),
        optional_price: Some(decimal("20.02")),
        optional_ratio: Some(bigdecimal("0.666666666666666666")),
    };
    let second = DecimalModel {
        id: 2,
        price: decimal("9.99"),
        ratio: bigdecimal("0.111111111111111111"),
        optional_price: None,
        optional_ratio: None,
    };

    db.insert(vec![first.clone(), second.clone()])
        .execute()
        .await?;

    let rows: Vec<DecimalModel> = db
        .select::<DecimalModel>()
        .filter(|m| m.price.ge(decimal("10.00")))
        .collect()
        .await?;

    assert_eq!(rows, vec![first]);
    Ok(())
}
