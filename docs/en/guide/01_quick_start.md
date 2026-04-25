# Quick Start

This guide will help you get started with Ormer in 5 minutes.

## Requirements

- Rust 1.70+
- Cargo package manager

## Installation

### 1. Create a New Project

```bash
cargo new my_project
cd my_project
```

### 2. Add Dependencies

Add Ormer dependency and async runtime to `Cargo.toml`:

```toml
[dependencies]
ormer = { version = "0.1", features = ["turso"] }
tokio = { version = "1", features = ["full"] }
```

**Choose Database Features:**

- `turso` - Turso/libSQL/SQLite database
- `postgresql` - PostgreSQL database
- `mysql` - MySQL database

You can enable multiple database features simultaneously:

```toml
ormer = { version = "0.1", features = ["turso", "postgresql"] }
```

## Your First Ormer Program

### Complete Example

Create `src/main.rs`:

```rust
use ormer::{Database, DbType, Model};

// 1. Define model
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
    // 2. Connect to database
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // 3. Create table
    db.create_table::<User>().await?;
    
    // 4. Insert data
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    db.insert(&User {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    }).await?;
    
    // 5. Query data
    let users: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name.asc())
        .collect::<Vec<_>>()
        .await?;
    
    // 6. Process results
    for user in &users {
        println!("User: {} (age: {})", user.name, user.age);
    }
    
    // 7. Cleanup
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

### Run the Program

```bash
cargo run
```

**Output:**
```
User: Alice (age: 25)
User: Bob (age: 30)
```

## Core Operations Overview

### Model Definition

Use `#[derive(Model)]` macro to define data models:

```rust
#[derive(Debug, Model)]
#[table = "table_name"]
struct ModelName {
    #[primary(auto)]      // Primary key, auto-increment
    id: i32,
    
    #[unique]             // Unique constraint
    name: String,
    
    #[index]              // Index
    age: i32,
    
    #[unique(group = 1)]  // Composite unique constraint
    field1: String,
    
    #[unique(group = 1)]
    field2: String,
    
    nullable_field: Option<String>,  // Nullable field
}
```

### Database Connection

```rust
// Turso/SQLite
let db = Database::connect(DbType::Turso, "file:test.db").await?;

// PostgreSQL
let db = Database::connect(
    DbType::PostgreSQL, 
    "postgresql://user:pass@localhost/dbname"
).await?;

// MySQL
let db = Database::connect(
    DbType::MySQL, 
    "mysql://user:pass@localhost/dbname"
).await?;
```

### Insert Data

```rust
// Single insert
db.insert(&user).await?;

// Batch insert (Vec)
db.insert(&vec![user1, user2, user3]).await?;

// Batch insert (array)
db.insert(&[user1, user2]).await?;

// Insert or update
db.insert_or_update(&user).await?;
```

### Query Data

```rust
// Query all
let all: Vec<User> = db.select::<User>().collect().await?;

// Conditional query
let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

// Sorting and pagination
let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;
```

### Update Data

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

println!("Updated {} rows", count);
```

### Delete Data

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

println!("Deleted {} rows", count);
```

### Aggregate Queries

```rust
// COUNT
let count: usize = db.select::<User>().count(|u| u.id).await?;

// SUM
let sum: Option<i32> = db.select::<User>().sum(|u| u.age).await?;

// AVG
let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

// MAX
let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

// MIN
let min: Option<i32> = db.select::<User>().min(|u| u.age).await?;
```
