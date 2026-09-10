# Database Connection

## Supported Databases

- Sqlite
- PostgreSQL
- QuestDB (PostgreSQL wire protocol)
- MySQL
- MSSQL
- DuckDB (local connections, CRUD, transactions, raw SQL, streams, schema introspection, and connection pools)
- ClickHouse (HTTP client, unified raw SQL, typed raw queries, health checks, table drops, schema introspection, and migrations)
- InfluxDB (HTTP client, Line Protocol writes, InfluxQL/SQL queries, time-range deletes, health checks, table drops, and migrations)

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

**QuestDB:**
- `postgresql://admin:quest@localhost:8812/qdb`
- Use `Database::connect(DbType::QuestDB, "...")` through the PostgreSQL wire protocol.
- Supports insert/select/update, create/drop table, raw SQL, streams, connection pools, and step-by-step migrations.
- A `#[hypertable(...)]` time field produces a QuestDB designated timestamp in `create_table`.
- Row-level deletes, transactions, constraints, auto-increment/`RETURNING`, upserts, COPY, row locks, db-first entity generation (`generate_entities`), and advanced grouping are unsupported.

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

**InfluxDB:**
- 2.x: `http://localhost:8086?org=dev&bucket=metrics&token=my-token`
- 1.x: `http://localhost:8086?database=telegraf&user=admin&password=secret`

DuckDB can be used through `Database::connect(DbType::DuckDB, "app.duckdb")`.
`Vec<i32>`, `Vec<i64>`, `Vec<Option<i64>>`, and `Vec<String>` fields map to
DuckDB lists.
ClickHouse is also used through `Database::connect(DbType::ClickHouse, "...")`.
Use `execute_sql` and `select_sql<T>` for native operations. Transactions,
relation writes, row updates, conflict writes, and `create_table::<T>()` without
engine metadata return `UnsupportedFeature`. Every ClickHouse table must specify
an engine such as `MergeTree ORDER BY (id)`. Declare `#[clickhouse(engine = ...)]`
on the model to call `create_table::<T>()` directly, or write the DDL with
`execute_sql(ormer::sql(...))` or `MigrationStep::Sql`. ClickHouse DDL is not
transactional, so migration steps execute one at a time and do not automatically
roll back earlier steps when a later step fails.
InfluxDB is used through `Database::connect(DbType::InfluxDB, "...")`.
`insert` renders Line Protocol batch writes and `select` reuses the unified query
builder to render an InfluxQL/SQL subset. Transactions, JOINs, relation loading,
row updates, upserts, `RETURNING`, and auto-increment return `UnsupportedFeature`;
deletes only support time ranges (`delete_blocks`).
The timestamp reuses `#[primary]` (exactly one time-typed field) and tags reuse
`#[index]` (must be `String`). Declare a retention policy with
`#[influxdb(retention = std::time::Duration::from_secs(30 * 86400))]`; `create_table`
then emits `CREATE RETENTION POLICY`.

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

ClickHouse unified raw SQL:

```rust
use ormer::{Database, DbType, ViewModel};

#[derive(Debug, ViewModel)]
struct EventStat {
    id: i64,
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(
        DbType::ClickHouse,
        "http://localhost:8123?database=default",
    )
    .await?;

    db.execute_sql(ormer::sql(
        "CREATE TABLE IF NOT EXISTS events (id Int64, name String) ENGINE = MergeTree ORDER BY (id)",
    ))
    .await?;
    db.execute_sql(ormer::sql(
        "INSERT INTO events (id, name) VALUES (1, 'created')",
    ))
    .await?;

    let rows = db
        .select_sql::<EventStat>(ormer::sql("SELECT id, name FROM events ORDER BY id"))
        .collect::<Vec<EventStat>>()
        .await?;
    assert_eq!(rows[0].name, "created");

    Ok(())
}
```
