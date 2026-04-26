# 数据操作

## 插入 (Create)

### 单条插入

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;
```

### 批量插入

```rust
// Vec
db.insert(&vec![user1, user2, user3]).await?;

// 数组
db.insert(&[user1, user2]).await?;
```

### 插入或更新

```rust
db.insert_or_update(&user).await?;
db.insert_or_update(&vec![user1, user2]).await?;
```

## 查询 (Read)

```rust
// 查询所有
let all: Vec<User> = db.select::<User>().collect().await?;

// 条件查询
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

## 更新 (Update)

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

// 多字段更新
db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "New Name".to_string())
    .set(|u| u.age, 26)
    .execute()
    .await?;
```

## 删除 (Delete)

```rust
// 条件删除
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

// 删除所有 (危险!)
db.delete::<User>().execute().await?;
```

## 表管理

```rust
// 创建表
db.create_table::<User>().execute().await?;

// 删除表
db.drop_table::<User>().execute().await?;
```

## 原生 SQL

```rust
// 查询返回模型
let users: Vec<User> = db
    .exec_table::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;

// 执行非查询
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
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    // 插入
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    // 批量插入
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // 查询
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    // 更新
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age, 26)
        .execute()
        .await?;
    
    // 删除
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## 性能提示

### 批量操作

批量插入比循环单次插入更高效：

```rust
// 批量插入
db.insert(&users).await?;
```

### 事务

多个相关操作可使用事务保证一致性：

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.commit().await?;
```

### 删除操作

删除操作建议添加过滤条件：

```rust
// 条件删除
db.delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;
```
