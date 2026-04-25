# Database Connection

Ormer supports multiple database backends with a unified connection interface.

## Supported Databases

- **Turso (libSQL/SQLite)** - Embedded database, suitable for development and lightweight applications
- **PostgreSQL** - Powerful enterprise-grade relational database
- **MySQL** - Popular open-source database

## Enabling Database Features

Enable the required database features in `Cargo.toml`:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
# or
ormer = { version = "0.1", features = ["postgresql"] }
# or
ormer = { version = "0.1", features = ["mysql"] }
# or enable multiple simultaneously
ormer = { version = "0.1", features = ["turso", "postgresql", "mysql"] }
```

## Connecting to Database

### Basic Connection

Use the `Database::connect()` method to connect to a database:

```rust
use ormer::{Database, DbType};

// Turso/SQLite
let db = Database::connect(DbType::Turso, "file:test.db").await?;

// PostgreSQL
let db = Database::connect(
    DbType::PostgreSQL,
    "postgresql://user:password@localhost/dbname"
).await?;

// MySQL
let db = Database::connect(
    DbType::MySQL,
    "mysql://user:password@localhost/dbname"
).await?;
```

### Connection String Formats

#### Turso/SQLite

```rust
// In-memory database (for testing)
"file::memory:"

// File database
"file:test.db"
"file:/path/to/database.db"

// Remote Turso database
"libsql://your-database-url.turso.io?authToken=your-token"
```

#### PostgreSQL

```rust
// Basic format
"postgresql://user:password@localhost/dbname"

// Full format
"postgresql://user:password@host:port/dbname?sslmode=require"

// Example
"postgresql://postgres:123456@localhost:5432/mydb"
```

#### MySQL

```rust
// Basic format
"mysql://user:password@localhost/dbname"

// Full format
"mysql://user:password@host:port/dbname"

// Example
"mysql://root:123456@localhost:3306/mydb"
```

## Complete Examples

### Turso/SQLite Example

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
    // Connect to database
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // Create table
    db.create_table::<User>().await?;
    
    // Use database...
    
    // Cleanup
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

### PostgreSQL Example

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
    // Connect to PostgreSQL
    let db = Database::connect(
        DbType::PostgreSQL,
        "postgresql://postgres:password@localhost/mydb"
    ).await?;
    
    // Create table
    db.create_table::<User>().await?;
    
    // Use database...
    
    Ok(())
}
```

### MySQL Example

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
    // Connect to MySQL
    let db = Database::connect(
        DbType::MySQL,
        "mysql://root:password@localhost/mydb"
    ).await?;
    
    // Create table
    db.create_table::<User>().await?;
    
    // Use database...
    
    Ok(())
}
```

## Multiple Database Support

If your application needs to connect to multiple databases, you can create multiple `Database` instances:

```rust
use ormer::{Database, DbType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Primary database (PostgreSQL)
    let primary_db = Database::connect(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/primary_db"
    ).await?;
    
    // Analytics database (MySQL)
    let analytics_db = Database::connect(
        DbType::MySQL,
        "mysql://user:pass@localhost/analytics_db"
    ).await?;
    
    // Cache database (SQLite)
    let cache_db = Database::connect(
        DbType::Turso,
        "file:cache.db"
    ).await?;
    
    // Use different database instances...
    
    Ok(())
}
```

## Connection Pool

For production environments, it's recommended to use a connection pool to manage database connections:

```rust
use ormer::{Database, DbType, ConnectionPool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create connection pool
    let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
        .max_size(10)  // Maximum connections
        .build()
        .await?;
    
    // Get connection from pool
    let conn = pool.get().await?;
    
    // Use connection...
    
    Ok(())
}
```

See [Connection Pool Documentation](08_connection_pool.md) for details.

## Error Handling

Connection failures return an `Error`:

```rust
use ormer::{Database, DbType, Error};

async fn connect_db() -> Result<(), Error> {
    let db = Database::connect(DbType::PostgreSQL, "invalid-url")
        .await
        .map_err(|e| {
            eprintln!("Failed to connect: {}", e);
            e
        })?;
    
    Ok(())
}
```

Common errors:

- Invalid connection string format
- Database service not running
- Authentication failure (wrong username/password)
- Network issues
- Database does not exist

## In-Memory Database for Testing

Using an in-memory database for testing is very convenient:

```rust
#[cfg(test)]
mod tests {
    use ormer::{Database, DbType, Model};
    
    #[derive(Debug, Model)]
    #[table = "test_users"]
    struct TestUser {
        #[primary(auto)]
        id: i32,
        name: String,
    }
    
    #[tokio::test]
    async fn test_user_operations() {
        // Use in-memory database
        let db = Database::connect(DbType::Turso, "file::memory:").await.unwrap();
        
        db.create_table::<TestUser>().await.unwrap();
        
        // Test logic...
    }
}
```

## Database Feature Check

Ormer checks at compile time whether at least one database feature is enabled:

```
compile_error!("At least one database feature must be enabled: turso, postgresql, or mysql");
```

If you see this error, make sure you have enabled at least one database feature in `Cargo.toml`.

## Best Practices

### 1. Use Environment Variables for Connection Strings

```rust
use std::env;

let db_url = env::var("DATABASE_URL")
    .expect("DATABASE_URL must be set");

let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### 2. Establish Connection at Application Startup

```rust
struct AppState {
    db: Database,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
    let state = AppState { db };
    
    // Use state...
    
    Ok(())
}
```

### 3. Use Connection Pool in Production

```rust
// Recommended: Use connection pool
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

// Avoid: Creating new connection for each operation
let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### 4. Graceful Connection Closure

```rust
// Connection automatically closes when it goes out of scope
{
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    // Use database...
} // db is dropped here, connection closes
```
