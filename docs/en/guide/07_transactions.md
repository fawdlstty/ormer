# Transaction Management

## Basic Operations

```rust
// Begin transaction
let mut txn = db.begin().await?;

// Commit
txn.commit().await?;

// Rollback
txn.rollback().await?;
```

## Operations in Transaction

### Insert

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.commit().await?;
```

### Query

```rust
let mut txn = db.begin().await?;
txn.insert(&user).await?;

// Can see uncommitted data in transaction
let users: Vec<User> = txn.select::<User>().collect().await?;
txn.commit().await?;
```

### Update

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

## Error Handling

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
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<Account>().execute().await?;
    
    db.insert(&Account { id: 1, name: "Alice".to_string(), balance: 1000.0 }).await?;
    db.insert(&Account { id: 2, name: "Bob".to_string(), balance: 500.0 }).await?;
    
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

## Notes
## Notes

### Transaction Visibility

Uncommitted data visible within transaction. After commit, visible to other connections.

### Deadlock Avoidance

Access resources in same order to avoid deadlocks.

### Performance

- Transactions increase database load
- Long transactions block other operations
- Use single transaction for batch operations
