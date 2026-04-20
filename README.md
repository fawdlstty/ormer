# ormer

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Former%2Fmain%2F/ormer/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/ormer/rust.yml)

English | [简体中文](README.zh.md)

The simplest ORM for Rust.

# Usage

<!-- [Online Documentation](https://ormer.fawdlstty.com) -->

Add the library reference:

```sh
cargo add ormer
cargo add tokio --features full
```

#### 1. Define Model

Use the `#[derive(Model)]` macro to define your database model:

```rust
use ormer::Model;

#[derive(Model, Debug, Clone)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}
```

#### 2. Connect to Database

```rust
use ormer::abstract_layer::Database;
use ormer::DbType;

#[tokio::main]
async fn main() {
    // ...
    let db = Database::connect(DbType::Turso, "data.db").await?;
    // ...
}
```

#### 3. Create Table

```rust
db.create_table::<User>().await?;
```

#### 4. Insert Data

```rust
db.insert(&User {
    id: 0, // 将自动生成
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
}).await?;
```

#### 5. Query Data

```rust
let users = db.select::<User>().collect::<Vec<User>>().await?;
println!("用户: {users:?}");
```

#### 6. Transaction

```rust
let txn = db.begin().await?;
txn.insert(&User {
    id: 0,
    name: "Bob".to_string(),
    age: 30,
    email: None,
}).await?;
txn.commit().await?;
```

### Features

- **turso** (default): Use Turso/libsql database backend
- **postgresql**: Use PostgreSQL database backend
- **mysql**: Use MySQL database backend

Enable features in your `Cargo.toml`:

```toml
[dependencies]
ormer = { version = "0.1", features = ["postgresql"] }
```