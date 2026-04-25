# Transaction Management

Transactions are the basic unit of database operations, ensuring that a group of operations either all succeed or all fail.

## What is a Transaction

Transactions have ACID properties:

- **A (Atomicity)** - All operations either complete entirely or not at all
- **C (Consistency)** - The database remains in a consistent state before and after transaction execution
- **I (Isolation)** - Concurrent transactions do not interfere with each other
- **D (Durability)** - Once a transaction is committed, the results are permanently saved

## Basic Transaction Operations

### Begin Transaction

```rust
let mut txn = db.begin().await?;
```

### Commit Transaction

```rust
txn.commit().await?;
```

### Rollback Transaction

```rust
txn.rollback().await?;
```

## Operations Within Transactions

### Insert Data

```rust
let mut txn = db.begin().await?;

txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.insert(&user3).await?;

txn.commit().await?;
```

### Query Data

```rust
let mut txn = db.begin().await?;

// Insert within transaction
txn.insert(&user).await?;

// Query within transaction (can see uncommitted data)
let users: Vec<User> = txn
    .select::<User>()
    .collect()
    .await?;

println!("Users in transaction: {}", users.len());

txn.commit().await?;
```

### Update Data

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

### Delete Data

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

## Transaction Rollback Example

Automatically rollback when an error occurs in the transaction:

```rust
let mut txn = db.begin().await?;

// Insert first record
txn.insert(&user1).await?;

// Insert second record (may fail)
match txn.insert(&user2).await {
    Ok(_) => {
        // All successful, commit transaction
        txn.commit().await?;
        println!("Transaction committed");
    }
    Err(e) => {
        // Error occurred, rollback transaction
        txn.rollback().await?;
        println!("Transaction rolled back: {}", e);
    }
}
```

## Complete Example

### Money Transfer Example

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
    
    // Initialize accounts
    db.insert(&Account { id: 1, name: "Alice".to_string(), balance: 1000.0 }).await?;
    db.insert(&Account { id: 2, name: "Bob".to_string(), balance: 500.0 }).await?;
    
    // Transfer function
    async fn transfer(
        db: &Database,
        from_id: i32,
        to_id: i32,
        amount: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut txn = db.begin().await?;
        
        // 1. Query source account
        let from_accounts: Vec<Account> = txn
            .select::<Account>()
            .filter(|a| a.id.eq(from_id))
            .collect()
            .await?;
        
        let from_account = from_accounts.into_iter().next()
            .ok_or("Source account not found")?;
        
        // 2. Check balance
        if from_account.balance < amount {
            txn.rollback().await?;
            return Err("Insufficient balance".into());
        }
        
        // 3. Debit
        txn.update::<Account>()
            .filter(|a| a.id.eq(from_id))
            .set(|a| a.balance, from_account.balance - amount)
            .execute()
            .await?;
        
        // 4. Query target account
        let to_accounts: Vec<Account> = txn
            .select::<Account>()
            .filter(|a| a.id.eq(to_id))
            .collect()
            .await?;
        
        let to_account = to_accounts.into_iter().next()
            .ok_or("Target account not found")?;
        
        // 5. Credit
        txn.update::<Account>()
            .filter(|a| a.id.eq(to_id))
            .set(|a| a.balance, to_account.balance + amount)
            .execute()
            .await?;
        
        // 6. Commit transaction
        txn.commit().await?;
        println!("Transfer successful: {} -> {}", from_account.name, to_account.name);
        
        Ok(())
    }
    
    // Execute transfer
    match transfer(&db, 1, 2, 100.0).await {
        Ok(_) => println!("Transfer completed"),
        Err(e) => println!("Transfer failed: {}", e),
    }
    
    // Query final balances
    let accounts: Vec<Account> = db.select::<Account>().collect().await?;
    for account in &accounts {
        println!("{}: ${:.2}", account.name, account.balance);
    }
    
    db.drop_table::<Account>().await?;
    Ok(())
}
```

## Transaction Best Practices

### 1. Keep Transactions Short

```rust
// ✅ Recommended: Short transaction
let mut txn = db.begin().await?;
txn.insert(&user).await?;
txn.commit().await?;

// ❌ Avoid: Long-running transaction
let mut txn = db.begin().await?;
// ... many operations ...
// ... network calls ...
// ... file I/O ...
txn.commit().await?;
```

### 2. Always Handle Errors

```rust
let mut txn = db.begin().await?;

match (txn.insert(&user1).await, txn.insert(&user2).await) {
    (Ok(_), Ok(_)) => {
        txn.commit().await?;
        println!("Both inserts successful");
    }
    _ => {
        txn.rollback().await?;
        println!("Transaction rolled back due to error");
    }
}
```

### 3. Use Transactions for Related Operations

```rust
// ✅ Related operations should be in same transaction
let mut txn = db.begin().await?;
txn.insert(&order).await?;
txn.insert(&order_item1).await?;
txn.insert(&order_item2).await?;
txn.commit().await?;

// ❌ Unrelated operations should be separate
let mut txn = db.begin().await?;
txn.insert(&order).await?;
txn.commit().await?;

let mut txn = db.begin().await?;
txn.insert(&unrelated_data).await?;
txn.commit().await?;
```

### 4. Check Results Before Commit

```rust
let mut txn = db.begin().await?;

let count = txn.insert(&batch).await?;

if count != batch.len() {
    txn.rollback().await?;
    return Err("Not all records were inserted".into());
}

txn.commit().await?;
```

## Common Transaction Patterns

### Pattern 1: Try-Commit-Rollback

```rust
async fn safe_transaction<F, T>(db: &Database, operation: F) -> Result<T, Box<dyn std::error::Error>>
where
    F: FnOnce(&mut Database) -> futures::future::BoxFuture<'_, Result<T, Box<dyn std::error::Error>>>,
{
    let mut txn = db.begin().await?;
    
    match operation(&mut txn).await {
        Ok(result) => {
            txn.commit().await?;
            Ok(result)
        }
        Err(e) => {
            txn.rollback().await?;
            Err(e)
        }
    }
}
```

### Pattern 2: Batch Operations

```rust
let mut txn = db.begin().await?;

for item in &items {
    txn.insert(item).await?;
}

txn.commit().await?;
```

### Pattern 3: Conditional Commit

```rust
let mut txn = db.begin().await?;

// Perform operations
txn.insert(&record).await?;

// Check condition
let count: usize = txn.select::<Record>().count(|r| r.id).await?;

if count > MAX_RECORDS {
    txn.rollback().await?;
    println!("Too many records, rolled back");
} else {
    txn.commit().await?;
    println!("Transaction committed");
}
```
