# Query Builder

## Basic Queries

```rust
let users: Vec<User> = db.select::<User>().collect().await?;

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
.filter(|u| u.name.eq("Alice".to_string()))
.filter(|u| u.age.ne(18))          // != not equal
.filter(|u| u.age.ge(18))          // >= greater than or equal
.filter(|u| u.age.gt(18))          // > greater than
.filter(|u| u.age.le(65))          // <= less than or equal
.filter(|u| u.age.lt(65))          // < less than
```

### LIKE Pattern Matching

```rust
.filter(|u| u.name.like("Al%"))           // custom pattern
.filter(|u| u.name.contains("alice"))      // contains substring
.filter(|u| u.name.starts_with("Al"))      // starts with
.filter(|u| u.name.ends_with("ce"))        // ends with
```

Can be combined with other conditions:

```rust
.filter(|u| u.name.contains("li").and(u.age.gt(29)))
```

### NULL Checks

```rust
.filter(|u| u.email.is_null())       // IS NULL
.filter(|u| u.email.is_not_null())   // IS NOT NULL
```

### BETWEEN Range

```rust
.filter(|u| u.age.between(18, 30))  // age BETWEEN 18 AND 30
```

### IN and NOT IN

```rust
.filter(|u| u.age.is_in(&vec![18, 20, 22]))
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))

.filter(|u| u.age.is_not_in(&vec![18, 20]))   // NOT IN
```

`is_in` and `is_not_in` also support subqueries:

```rust
.filter(|u| u.id.is_in(db.select::<Role>().map_to(|r| r.user_id)))
.filter(|u| u.id.is_not_in(db.select::<Role>().map_to(|r| r.user_id)))
```

### Combined Conditions

```rust
.filter(|u| u.age.ge(18))
.filter(|u| u.age.le(65))

.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## Sorting

```rust
.order_by(|u| u.name.asc())

.order_by_desc(|u| u.age)

.order_by(|u| u.age.desc())
.order_by(|u| u.name.asc())
```

## Pagination

```rust
.range(0..10)
.range(10..20)
.range(..5)
.range(10..)
```

## Distinct Queries

```rust
// SELECT DISTINCT *
let users = db.select::<User>().distinct().collect().await?;

// SELECT DISTINCT name
let names: Vec<String> = db.select::<User>().distinct().map_to(|u| u.name).collect().await?;
```

Can be combined with `filter`, `order_by`, `range`, etc.

## Single Record Query

```rust
// Equivalent to range(..1), returns only the first record
let user: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

## Streaming Queries (stream)

```rust
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

## Field Projection (map_to)

```rust
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## Query Composition

```rust
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
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
