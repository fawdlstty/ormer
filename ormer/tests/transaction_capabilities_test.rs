#![cfg(any(feature = "duckdb", feature = "clickhouse"))]

#[cfg(any(feature = "duckdb", feature = "clickhouse"))]
use ormer::{DbType, IsolationLevel, OrmerError, TransactionOptions};

#[tokio::test]
#[cfg(feature = "duckdb")]
async fn duckdb_transaction_options_are_capability_gated() -> Result<(), Box<dyn std::error::Error>>
{
    let db = ormer::Database::connect(DbType::DuckDB, ":memory:").await?;

    for options in [
        TransactionOptions::new().isolation(IsolationLevel::Serializable),
        TransactionOptions::new().read_only(),
    ] {
        let error = db
            .transaction_opts(options, |_txn| Box::pin(async { Ok(()) }))
            .await
            .expect_err("DuckDB transaction options must be capability gated");
        assert!(matches!(
            error,
            OrmerError::UnsupportedFeature {
                backend: DbType::DuckDB,
                feature: "transaction options on DuckDB",
            }
        ));
    }

    db.transaction(|_txn| Box::pin(async { Ok(()) })).await?;
    Ok(())
}

#[tokio::test]
#[cfg(feature = "clickhouse")]
async fn clickhouse_transactions_are_capability_gated() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::ClickHouse, "http://localhost:8123").await?;

    let error = db
        .transaction(|_txn| Box::pin(async { Ok(()) }))
        .await
        .expect_err("ClickHouse transactions must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "transactions on ClickHouse",
        }
    ));

    let error = db
        .transaction_opts(TransactionOptions::new().read_only(), |_txn| {
            Box::pin(async { Ok(()) })
        })
        .await
        .expect_err("ClickHouse transaction_opts must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "transactions on ClickHouse",
        }
    ));

    let pool = ormer::Database::create_pool(DbType::ClickHouse, "http://localhost:8123")
        .range(0..1)
        .build()
        .await?;
    let connection = pool.get().await?;
    let error = connection
        .transaction(|_txn| Box::pin(async { Ok(()) }))
        .await
        .expect_err("ClickHouse pooled transactions must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "transactions on ClickHouse",
        }
    ));

    let error = connection
        .transaction_opts(TransactionOptions::new().read_only(), |_txn| {
            Box::pin(async { Ok(()) })
        })
        .await
        .expect_err("ClickHouse pooled transaction_opts must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::ClickHouse,
            feature: "transactions on ClickHouse",
        }
    ));

    Ok(())
}
