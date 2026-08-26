# 数据库连接

## 支持的数据库

- Sqlite
- PostgreSQL
- MySQL
- MSSQL
- DuckDB（支持本地连接、建表、CRUD、事务、原生 SQL、流式查询、schema introspection 和连接池）
- ClickHouse（支持 HTTP 客户端、原生 SQL、JSON 查询、静态与流式查询、健康检查、带 engine 的建表和删表、schema introspection）

## 启用特性

```toml
[dependencies]
ormer = { version = "0.2", features = ["sqlite"] }
```

## 连接字符串

**Sqlite:**
- 内存: `:memory:`
- 文件: `file:test.db`
- 远程: `libsql://url.Sqlite.io?authToken=token`

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
- 可选 query 参数：`user`、`password`、`access_token`、`compression=none|lz4`
  以及其他 ClickHouse settings（例如 `max_execution_time=3`）。

DuckDB 可通过统一的 `Database::connect(DbType::DuckDB, "app.duckdb")` 使用。
`Vec<i32>`、`Vec<i64>`、`Vec<Option<i64>>` 和 `Vec<String>` 字段会映射为 DuckDB list。
ClickHouse 使用专用的 `ClickHouseDatabase`，因为 ClickHouse HTTP 接口不提供
当前统一 ORM executor 所需的事务和动态 `FromRowValues` 行解码契约。
`ClickHouseDatabase` 支持 `execute_sql`、`select_json`、`select`、`select_one`、
`select_optional`、`select_stream`、`select_json_stream`、`insert_rows`、`is_valid`、
`create_table::<T>(engine)`、`drop_table::<T>()`、`generate_entities(schema)`，以及非事务版本迁移
`migration_history`、`pending_migrations` 和 `apply_migrations`；
`select`、`select_one` 与 `insert_rows` 使用 `ormer::clickhouse::Row` 和 Serde 派生的静态行类型，
应用依赖中还需要添加 `serde = { version = "1", features = ["derive"] }`。
建表时必须显式指定 engine，例如 `MergeTree ORDER BY (id)`。
ClickHouse DDL 不支持事务，原生迁移逐条执行；中途失败时已执行的步骤不会自动回滚。

## 示例

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

ClickHouse 原生类型行示例：

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

ClickHouse 原生操作示例：

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
