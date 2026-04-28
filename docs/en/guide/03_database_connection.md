# Database Connection

## Supported Databases

- Turso/SQLite
- PostgreSQL
- MySQL

## Enable Features

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
```

## Connection Strings

**Turso/SQLite:**
- Memory: `:memory:`
- File: `file:test.db`
- Remote: `libsql://url.turso.io?authToken=token`

**PostgreSQL:**
- `postgresql://user:password@localhost/dbname`

**MySQL:**
- `mysql://user:password@localhost/dbname`

## Example

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
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```
