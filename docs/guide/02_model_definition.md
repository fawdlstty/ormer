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
- `#[primary]` - 主键（支持复合主键）
- `#[primary(auto)]` - 自增主键（仅单主键或复合主键的第一个字段）
- `#[unique]` - 唯一约束（支持 `group` 参数创建联合唯一）
- `#[index]` - 索引
- `#[foreign(Type)]` - 外键关系
- `#[data_type(i64)]` - 数据库类型覆盖（如 Rust 字段为 i32 但数据库使用 BIGINT）
- `#[hypertable(Duration::from_secs(86400))]` - TimescaleDB 超表分片时长
- `#[compress]` - PostgreSQL 列级压缩（生成 `COMPRESSION pglz`）

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

## 支持的类型

| Rust 类型 | SQL 类型 (SQLite) | SQL 类型 (PostgreSQL) | SQL 类型 (MySQL) | SQL 类型 (MSSQL) |
|-----------|-------------------|----------------------|------------------|------------------|
| `i32` | INTEGER | INTEGER | INT | INT |
| `i64` | INTEGER | BIGINT | BIGINT | BIGINT |
| `f64` | REAL | DOUBLE | DOUBLE | FLOAT |
| `String` | TEXT | TEXT | TEXT | NVARCHAR(255) |
| `bool` | INTEGER (0/1) | BOOLEAN | BOOLEAN | BIT |
| `Vec<u8>` | BLOB | BYTEA | BLOB | VARBINARY(MAX) |

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


