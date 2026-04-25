# Connection Pool

Connection pooling is an important mechanism for managing database connections, significantly improving application performance and resource utilization.

## What is a Connection Pool

A connection pool maintains a set of pre-created database connections that applications can reuse, avoiding the overhead of frequently creating and destroying connections.

### Advantages

- **Performance Improvement** - Avoids creating new connections for each operation
- **Resource Control** - Limits maximum connections to prevent database overload
- **Connection Reuse** - Automatically manages connection lifecycle
- **Fault Recovery** - Automatically detects and replaces failed connections

## Creating a Connection Pool

### Basic Usage

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)  // Maximum connections
    .build()
    .await?;
```

### Configuration Options

```rust
let pool = Database::create_pool(DbType::PostgreSQL, connection_string)
    .max_size(20)        // Maximum connections (default: 10)
    .min_size(5)         // Minimum idle connections (optional)
    .idle_timeout(300)   // Idle timeout (seconds) (optional)
    .build()
    .await?;
```

## Using Connection Pool

### Getting Connections

```rust
// Get connection from pool
let conn = pool.get().await?;

// Use connection
let users: Vec<User> = conn
    .select::<User>()
    .collect()
    .await?;

// Connection automatically returns to pool
```

### Automatic Connection Management

The connection pool automatically manages connections:

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    // Get connection
    let conn = pool.get().await?;
    
    // Execute operations
    conn.insert(&user).await?;
    
    // conn automatically returns to pool when out of scope
    Ok(())
}
```

## Complete Examples

### Web Application Example

```rust
use ormer::{Database, DbType, ConnectionPool, Model};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

struct AppState {
    pool: ConnectionPool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create connection pool
    let pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/mydb"
    )
    .max_size(20)
    .build()
    .await?;
    
    let state = Arc::new(AppState { pool });
    
    // Simulate handling multiple requests
    let mut handles = vec![];
    
    for i in 0..10 {
        let state = state.clone();
        let handle = tokio::spawn(async move {
            // Get connection from pool
            let conn = state.pool.get().await.unwrap();
            
            // Execute database operations
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
    
    // Wait for all requests to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    Ok(())
}
```

### Connection Pool Configuration Example

```rust
use ormer::{Database, DbType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Development environment: Few connections
    let dev_pool = Database::create_pool(
        DbType::Turso,
        "file:dev.db"
    )
    .max_size(5)
    .build()
    .await?;
    
    // Production environment: More connections
    let prod_pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/prod_db"
    )
    .max_size(50)
    .min_size(10)
    .idle_timeout(600)  // 10 minutes
    .build()
    .await?;
    
    Ok(())
}
```

## Connection Pool Best Practices

### 1. Set Connection Count Reasonably

```rust
// ✅ Recommended: Set based on server configuration
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(num_cpus::get() * 4)  // 4 times CPU cores
    .build()
    .await?;

// ❌ Avoid: Connection count too large or too small
.max_size(1000)  // Too large, wastes resources
.max_size(1)     // Too small, poor performance
```

### 2. Use Connection Pool Instead of Single Connection

```rust
// ✅ Recommended: Use connection pool
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

async fn handle_request(pool: &ConnectionPool) {
    let conn = pool.get().await?;
    // Use conn...
}

// ❌ Avoid: Create new connection each time
async fn handle_request_bad(db_url: &str) {
    let db = Database::connect(DbType::PostgreSQL, db_url).await?;
    // Use db...
} // Connection is dropped
```

### 3. Release Connections Promptly

```rust
// ✅ Recommended: Connection automatically returns to pool after use
{
    let conn = pool.get().await?;
    conn.insert(&user).await?;
} // conn is released here

// ❌ Avoid: Holding connections for too long
let conn = pool.get().await?;
tokio::time::sleep(Duration::from_secs(60)).await;  // Bad!
conn.insert(&user).await?;
```

### 4. Error Handling

```rust
async fn safe_query(pool: &ConnectionPool) -> Result<Vec<User>, Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    
    let users = conn
        .select::<User>()
        .collect::<Vec<_>>()
        .await?;
    
    Ok(users)
}
```

## Monitoring Connection Pool

### Connection Pool Status

```rust
// Some connection pool implementations provide status monitoring
println!("Active connections: {}", pool.active_connections());
println!("Idle connections: {}", pool.idle_connections());
println!("Total connections: {}", pool.total_connections());
```

### Performance Metrics

Monitor the following metrics:

- **Wait Time** - Time waiting to get a connection
- **Active Connections** - Number of connections in use
- **Idle Connections** - Number of available connections
- **Connection Error Rate** - Number of connection failures

## Connection Pool Configuration for Different Databases

### Turso/SQLite

```rust
let pool = Database::create_pool(DbType::Turso, "file:test.db")
    .max_size(10)  // SQLite typically doesn't need many connections
    .build()
    .await?;
```

### PostgreSQL

```rust
let pool = Database::create_pool(
    DbType::PostgreSQL,
    "postgresql://user:pass@localhost/dbname"
)
.max_size(20)  // PostgreSQL supports many concurrent connections
.min_size(5)
.build()
.await?;
```

### MySQL

```rust
let pool = Database::create_pool(
    DbType::MySQL,
    "mysql://user:pass@localhost/dbname"
)
.max_size(20)
.min_size(5)
.build()
.await?;
```

## Common Issues

### 1. Connection Pool Exhaustion

When all connections are in use, new requests will wait:

```rust
// Connection pool is full, this will block until a connection is available
let conn = pool.get().await?;
```

**Solutions**:
- Increase `max_size`
- Optimize queries to reduce connection占用 time
- Use timeout mechanism

```rust
// Get connection with timeout
let conn = tokio::time::timeout(
    Duration::from_secs(5),
    pool.get()
).await??;
```

### 2. Connection Leaks

Forgetting to release connections leads to connection leaks:

```rust
// ❌ Wrong: Connection not released
let conn = pool.get().await?;
// Function returns without releasing conn

// ✅ Correct: Use scope
{
    let conn = pool.get().await?;
    // Use conn
} // conn automatically released
```

### 3. Connection Failures

Database restarts or network issues can cause connection failures:

```rust
// Connection pool typically handles this automatically
let conn = pool.get().await?;

// If connection fails, reacquire
match conn.select::<User>().collect().await {
    Ok(users) => println!("Got {} users", users.len()),
    Err(_) => {
        // Connection pool will automatically replace failed connection
        let conn = pool.get().await?;
        let users: Vec<User> = conn.select::<User>().collect().await?;
    }
}
```

## Performance Tuning

### 1. Adjust Connection Count Based on Load

```rust
// Low load application
.max_size(5)

// Medium load
.max_size(20)

// High load application
.max_size(50)
```

### 2. Set Reasonable Timeouts

```rust
// Connection acquisition timeout
.max_wait_time(5)  // 5 seconds

// Idle timeout
.idle_timeout(300)  // 5 minutes
```

### 3. Pre-create Connections

```rust
// Create minimum connections at startup
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .min_size(10)  // Pre-create 10 connections
    .build()
    .await?;
```
