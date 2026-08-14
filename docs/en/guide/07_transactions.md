# Transaction Management

## Basic Operations

```rust
let mut txn = db.begin().await?;

txn.commit().await?;

txn.rollback().await?;
```

## Closure Transactions

Stable Rust uses a boxed future:

```rust
let user = user.clone();
db.transaction(|txn| Box::pin(async move {
    txn.insert(&user).execute().await?;
    Ok(())
})).await?;
```

The transaction is committed when the closure returns `Ok` and rolled back
when it returns `Err`. Use `TransactionOptions` for isolation and read-only
transactions:

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

SQLite treats transaction options as compatible no-ops.

## Savepoints

Use a savepoint to roll back only the work inside a nested closure:

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

## Operations in Transaction

### Insert

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).execute().await?;
txn.insert(&user2).execute().await?;
txn.commit().await?;
```

### Query

```rust
let mut txn = db.begin().await?;
txn.insert(&user).execute().await?;

let users: Vec<User> = txn.select::<User>().collect().await?;
txn.commit().await?;
```

### Update

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

### Delete

```rust
let mut txn = db.begin().await?;
let count = txn
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;
txn.commit().await?;
```

### Raw SQL

Transactions also support raw SQL with parameter binding:

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

### Insert or Update, Insert or Ignore

Transactions expose the same upsert and ignore operations as `Database`:

```rust
let mut txn = db.begin().await?;
txn.insert_or_update(&user).execute().await?;
txn.insert_or_ignore(&user).execute().await?;
txn.commit().await?;
```

## Error Handling

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

## Complete Example - Transfer

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
    
    // Transfer
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
