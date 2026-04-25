# Query Builder

Ormer provides a powerful type-safe query builder with chainable API and compile-time type checking.

## Basic Queries

### Query All Records

```rust
let users: Vec<User> = db
    .select::<User>()
    .collect::<Vec<_>>()
    .await?;
```

### Single Record Query

```rust
// Query the first matching record
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.eq(1))
    .range(..1)  // Only get the first one
    .collect()
    .await?;

if let Some(user) = users.into_iter().next() {
    println!("Found: {:?}", user);
}
```

## Filter Conditions

### Comparison Operators

```rust
// Equal
.filter(|u| u.name.eq("Alice".to_string()))

// Greater than or equal
.filter(|u| u.age.ge(18))

// Greater than
.filter(|u| u.age.gt(18))

// Less than or equal
.filter(|u| u.age.le(65))

// Less than
.filter(|u| u.age.lt(65))
```

### IN Queries

```rust
// In collection
.filter(|u| u.age.is_in(&vec![18, 20, 22, 25]))

// String IN
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))
```

### Combined Conditions

```rust
// AND conditions (multiple filters)
db.select::<User>()
    .filter(|u| u.age.ge(18))
    .filter(|u| u.age.le(65))
    .collect()
    .await?;

// Using and() to combine
.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))

// Using or() to combine
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## Sorting

### Ascending Order

```rust
db.select::<User>()
    .order_by(|u| u.name.asc())
    .collect()
    .await?;
```

### Descending Order

```rust
db.select::<User>()
    .order_by_desc(|u| u.age)
    .collect()
    .await?;
```

### Multi-Field Sorting

```rust
db.select::<User>()
    .order_by(|u| u.age.desc())
    .order_by(|u| u.name.asc())
    .collect()
    .await?;
```

## Pagination

### Using range

```rust
// First 10 records
.range(0..10)

// Page 2 (10 per page)
.range(10..20)

// Only first 5 records
.range(..5)

// From record 10 to the end
.range(10..)
```

### Pagination Example

```rust
fn get_page(db: &Database, page: usize, page_size: usize) {
    let start = page * page_size;
    let end = start + page_size;
    
    db.select::<User>()
        .order_by(|u| u.id.asc())
        .range(start..end)
        .collect::<Vec<_>>()
        .await
}

// Usage
let page1 = get_page(&db, 0, 10).await?;  // Page 1
let page2 = get_page(&db, 1, 10).await?;  // Page 2
```

## Field Projection (map_to)

Query only the fields you need to improve performance:

### Single Field Projection

```rust
// Query only names
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

// Query only IDs
let ids: Vec<i32> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect::<Vec<i32>>()
    .await?;
```

### Tuple Projection

```rust
// Two-element tuple
let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

// Three-element tuple
let user_info: Vec<(i32, String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.id, u.name, u.age))
    .collect()
    .await?;
```

### Convert to Custom Model

```rust
#[derive(Debug, Model)]
#[table = "user_ids"]
struct UserId {
    #[primary]
    id: i32,
}

let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## Query Builder Composition

### Chained Calls

```rust
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .filter(|u| u.name.eq("Alice".to_string()))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;
```

### Query Reuse

```rust
// Create base query
let base_query = db
    .select::<User>()
    .filter(|u| u.age.ge(18));

// Reuse with different conditions
let adults_in_china = base_query.clone()
    .filter(|u| u.country.eq("CN".to_string()))
    .collect::<Vec<_>>()
    .await?;

let adults_in_usa = base_query
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
    db.create_table::<User>().await?;
    
    // Insert test data
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, email: None },
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
        User { id: 4, name: "David".to_string(), age: 28, email: None },
    ]).await?;
    
    // 1. Basic query
    let all: Vec<User> = db.select::<User>().collect().await?;
    println!("All users: {}", all.len());
    
    // 2. Conditional query
    let adults: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .collect()
        .await?;
    println!("Adults: {}", adults.len());
    
    // 3. Sorted query
    let sorted: Vec<User> = db
        .select::<User>()
        .order_by_desc(|u| u.age)
        .collect()
        .await?;
    println!("Oldest: {:?}", sorted.first());
    
    // 4. Paginated query
    let page1: Vec<User> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .collect()
        .await?;
    println!("Page 1: {:?}", page1);
    
    // 5. Field projection
    let names: Vec<String> = db
        .select::<User>()
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;
    println!("Names: {:?}", names);
    
    // 6. Combined query
    let result: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(25))
        .filter(|u| u.age.le(35))
        .order_by(|u| u.name.asc())
        .range(0..10)
        .collect()
        .await?;
    println!("Filtered: {:?}", result);
    
    db.drop_table::<User>().await?;
    Ok(())
}
```

## Best Practices

### 1. Query Only Needed Fields

```rust
// ✅ Recommended: Use map_to
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect()
    .await?;

// ❌ Avoid: Query all fields
let users: Vec<User> = db.select::<User>().collect().await?;
let names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
```

### 2. Use Indexes Reasonably

```rust
// Add indexes for frequently filtered fields
#[derive(Debug, Model)]
struct User {
    #[index]
    age: i32,
    
    #[index]
    status: String,
}
```

### 3. Avoid N+1 Queries

```rust
// ✅ Recommended: Use IN query
let ids = vec![1, 2, 3, 4, 5];
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(&ids))
    .collect()
    .await?;

// ❌ Avoid: Loop queries
for id in ids {
    let user: Vec<User> = db
        .select::<User>()
        .filter(|u| u.id.eq(id))
        .collect()
        .await?;
}
```
