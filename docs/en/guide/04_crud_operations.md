# Data Operations

## Insert (Create)

### Single Insert

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
})
.execute()
.await?;
```

> `execute()` returns the auto-increment ID: if the model's primary key has `#[primary(auto)]`, `execute()` returns the auto-generated ID (e.g. `i32`) instead of the affected row count.

### Insert with RETURNING

Insert and return all inserted rows (PostgreSQL, SQLite):

```rust
let users: Vec<User> = db.insert(&vec![user1, user2]).returning().await?;
```

### Batch Insert

```rust
db.insert(&vec![user1, user2, user3])
    .execute()
    .await?;

db.insert(&[user1, user2])
    .execute()
    .await?;
```

### Insert or Update

```rust
db.insert_or_update(&user)
    .execute()
    .await?;
db.insert_or_update(&vec![user1, user2])
    .execute()
    .await?;
```

### Insert or Ignore

Silently ignore duplicates:

```rust
db.insert_or_ignore(&user)
    .execute()
    .await?;
db.insert_or_ignore(&vec![user1, user2])
    .execute()
    .await?;
```

## Read (Query)

```rust
let all: Vec<User> = db.select::<User>().collect().await?;

let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// Get only the first record
let first: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

### Find by ID

Supports single and composite primary keys:

```rust
// Single primary key
let user: Option<User> = db.find_by_id::<User>(1).await?;

// Composite primary key
let item: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 100)).await?;
```

Can also be used within transactions:

```rust
let txn = db.begin().await?;
let user: Option<User> = txn.find_by_id::<User>(1).await?;
txn.commit().await?;
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

// Update using a model instance (auto-skips primary key fields)
db.update::<User>()
    .set_model(&updated_user)
    .execute()
    .await?;
```

## Delete

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

db.delete::<User>().execute().await?;
```

## Table Management

```rust
db.create_table::<User>().execute().await?;

db.drop_table::<User>().execute().await?;
```

## Raw SQL

```rust
let users: Vec<User> = db
    .execute::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;

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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    // Insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .execute()
    .await?;
    
    // Batch insert
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ])
    .execute()
    .await?;
    
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

## Hooks System

Ormer provides a hooks system that allows you to automatically execute custom logic before and after data operations. Supported hooks include:

- `BeforeInsert` / `AfterInsert` - Before and after insert
- `BeforeUpdate` / `AfterUpdate` - Before and after update
- `BeforeDelete` / `AfterDelete` - Before and after delete

### Example

```rust
use ormer::{Model, BeforeInsert, BeforeUpdate};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}
```

For detailed documentation, please refer to: [Hooks System](09_hooks.md)
