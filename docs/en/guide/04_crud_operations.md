# Data Operations

This document details Ormer's CRUD (Create, Read, Update, Delete) operations.

## Insert Data (Create)

### Single Insert

Use the `insert()` method to insert a single record:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

// Insert single record
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;
```

### Batch Insert

#### Using Vec

```rust
let users = vec![
    User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: None,
    },
    User {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    },
    User {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
        email: Some("charlie@example.com".to_string()),
    },
];

db.insert(&users).await?;
```

#### Using Array

```rust
db.insert(&[
    User { id: 1, name: "Alice".to_string(), age: 25, email: None },
    User { id: 2, name: "Bob".to_string(), age: 30, email: None },
]).await?;
```

#### Using Slice

```rust
let users = vec![/* ... */];
db.insert(&users[..]).await?;
```

### Insert or Update (Upsert)

Automatically update existing records when encountering duplicate keys:

```rust
// First insert
db.insert_or_update(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;

// Update existing record
db.insert_or_update(&User {
    id: 1,  // Same ID
    name: "Alice Smith".to_string(),  // Update name
    age: 26,  // Update age
    email: Some("alice.smith@example.com".to_string()),
}).await?;
```

Batch insert or update:

```rust
db.insert_or_update(&vec![
    User { id: 1, name: "Alice".to_string(), age: 25, email: None },
    User { id: 2, name: "Bob".to_string(), age: 30, email: None },
]).await?;
```

## Read Data (Read)

### Query All Records

```rust
let all_users: Vec<User> = db
    .select::<User>()
    .collect::<Vec<_>>()
    .await?;
```

### Conditional Query

See [Query Builder Documentation](05_query_builder.md) for details.

Basic example:

```rust
// Users aged 18 or older
let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

// Name match
let alice: Vec<User> = db
    .select::<User>()
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

### Sorting and Pagination

```rust
// Sort by name ascending, get first 10
let users: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// Sort by age descending, paginated
let page2: Vec<User> = db
    .select::<User>()
    .order_by_desc(|u| u.age)
    .range(10..20)  // Page 2 (10 per page)
    .collect()
    .await?;
```

## Update Data (Update)

Use the `update()` method to update records:

### Basic Update

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name, "Adult".to_string())
    .execute()
    .await?;

println!("Updated {} rows", count);
```

### Multiple Fields Update

```rust
let count = db
    .update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "Alice Smith".to_string())
    .set(|u| u.age, 26)
    .set(|u| u.email, Some("alice.smith@example.com".to_string()))
    .execute()
    .await?;
```

### Conditional Update

```rust
// Update all users under 18
let count = db
    .update::<User>()
    .filter(|u| u.age.lt(18))
    .set(|u| u.name, "Minor".to_string())
    .execute()
    .await?;
```

## Delete Data (Delete)

Use the `delete()` method to delete records:

### Conditional Delete

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

println!("Deleted {} rows", count);
```

### Delete Specific Record

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;
```

### Delete All Records

⚠️ **Warning**: This will delete all data in the table!

```rust
let count = db
    .delete::<User>()
    .execute()
    .await?;
```

## Table Management

### Create Table

```rust
db.create_table::<User>().await?;
```

This generates a `CREATE TABLE IF NOT EXISTS` SQL statement.

### Drop Table

```rust
db.drop_table::<User>().await?;
```

Generates a `DROP TABLE IF EXISTS` SQL statement.

## Execute Raw SQL

### Query and Return Models

```rust
let users: Vec<User> = db
    .exec_table::<User>("SELECT * FROM users WHERE age >= 18")
    .await?;
```

### Execute Non-Query SQL

```rust
let affected_rows = db
    .exec_non_query("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;

println!("Affected {} rows", affected_rows);
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
    db.create_table::<User>().await?;
    
    // 1. Insert data
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // 2. Query data
    let all_users: Vec<User> = db.select::<User>().collect().await?;
    println!("All users: {:?}", all_users);
    
    // 3. Update data
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age, 26)
        .execute()
        .await?;
    
    // 4. Delete data
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    // 5. Cleanup
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

## Best Practices

### 1. Batch Operations Over Loops

```rust
// ✅ Recommended: Batch insert
db.insert(&users).await?;

// ❌ Avoid: Loop insert
for user in &users {
    db.insert(user).await?;  // Slow!
}
```

### 2. Use Transactions for Consistency

```rust
let mut txn = db.begin().await?;

txn.insert(&user1).await?;
txn.insert(&user2).await?;

txn.commit().await?;
```

### 3. Set Filter Conditions Properly

```rust
// ✅ Explicitly specify conditions
db.delete::<User>()
    .filter(|u| u.id.eq(1))
    .execute()
    .await?;

// ❌ Dangerous: Forgot filter condition
db.delete::<User>().execute().await?;  // Deletes all!
```

### 4. Check Operation Results

```rust
let count = db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name, "New Name".to_string())
    .execute()
    .await?;

if count == 0 {
    println!("No rows updated");
}
```
