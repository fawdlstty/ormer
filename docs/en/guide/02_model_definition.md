# Model Definition

Models define the mapping between database table structures and Rust types.

## Basic Definition

```rust
use ormer::Model;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: Option<String>,
}
```

### Attributes

- `#[table = "table_name"]` - Specifies the table name
- `#[primary]` - Primary key
- `#[primary(auto)]` - Auto-increment primary key
- `#[unique]` - Unique constraint (supports `group` parameter for composite unique)
- `#[index]` - Index
- `#[foreign(Type)]` - Foreign key relationship

## Field Attributes

### Unique Constraint

#### Single Column Unique

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    
    #[unique]
    email: String,
}
```

#### Composite Unique

Use the `group` parameter to create composite unique indexes:

```rust
#[derive(Debug, Model)]
#[table = "user_roles"]
struct UserRole {
    #[primary(auto)]
    id: i32,
    
    #[unique(group = 1)]
    user_id: i32,
    
    #[unique(group = 1)]
    role_id: i32,
    // (user_id, role_id) combination must be unique
}
```

### Indexes

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    
    #[index]
    age: i32,
    
    #[index]
    created_at: String,
}
```

### Nullable Fields

Use `Option<T>` to represent nullable fields:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    
    email: Option<String>,
    phone: Option<String>,
}
```

## Supported Types

| Rust Type | SQL Type (SQLite) | SQL Type (PostgreSQL) | SQL Type (MySQL) |
|-----------|-------------------|----------------------|------------------|
| `i32` | INTEGER | INTEGER | INT |
| `i64` | INTEGER | BIGINT | BIGINT |
| `f64` | REAL | DOUBLE | DOUBLE |
| `String` | TEXT | TEXT | TEXT |
| `bool` | INTEGER (0/1) | BOOLEAN | BOOLEAN |

All basic types can be wrapped with `Option<T>` for nullable fields.

## Complete Example

```rust
use ormer::Model;

#[derive(Debug, Model, Clone)]
#[table = "products"]
struct Product {
    #[primary(auto)]
    id: i32,
    
    #[unique]
    sku: String,
    name: String,
    price: f64,
    
    #[index]
    category_id: i32,
    stock: i32,
    
    description: Option<String>,
    is_active: bool,
}
```

## Foreign Key Relationships

```rust
#[derive(Debug, Model)]
#[table = "posts"]
struct Post {
    #[primary(auto)]
    id: i32,
    
    #[foreign(User)]
    user_id: i32,
    
    title: String,
    content: String,
}
```

## Table Operations

### Creating Tables

```rust
db.create_table::<User>().execute().await?;
```

### Validating Tables

```rust
db.validate_table::<User>().await?;
```

Validates whether the table schema matches the model definition (table existence, column count, names, types, constraints, etc.).

### Dropping Tables

```rust
db.drop_table::<User>().execute().await?;
```

## Model Wrappers

Use tuple struct wrappers to reuse table structure with different table names:

```rust
// Base model
#[derive(Debug, Model, Clone)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

// Wrapper - different table name
#[derive(Debug, Model)]
#[table = "archive_users"]
struct ArchiveUser(User);

#[derive(Debug, Model)]
#[table = "temp_users"]
struct TempUser(User);
```

### Usage Example

```rust
// Create tables
db.create_table::<User>().execute().await?;
db.create_table::<ArchiveUser>().execute().await?;

// Insert data
db.insert(&User {
    id: 0,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;

// Insert using wrapper
let archive_user = ArchiveUser(User {
    id: 0,
    name: "Bob".to_string(),
    age: 30,
    email: Some("bob@example.com".to_string()),
});
db.insert(&archive_user).await?;

// Query archive table
let archived: Vec<ArchiveUser> = db
    .select::<ArchiveUser>()
    .collect::<Vec<_>>()
    .await?;

// Access inner data
for au in &archived {
    println!("User: {}", au.inner().name);
}
```


