# 数据库连接

## 支持的数据库

- Sqlite
- PostgreSQL
- MySQL
- MSSQL
- DuckDB（当前仅提供 SQL 类型映射，连接执行暂未实现）
- ClickHouse（当前仅提供 SQL 类型映射，连接执行暂未实现）

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

DuckDB 与 ClickHouse feature 已预留并会参与 SQL 方言生成；由于当前版本尚未
完成可验证的异步连接、事务和结果流适配，调用 `Database::connect` 会返回
`UnsupportedFeature`，不会伪装成已连接。

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
