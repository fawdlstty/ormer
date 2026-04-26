# 快速开始

## 环境要求

- Rust 1.70+
- Cargo

## 安装

### 添加依赖

在 `Cargo.toml` 中添加 Ormer 依赖和异步运行时:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
tokio = { version = "1", features = ["full"] }
```

**数据库特性:**
- `turso` - Turso/libSQL/SQLite
- `postgresql` - PostgreSQL
- `mysql` - MySQL



## 完整示例

```rust
use ormer::{Database, DbType, Model};

// 1. 定义模型
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 2. 连接数据库
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // 3. 创建表
    db.create_table::<User>().execute().await?;
    
    // 4. 插入数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    db.insert(&User {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    }).await?;
    
    // 5. 查询数据
    let users: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name.asc())
        .collect::<Vec<_>>()
        .await?;
    
    // 6. 处理结果
    for user in &users {
        println!("User: {} (age: {})", user.name, user.age);
    }
    
    // 7. 清理
    db.drop_table::<User>().execute().await?;
    
    Ok(())
}
```

运行: `cargo run`

## 核心操作速览

### 模型定义

使用 `#[derive(Model)]` 宏定义数据模型:

```rust
#[derive(Debug, Model)]
#[table = "表名"]
struct ModelName {
    #[primary(auto)]      // 主键,自动递增
    id: i32,
    
    #[unique]             // 唯一约束
    name: String,
    
    #[index]              // 索引
    age: i32,
    
    #[unique(group = 1)]  // 联合唯一约束
    field1: String,
    
    #[unique(group = 1)]
    field2: String,
    
    nullable_field: Option<String>,  // 可空字段
}
```

### 数据库连接

```rust
// Turso/SQLite
let db = Database::connect(DbType::Turso, "file:test.db").await?;

// PostgreSQL
let db = Database::connect(
    DbType::PostgreSQL, 
    "postgresql://user:pass@localhost/dbname"
).await?;

// MySQL
let db = Database::connect(
    DbType::MySQL, 
    "mysql://user:pass@localhost/dbname"
).await?;
```

### 插入数据

```rust
// 单条插入
db.insert(&user).await?;

// 批量插入 (Vec)
db.insert(&vec![user1, user2, user3]).await?;

// 批量插入 (数组)
db.insert(&[user1, user2]).await?;

// 插入或更新
db.insert_or_update(&user).await?;
```

### 查询数据

```rust
// 查询所有
let all: Vec<User> = db.select::<User>().collect().await?;

// 带条件查询
let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

// 排序和分页
let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;
```

### 更新数据

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

println!("Updated {} rows", count);
```

### 删除数据

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

println!("Deleted {} rows", count);
```

### 聚合查询

```rust
// COUNT
let count: usize = db.select::<User>().count(|u| u.id).await?;

// SUM
let sum: Option<i32> = db.select::<User>().sum(|u| u.age).await?;

// AVG
let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

// MAX
let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

// MIN
let min: Option<i32> = db.select::<User>().min(|u| u.age).await?;
```
