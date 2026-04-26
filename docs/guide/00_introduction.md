# Ormer 简介

Ormer 是极简、高性能的 Rust ORM 框架，提供类型安全的数据库操作。

## 核心特性

- **类型安全**: 编译期类型检查，强类型查询构建器
- **高性能**: 零成本抽象，异步运行时，连接池支持
- **多数据库**: Turso/SQLite、PostgreSQL、MySQL
- **优雅 API**: 链式查询，宏驱动模型，直观语法

## 查询能力
- 基础 CRUD 操作
- 复杂过滤条件 (比较、IN、LIKE 等)
- 聚合查询 (COUNT, SUM, AVG, MAX, MIN)
- 字段投影 (map_to)
- JOIN 查询 (LEFT, INNER, RIGHT)
- 多表关联查询 (2-4 表)
- 子查询支持
- 分页查询 (LIMIT/OFFSET)

### 💾 事务支持
- ACID 事务保证
- 事务内查询、插入、更新、删除
- 提交和回滚控制






## 快速预览

```rust
use ormer::{Database, DbType, Model};

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
    // 连接数据库
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // 创建表
    db.create_table::<User>().execute().await?;
    
    // 插入数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    // 查询数据
    let users: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name.asc())
        .range(0..10)
        .collect::<Vec<_>>()
        .await?;
    
    println!("Found {} users", users.len());
    
    Ok(())
}
```
