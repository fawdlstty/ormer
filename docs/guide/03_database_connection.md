# 数据库连接

## 支持的数据库

- Turso/SQLite
- PostgreSQL
- MySQL

## 启用特性

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
```

## 连接字符串

**Turso/SQLite:**
- 内存: `:memory:`
- 文件: `file:test.db`
- 远程: `libsql://url.turso.io?authToken=token`

**PostgreSQL:**
- `postgresql://user:password@localhost/dbname`

**MySQL:**
- `mysql://user:password@localhost/dbname`

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
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```
