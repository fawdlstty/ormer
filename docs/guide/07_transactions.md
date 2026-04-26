# 事务管理

## 基本操作

```rust
// 开始事务
let mut txn = db.begin().await?;

// 提交
txn.commit().await?;

// 回滚
txn.rollback().await?;
```

## 事务中的操作

### 插入

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.commit().await?;
```

### 查询

```rust
let mut txn = db.begin().await?;
txn.insert(&user).await?;

// 事务内可见未提交数据
let users: Vec<User> = txn.select::<User>().collect().await?;
txn.commit().await?;
```

### 更新

```rust
let mut txn = db.begin().await?;
let count = txn
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;
txn.commit().await?;
```

### 删除

```rust
let mut txn = db.begin().await?;
let count = txn
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;
txn.commit().await?;
```

## 错误处理

```rust
let mut txn = db.begin().await?;

match txn.insert(&user2).await {
    Ok(_) => txn.commit().await?,
    Err(e) => {
        txn.rollback().await?;
        return Err(e.into());
    }
}
```

## 完整示例 - 转账

```rust
use ormer::{Database, DbType, Model};

#[derive(Debug, Model)]
#[table = "accounts"]
struct Account {
    #[primary]
    id: i32,
    name: String,
    balance: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<Account>().execute().await?;
    
    db.insert(&Account { id: 1, name: "Alice".to_string(), balance: 1000.0 }).await?;
    db.insert(&Account { id: 2, name: "Bob".to_string(), balance: 500.0 }).await?;
    
    // 转账
    let mut txn = db.begin().await?;
    
    let from: Vec<Account> = txn
        .select::<Account>()
        .filter(|a| a.id.eq(1))
        .collect()
        .await?;
    
    let from_account = from.into_iter().next().ok_or("Account not found")?;
    
    if from_account.balance < 200.0 {
        txn.rollback().await?;
        return Err("Insufficient balance".into());
    }
    
    txn.update::<Account>()
        .filter(|a| a.id.eq(1))
        .set(|a| a.balance, from_account.balance - 200.0)
        .execute()
        .await?;
    
    txn.update::<Account>()
        .filter(|a| a.id.eq(2))
        .set(|a| a.balance, 700.0)
        .execute()
        .await?;
    
    txn.commit().await?;
    
    let accounts: Vec<Account> = db.select::<Account>().collect().await?;
    for account in &accounts {
        println!("{}: ${:.2}", account.name, account.balance);
    }
    
    db.drop_table::<Account>().execute().await?;
    Ok(())
}
```

## 说明







## 注意事项

### 事务可见性

事务内可查询未提交数据，提交后其他连接才可见。

### 死锁避免

按相同顺序访问资源，避免多个事务互相等待。

### 性能

- 事务会增加数据库负载
- 长时间事务会阻塞其他操作
- 批量操作使用单个事务
