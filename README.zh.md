# ormer

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Former%2Fmain%2F/ormer/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/ormer/rust.yml)

[English](README.md) | 简体中文

一款极简语法的ORM框架，支持Sqlite、PostgreSQL、MySQL、SqlServer。

[在线文档](https://ormer.fawdlstty.com/)

## 快速示例

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
    // 连接数据库并创建表
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<User>().execute().await?;

    // 插入数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .execute()
    .await?;

    // 查询数据
    let users = db
        .select::<User>()
        .filter(|p| p.age.ge(18))
        .collect::<Vec<_>>()
        .await?;
    println!("users: {users:?}");

    Ok(())
}
```
