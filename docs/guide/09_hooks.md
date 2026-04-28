# 钩子系统 (Hooks)

## 概述

Ormer 提供了钩子（Hooks）系统，允许您在数据操作的关键生命周期节点自动执行自定义逻辑。钩子在插入、更新、删除操作前后触发，非常适合用于：

- 自动设置时间戳（created_at, updated_at）
- 数据验证和清洗
- 审计日志记录
- 缓存失效处理
- 关联数据同步

## 支持的钩子类型

Ormer 提供了 6 种钩子 trait：

| 钩子 Trait | 触发时机 | 方法签名 | 用途 |
|-----------|---------|---------|------|
| `BeforeInsert` | 插入数据前 | `async fn before_insert(&mut self)` | 设置初始值、验证数据 |
| `AfterInsert` | 插入数据后 | `async fn after_insert(&self)` | 记录日志、触发事件 |
| `BeforeUpdate` | 更新数据前 | `async fn before_update(&mut self)` | 更新修改时间、验证变更 |
| `AfterUpdate` | 更新数据后 | `async fn after_update(&self)` | 清除缓存、通知相关方 |
| `BeforeDelete` | 删除数据前 | `async fn before_delete(&self)` | 检查依赖、备份数据 |
| `AfterDelete` | 删除数据后 | `async fn after_delete(&self)` | 清理关联资源 |

## 使用示例

### 基本用法

```rust
use ormer::{Model, BeforeInsert, BeforeUpdate, AfterInsert};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

// 实现插入前钩子 - 自动设置时间戳
#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
    }
}

// 实现更新前钩子 - 自动更新修改时间
#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}

// 实现插入后钩子 - 记录日志
#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self) {
        println!("新用户已创建: {} ({})", self.name, self.email);
    }
}
```

### 使用钩子

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Turso, "mydb.db").await?;
    
    // 创建表
    db.create_table::<User>().execute().await?;
    
    // 插入数据 - BeforeInsert 和 AfterInsert 会自动触发
    let user = User {
        id: 0,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        created_at: Default::default(), // 会被钩子覆盖
        updated_at: Default::default(), // 会被钩子覆盖
    };
    
    db.insert(&user).execute().await?;
    
    Ok(())
}
```

## 完整示例：用户管理

```rust
use ormer::{Model, Database, DbType, BeforeInsert, BeforeUpdate, BeforeDelete, AfterDelete};
use std::sync::atomic::{AtomicUsize, Ordering};

static USER_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    username: String,
    email: String,
    status: String,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        // 验证邮箱格式
        if !self.email.contains('@') {
            panic!("Invalid email format");
        }
        
        // 设置默认状态
        self.status = "active".to_string();
        
        println!("准备创建用户: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        // 防止修改已禁用的用户
        if self.status == "disabled" {
            panic!("Cannot update disabled user");
        }
        
        println!("准备更新用户: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeDelete for User {
    async fn before_delete(&self) {
        // 检查是否有依赖数据
        println!("准备删除用户: {} (状态: {})", self.username, self.status);
    }
}

#[async_trait::async_trait]
impl AfterDelete for User {
    async fn after_delete(&self) {
        USER_COUNT.fetch_sub(1, Ordering::SeqCst);
        println!("用户已删除: {}", self.username);
    }
}
```

## 注意事项

### 1. 异步支持

所有钩子方法都是异步的（`async fn`），您可以在钩子中执行异步操作，如：
- 数据库查询
- 网络请求
- 文件 I/O

```rust
#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self) {
        // 发送欢迎邮件
        send_welcome_email(&self.email).await;
        
        // 记录审计日志
        audit_log("user_created", &self.id.to_string()).await;
    }
}
```

### 2. 错误处理

当前版本的钩子系统不会传播错误。如果钩子中发生 panic，会影响整个操作。建议在钩子中使用 `Result` 类型并妥善处理错误：

```rust
#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        if let Err(e) = self.validate() {
            // 记录错误而不是 panic
            eprintln!("Validation failed: {}", e);
            // 或者使用更优雅的错误处理方式
        }
    }
}
```

### 3. 性能考虑

- 钩子会增加额外的函数调用开销
- 批量操作时，钩子会为每条记录调用一次
- 避免在钩子中执行耗时操作

### 4. 自动触发机制

**当前状态**：钩子 traits 已定义并可正常实现，但自动触发机制（在执行器中自动调用钩子）由于 Rust 类型系统限制尚未完全实现。

**当前用法**：您可以手动调用钩子方法：

```rust
let mut user = User { /* ... */ };

// 手动调用钩子
user.before_insert().await;

// 然后执行数据库操作
db.insert(&user).execute().await?;
```

**未来计划**：后续版本将通过更复杂的泛型特化机制实现完全自动触发。

## 最佳实践

1. **保持钩子简洁**：钩子应该执行快速、轻量的操作
2. **避免副作用**：钩子不应该修改其他不相关的数据
3. **日志记录**：在钩子中添加适当的日志以便调试
4. **数据验证**：在 `Before*` 钩子中进行数据验证
5. **资源清理**：在 `After*` 钩子中清理缓存或发送通知

## 与 SeaORM 对比

| 特性 | Ormer | SeaORM |
|-----|-------|--------|
| BeforeInsert | ✅ | ✅ |
| AfterInsert | ✅ | ✅ |
| BeforeUpdate | ✅ | ✅ |
| AfterUpdate | ✅ | ✅ |
| BeforeDelete | ✅ | ✅ |
| AfterDelete | ✅ | ✅ |
| 异步支持 | ✅ | ✅ |
| 自动触发 | 🚧 开发中 | ✅ |

## 相关文档

- [CRUD 操作](04_crud_operations.md) - 了解基本的增删改查操作
- [模型定义](02_model_definition.md) - 学习如何定义数据模型
- [事务处理](07_transactions.md) - 在事务中使用钩子

