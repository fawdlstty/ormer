# Data Operations

## Insert (Create)

### Single Insert

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
})
.execute()
.await?;
```

> `execute()` returns the auto-increment ID: if the model's primary key has `#[primary(auto)]`, `execute()` returns the auto-generated ID (e.g. `i32`) instead of the affected row count.

### Insert with RETURNING

Insert and return all inserted rows (PostgreSQL, SQLite, MSSQL):

```rust
let users: Vec<User> = db.insert(&vec![user1, user2]).returning().await?;
```

`update().returning()` and `delete().returning()` support the same backends. MySQL does not support DML returning and returns `UnsupportedFeature`; use `execute()` first, then `find_by_id` or `select` when returned rows are needed.

### Batch Insert

```rust
db.insert(&vec![user1, user2, user3])
    .execute()
    .await?;

db.insert(&[user1, user2])
    .execute()
    .await?;
```

Large inserts are automatically split by backend parameter limits; PostgreSQL uses `COPY FROM STDIN` when there is no conflict handling, no auto-increment key return, and all values can be serialized safely.

### Partial Insert and Form Models

`insert_partial::<User>()` writes only columns selected with `set`; `default` omits the column from the INSERT so the database default is used:

```rust
db.insert_partial::<User>()
    .set(|u| u.name.set("Alice"))
    .set(|u| (u.email, Some("alice@example.com".to_string())))
    .default(|u| u.created_at)
    .execute()
    .await?;
```

Form structs can derive `InsertModel` and be inserted with `insert_model::<User>(&value)`. `ActiveValue::not_set()` omits a column, while `set(value)` and `unchanged(value)` write the value:

```rust
#[derive(ormer::InsertModel)]
#[table = "users"]
struct NewUser {
    name: String,
    email: Option<String>,
    created_at: ormer::ActiveValue<chrono::NaiveDateTime>,
}

let new_user = NewUser {
    name: "Bob".to_string(),
    email: None,
    created_at: ormer::ActiveValue::not_set(),
};

db.insert_model::<User>(&new_user)
    .execute()
    .await?;
```

### Insert or Update

```rust
db.insert_or_update(&user)
    .execute()
    .await?;
db.insert_or_update(&vec![user1, user2])
    .execute()
    .await?;
```

`upsert` is a short alias for `insert_or_update` and keeps the primary-key conflict behavior:

```rust
db.upsert(&user).execute().await?;
```

### Object Graph Insert and Update

`insert_graph` inserts the root object first, then handles non-empty relation collections in one transaction:

```rust
db.insert_graph(&mut user).execute().await?;
```

`update_graph` updates the root object and upserts or synchronizes non-empty `has_many`, `has_one`, and `through` relations:

```rust
db.update_graph(&mut user).execute().await?;
```

An empty `Vec` means the relation is not part of this operation; existing relations are not cleared.

### Configurable Insert Conflict

Use `insert()` when you need a unique-key conflict target, selected update fields, or an update condition:

```rust
db.insert(&user)
    .on_conflict(|u| u.email)
    .do_update_if(|u| u.active.eq(true))
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;

db.insert(&user)
    .on_conflict(|u| u.email)
    .do_nothing()
    .execute()
    .await?;

db.insert(&membership)
    .on_conflict(|m| (m.org_id, m.user_id))
    .do_update()
    .set(|m| m.role = m.role.incoming())
    .execute()
    .await?;

db.insert(&user)
    .on_conflict(|u| u.email)
    .conflict_where(|u| u.deleted_at.is_null())
    .do_update()
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;
```

PostgreSQL can target a named constraint:

```rust
db.insert(&user)
    .on_constraint("users_email_key")
    .do_nothing()
    .execute()
    .await?;
```

MySQL maps this to `ON DUPLICATE KEY UPDATE` or `INSERT IGNORE`; it cannot select a specific unique key, partial index, or `DO UPDATE WHERE`.

### Insert or Ignore

Silently ignore duplicates:

```rust
db.insert_or_ignore(&user)
    .execute()
    .await?;
db.insert_or_ignore(&vec![user1, user2])
    .execute()
    .await?;
```

## Read (Query)

```rust
let all: Vec<User> = db.select::<User>().collect().await?;

let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// Get only the first record
let first: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

### Find by ID

Supports single and composite primary keys:

```rust
// Single primary key
let user: Option<User> = db.find_by_id::<User>(1).await?;

// Composite primary key
let item: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 100)).await?;
```

Can also be used within transactions:

```rust
let txn = db.begin().await?;
let user: Option<User> = txn.find_by_id::<User>(1).await?;
txn.commit().await?;
```

## Update

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name = u.name.set("Adult".to_string()))
    .execute()
    .await?;

// Multiple fields
db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name = u.name.set("New Name".to_string()))
    .set(|u| u.age = u.age.set(26))
    .execute()
    .await?;

db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.age += 1)
    .execute()
    .await?;

// Update using a model instance (auto-skips primary key fields)
db.update::<User>()
    .set_model(&updated_user)
    .execute()
    .await?;

// Models with #[version(u64)] automatically check and increment version

// Update only selected model fields without overwriting other columns
db.update::<User>()
    .set_model_fields(&updated_user, |u| (u.name, u.age))
    .execute()
    .await?;
```

When `set_model(&users)` or `set_model_fields(&users, ...)` receives a model collection, Ormer emits a single differential UPDATE when there is no optimistic-lock row attribution requirement and primary keys are unique; otherwise it keeps the per-row update semantics.

### Tracked Save

After loading a model from the database, call `track()` to record its current snapshot. `save()` updates only changed non-primary-key fields; when nothing changed, it returns `0` without running an UPDATE:

```rust
let mut user = db.find_by_id::<User>(1).await?.unwrap().track();
user.name = "New Name".to_string();
user.email = Some("new@example.com".to_string());

db.save(&mut user).execute().await?;
```

Load relations before calling `track()` to capture the original object graph. On `save()`, removed, inserted, and modified related rows are synchronized in the same transaction; both collections are sorted by primary key before the diff is calculated, and clearing a collection deletes its related rows:

```rust
let mut user = db
    .select::<User>()
    .include(|user| user.posts)
    .collect::<Vec<_>>()
    .await?
    .into_iter()
    .next()
    .unwrap()
    .track();

user.posts.pop();
user.posts.push(Post {
    id: 0,
    user_id: user.id,
    title: "New post".to_string(),
});

db.save(&mut user).execute().await?;
```

## Delete

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

db.delete::<User>().execute().await?;

// Delete by model; #[version(u64)] adds primary-key and version conditions
db.delete::<User>()
    .model(&user)
    .execute()
    .await?;
```

## Table Management

```rust
db.create_table::<User>().execute().await?;

db.drop_table::<User>().execute().await?;
```

## Typed DSL Raw Expressions

Use `ormer::raw!` inside `filter`, `order_by` / `order_by_desc`, and `map_to` for database functions or dialect expressions. Fields inside `{...}` render as column references, while variables and literals are bound as parameters; escape literal braces with `{{` / `}}`.

```rust
let term = "%alice%";

let names: Vec<String> = db
    .select::<User>()
    .filter(|u| ormer::raw!("LOWER({u.name}) LIKE LOWER({term})"))
    .order_by_desc(|u| ormer::raw!("LENGTH({u.name})"))
    .map_to(|u| {
        ormer::raw!("LOWER({u.name})")
            .typed::<String>()
            .alias("name_lower")
    })
    .collect()
    .await?;

db.update::<User>()
    .set(|u| u.name = u.name.set_expr(ormer::raw!("LOWER({u.name})")))
    .execute()
    .await?;
```

## Raw SQL

```rust
let users: Vec<User> = db
    .select_sql::<User>("SELECT * FROM users WHERE age >= 18")
    .collect::<Vec<User>>()
    .await?;

let affected = db
    .execute_sql("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;
```

Use `sql!`, `bind`, or `bind_named` for parameters instead of concatenating user input:

```rust
db.execute_sql(ormer::sql!(
    "UPDATE users SET name = {name} WHERE id = {id}",
    name = "Alice".to_string(),
    id = 1,
))
.await?;

let users: Vec<User> = db
    .select_sql::<User>(
        ormer::sql("SELECT * FROM users WHERE age >= :age")
            .bind_named("age", 18),
    )
    .collect()
    .await?;
```

`{}` is a positional parameter, while `{name}` and `:name` are named parameters. Rendering converts them to backend placeholders such as `?` or `$1`; similar text inside strings and comments is not replaced. Transactions and pooled connections expose the same `select_sql` and `execute_sql` methods.

Use `RawSql::plain()` to disable placeholder parsing when SQL contains dialect syntax or literal colon text:

```rust
let users: Vec<User> = db
    .select_sql::<User>(ormer::RawSql::plain("SELECT * FROM users WHERE note = ':literal'"))
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
    email: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    // Insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .execute()
    .await?;
    
    // Batch insert
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ])
    .execute()
    .await?;
    
    // Query
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    // Update
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age = u.age.set(26))
        .execute()
        .await?;
    
    // Delete
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## Hooks System

Ormer provides fallible hooks around write operations. Inserts trigger implemented hooks automatically; updates and deletes require `execute_with_hooks` or `execute_models_with_hooks` to supply the hook subject.

- `BeforeInsert` / `AfterInsert` - Before and after insert
- `BeforeUpdate` / `AfterUpdate` - Before and after update
- `BeforeDelete` / `AfterDelete` - Before and after delete

### Example

```rust
use ormer::{AfterInsert, BeforeInsert, BeforeUpdate, HookContext, Model};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        self.updated_at = chrono::Utc::now();
        Ok(())
    }
}
```

For detailed documentation, please refer to: [Hooks System](09_hooks.md)
