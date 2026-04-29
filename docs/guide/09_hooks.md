# 钩子系统 (Hooks)

## 支持的钩子类型

| 钩子 Trait | 触发时机 | 方法签名 |
|-----------|---------|---------|
| `BeforeInsert` | 插入数据前 | `async fn before_insert(&mut self)` |
| `AfterInsert` | 插入数据后 | `async fn after_insert(&self)` |
| `BeforeUpdate` | 更新数据前 | `async fn before_update(&mut self)` |
| `AfterUpdate` | 更新数据后 | `async fn after_update(&self)` |
| `BeforeDelete` | 删除数据前 | `async fn before_delete(&self)` |
| `AfterDelete` | 删除数据后 | `async fn after_delete(&self)` |

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

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}

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
    let db = Database::connect(DbType::Sqlite, "mydb.db").await?;
    
    db.create_table::<User>().execute().await?;
    
    let user = User {
        id: 0,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        created_at: Default::default(),
        updated_at: Default::default(),
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
        if !self.email.contains('@') {
            panic!("Invalid email format");
        }
        
        self.status = "active".to_string();
        
        println!("准备创建用户: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        if self.status == "disabled" {
            panic!("Cannot update disabled user");
        }
        
        println!("准备更新用户: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeDelete for User {
    async fn before_delete(&self) {
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

所有钩子方法都是异步的（`async fn`），您可以在钩子中执行异步操作。

### 2. 错误处理

当前版本的钩子系统不会传播错误。如果钩子中发生 panic，会影响整个操作。

### 3. 性能考虑

- 钩子会增加额外的函数调用开销
- 批量操作时，钩子会为每条记录调用一次
- 避免在钩子中执行耗时操作

### 4. 自动触发机制

**当前状态**：钩子 traits 已定义并可正常实现，但自动触发机制（在执行器中自动调用钩子）由于 Rust 类型系统限制尚未完全实现。

**当前用法**：您可以手动调用钩子方法：

```rust
let mut user = User { /* ... */ };

user.before_insert().await;

db.insert(&user).execute().await?;
```

**未来计划**：后续版本将通过更复杂的泛型特化机制实现完全自动触发。

