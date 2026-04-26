# Connection Pool

## What is Connection Pool

Connection pool reuses database connections, avoiding frequent creation and destruction.

### Benefits

- Performance - avoid creating new connections
- Resource control - limit max connections
- Reuse - automatic lifecycle management
- Recovery - auto replace failed connections

## Create Pool

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)        // Max connections
    .min_size(5)         // Min idle (optional)
    .idle_timeout(300)   // Idle timeout seconds (optional)
    .build()
    .await?;
```

## Use Pool

```rust
// Get connection
let conn = pool.get().await?;

// Use
let users: Vec<User> = conn.select::<User>().collect().await?;

// Connection auto returns to pool
```

### Auto Management

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    conn.insert(&user).await?;
    // conn auto returns to pool when out of scope
    Ok(())
}
```

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
    .max_size(20)
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

## Configuration

### Development

```rust
.max_size(5)
```

### Production

```rust
.max_size(num_cpus::get() * 4)  // 4x CPU cores
.min_size(10)
.idle_timeout(600)
```

### Different Databases

```rust
// SQLite - few connections
.max_size(10)

// PostgreSQL/MySQL - more concurrent
.max_size(20)
.min_size(5)
```

## Best Practices

### Use Connection Pool

```rust
// ✅ Recommended
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

// ❌ Avoid - create new connection each time
let db = Database::connect(DbType::PostgreSQL, db_url).await?;
```

### Release Connections Promptly

```rust
// ✅ Recommended - use scope
{
    let conn = pool.get().await?;
    conn.insert(&user).await?;
} // auto release

// ❌ Avoid - hold too long
let conn = pool.get().await?;
tokio::time::sleep(Duration::from_secs(60)).await;
```

### Get with Timeout

```rust
let conn = tokio::time::timeout(
    Duration::from_secs(5),
    pool.get()
).await??;
```

## Common Issues

### Pool Exhausted

Increase `max_size` or optimize queries to reduce hold time.

### Connection Leak

Use scope to ensure connections auto release.

### Failed Connections

Pool auto replaces failed connections, just get a new one.

## Performance Tuning

- Adjust connections based on load (5-50)
- Set reasonable timeouts (5s get, 5min idle)
- Pre-create connections (`min_size`)
