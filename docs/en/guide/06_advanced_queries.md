# Advanced Queries

## Aggregate Queries

```rust
// COUNT
let count: usize = db.select::<User>().count(|u| u.id).await?;

// SUM
let total: Option<i32> = db.select::<Product>().sum(|p| p.price).await?;

// AVG
let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

// MAX
let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

// MIN
let min: Option<i32> = db.select::<User>().min(|u| u.age).await?;
```

With conditions:

```rust
let adult_count: usize = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .count(|u| u.id)
    .await?;
```

## GROUP BY and HAVING Queries

### Basic Grouping

```rust
use ormer::Select;

// Count users by age group
let sql = Select::<User>::new()
    .select_column(|u| u.id.count())
    .group_by(|u| u.age)
    .to_sql();

// Generated SQL: SELECT COUNT(id) FROM users GROUP BY age
```

### Multiple Columns + Grouping

```rust
// Select department and user count
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .to_sql();

// Generated SQL: SELECT department, COUNT(id) FROM users GROUP BY department
```

### HAVING Condition Filter

```rust
// Count departments with more than 5 users
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(5))
    .to_sql();

// Generated SQL: SELECT department, COUNT(id) FROM users GROUP BY department HAVING COUNT(id) > ?
```

### Multi-Column Grouping

```rust
// Group by department and age, count and calculate average score
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.age, u.id.count(), u.score.avg()))
    .group_by(|u| (u.department, u.age))
    .to_sql();

// Generated SQL: SELECT department, age, COUNT(id), AVG(score) FROM users GROUP BY department, age
```

### Complete Query: WHERE + GROUP BY + HAVING + ORDER BY + LIMIT

```rust
let sql = Select::<User>::new()
    .filter(|u| u.age.ge(18))
    .select_column(|u| (u.department, u.id.count(), u.score.avg()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(0))
    .order_by(|u| u.department)
    .range(0..10)
    .to_sql();

// Generated SQL:
// SELECT department, COUNT(id), AVG(score)
// FROM users
// WHERE age >= ?
// GROUP BY department
// HAVING COUNT(id) > ?
// ORDER BY department ASC
// LIMIT 10
```

### Supported Aggregate Functions

- `count()` - Count, returns `usize`
- `sum()` - Sum, returns original type (numeric types)
- `avg()` - Average, returns `f64`
- `max()` - Maximum, returns original type (numeric types)
- `min()` - Minimum, returns original type (numeric types)

### Notes

- `group_by()` can be called before or after `having()`, but it's recommended to call `group_by()` first
- `filter()` adds WHERE conditions (pre-group filter), `having()` adds HAVING conditions (post-group filter)
- All aggregate functions generate parameterized SQL queries for safety

## JOIN Queries

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
```

### LEFT JOIN

```rust
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### INNER JOIN

```rust
let user_roles: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### RIGHT JOIN

```rust
let user_roles: Vec<(Option<User>, Role)> = db
    .select::<User>()
    .right_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### JOIN with Filter

```rust
let admin_users: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

## Multi-Table Joins

### Two Tables (from)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from::<User, Role>()
    .filter(|u, r| u.id.eq(r.user_id))
    .filter(|_, r| r.role_name.eq("admin".to_string()))
    .collect()
    .await?;
```

### Three Tables (from3)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from3::<User, Role, Permission>()
    .filter(|u, r, p| u.id.eq(r.user_id).and(r.id.eq(p.role_id)))
    .collect()
    .await?;
```

### Four Tables (from4)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from4::<User, Role, Permission, Department>()
    .filter(|u, r, p, d| {
        u.id.eq(r.user_id)
            .and(r.id.eq(p.role_id))
            .and(u.department_id.eq(d.id))
    })
    .collect()
    .await?;
```

## Subqueries

### IN Subquery

```rust
let subquery = db.select::<Role>().map_to(|r| r.user_id);

let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(subquery))
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
}

#[derive(Debug, Model)]
#[table = "roles"]
struct Role {
    #[primary(auto)]
    id: i32,
    user_id: i32,
    role_name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    db.create_table::<Role>().execute().await?;
    
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25 },
        User { id: 2, name: "Bob".to_string(), age: 30 },
    ]).await?;
    
    db.insert(&vec![
        Role { id: 1, user_id: 1, role_name: "admin".to_string() },
        Role { id: 2, user_id: 2, role_name: "user".to_string() },
    ]).await?;
    
    // Aggregate
    let count: usize = db.select::<User>().count(|u| u.id).await?;
    let avg_age: Option<f64> = db.select::<User>().avg(|u| u.age).await?;
    
    // LEFT JOIN
    let user_roles: Vec<(User, Option<Role>)> = db
        .select::<User>()
        .left_join::<Role>(|u, r| u.id.eq(r.user_id))
        .collect()
        .await?;
    
    // Multi-table
    let admin_users: Vec<User> = db
        .select::<User>()
        .from::<User, Role>()
        .filter(|u, r| u.id.eq(r.user_id))
        .filter(|_, r| r.role_name.eq("admin".to_string()))
        .collect()
        .await?;
    
    // Subquery
    let users_with_roles: Vec<User> = db
        .select::<User>()
        .filter(|u| u.id.is_in(
            db.select::<Role>().map_to(|r| r.user_id)
        ))
        .collect()
        .await?;
    
    db.drop_table::<Role>().execute().await?;
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## Notes

### JOIN Types

- **LEFT JOIN**: Returns all records from left table, NULL for unmatched right table
- **INNER JOIN**: Returns only matched records from both tables
- **RIGHT JOIN**: Returns all records from right table, NULL for unmatched left table

### Aggregate Functions

Aggregate functions compute at database level, more efficient than fetching all data and computing in application:

```rust
let count: usize = db.select::<User>().count(|u| u.id).await?;
```
