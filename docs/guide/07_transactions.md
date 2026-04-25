# 事务管理

事务是数据库操作的基本单元,确保一组操作要么全部成功,要么全部失败。

## 什么是事务

事务具有 ACID 特性:

- **A (Atomicity)** - 原子性: 所有操作要么全部完成,要么全部不完成
- **C (Consistency)** - 一致性: 事务执行前后,数据库保持一致状态
- **I (Isolation)** - 隔离性: 并发事务互不干扰
- **D (Durability)** - 持久性: 事务完成后,结果永久保存

## 基本事务操作

### 开始事务

```rust
let mut txn = db.begin().await?;
```

### 提交事务

```rust
txn.commit().await?;
```

### 回滚事务

```rust
txn.rollback().await?;
```

## 事务中的操作

### 插入数据

```rust
let mut txn = db.begin().await?;

txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.insert(&user3).await?;

txn.commit().await?;
```

### 查询数据

```rust
let mut txn = db.begin().await?;

// 在事务中插入
txn.insert(&user).await?;

// 在事务中查询 (可以看到未提交的数据)
let users: Vec<User> = txn
    .select::<User>()
    .collect()
    .await?;

println!("Users in transaction: {}", users.len());

txn.commit().await?;
```

### 更新数据

```rust
let txn = db.begin().await?;

let count = txn
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

txn.commit().await?;

println!("Updated {} rows", count);
```

### 删除数据

```rust
let txn = db.begin().await?;

let count = txn
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

txn.commit().await?;

println!("Deleted {} rows", count);
```

## 事务回滚示例

当事务中发生错误时,自动回滚:

```rust
let mut txn = db.begin().await?;

// 插入第一条记录
txn.insert(&user1).await?;

// 插入第二条记录 (可能失败)
match txn.insert(&user2).await {
    Ok(_) => {
        // 全部成功,提交事务
        txn.commit().await?;
        println!("Transaction committed");
    }
    Err(e) => {
        // 发生错误,回滚事务
        txn.rollback().await?;
        println!("Transaction rolled back: {}", e);
    }
}
```

## 完整示例

### 转账示例

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
    db.create_table::<Account>().await?;
    
    // 初始化账户
    db.insert(&Account { id: 1, name: "Alice".to_string(), balance: 1000.0 }).await?;
    db.insert(&Account { id: 2, name: "Bob".to_string(), balance: 500.0 }).await?;
    
    // 转账函数
    async fn transfer(
        db: &Database,
        from_id: i32,
        to_id: i32,
        amount: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut txn = db.begin().await?;
        
        // 1. 查询源账户
        let from_accounts: Vec<Account> = txn
            .select::<Account>()
            .filter(|a| a.id.eq(from_id))
            .collect()
            .await?;
        
        let from_account = from_accounts.into_iter().next()
            .ok_or("Source account not found")?;
        
        // 2. 检查余额
        if from_account.balance < amount {
            txn.rollback().await?;
            return Err("Insufficient balance".into());
        }
        
        // 3. 扣款
        txn.update::<Account>()
            .filter(|a| a.id.eq(from_id))
            .set(|a| a.balance, from_account.balance - amount)
            .execute()
            .await?;
        
        // 4. 查询目标账户
        let to_accounts: Vec<Account> = txn
            .select::<Account>()
            .filter(|a| a.id.eq(to_id))
            .collect()
            .await?;
        
        let to_account = to_accounts.into_iter().next()
            .ok_or("Target account not found")?;
        
        // 5. 存款
        txn.update::<Account>()
            .filter(|a| a.id.eq(to_id))
            .set(|a| a.balance, to_account.balance + amount)
            .execute()
            .await?;
        
        // 6. 提交事务
        txn.commit().await?;
        
        Ok(())
    }
    
    // 执行转账
    match transfer(&db, 1, 2, 200.0).await {
        Ok(_) => println!("Transfer successful"),
        Err(e) => println!("Transfer failed: {}", e),
    }
    
    // 验证结果
    let accounts: Vec<Account> = db.select::<Account>().collect().await?;
    for account in &accounts {
        println!("{}: ${:.2}", account.name, account.balance);
    }
    
    db.drop_table::<Account>().await?;
    Ok(())
}
```

### 批量操作示例

```rust
async fn batch_insert_users(db: &Database, users: Vec<User>) -> Result<(), Box<dyn std::error::Error>> {
    let mut txn = db.begin().await?;
    
    for user in users {
        txn.insert(&user).await?;
    }
    
    txn.commit().await?;
    Ok(())
}
```

## 事务的最佳实践

### 1. 及时提交或回滚

```rust
// ✅ 推荐: 使用 match 或 ? 运算符
let mut txn = db.begin().await?;

txn.insert(&user).await?;
txn.commit().await?;

// ❌ 避免: 忘记提交
let mut txn = db.begin().await?;
txn.insert(&user).await?;
// 事务未提交,数据丢失
```

### 2. 错误处理

```rust
let mut txn = db.begin().await?;

if let Err(e) = txn.insert(&user).await {
    txn.rollback().await?;
    return Err(e.into());
}

txn.commit().await?;
```

### 3. 避免长时间持有事务

```rust
// ✅ 推荐: 快速完成事务
let mut txn = db.begin().await?;
txn.insert(&user).await?;
txn.commit().await?;

// ❌ 避免: 事务中执行耗时操作
let mut txn = db.begin().await?;
txn.insert(&user).await?;
tokio::time::sleep(Duration::from_secs(60)).await;  // 不好!
txn.commit().await?;
```

### 4. 使用 RAII 模式自动回滚

```rust
async fn safe_transaction(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let mut txn = db.begin().await?;
    
    // 使用 defer 模式 (需要额外库支持)
    // 或者使用作用域
    
    let result = (async {
        txn.insert(&user1).await?;
        txn.insert(&user2).await?;
        txn.commit().await
    }).await;
    
    if result.is_err() {
        let _ = txn.rollback().await;
    }
    
    result
}
```

## 事务隔离级别

不同数据库支持不同的事务隔离级别:

- **Read Uncommitted** - 读未提交
- **Read Committed** - 读已提交
- **Repeatable Read** - 可重复读
- **Serializable** - 可序列化

Ormer 使用数据库的默认隔离级别。如需自定义,可以在连接字符串中指定:

```rust
// PostgreSQL 示例
let db = Database::connect(
    DbType::PostgreSQL,
    "postgresql://user:pass@localhost/dbname?options=-c%20default_transaction_isolation=serializable"
).await?;
```

## 注意事项

### 1. 事务中的查询可见性

```rust
let mut txn = db.begin().await?;

txn.insert(&user).await?;

// 在同一个事务中可以查询到未提交的数据
let users: Vec<User> = txn.select::<User>().collect().await?;
println!("In transaction: {} users", users.len());  // 包含新插入的

txn.commit().await?;

// 事务提交后,其他连接才能看到
```

### 2. 死锁

当多个事务互相等待对方释放锁时,会发生死锁:

```rust
// 事务 1
let mut txn1 = db.begin().await?;
txn1.update::<Account>().filter(|a| a.id.eq(1))...
// 等待更新账户 2

// 事务 2
let mut txn2 = db.begin().await?;
txn2.update::<Account>().filter(|a| a.id.eq(2))...
// 等待更新账户 1

// 死锁! 数据库会检测并回滚其中一个事务
```

**避免方法**: 按相同顺序访问资源

```rust
// ✅ 两个事务都先访问账户 1,再访问账户 2
```

### 3. 性能考虑

- 事务会增加数据库负载
- 长时间运行的事务会阻塞其他操作
- 批量操作使用单个事务优于多个小事务
