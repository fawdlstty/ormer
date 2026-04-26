# Data Operations

## Insert (Create)

### Single Insert

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;
```

### Batch Insert

```rust
// Vec
db.insert(&vec![user1, user2, user3]).await?;

// Array
db.insert(&[user1, user2]).await?;
```

### Insert or Update

```rust
db.insert_or_update(&user).await?;
db.insert_or_update(&vec![user1, user2]).await?;
```

## Read (Query)

```rust
// Query all
let all: Vec<User> = db.select::<User>().collect().await?;

// Conditional query
let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

// Sort and paginate
let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;
```

## Update

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

// Multiple fields
db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "New Name".to_string())
    .set(|u| u.age, 26)
    .execute()
    .await?;
```

## Delete

```rust
// Conditional delete
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

// Delete all (dangerous!)
db.delete::<User>().execute().await?;
```

## Table Management

```rust
// Create table
db.create_table::<User>().execute().await?;

// Drop table
db.drop_table::<User>().execute().await?;
```

## Raw SQL

```rust
// Query returning models
let users: Vec<User> = db
    .exec_table::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;

// Execute non-query
let affected = db
    .exec_non_query("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;
```

## Complete Example

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
    
    // Insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    // Batch insert
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // Query
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    // Update
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age, 26)
        .execute()
        .await?;
    
    // Delete
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## Performance Tips

### Batch Operations

Batch insert is more efficient than individual inserts:

```rust
// Batch insert
db.insert(&users).await?;
```

### Transactions

Multiple related operations can use transactions for consistency:

```rust
let mut txn = db.begin().await?;
txn.insert(&user1).await?;
txn.insert(&user2).await?;
txn.commit().await?;
```

### Delete Operations

It's recommended to add filter conditions for delete operations:

```rust
// Conditional delete
db.delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;
```
