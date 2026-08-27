# 数据库连接

## 支持的数据库

- Sqlite
- PostgreSQL
- MySQL
- MSSQL
- DuckDB（支持本地连接、建表、CRUD、事务、原生 SQL、流式查询、schema introspection 和连接池）
- ClickHouse（支持 HTTP 客户端、统一原生 SQL、类型化原生查询、健康检查、删表、schema introspection 和迁移）

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
ClickHouse 同样通过统一的 `Database::connect(DbType::ClickHouse, "...")` 使用。
原生操作使用 `execute_sql` 和 `select_sql<T>`；事务、关系写入、行式更新、冲突写入和没有 engine 元数据的 `create_table::<T>()` 会返回 `UnsupportedFeature`。
ClickHouse 建表必须显式指定 engine，例如 `MergeTree ORDER BY (id)`；需要 engine 的 DDL 请使用 `execute_sql(ormer::sql(...))` 或 `MigrationStep::Sql`。
ClickHouse DDL 不支持事务，迁移步骤逐条执行；中途失败时已执行的步骤不会自动回滚。

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

ClickHouse 统一原生 SQL 示例：

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
