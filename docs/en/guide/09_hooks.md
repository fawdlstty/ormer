# Hooks System

## Overview

Ormer provides a hooks system that allows you to automatically execute custom logic at key lifecycle points of data operations. Hooks are triggered before and after insert, update, and delete operations, making them perfect for:

- Automatically setting timestamps (created_at, updated_at)
- Data validation and sanitization
- Audit logging
- Cache invalidation
- Related data synchronization

## Supported Hook Types

Ormer provides 6 hook traits:

| Hook Trait | Trigger Timing | Method Signature | Use Case |
|-----------|---------------|------------------|----------|
| `BeforeInsert` | Before inserting data | `async fn before_insert(&mut self)` | Set initial values, validate data |
| `AfterInsert` | After inserting data | `async fn after_insert(&self)` | Log records, trigger events |
| `BeforeUpdate` | Before updating data | `async fn before_update(&mut self)` | Update modification time, validate changes |
| `AfterUpdate` | After updating data | `async fn after_update(&self)` | Clear cache, notify parties |
| `BeforeDelete` | Before deleting data | `async fn before_delete(&self)` | Check dependencies, backup data |
| `AfterDelete` | After deleting data | `async fn after_delete(&self)` | Clean up related resources |

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

// Implement before insert hook - automatically set timestamps
#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
    }
}

// Implement before update hook - automatically update modification time
#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}

// Implement after insert hook - log records
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
    let db = Database::connect(DbType::Turso, "mydb.db").await?;
    
    // Create table
    db.create_table::<User>().execute().await?;
    
    // Insert data - BeforeInsert and AfterInsert will be triggered automatically
    let user = User {
        id: 0,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        created_at: Default::default(), // Will be overridden by hook
        updated_at: Default::default(), // Will be overridden by hook
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
        // Validate email format
        if !self.email.contains('@') {
            panic!("Invalid email format");
        }
        
        // Set default status
        self.status = "active".to_string();
        
        println!("Preparing to create user: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self) {
        // Prevent updating disabled users
        if self.status == "disabled" {
            panic!("Cannot update disabled user");
        }
        
        println!("Preparing to update user: {}", self.username);
    }
}

#[async_trait::async_trait]
impl BeforeDelete for User {
    async fn before_delete(&self) {
        // Check for dependent data
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

All hook methods are asynchronous (`async fn`), allowing you to perform async operations in hooks, such as:
- Database queries
- Network requests
- File I/O

```rust
#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self) {
        // Send welcome email
        send_welcome_email(&self.email).await;
        
        // Log audit trail
        audit_log("user_created", &self.id.to_string()).await;
    }
}
```

### 2. Error Handling

The current version of the hooks system does not propagate errors. If a panic occurs in a hook, it will affect the entire operation. It is recommended to use `Result` types in hooks and handle errors properly:

```rust
#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self) {
        if let Err(e) = self.validate() {
            // Log error instead of panicking
            eprintln!("Validation failed: {}", e);
            // Or use more elegant error handling
        }
    }
}
```

### 3. Performance Considerations

- Hooks add additional function call overhead
- For batch operations, hooks are called once per record
- Avoid time-consuming operations in hooks

### 4. Automatic Trigger Mechanism

**Current Status**: Hook traits are defined and can be implemented correctly, but the automatic trigger mechanism (automatically calling hooks in executors) is not fully implemented due to Rust type system limitations.

**Current Usage**: You can manually call hook methods:

```rust
let mut user = User { /* ... */ };

// Manually call hook
user.before_insert().await;

// Then execute database operation
db.insert(&user).execute().await?;
```

**Future Plans**: Future versions will implement fully automatic triggering through more complex generic specialization mechanisms.

## Best Practices

1. **Keep hooks simple**: Hooks should perform fast, lightweight operations
2. **Avoid side effects**: Hooks should not modify other unrelated data
3. **Logging**: Add appropriate logging in hooks for debugging
4. **Data validation**: Perform data validation in `Before*` hooks
5. **Resource cleanup**: Clean up cache or send notifications in `After*` hooks

## Comparison with SeaORM

| Feature | Ormer | SeaORM |
|-----|-------|--------|
| BeforeInsert | ✅ | ✅ |
| AfterInsert | ✅ | ✅ |
| BeforeUpdate | ✅ | ✅ |
| AfterUpdate | ✅ | ✅ |
| BeforeDelete | ✅ | ✅ |
| AfterDelete | ✅ | ✅ |
| Async Support | ✅ | ✅ |
| Auto Trigger | 🚧 In Development | ✅ |

## Related Documentation

- [CRUD Operations](04_crud_operations.md) - Learn basic create, read, update, delete operations
- [Model Definition](02_model_definition.md) - Learn how to define data models
- [Transactions](07_transactions.md) - Using hooks in transactions
