# Introduction to Ormer

Ormer is a minimal, high-performance Rust ORM framework focused on providing type-safe, compile-time optimized database operation experience.

## Core Features

### 🎯 Type Safety
- Compile-time type checking to avoid runtime errors
- Field-level type inference
- Strongly-typed query builder

### ⚡ High Performance
- Built on Rust async runtime
- Zero-cost abstraction, no runtime overhead
- Connection pool management support

### 🔧 Multi-Database Support
- **Turso (libSQL/SQLite)** - Embedded database
- **PostgreSQL** - Enterprise-grade relational database
- **MySQL** - Popular open-source database

### 📝 Elegant API Design
- Chainable query building
- Macro-driven model definition
- Intuitive filtering and sorting syntax

### 🔍 Rich Query Capabilities
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

## Design Philosophy

Ormer's design philosophy is **"simple but not simplistic"**:

1. **Compile-time Optimization**: Catch errors at compile time rather than runtime
2. **Type Safety**: Leverage Rust's type system to ensure correctness of data operations
3. **Minimal API**: Complete the most common operations with the least code
4. **Extensibility**: Support custom extensions through trait system

## Use Cases

- Web application backend development
- Data-intensive applications
- Microservice architecture
- Scenarios requiring high-performance database operations
- Projects with strict type safety requirements


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
    db.create_table::<User>().await?;
    
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
    db.drop_table::<User>().await?;
    
    Ok(())
}
```
