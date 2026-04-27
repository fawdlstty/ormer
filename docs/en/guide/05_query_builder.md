# Query Builder

## Basic Queries

```rust
// Query all
let users: Vec<User> = db.select::<User>().collect().await?;

// Single record
let user: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.eq(1))
    .range(..1)
    .collect()
    .await?;
```

## Filters

### Comparison

```rust
.filter(|u| u.name.eq("Alice".to_string()))  // Equal
.filter(|u| u.age.ge(18))                    // Greater or equal
.filter(|u| u.age.gt(18))                    // Greater
.filter(|u| u.age.le(65))                    // Less or equal
.filter(|u| u.age.lt(65))                    // Less
```

### IN Queries

```rust
.filter(|u| u.age.is_in(&vec![18, 20, 22]))
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))
```

### Combined Conditions

```rust
// AND
.filter(|u| u.age.ge(18))
.filter(|u| u.age.le(65))

// Using and()/or()
.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## Sorting

```rust
// Ascending
.order_by(|u| u.name.asc())

// Descending
.order_by_desc(|u| u.age)

// Multi-field
.order_by(|u| u.age.desc())
.order_by(|u| u.name.asc())
```

## Pagination

```rust
.range(0..10)    // First 10
.range(10..20)   // Page 2
.range(..5)      // First 5
.range(10..)     // From 10 onwards
```

## Streaming Queries (stream)

When processing large datasets, streaming queries fetch results row by row, avoiding loading all data into memory at once:

```rust
// Basic streaming query
let mut stream = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .stream()
    .into_iter()
    .await?;

while let Some(user_result) = stream.next().await {
    let user = user_result?;
    println!("{:?}", user);
}
```

### Streaming Query Features

- **Memory Efficient**: Fetches data row by row, suitable for large datasets
- **Async Iteration**: Supports async row-by-row processing without blocking threads
- **Supports All Query Options**: Works with filter, order_by, range, etc.

### Streaming Query with Filter and Sorting

```rust
let mut stream = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by_desc(|u| u.age)
    .range(0..100)
    .stream()
    .into_iter()
    .await?;

while let Some(user_result) = stream.next().await {
    let user = user_result?;
    // Process each row
}
```

## Field Projection (map_to)

```rust
// Single field
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

// Tuple
let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

// Convert to custom type
let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## Query Composition

```rust
// Chained calls
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// Reuse query
let base_query = db.select::<User>().filter(|u| u.age.ge(18));

let adults_cn = base_query.clone()
    .filter(|u| u.country.eq("CN".to_string()))
    .collect::<Vec<_>>()
    .await?;

let adults_us = base_query
    .filter(|u| u.country.eq("US".to_string()))
    .collect::<Vec<_>>()
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
    
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, email: None },
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // Basic query
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    // Conditional query
    let adults: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .collect()
        .await?;
    
    // Sort
    let sorted: Vec<User> = db
        .select::<User>()
        .order_by_desc(|u| u.age)
        .collect()
        .await?;
    
    // Pagination
    let page: Vec<User> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .collect()
        .await?;
    
    // Field projection
    let names: Vec<String> = db
        .select::<User>()
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## Performance Tips

### Field Projection

Query only needed fields to reduce data transfer:

```rust
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect()
    .await?;
```

### Avoid N+1 Queries

Use IN query instead of loop queries:

```rust
// Use IN query
let ids = vec![1, 2, 3, 4, 5];
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(&ids))
    .collect()
    .await?;
```
