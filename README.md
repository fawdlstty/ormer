# ormer

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Former%2Fmain%2F/ormer/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/ormer/rust.yml)

English | [简体中文](README.zh.md)

A minimalist ORM framework that supports SQLite, PostgreSQL, MySQL, and SqlServer.

[Online Documentation](https://ormer.fawdlstty.com/en/)

## Quick Example

```rust
#[derive(Debug, ormer::Model)]
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
    // connect to database and create table
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<User>().execute().await?;

    // insert data
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .execute()
    .await?;

    // query data
    let users = db
        .select::<User>()
        .filter(|p| p.age.ge(18))
        .collect::<Vec<_>>()
        .await?;
    println!("users: {users:?}");

    Ok(())
}
```
