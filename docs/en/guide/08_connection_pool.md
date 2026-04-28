# Connection Pool

## Create Pool

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)
    .min_size(5)
    .idle_timeout(300)
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
