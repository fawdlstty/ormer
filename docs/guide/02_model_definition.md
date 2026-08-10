# 模型定义

## 基本定义

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

### 属性

- `#[table = "表名"]` - 指定表名
- `#[table(schema = "模式", name = "表名")]` - 独立指定 schema 和表名
- `#[column(name = "列名")]` - 指定 SQL 列名
- `#[primary]` - 主键（支持复合主键）
- `#[primary(auto)]` - 自增主键（仅单主键或复合主键的第一个字段）
- `#[unique]` - 唯一约束（支持 `group`、`name` 参数）
- `#[index]` - 索引（支持 `group`、`name`、`order`、`where` 参数）
- `#[default(...)]` - 数据库默认值；SQL 表达式使用 `#[default(expr = "...")]`
- `#[check(expr = "...")]` - CHECK 约束，可配置 `name`
- `#[foreign(Type)]` - 外键关系；可配置 `name`、`on_delete`、`on_update`
- `#[data_type(i64)]` - 数据库类型覆盖（如 Rust 字段为 i32 但数据库使用 BIGINT）
- `#[hypertable(Duration::from_secs(86400))]` - TimescaleDB 超表分片时长
- `#[compress]` - PostgreSQL 列级压缩（生成 `COMPRESSION pglz`）

PostgreSQL 和 MSSQL 会保留 `#[table = "schema.table"]` 中的 schema 前缀；SQLite 和 MySQL 会使用最后一段表名。

## 字段属性

### 唯一约束

#### 单列唯一

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

#### 联合唯一

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

### 索引

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

### 可空字段

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

### 列名、默认值和约束

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

`insert(&model)` 仍会显式写入模型的全部字段；数据库默认值只会在 INSERT 省略列时生效，可用 `insert_partial` 或 `insert_model` 省略列。

## 支持的类型

| Rust 类型 | SQL 类型 (SQLite) | SQL 类型 (PostgreSQL) | SQL 类型 (MySQL) | SQL 类型 (MSSQL) |
|-----------|-------------------|----------------------|------------------|------------------|
| `i32` | INTEGER | INTEGER | INT | INT |
| `i64` | INTEGER | BIGINT | BIGINT | BIGINT |
| `f64` | REAL | DOUBLE | DOUBLE | FLOAT |
| `String` | TEXT | TEXT | TEXT | NVARCHAR(255) |
| `bool` | INTEGER (0/1) | BOOLEAN | BOOLEAN | BIT |
| `Vec<u8>` | BLOB | BYTEA | BLOB | VARBINARY(MAX) |
| `chrono::DateTime<chrono::Utc>` | TEXT | TIMESTAMPTZ | DATETIME | DATETIME2 |
| `chrono::NaiveDateTime` | TEXT | TIMESTAMPTZ | DATETIME | DATETIME2 |
| `chrono::NaiveDate` | TEXT | DATE | DATE | DATE |
| `chrono::NaiveTime` | TEXT | TIME | TIME | TIME |

所有基本类型都可使用 `Option<T>` 包装为可空字段。

## 枚举类型

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

支持 `Option<EnumType>` 表示可空枚举字段。

`ModelEnum` 枚举也可以用于 `IN`、比较和排序条件：

```rust
let active: Vec<User> = db
    .select::<User>()
    .filter(|u| u.status.is_in([UserStatus::Active, UserStatus::Banned]))
    .collect()
    .await?;
```

如果已有数值枚举或包装类型，不需要派生 `ModelEnum`，可以用 `#[data_type(i32)]` 指定数据库类型。可空字段必须同时使用 `#[data_type(Option<i32>)]`：

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

### PostgreSQL 数组

PostgreSQL 支持 `Vec<i32>`、`Vec<i64>`、`Vec<Option<i64>>` 和 `Vec<String>`。这些字段映射到原生数组类型；`Vec<String>` 使用 `TEXT[]`，不是 JSON 字段：

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

对 `Vec<T>` 中的自定义数值枚举，可以使用 `#[data_type(Vec<i32>)]`：

```rust
#[data_type(Vec<i32>)]
roles: Vec<Status>,
```

数组类型和上述写法目前用于 PostgreSQL 后端。

## 完整示例

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

## 外键关系

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

## 模型关系

外键字段只描述数据库约束；需要加载关联模型时，可使用 `#[has_many]` 和 `#[belongs_to]`：

```rust
#[derive(Debug, Clone, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    #[has_many(Post.user_id)]
    posts: Vec<Post>,
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

关系字段不会成为数据库列；`#[belongs_to]` 字段必须是 `Option<T>`，`#[has_many]` 字段必须是 `Vec<T>`。

## 复合主键

为多个字段添加 `#[primary]` 即可定义复合主键：

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

只有第一个主键字段可使用 `auto`：
```rust
#[primary(auto)]
id: i32,
#[primary]
product_id: i32,
```

通过 `primary_key_columns()` 获取主键列名列表。

## 表操作

### 创建表

```rust
db.create_table::<User>().execute().await?;
```

### 验证表

```rust
db.validate_table::<User>().await?;
```

### 删除表

```rust
db.drop_table::<User>().execute().await?;
```

## 模型包装器

```rust
// 基础模型
#[derive(Debug, Model, Clone)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

// 包装器 - 使用不同表名
#[derive(Debug, Model)]
#[table = "archive_users"]
struct ArchiveUser(User);

#[derive(Debug, Model)]
#[table = "temp_users"]
struct TempUser(User);
```

### 使用示例

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
