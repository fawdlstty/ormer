# 事务管理

## 基本操作

```rust
let mut txn = db.begin().await?;

txn.commit().await?;

txn.rollback().await?;
```

`commit` 和 `rollback` 会消费事务所有权。若要在业务路径中取消事务，
优先使用 `close()`；它等价于显式回滚：

```rust
txn.close().await?;
```

已激活的事务被 Drop 时，SQLite 和 DuckDB 会同步回滚；其他后端会尽力回滚
其专属事务连接。Drop 兜底不适合依赖错误传播，正常流程仍应显式关闭。

## 闭包式事务

稳定 Rust 使用 boxed future：

```rust
let user = user.clone();
db.transaction(|txn| Box::pin(async move {
    txn.insert(&user).execute().await?;
    Ok(())
})).await?;
```

闭包返回 `Err` 时事务会自动回滚，返回 `Ok` 时自动提交。可通过
`TransactionOptions` 设置隔离级别和只读事务：

```rust
use ormer::{IsolationLevel, TransactionOptions};

db.transaction_opts(
    TransactionOptions::new()
        .isolation(IsolationLevel::Serializable)
        .read_only(),
    |txn| Box::pin(async move {
        let _: Vec<User> = txn.select::<User>().collect().await?;
        Ok(())
    }),
).await?;
```

SQLite 不支持事务选项。MSSQL 会应用隔离级别，但显式拒绝 `read_only()`。
PostgreSQL 和 MySQL 会应用这两类选项。

## Savepoint

事务中可以使用 savepoint，只回滚闭包内的操作：

```rust
db.transaction(|txn| Box::pin(async move {
    txn.insert(&user1).execute().await?;

    let nested = txn.savepoint(|txn| Box::pin(async move {
        txn.insert(&user2).execute().await?;
        Err::<(), _>(ormer::ormer_error!("cancel nested work"))
    })).await;
    assert!(nested.is_err());

    Ok(())
})).await?;
```

## 事务中的操作

### 插入

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).execute().await?;
txn.insert(&user2).execute().await?;
txn.commit().await?;
```

### 查询

```rust
let mut txn = db.begin().await?;
txn.insert(&user).execute().await?;

let users: Vec<User> = txn.select::<User>().collect().await?;
txn.commit().await?;
```

### 更新

```rust
let mut txn = db.begin().await?;
let count = txn
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name = u.name.set("Adult".to_string()))
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

### 原生 SQL

事务对象也支持原生 SQL 和参数绑定：

```rust
let mut txn = db.begin().await?;

let users: Vec<User> = txn
    .select_sql::<User>(
        ormer::sql("SELECT * FROM users WHERE age >= {}").bind(18),
    )
    .collect()
    .await?;

txn.execute_sql(
    ormer::sql("UPDATE users SET name = {} WHERE id = {}")
        .bind("Adult")
        .bind(1),
)
.await?;

txn.commit().await?;
```

### 插入或更新、插入或忽略

事务对象也提供与数据库对象相同的 upsert 和 ignore 操作：

```rust
let mut txn = db.begin().await?;
txn.insert_or_update(&user).execute().await?;
txn.insert_or_ignore(&user).execute().await?;
txn.commit().await?;
```

SQLite 上这些操作是模拟语义：`insert_or_update` 使用 `DELETE` + `INSERT`，
`insert_or_ignore` 只捕获唯一约束错误；生成 SQL 会带有模拟语义标记。
自增主键和插入 hook 的行为可能与原生原子 upsert 不同。

### MySQL 两步回查

MySQL 没有 DML `RETURNING`。以下事务 helper 会先写入，再在同一事务连接上按主键回查：

```rust
let inserted: Vec<User> = txn.insert_returning(&user).await?;
let updated: Option<User> = txn.update_model_returning(&user).await?;
let deleted: Option<User> = txn.delete_model_returning(&user).await?;
```

这不是单条 SQL 的原子 `RETURNING`；写入和回查的一致性依赖外层事务。

## 错误处理

```rust
let mut txn = db.begin().await?;

match txn.insert(&user2).execute().await {
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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<Account>().execute().await?;
    
    db.insert(&Account { id: 1, name: "Alice".to_string(), balance: 1000.0 })
        .execute()
        .await?;
    db.insert(&Account { id: 2, name: "Bob".to_string(), balance: 500.0 })
        .execute()
        .await?;
    
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
        .set(|a| a.balance = a.balance.set(from_account.balance - 200.0))
        .execute()
        .await?;
    
    txn.update::<Account>()
        .filter(|a| a.id.eq(2))
        .set(|a| a.balance = a.balance.set(700.0))
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
