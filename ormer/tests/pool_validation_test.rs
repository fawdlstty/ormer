#![cfg(feature = "sqlite")]

use ormer::{ConnectionPool, Database, DbType, OrmerError};

fn assert_invalid_pool_range(result: Result<(), OrmerError>, expected: &str) {
    match result {
        Ok(()) => panic!("invalid pool range was accepted"),
        Err(OrmerError::InvalidOperation { message }) => {
            assert!(message.contains(expected), "unexpected error: {message}");
        }
        Err(error) => panic!("unexpected error variant: {error}"),
    }
}

#[tokio::test]
async fn pool_builder_rejects_invalid_ranges() {
    for (range, expected) in [(0..0, "max_size"), (2..1, "min_size")] {
        let result = Database::create_pool(DbType::Sqlite, ":memory:")
            .range(range)
            .build()
            .await
            .map(|_| ());
        assert_invalid_pool_range(result, expected);
    }
}

#[tokio::test]
async fn replicated_pool_builder_rejects_invalid_ranges() {
    for (range, expected) in [(0..0, "max_size"), (2..1, "min_size")] {
        let result = ConnectionPool::replicated(DbType::Sqlite)
            .write(":memory:")
            .range(range)
            .connect()
            .await
            .map(|_| ());
        assert_invalid_pool_range(result, expected);
    }
}
