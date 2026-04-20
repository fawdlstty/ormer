# ormer

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Former%2Fmain%2F/ormer/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/ormer/rust.yml)

[English](README.md) | 简体中文

极简的Rust ORM框架。

# 用法

<!-- [在线文档](https://ormer.fawdlstty.com) -->

加入库的引用：

```sh
cargo add ormer
cargo add tokio --features full
```

#### 1. 定义模型

使用 `#[derive(Model)]` 宏来定义你的数据库模型：

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

#### 2. 连接数据库

```rust
use ormer::abstract_layer::Database;
use ormer::DbType;

#[tokio::main]
async fn main() {
    let db = Database::connect(DbType::Turso, ":memory:")
        .await
        .expect("连接数据库失败");
}
```

#### 3. 创建表

```rust
db.create_table::<User>().await.expect("创建表失败");
```

#### 4. 插入数据

```rust
let user = User {
    id: 0, // 将自动生成
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
};

db.insert(&user).await.expect("插入数据失败");
```

#### 5. 查询数据

```rust
let users: Vec<User> = db
    .select::<User>()
    .collect::<Vec<User>>()
    .await
    .expect("查询数据失败");

for user in users {
    println!("用户: {:?}", user);
}
```

#### 6. 事务

```rust
let txn = db.begin().await.expect("开启事务失败");

let user = User {
    id: 0,
    name: "Bob".to_string(),
    age: 30,
    email: None,
};

txn.insert(&user).await.expect("插入数据失败");
txn.commit().await.expect("提交事务失败");
```

### 特性 (Features)

- **turso** (默认): 使用 Turso/libsql 数据库后端
- **postgresql**: 使用 PostgreSQL 数据库后端
- **mysql**: 使用 MySQL 数据库后端

在 `Cargo.toml` 中启用特性：

```toml
[dependencies]
ormer = { version = "0.1", features = ["postgresql"] }
```