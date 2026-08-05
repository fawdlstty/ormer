# Connection Pool

## Create Pool

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .range(5..10)
    .build()
    .await?;
```

## Use Pool

```rust
let conn = pool.get().await?;

let users: Vec<User> = conn.select::<User>().collect().await?;
```

### Auto Management

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    conn.insert(&user).execute().await?;
    Ok(())
}
```

`PooledConnection` also supports `select_sql` and `execute_sql`, so you can run raw SQL directly from the pool:

```rust
let conn = pool.get().await?;
let users: Vec<User> = conn
    .select_sql::<User>(ormer::sql("SELECT * FROM users WHERE age >= {}").bind(18))
    .collect()
    .await?;
conn.execute_sql(
    ormer::sql("UPDATE users SET name = {} WHERE id = {}")
        .bind("Bob")
        .bind(1),
)
.await?;
```

## SQLite Backend Considerations

The SQLite (turso) backend, due to its embedded nature, does not officially support multi-threaded shared connections. Recommendations:

1. **Connection Pool Configuration**: Set `max_size=1` for a single connection pool
   ```rust
   let pool = Database::create_pool(DbType::Sqlite, "path/to/database.db")
       .range(0..1)  // Single connection recommended
       .build()
       .await?;
   ```

2. **Concurrent Scenarios**: For high concurrency read/write, consider enabling MVCC mode
   ```rust
   let conn = pool.get().await?;
   conn.execute_sql("PRAGMA journal_mode = 'mvcc'").await?;
   // Use BEGIN CONCURRENT for concurrent writes
   ```

3. **Transaction Handling**: Avoid holding connections for long periods, return them to the pool promptly
   ```rust
   {
       let conn = pool.get().await?;
       // Perform operations
       // conn is automatically returned when out of scope
   }
   ```

4. **Multi-Process Access**: SQLite does not support multiple processes accessing the same database file simultaneously. For multi-process scenarios, consider using PostgreSQL or MySQL

## Complete Example

```rust
use ormer::{Database, DbType, ConnectionPool, Model};
use std::sync::Arc;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/mydb"
    )
    .range(0..20)
    .build()
    .await?;
    
    let state = Arc::new(pool);
    
    // Concurrent requests
    let mut handles = vec![];
    for i in 0..10 {
        let state = state.clone();
        let handle = tokio::spawn(async move {
            let conn = state.get().await.unwrap();
            let users: Vec<User> = conn
                .select::<User>()
                .range(0..10)
                .collect()
                .await
                .unwrap();
            println!("Request {}: {} users", i, users.len());
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    Ok(())
}
```
