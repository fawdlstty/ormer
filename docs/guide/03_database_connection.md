# 数据库连接

## 支持的数据库

- Turso/SQLite - 嵌入式数据库
- PostgreSQL - 企业级数据库
- MySQL - 开源数据库

## 启用特性

在 `Cargo.toml` 中启用:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
# 或 postgresql, mysql，或多个
```

## 连接

### 连接字符串

**Turso/SQLite:**
- 内存: `:memory:`
- 文件: `file:test.db`
- 远程: `libsql://url.turso.io?authToken=token`

**PostgreSQL:**
- 基本: `postgresql://user:password@localhost/dbname`
- 完整: `postgresql://user:password@host:port/dbname?sslmode=require`

**MySQL:**
- 基本: `mysql://user:password@localhost/dbname`
- 完整: `mysql://user:password@host:port/dbname`

### 示例

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

## 多数据库

可创建多个 `Database` 实例连接不同数据库。

## 连接池

生产环境使用连接池，详见 [连接池文档](08_connection_pool.md)。

## 测试

使用内存数据库:

```rust
let db = Database::connect(DbType::Turso, ":memory:").await?;
```

## 最佳实践

### 环境变量

```rust
let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### 连接池

生产环境使用连接池。

### 自动关闭

连接超出作用域时自动关闭。
