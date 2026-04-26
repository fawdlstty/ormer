# Database Connection

## Supported Databases

- Turso/SQLite - Embedded database
- PostgreSQL - Enterprise database
- MySQL - Open-source database

## Enable Features

Enable in `Cargo.toml`:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
# or postgresql, mysql, or multiple
```

## Connection

### Connection Strings

**Turso/SQLite:**
- Memory: `:memory:`
- File: `file:test.db`
- Remote: `libsql://url.turso.io?authToken=token`

**PostgreSQL:**
- Basic: `postgresql://user:password@localhost/dbname`
- Full: `postgresql://user:password@host:port/dbname?sslmode=require`

**MySQL:**
- Basic: `mysql://user:password@localhost/dbname`
- Full: `mysql://user:password@host:port/dbname`

### Example

```rust
use ormer::{Database, DbType, Model};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Turso/SQLite
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // PostgreSQL
    // let db = Database::connect(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname").await?;
    
    // MySQL
    // let db = Database::connect(DbType::MySQL, "mysql://user:pass@localhost/dbname").await?;
    
    db.create_table::<User>().execute().await?;
    db.drop_table::<User>().execute().await?;
    
    Ok(())
}
```

## Multiple Databases

Create multiple `Database` instances for different databases.

## Connection Pool

For production, use connection pool. See [Connection Pool Docs](08_connection_pool.md).

## Testing

Use in-memory database:

```rust
let db = Database::connect(DbType::Turso, ":memory:").await?;
```

## Best Practices

### Environment Variables

```rust
let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### Connection Pool

Use connection pool in production.

### Auto Close

Connections close automatically when out of scope.
