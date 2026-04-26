# Introduction to Ormer

Ormer is a minimal, high-performance Rust ORM framework providing type-safe database operations.

## Core Features

- **Type Safety**: Compile-time type checking, strongly-typed query builder
- **High Performance**: Zero-cost abstraction, async runtime, connection pool support
- **Multi-Database**: Turso/SQLite, PostgreSQL, MySQL
- **Elegant API**: Chainable queries, macro-driven models, intuitive syntax

## Query Capabilities
- Basic CRUD operations
- Complex filter conditions (comparison, IN, LIKE, etc.)
- Aggregate queries (COUNT, SUM, AVG, MAX, MIN)
- Field projection (map_to)
- JOIN queries (LEFT, INNER, RIGHT)
- Multi-table association queries (2-4 tables)
- Subquery support
- Pagination queries (LIMIT/OFFSET)

### 💾 Transaction Support
- ACID transaction guarantees
- Query, insert, update, delete within transactions
- Commit and rollback control






## Quick Preview

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
    // Connect to database
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    // Create table
    db.create_table::<User>().execute().await?;
    
    // Insert data
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    
    // Query data
    let users: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name.asc())
        .collect::<Vec<_>>()
        .await?;
    
    for user in &users {
        println!("User: {} (age: {})", user.name, user.age);
    }
    
    // Cleanup
    db.drop_table::<User>().execute().await?;
    
    Ok(())
}
```
