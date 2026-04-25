# 数据操作

本文档详细介绍 Ormer 的 CRUD (创建、读取、更新、删除) 操作。

## 插入数据 (Create)

### 单条插入

使用 `insert()` 方法插入单条记录:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

// 插入单条记录
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;
```

### 批量插入

#### 使用 Vec

```rust
let users = vec![
    User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: None,
    },
    User {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    },
    User {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
        email: Some("charlie@example.com".to_string()),
    },
];

db.insert(&users).await?;
```

#### 使用数组

```rust
db.insert(&[
    User { id: 1, name: "Alice".to_string(), age: 25, email: None },
    User { id: 2, name: "Bob".to_string(), age: 30, email: None },
]).await?;
```

#### 使用切片

```rust
let users = vec![/* ... */];
db.insert(&users[..]).await?;
```

### 插入或更新 (Upsert)

当遇到重复键时,自动更新现有记录:

```rust
// 首次插入
db.insert_or_update(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;

// 更新已存在的记录
db.insert_or_update(&User {
    id: 1,  // 相同的 ID
    name: "Alice Smith".to_string(),  // 更新名字
    age: 26,  // 更新年龄
    email: Some("alice.smith@example.com".to_string()),
}).await?;
```

批量插入或更新:

```rust
db.insert_or_update(&vec![
    User { id: 1, name: "Alice".to_string(), age: 25, email: None },
    User { id: 2, name: "Bob".to_string(), age: 30, email: None },
]).await?;
```

## 读取数据 (Read)

### 查询所有记录

```rust
let all_users: Vec<User> = db
    .select::<User>()
    .collect::<Vec<_>>()
    .await?;
```

### 条件查询

详见 [查询构建器文档](05_query_builder.md)。

基本示例:

```rust
// 年龄大于等于 18 的用户
let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

// 名字匹配
let alice: Vec<User> = db
    .select::<User>()
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

### 排序和分页

```rust
// 按名字升序,取前 10 条
let users: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// 按年龄降序,分页
let page2: Vec<User> = db
    .select::<User>()
    .order_by_desc(|u| u.age)
    .range(10..20)  // 第 2 页 (每页 10 条)
    .collect()
    .await?;
```

## 更新数据 (Update)

使用 `update()` 方法更新记录:

### 基本更新

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

println!("Updated {} rows", count);
```

### 多字段更新

```rust
let count = db
    .update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "Alice Smith".to_string())
    .set(|u| u.age, 26)
    .set(|u| u.email, Some("alice.smith@example.com".to_string()))
    .execute()
    .await?;
```

### 条件更新

```rust
// 更新所有年龄小于 18 的用户
let count = db
    .update::<User>()
    .filter(|u| u.age.lt(18))
    .set(|u| u.name, "Minor".to_string())
    .execute()
    .await?;
```

## 删除数据 (Delete)

使用 `delete()` 方法删除记录:

### 条件删除

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

println!("Deleted {} rows", count);
```

### 删除特定记录

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;
```

### 删除所有记录

⚠️ **警告**: 这会删除表中的所有数据!

```rust
let count = db
    .delete::<User>()
    .execute()
    .await?;
```

## 表管理

### 创建表

```rust
db.create_table::<User>().await?;
```

这会生成 `CREATE TABLE IF NOT EXISTS` SQL 语句。

### 删除表

```rust
db.drop_table::<User>().await?;
```

生成 `DROP TABLE IF EXISTS` SQL 语句。

## 执行原生 SQL

### 查询并返回模型

```rust
let users: Vec<User> = db
    .exec_table::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;
```

### 执行非查询 SQL

```rust
let affected_rows = db
    .exec_non_query("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;

println!("Affected {} rows", affected_rows);
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
    db.create_table::<User>().await?;
    
    // 1. 插入数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // 2. 查询数据
    let all_users: Vec<User> = db.select::<User>().collect().await?;
    println!("All users: {:?}", all_users);
    
    // 3. 更新数据
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age, 26)
        .execute()
        .await?;
    
    // 4. 删除数据
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    // 5. 清理
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

## 最佳实践

### 1. 批量操作优于循环

```rust
// ✅ 推荐: 批量插入
db.insert(&users).await?;

// ❌ 避免: 循环插入
for user in &users {
    db.insert(user).await?;  // 慢!
}
```

### 2. 使用事务保证一致性

```rust
let mut txn = db.begin().await?;

txn.insert(&user1).await?;
txn.insert(&user2).await?;

txn.commit().await?;
```

### 3. 合理设置过滤条件

```rust
// ✅ 明确指定条件
db.delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;

// ❌ 危险: 忘记过滤条件
db.delete::<User>().execute().await?;  // 删除所有!
```

### 4. 检查操作结果

```rust
let count = db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "New Name".to_string())
    .execute()
    .await?;

if count == 0 {
    println!("No rows updated");
}
```
