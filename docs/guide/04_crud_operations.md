# 数据操作

## 插入 (Create)

### 单条插入

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
})
.execute()
.await?;
```

> `execute()` 返回自增ID：若模型主键标注了 `#[primary(auto)]`，`execute()` 返回自动生成的ID（如 `i32`），而非影响行数。

### 插入并返回 (RETURNING)

插入后返回所有插入的行数据（支持 PostgreSQL、SQLite）：

```rust
let users: Vec<User> = db.insert(&vec![user1, user2]).returning().await?;
```

### 批量插入

```rust
db.insert(&vec![user1, user2, user3])
    .execute()
    .await?;

db.insert(&[user1, user2])
    .execute()
    .await?;
```

### 插入或更新

```rust
db.insert_or_update(&user)
    .execute()
    .await?;

db.insert_or_update(&vec![user1, user2])
    .execute()
    .await?;
```

### 插入或忽略

存在重复主键时静默忽略：

```rust
db.insert_or_ignore(&user)
    .execute()
    .await?;

db.insert_or_ignore(&vec![user1, user2])
    .execute()
    .await?;
```

## 查询 (Read)

```rust
let all: Vec<User> = db.select::<User>().collect().await?;

let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// 只取第一条
let first: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

### 根据主键查找

支持单主键和复合主键：

```rust
// 单主键
let user: Option<User> = db.find_by_id::<User>(1).await?;

// 复合主键
let item: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 100)).await?;
```

也可在事务中使用：

```rust
let txn = db.begin().await?;
let user: Option<User> = txn.find_by_id::<User>(1).await?;
txn.commit().await?;
```

## 更新 (Update)

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "New Name".to_string())
    .set(|u| u.age, 26)
    .execute()
    .await?;

// 使用模型实例更新（自动跳过主键字段）
db.update::<User>()
    .set_model(&updated_user)
    .execute()
    .await?;
```

## 删除 (Delete)

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

db.delete::<User>().execute().await?;
```

## 表管理

```rust
db.create_table::<User>().execute().await?;

db.drop_table::<User>().execute().await?;
```

## 原生 SQL

```rust
let users: Vec<User> = db
    .execute::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;

let affected = db
    .exec_non_query("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;
```

## 完整示例

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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .execute()
    .await?;
    
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ])
    .execute()
    .await?;
    
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age, 26)
        .execute()
        .await?;
    
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## 钩子系统 (Hooks)

Ormer 提供了钩子系统，允许您在数据操作前后自动执行自定义逻辑。支持的钩子包括：

- `BeforeInsert` / `AfterInsert` - 插入前后
- `BeforeUpdate` / `AfterUpdate` - 更新前后
- `BeforeDelete` / `AfterDelete` - 删除前后

### 使用示例

```rust
use ormer::{Model, BeforeInsert, BeforeUpdate};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
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
```

详细文档请参考：[钩子系统](09_hooks.md)
