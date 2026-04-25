# 数据库连接

Ormer 支持多种数据库后端,提供统一的连接接口。

## 支持的数据库

- **Turso (libSQL/SQLite)** - 嵌入式数据库,适合开发和轻量级应用
- **PostgreSQL** - 功能强大的企业级关系型数据库
- **MySQL** - 流行的开源数据库

## 启用数据库特性

在 `Cargo.toml` 中启用需要的数据库特性:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
# 或
ormer = { version = "0.1", features = ["postgresql"] }
# 或
ormer = { version = "0.1", features = ["mysql"] }
# 或同时启用多个
ormer = { version = "0.1", features = ["turso", "postgresql", "mysql"] }
```

## 连接数据库

### 基本连接

使用 `Database::connect()` 方法连接数据库:

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

### 连接字符串格式

#### Turso/SQLite

```rust
// 内存数据库 (测试用)
"file::memory:"

// 文件数据库
"file:test.db"
"file:/path/to/database.db"

// 远程 Turso 数据库
"libsql://your-database-url.turso.io?authToken=your-token"
```

#### PostgreSQL

```rust
// 基本格式
"postgresql://user:password@localhost/dbname"

// 完整格式
"postgresql://user:password@host:port/dbname?sslmode=require"

// 示例
"postgresql://postgres:123456@localhost:5432/mydb"
```

#### MySQL

```rust
// 基本格式
"mysql://user:password@localhost/dbname"

// 完整格式
"mysql://user:password@host:port/dbname"

// 示例
"mysql://root:123456@localhost:3306/mydb"
```

## 完整示例

### Turso/SQLite 示例

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
    // 连接数据库
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // 创建表
    db.create_table::<User>().await?;
    
    // 使用数据库...
    
    // 清理
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

### PostgreSQL 示例

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
    // 连接 PostgreSQL
    let db = Database::connect(
        DbType::PostgreSQL,
        "postgresql://postgres:password@localhost/mydb"
    ).await?;
    
    // 创建表
    db.create_table::<User>().await?;
    
    // 使用数据库...
    
    Ok(())
}
```

### MySQL 示例

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
    // 连接 MySQL
    let db = Database::connect(
        DbType::MySQL,
        "mysql://root:password@localhost/mydb"
    ).await?;
    
    // 创建表
    db.create_table::<User>().await?;
    
    // 使用数据库...
    
    Ok(())
}
```

## 多数据库支持

如果你的应用需要连接多个数据库,可以创建多个 `Database` 实例:

```rust
use ormer::{Database, DbType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 主数据库 (PostgreSQL)
    let primary_db = Database::connect(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/primary_db"
    ).await?;
    
    // 分析数据库 (MySQL)
    let analytics_db = Database::connect(
        DbType::MySQL,
        "mysql://user:pass@localhost/analytics_db"
    ).await?;
    
    // 缓存数据库 (SQLite)
    let cache_db = Database::connect(
        DbType::Turso,
        "file:cache.db"
    ).await?;
    
    // 使用不同的数据库实例...
    
    Ok(())
}
```

## 连接池

对于生产环境,建议使用连接池来管理数据库连接:

```rust
use ormer::{Database, DbType, ConnectionPool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建连接池
    let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
        .max_size(10)  // 最大连接数
        .build()
        .await?;
    
    // 从连接池获取连接
    let conn = pool.get().await?;
    
    // 使用连接...
    
    Ok(())
}
```

详见 [连接池文档](08_connection_pool.md)。

## 错误处理

连接失败时会返回 `Error`:

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

常见错误:

- 连接字符串格式错误
- 数据库服务未启动
- 认证失败 (用户名/密码错误)
- 网络问题
- 数据库不存在

## 测试用内存数据库

在测试中使用内存数据库非常方便:

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
        // 使用内存数据库
        let db = Database::connect(DbType::Turso, "file::memory:").await.unwrap();
        
        db.create_table::<TestUser>().await.unwrap();
        
        // 测试逻辑...
    }
}
```

## 数据库特性检查

Ormer 在编译时会检查是否启用了至少一个数据库特性:

```
compile_error!("At least one database feature must be enabled: turso, postgresql, or mysql");
```

如果看到此错误,请确保在 `Cargo.toml` 中启用了至少一个数据库特性。

## 最佳实践

### 1. 使用环境变量管理连接字符串

```rust
use std::env;

let db_url = env::var("DATABASE_URL")
    .expect("DATABASE_URL must be set");

let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### 2. 在应用启动时建立连接

```rust
struct AppState {
    db: Database,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
    let state = AppState { db };
    
    // 使用 state...
    
    Ok(())
}
```

### 3. 生产环境使用连接池

```rust
// 推荐: 使用连接池
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

// 避免: 每次操作都创建新连接
let db = Database::connect(DbType::PostgreSQL, &db_url).await?;
```

### 4. 优雅关闭连接

```rust
// 连接会在超出作用域时自动关闭
{
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    // 使用数据库...
} // db 在这里被丢弃,连接关闭
```
