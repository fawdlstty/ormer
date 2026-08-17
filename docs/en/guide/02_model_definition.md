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
- `#[table(schema = "schema", name = "table_name")]` - Specifies schema and table independently
- `#[column(name = "column_name")]` - Specifies the SQL column name
- `#[primary]` - Primary key (supports composite primary keys)
- `#[primary(auto)]` - Auto-increment primary key (only for single primary key or the first field of composite primary key)
- `#[unique]` - Unique constraint (supports `group` and `name`)
- `#[index]` - Index (supports `group`, `name`, `order`, and `where`)
- `#[default(...)]` - Database default; use `#[default(expr = "...")]` for SQL expressions
- `#[check(expr = "...")]` - CHECK constraint, with optional `name`
- `#[foreign(Type)]` - Foreign key relationship, with optional `name`, `on_delete`, and `on_update`
- `#[embed(prefix = "prefix_")]` - Embeds a value object and expands its fields into prefixed columns
- `#[data_type(i64)]` - Database type override (e.g., Rust i32 field mapped to BIGINT in database)
- `#[hypertable(Duration::from_secs(86400))]` - TimescaleDB hypertable time chunk interval
- `#[hypertable]` - Marks a `String` field as a PostgreSQL/TimescaleDB string table route key
- `#[compress]` - PostgreSQL column-level compression (generates `COMPRESSION pglz`)
- `#[filter(filter_name, |m, ...| ...)]` - Model-level reusable filter; the name must start with `filter_`
- `#[version(u64)]` - Adds an automatic `version` column for optimistic locking
- `#[ormer_ignore]` - Excludes a field from database columns; useful for dynamic table route values

PostgreSQL and MSSQL preserve the schema prefix in `#[table = "schema.table"]`; SQLite and MySQL use the final table-name component.

## Optimistic Lock Version

```rust
#[derive(Debug, Model)]
#[version(u64)]
#[table = "orders"]
struct Order {
    #[primary]
    id: i32,
    status: String,
}

let version = order.version();
```

`#[version(u64)]` creates an invisible `version` column with initial value `1`. After loading a model, `version()` returns the current version; `set_model` updates automatically add the version condition and increment the version.

## Model Filters

```rust
#[derive(Debug, Model)]
#[table = "orders"]
#[filter(filter_valid, |o| o.deleted_at.is_null())]
#[filter(filter_tenant, |o, tenant_id: i64| o.tenant_id.eq(tenant_id))]
struct Order {
    #[primary]
    id: i64,
    tenant_id: i64,
    deleted_at: Option<chrono::NaiveDateTime>,
}

use OrderFilterExt;

let orders: Vec<Order> = db
    .select::<Order>()
    .filter_tenant(tenant_id)
    .filter_valid()
    .collect()
    .await?;

let scoped = db.scope().filter_tenant(tenant_id).filter_valid();
let orders: Vec<Order> = scoped.select::<Order>().collect().await?;

let include_deleted: Vec<Order> = scoped
    .select::<Order>()
    .unset_filter_valid()
    .collect()
    .await?;
```

Filters enabled on `scope()` are inherited by queries, relationship loading,
updates, and deletes. `unset_filter_*()` disables only an inherited named
filter; it does not remove filters added directly with `filter(...)`.

## Dynamic Table Routing

Table names may contain `{variable}` placeholders. Use `route_table` for queries; inserts read matching values from model fields.

```rust
#[derive(Debug, Model)]
#[table = "orders_{tenant_id}"]
struct Order {
    #[primary]
    id: i64,
    tenant_id: i64,
}

let orders: Vec<Order> = db
    .select::<Order>()
    .route_table("tenant_id", tenant_id)
    .collect()
    .await?;
```

If a route value should not be a database column, mark it with `#[ormer_ignore]`:

```rust
#[derive(Debug, Model)]
#[table = "events_{tenant_id}"]
struct Event {
    #[primary]
    id: i64,
    name: String,
    #[ormer_ignore]
    tenant_id: i64,
}
```

For TimescaleDB, bare `#[hypertable]` can mark a `String` field so PostgreSQL routes different string values to different physical tables; SQLite, MySQL, and MSSQL do not enable this automatic split. The route key is the field's SQL column name (or the Rust field name when `#[column]` is not set). Inserts read it from the model value; queries and table creation pass it explicitly with `route_table`.

```rust
#[derive(Debug, Model)]
#[table = "events"]
struct Event {
    #[primary]
    id: i64,
    payload: String,
    #[hypertable]
    #[ormer_ignore]
    tenant: String,
    #[hypertable(std::time::Duration::from_secs(86400))]
    created_at: chrono::NaiveDateTime,
}

db.create_table::<Event>()
    .route_table("tenant", "acme")
    .execute()
    .await?;

let rows: Vec<Event> = db
    .select::<Event>()
    .route_table("tenant", "acme")
    .collect()
    .await?;
```

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

### Column Names, Defaults, and Constraints

```rust
#[derive(Debug, Model)]
#[table(schema = "auth", name = "users")]
struct User {
    #[primary(auto)]
    id: i32,
    #[column(name = "display_name")]
    #[default("")]
    #[check(expr = "length(display_name) > 0")]
    name: String,
    #[default(expr = "CURRENT_TIMESTAMP")]
    created_at: chrono::NaiveDateTime,
}
```

`insert(&model)` still writes all model fields explicitly; a database default applies only when the column is omitted from the INSERT, which `insert_partial` or `insert_model` can do.

## Embedded Value Objects

Value objects can derive `Embed`, then model fields can use `#[embed(prefix = "...")]` to expand them into multiple columns:

```rust
#[derive(Debug, Clone, ormer::Embed)]
struct Address {
    city: String,
    street: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    #[embed(prefix = "addr_")]
    address: Address,
}

let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.address.city.eq("Shanghai"))
    .collect()
    .await?;
```

## Supported Types

| Rust Type | SQL Type (SQLite) | SQL Type (PostgreSQL) | SQL Type (MySQL) | SQL Type (MSSQL) |
|-----------|-------------------|----------------------|------------------|------------------|
| `i32` | INTEGER | INTEGER | INT | INT |
| `i64` | INTEGER | BIGINT | BIGINT | BIGINT |
| `f64` | REAL | DOUBLE | DOUBLE | FLOAT |
| `String` | TEXT | TEXT | TEXT | NVARCHAR(255) |
| `bool` | INTEGER (0/1) | BOOLEAN | BOOLEAN | BIT |
| `Vec<u8>` | BLOB | BYTEA | BLOB | VARBINARY(MAX) |
| `uuid::Uuid` | TEXT | UUID | CHAR(36) | UNIQUEIDENTIFIER |
| `chrono::DateTime<chrono::Utc>` | TEXT | TIMESTAMPTZ | DATETIME | DATETIME2 |
| `chrono::NaiveDateTime` | TEXT | TIMESTAMPTZ | DATETIME | DATETIME2 |
| `chrono::NaiveDate` | TEXT | DATE | DATE | DATE |
| `chrono::NaiveTime` | TEXT | TIME | TIME | TIME |
| `rust_decimal::Decimal` | TEXT | NUMERIC | DECIMAL(65,30) | DECIMAL(38,18) |
| `bigdecimal::BigDecimal` | TEXT | NUMERIC | DECIMAL(65,30) | DECIMAL(38,18) |

All basic types can be wrapped with `Option<T>` for nullable fields.

UUID fields can use `uuid::Uuid` or `Option<uuid::Uuid>` directly:

```rust
#[derive(Debug, Clone, ormer::Model)]
#[table = "sessions"]
struct Session {
    #[primary]
    id: uuid::Uuid,
    user_id: uuid::Uuid,
    revoked_at: Option<chrono::NaiveDateTime>,
}
```

UUID values are generated by the application, for example with `uuid::Uuid::new_v4()`; the application must enable the `v4` feature of the `uuid` crate, and the ORM does not generate UUIDs automatically. SQLite uses `TEXT`, MySQL uses `CHAR(36)` for canonical UUID strings, and MSSQL uses the native `UNIQUEIDENTIFIER` type.

## Field Types

```rust
use ormer::{FieldType, Model};

#[derive(Debug, Clone, FieldType, PartialEq)]
enum UserStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, FieldType, PartialEq)]
pub struct ExceptionType(pub u16);

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    status: UserStatus,
    exception_type: ExceptionType,
    name: String,
}
```

`FieldType` works for enums and single-field tuple struct wrappers. Wrapper types use their inner field type for database columns, so `ExceptionType(pub u16)` is stored as `u16`. Use `Option<FieldType>` for nullable fields.

`FieldType` values can also be used with `IN`, comparison, and ordering conditions:

```rust
let active: Vec<User> = db
    .select::<User>()
    .filter(|u| u.status.is_in([UserStatus::Active, UserStatus::Banned]))
    .collect()
    .await?;
```

For an existing numeric enum or wrapper type where you do not want to derive `FieldType`, use `#[data_type(i32)]`. A nullable field must use `#[data_type(Option<i32>)]`:

```rust
#[repr(i32)]
#[derive(Debug, Clone, Copy)]
enum Status {
    Active = 1,
    Disabled = 0,
}

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary]
    id: i32,
    #[data_type(i32)]
    status: Status,
    #[data_type(Option<i32>)]
    old_status: Option<Status>,
}
```

### PostgreSQL Arrays

PostgreSQL supports `Vec<i32>`, `Vec<i64>`, `Vec<Option<i64>>`, and `Vec<String>`. These fields map to native array types; `Vec<String>` uses `TEXT[]`, not a JSON column:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary]
    id: i32,
    tags: Vec<String>,
    scores: Vec<i32>,
}
```

For a custom numeric enum inside `Vec<T>`, use `#[data_type(Vec<i32>)]`:

```rust
#[data_type(Vec<i32>)]
roles: Vec<Status>,
```

Array types and this syntax currently target the PostgreSQL backend.

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

## Model Relations

Foreign-key fields describe the database constraint. To load related models, use `#[has_many]`, `#[belongs_to]`, `#[has_one]`, and `#[through]`:

```rust
#[derive(Debug, Clone, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    #[has_many(Post.user_id)]
    posts: Vec<Post>,
    #[has_one(Profile.user_id)]
    profile: Option<Profile>,
    #[has_many(UserRole.user_id)]
    user_roles: Vec<UserRole>,
    #[through(user_roles.role)]
    roles: Vec<Role>,
}

#[derive(Debug, Clone, Model)]
#[table = "posts"]
struct Post {
    #[primary(auto)]
    id: i32,
    #[foreign(User.id)]
    user_id: i32,
    #[belongs_to(user_id)]
    user: Option<User>,
    title: String,
}
```

Relation fields are not database columns. `#[belongs_to]` and `#[has_one]` fields must be `Option<T>`, while `#[has_many]` and `#[through]` fields use `Vec<T>`. `#[through(user_roles.role)]` follows this model's `user_roles` relation and then the intermediate model's `role` relation.

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

Use `primary_field_names()` to get the Rust primary key field names, `model.promary_fields()` to get the current primary key field value tuple, and `primary_key_columns()` to get the SQL primary key column names. Composite primary keys are returned in field declaration order.

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
