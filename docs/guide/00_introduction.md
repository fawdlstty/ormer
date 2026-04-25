# Ormer 简介

Ormer 是一个极简、高性能的 Rust ORM 框架,专注于提供类型安全、编译期优化的数据库操作体验。

## 核心特性

### 🎯 类型安全
- 编译期类型检查,避免运行时错误
- 字段级别的类型推断
- 强类型查询构建器

### ⚡ 高性能
- 基于 Rust 异步运行时实现
- 零成本抽象,无运行时开销
- 支持连接池管理

### 🔧 多数据库支持
- **Turso (libSQL/SQLite)** - 嵌入式数据库
- **PostgreSQL** - 企业级关系型数据库
- **MySQL** - 流行的开源数据库

### 📝 优雅的 API 设计
- 链式查询构建
- 宏驱动的模型定义
- 直观的过滤和排序语法

### 🔍 丰富的查询能力
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

## 设计理念

Ormer 的设计哲学是**"简单但不简陋"**:

1. **编译期优化**:尽可能在编译期捕获错误,而非运行时
2. **类型安全**:利用 Rust 类型系统确保数据操作的正确性
3. **极简 API**:用最少的代码完成最常见的操作
4. **可扩展性**:通过特征 (trait) 系统支持自定义扩展

## 适用场景

- Web 应用后端开发
- 数据密集型应用
- 微服务架构
- 需要高性能数据库操作的场景
- 对类型安全有严格要求的项目


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
    db.create_table::<User>().await?;
    
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
