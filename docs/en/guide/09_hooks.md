# Hooks System

## Supported Hook Types

| Hook Trait | Trigger Timing | Method Signature |
|-----------|---------------|------------------|
| `BeforeInsert` | Before inserting data | `async fn before_insert(&mut self)` |
| `AfterInsert` | After inserting data | `async fn after_insert(&self)` |
| `BeforeUpdate` | Before updating data | `async fn before_update(&mut self)` |
| `AfterUpdate` | After updating data | `async fn after_update(&self)` |
| `BeforeDelete` | Before deleting data | `async fn before_delete(&self)` |
| `AfterDelete` | After deleting data | `async fn after_delete(&self)` |

## Basic Usage

## Basic Usage

```rust
use ormer::{Model, BeforeInsert, BeforeUpdate, AfterInsert};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}

#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self) {
        println!("New user created: {} ({})", self.name, self.email);
    }
}
```

### Using Hooks

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Sqlite, "mydb.db").await?;
    
    db.create_table::<User>().execute().await?;
    
    let user = User {
        id: 0,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        created_at: Default::default(),
        updated_at: Default::default(),
    };
    
    db.insert(&user).execute().await?;
    
    Ok(())
}
```

## Complete Example: User Management

```rust
use ormer::{Model, Database, DbType, BeforeInsert, BeforeUpdate, BeforeDelete, AfterDelete};
use std::sync::atomic::{AtomicUsize, Ordering};

static USER_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    username: String,
    email: String,
    status: String,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        if !self.email.contains('@') {
            panic!("Invalid email format");
        }
        
        self.status = "active".to_string();
        
        println!("Preparing to create user: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        if self.status == "disabled" {
            panic!("Cannot update disabled user");
        }
        
        println!("Preparing to update user: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeDelete for User {
    async fn before_delete(&self) {
        println!("Preparing to delete user: {} (status: {})", self.username, self.status);
    }
}

#[async_trait::async_trait]
impl AfterDelete for User {
    async fn after_delete(&self) {
        USER_COUNT.fetch_sub(1, Ordering::SeqCst);
        println!("User deleted: {}", self.username);
    }
}
```

## Important Notes

### 1. Async Support

All hook methods are asynchronous (`async fn`), allowing you to perform async operations in hooks.

### 2. Error Handling

The current version of the hooks system does not propagate errors. If a panic occurs in a hook, it will affect the entire operation.

### 3. Performance Considerations

- Hooks add additional function call overhead
- For batch operations, hooks are called once per record
- Avoid time-consuming operations in hooks

### 4. Automatic Trigger Mechanism

**Current Status**: Hook traits are defined and can be implemented correctly, but the automatic trigger mechanism (automatically calling hooks in executors) is not fully implemented due to Rust type system limitations.

**Current Usage**: You can manually call hook methods:

```rust
let mut user = User { /* ... */ };

user.before_insert().await;

db.insert(&user).execute().await?;
```

**Future Plans**: Future versions will implement fully automatic triggering through more complex generic specialization mechanisms.
