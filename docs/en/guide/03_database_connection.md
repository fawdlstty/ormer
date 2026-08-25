# Database Connection

## Supported Databases

- Sqlite
- PostgreSQL
- MySQL
- MSSQL
- DuckDB (SQL type mapping only; connection execution is not implemented yet)
- ClickHouse (SQL type mapping only; connection execution is not implemented yet)

## Enable Features

```toml
[dependencies]
ormer = { version = "0.2", features = ["sqlite"] }
```

## Connection Strings

**Sqlite:**
- Memory: `:memory:`
- File: `file:test.db`
- Remote: `libsql://url.Sqlite.io?authToken=token`

**PostgreSQL:**
- `postgresql://user:password@localhost/dbname`

**MySQL:**
- `mysql://user:password@localhost/dbname`

**MSSQL:**
- `mssql://user:password@localhost/dbname`

**DuckDB:**
- `app.duckdb`

**ClickHouse:**
- `http://localhost:8123?database=default`

The DuckDB and ClickHouse features are available for dialect/type generation.
Because a verified async connection, transaction, and streaming adapter is not
available yet, `Database::connect` returns `UnsupportedFeature` instead of
pretending that a connection was established.

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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```
