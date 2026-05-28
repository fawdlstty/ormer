# Model Definition

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
- `#[primary]` - Primary key (supports composite primary keys)
- `#[primary(auto)]` - Auto-increment primary key (only for single primary key or the first field of composite primary key)
- `#[unique]` - Unique constraint (supports `group` parameter for composite unique)
- `#[index]` - Index
- `#[foreign(Type)]` - Foreign key relationship
- `#[data_type(i64)]` - Database type override (e.g., Rust i32 field mapped to BIGINT in database)
- `#[hypertable(Duration::from_secs(86400))]` - TimescaleDB hypertable chunk interval

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

## Enum Types

```rust
use ormer::{Model, ModelEnum};

#[derive(Debug, Clone, ModelEnum, PartialEq)]
enum UserStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    status: UserStatus,
    name: String,
}
```

Supports `Option<EnumType>` for nullable enum fields.

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

## Composite Primary Keys

Add `#[primary]` to multiple fields to define a composite primary key:

```rust
#[derive(Debug, Model)]
#[table = "user_roles"]
struct UserRole {
    #[primary]
    user_id: i32,
    #[primary]
    role_id: i32,
    assigned_at: String,
}
```

Only the first primary key field can use `auto`:
```rust
#[primary(auto)]
id: i32,
#[primary]
product_id: i32,
```

Use `primary_key_columns()` to get the list of primary key column names.

## Table Operations

### Creating Tables

```rust
db.create_table::<User>().execute().await?;
```

### Validating Tables

```rust
db.validate_table::<User>().await?;
```

### Dropping Tables

```rust
db.drop_table::<User>().execute().await?;
```

## Model Wrappers

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
db.create_table::<User>().execute().await?;
db.create_table::<ArchiveUser>().execute().await?;

db.insert(&User {
    id: 0,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;

let archive_user = ArchiveUser(User {
    id: 0,
    name: "Bob".to_string(),
    age: 30,
    email: Some("bob@example.com".to_string()),
});
db.insert(&archive_user).execute().await?;

let archived: Vec<ArchiveUser> = db
    .select::<ArchiveUser>()
    .collect::<Vec<_>>()
    .await?;

for au in &archived {
    println!("User: {}", au.inner().name);
}
```


