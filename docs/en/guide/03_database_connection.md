# Database Connection

## Supported Databases

- Sqlite
- PostgreSQL
- MySQL
- MSSQL
- DuckDB (local connections, CRUD, transactions, raw SQL, streams, schema introspection, and connection pools)
- ClickHouse (HTTP client, raw SQL, JSON queries, typed and streaming queries, health checks, engine-aware table creation, table drops, and schema introspection)

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
- Optional query parameters include `user`, `password`, `access_token`,
  `compression=none|lz4`, and other ClickHouse settings such as
  `max_execution_time=3`.

DuckDB can be used through `Database::connect(DbType::DuckDB, "app.duckdb")`.
`Vec<i32>`, `Vec<i64>`, `Vec<Option<i64>>`, and `Vec<String>` fields map to
DuckDB lists.
ClickHouse uses the dedicated `ClickHouseDatabase` because the ClickHouse HTTP
interface does not provide the transaction and dynamic `FromRowValues` row
decoding contract required by the unified ORM executors. `ClickHouseDatabase`
supports `execute_sql`, `select_json`, `select`, `select_one`, `select_optional`,
`select_stream`, `select_json_stream`, `insert_rows`, `is_valid`,
`create_table::<T>(engine)`, `drop_table::<T>()`, `generate_entities(schema)`, and native versioned migration
methods `migration_history`, `pending_migrations`, and `apply_migrations`. `select`,
`select_one`, and `insert_rows` use static row types derived with
`ormer::clickhouse::Row` and Serde. Add `serde = { version = "1", features = ["derive"] }`
to the application dependencies. Every ClickHouse table must specify an engine such
as `MergeTree ORDER BY (id)`. ClickHouse DDL is not transactional, so native
migrations execute one step at a time and do not automatically roll back earlier
steps when a later step fails.

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

ClickHouse native typed rows:

```rust
#[derive(ormer::clickhouse::Row, serde::Serialize, serde::Deserialize)]
struct Event {
    id: u64,
    name: String,
}

let events: Vec<Event> = db.select("SELECT id, name FROM events").await?;
db.insert_rows("events", [Event {
    id: 1,
    name: "created".to_string(),
}])
.await?;
```

ClickHouse native operation:

```rust
use ormer::ClickHouseDatabase;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = ClickHouseDatabase::connect("http://localhost:8123?database=default")?;
    db.execute_sql("SELECT 1").await?;
    let rows = db.select_json("SELECT 1 AS id").await?;
    assert_eq!(rows[0]["id"], 1);
    Ok(())
}
```
