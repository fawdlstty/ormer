# Advanced Queries

This document introduces Ormer's advanced query features, including aggregate queries, JOIN queries, multi-table associations, and subqueries.

## Aggregate Queries

Aggregate queries are used to calculate summary data such as counts, sums, averages, etc.

### COUNT - Count

```rust
let count: usize = db
    .select::<User>()
    .count(|u| u.id)
    .await?;

println!("Total users: {}", count);
```

Count with conditions:

```rust
let adult_count: usize = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .count(|u| u.id)
    .await?;
```

### SUM - Sum

```rust
let total_age: Option<i32> = db
    .select::<User>()
    .sum(|u| u.age)
    .await?;

if let Some(sum) = total_age {
    println!("Total age: {}", sum);
}
```

### AVG - Average

```rust
let avg_age: Option<f64> = db
    .select::<User>()
    .avg(|u| u.age)
    .await?;

if let Some(avg) = avg_age {
    println!("Average age: {:.2}", avg);
}
```

### MAX - Maximum

```rust
let max_age: Option<i32> = db
    .select::<User>()
    .max(|u| u.age)
    .await?;

if let Some(max) = max_age {
    println!("Max age: {}", max);
}
```

### MIN - Minimum

```rust
let min_age: Option<i32> = db
    .select::<User>()
    .min(|u| u.age)
    .await?;

if let Some(min) = min_age {
    println!("Min age: {}", min);
}
```

### Aggregate Query Example

```rust
#[derive(Debug, Model)]
#[table = "products"]
struct Product {
    #[primary(auto)]
    id: i32,
    name: String,
    price: f64,
    stock: i32,
}

// Count products
let count: usize = db.select::<Product>().count(|p| p.id).await?;

// Calculate total stock
let total_stock: Option<i32> = db.select::<Product>().sum(|p| p.stock).await?;

// Calculate average price
let avg_price: Option<f64> = db.select::<Product>().avg(|p| p.price).await?;

// Find highest price
let max_price: Option<f64> = db.select::<Product>().max(|p| p.price).await?;

// Find lowest price
let min_price: Option<f64> = db.select::<Product>().min(|p| p.price).await?;
```

## JOIN Queries

JOIN queries are used to retrieve data from multiple related tables.

### LEFT JOIN

Returns all records from the left table, even if there's no match in the right table:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Model)]
#[table = "roles"]
struct Role {
    #[primary]
    id: i32,
    user_id: i32,
    role_name: String,
}

// LEFT JOIN: Get all users and their roles (may have no role)
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

for (user, role) in &user_roles {
    match role {
        Some(r) => println!("{} has role: {}", user.name, r.role_name),
        None => println!("{} has no role", user.name),
    }
}
```

### INNER JOIN

Returns only records that have matches in both tables:

```rust
let user_roles: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// Only users with roles will be returned
for (user, role) in &user_roles {
    println!("{} has role: {}", user.name, role.role_name);
}
```

### RIGHT JOIN

Returns all records from the right table, even if there's no match in the left table:

```rust
let user_roles: Vec<(Option<User>, Role)> = db
    .select::<User>()
    .right_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// Note: SQLite does not support RIGHT JOIN
```

### JOIN with Filter Conditions

```rust
let admin_users: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

## Multi-Table Association Queries

### Two-Table Association (from)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### Three-Table Association

```rust
let results: Vec<(User, Role, Department)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .inner_join::<Department>(|u, d| u.department_id.eq(d.id))
    .collect()
    .await?;
```

### Four-Table Association

```rust
let results: Vec<(User, Role, Department, Company)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .inner_join::<Department>(|u, d| u.department_id.eq(d.id))
    .inner_join::<Company>(|u, c| u.company_id.eq(c.id))
    .collect()
    .await?;
```

## Subqueries

Subqueries allow you to use the results of one query within another query.

### Subquery in Filter

```rust
// Find users whose age is greater than the average age
let above_avg_age_users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.gt(
        db.select::<User>().avg(|u| u.age)
    ))
    .collect()
    .await?;
```

### Subquery with IN

```rust
// Find users who have roles
let users_with_roles: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(
        db.select::<Role>().map_to(|r| r.user_id)
    ))
    .collect()
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
    department_id: i32,
}

#[derive(Debug, Model)]
#[table = "departments"]
struct Department {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    
    db.create_table::<User>().await?;
    db.create_table::<Department>().await?;
    
    // Insert departments
    db.insert(&vec![
        Department { id: 1, name: "Engineering".to_string() },
        Department { id: 2, name: "Marketing".to_string() },
    ]).await?;
    
    // Insert users
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, department_id: 1 },
        User { id: 2, name: "Bob".to_string(), age: 30, department_id: 1 },
        User { id: 3, name: "Charlie".to_string(), age: 35, department_id: 2 },
    ]).await?;
    
    // 1. Aggregate query
    let count: usize = db.select::<User>().count(|u| u.id).await?;
    println!("Total users: {}", count);
    
    let avg_age: Option<f64> = db.select::<User>().avg(|u| u.age).await?;
    println!("Average age: {:?}", avg_age);
    
    // 2. JOIN query
    let user_depts: Vec<(User, Department)> = db
        .select::<User>()
        .inner_join::<Department>(|u, d| u.department_id.eq(d.id))
        .collect()
        .await?;
    
    for (user, dept) in &user_depts {
        println!("{} works in {}", user.name, dept.name);
    }
    
    // 3. Multi-table query with filtering
    let eng_users: Vec<(User, Department)> = db
        .select::<User>()
        .inner_join::<Department>(|u, d| u.department_id.eq(d.id))
        .filter(|u| u.age.ge(25))
        .collect()
        .await?;
    
    println!("Engineering users 25+: {}", eng_users.len());
    
    // Cleanup
    db.drop_table::<User>().await?;
    db.drop_table::<Department>().await?;
    
    Ok(())
}
```

## Best Practices

### 1. Use Appropriate JOIN Types

```rust
// ✅ Use LEFT JOIN when right table may not have matches
let users_with_optional_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// ✅ Use INNER JOIN when both tables must have matches
let users_with_roles: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### 2. Add Indexes for JOIN Columns

```rust
#[derive(Debug, Model)]
struct User {
    #[primary(auto)]
    id: i32,
    
    #[index]  // Add index for foreign key
    department_id: i32,
}
```

### 3. Use Subqueries Judiciously

```rust
// ✅ Good: Simple subquery
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(
        db.select::<Role>().map_to(|r| r.user_id)
    ))
    .collect()
    .await?;

// ⚠️ Caution: Complex subqueries may impact performance
```
