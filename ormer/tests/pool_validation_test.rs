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

#[tokio::test]
async fn explicit_return_releases_the_only_pool_connection() {
    let pool = Database::create_pool(DbType::Sqlite, ":memory:")
        .range(1..1)
        .build()
        .await
        .unwrap();

    pool.get()
        .await
        .unwrap()
        .return_()
        .await
        .expect("healthy connections should return");
    pool.get()
        .await
        .expect("returned capacity should be reusable");
}

#[tokio::test]
async fn explicit_close_releases_pool_capacity() {
    let pool = Database::create_pool(DbType::Sqlite, ":memory:")
        .range(1..1)
        .build()
        .await
        .unwrap();

    pool.get()
        .await
        .unwrap()
        .close()
        .await
        .expect("closed connections should retire their lease");
    pool.get()
        .await
        .expect("retired capacity should be replaceable");
}

#[test]
fn drop_outside_runtime_does_not_leak_pool_capacity() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let pool = runtime.block_on(async {
        Database::create_pool(DbType::Sqlite, ":memory:")
            .range(1..1)
            .build()
            .await
            .unwrap()
    });
    let conn = runtime.block_on(async { pool.get().await.unwrap() });
    drop(conn);
    drop(runtime);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime
        .block_on(async { pool.get().await })
        .expect("a connection dropped outside the runtime must not leak capacity");
}
