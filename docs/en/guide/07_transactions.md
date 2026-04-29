# Transaction Management

## Basic Operations

```rust
let mut txn = db.begin().await?;

txn.commit().await?;

txn.rollback().await?;
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
