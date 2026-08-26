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
- `#[embed(prefix = "前缀_")]` - 嵌入值对象，并把字段展开为带前缀的列
- `#[data_type(i64)]` - 数据库类型覆盖（如 Rust 字段为 i32 但数据库使用 BIGINT）
- `#[hypertable(Duration::from_secs(86400))]` - TimescaleDB 超表时间分片时长
- `#[hypertable]` - 标注 `String` 字段作为 PostgreSQL/TimescaleDB 字符串拆表键
- `#[compress]` - 列压缩，默认使用 PostgreSQL `pglz`
- `#[compress(lz4)]` - 指定压缩算法；PostgreSQL 按列生成 `COMPRESSION lz4`，MySQL 按表生成 `COMPRESSION='LZ4'`
- `#[filter(filter_name, |m, ...| ...)]` - 模型级可复用过滤器，名称必须以 `filter_` 开头
- `#[version(u64)]` - 自动添加 `version` 列，用于乐观锁
- `#[ormer_ignore]` - 字段不映射为数据库列，可用于动态表路由值

PostgreSQL 和 MSSQL 会保留 `#[table = "schema.table"]` 中的 schema 前缀；SQLite 和 MySQL 会使用最后一段表名。

## DbFirst 生成实体

```rust
let code = db.generate_entities(None).await?;
```

PostgreSQL 可传 `Some("public")`，MSSQL 可传 `Some("dbo")`；ClickHouse 使用数据库名作为 schema；
未指定时使用后端默认 schema。DuckDB 和 ClickHouse 会根据实际列类型生成对应的 Rust 字段类型。

## 乐观锁版本列

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

`#[version(u64)]` 会创建不可见的 `version` 列，初始值为 `1`。从数据库读取模型后，`version()` 返回当前版本；`set_model` 更新会自动追加版本条件并把版本加一。

## 模型级过滤器

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

`scope()` 上启用的过滤器会继承到查询、关系加载、更新和删除；`unset_filter_*()` 只关闭继承的同名过滤器，不移除当前查询手写的 `filter(...)`。

## 动态表路由

表名可以包含 `{变量}` 占位符，查询时用 `route_table` 指定值；写入模型时会从同名字段自动取值。

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

如果路由值不需要数据库列，使用 `#[ormer_ignore]`：

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

TimescaleDB 可用无参 `#[hypertable]` 标注 `String` 字段，让 PostgreSQL 按字段值拆成不同物理表；SQLite、MySQL、MSSQL 不启用该自动拆表。路由键使用字段的 SQL 列名（未配置 `#[column]` 时就是字段名），写入时自动从模型字段取值，查询和建表时显式传入 route。

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

## 嵌入值对象

值对象可以派生 `Embed`，再在模型字段上使用 `#[embed(prefix = "...")]` 展开为多列：

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

## 支持的类型

| Rust 类型 | SQL 类型 (SQLite) | SQL 类型 (PostgreSQL) | SQL 类型 (MySQL) | SQL 类型 (MSSQL) |
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

所有基本类型都可使用 `Option<T>` 包装为可空字段。

UUID 字段可以直接使用 `uuid::Uuid` 或 `Option<uuid::Uuid>`：

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

UUID 值由应用层生成，例如 `uuid::Uuid::new_v4()`；应用需要自行启用 `uuid` crate 的 `v4` feature，ORM 不会自动生成 UUID。SQLite 使用 `TEXT`，MySQL 使用 `CHAR(36)` 保存规范 UUID 字符串，MSSQL 使用原生 `UNIQUEIDENTIFIER`。

## 字段类型

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

`FieldType` 可用于枚举，也可用于单字段 tuple struct 包装类型。包装类型使用内部字段类型映射数据库列，例如 `ExceptionType(pub u16)` 按 `u16` 存储。支持 `Option<FieldType>` 表示可空字段。

`FieldType` 值也可以用于 `IN`、比较和排序条件：

```rust
let active: Vec<User> = db
    .select::<User>()
    .filter(|u| u.status.is_in([UserStatus::Active, UserStatus::Banned]))
    .collect()
    .await?;
```

带具名字段的 `ModelEnum` 可作为模型内的多态字段。字段本身的列保存鉴别器值，变体字段会平铺为同一张表的可空列：

```rust
#[derive(Debug, Model)]
#[table = "documents"]
struct Document {
    #[primary]
    id: i64,
    title: String,
    body: DocumentBody,
}

#[derive(Debug, Clone, PartialEq, ormer::ModelEnum)]
#[db_type(String)]
enum DocumentBody {
    Article {
        article_body: String,
        article_word_count: i32,
    },
    Video {
        video_url: String,
        video_duration_seconds: i32,
    },
}
```

`#[db_type(String)]` 使用 snake_case 变体名作为鉴别器值，例如 `Article` 存为 `"article"`。`Document::columns()` 会包含 `body`、`article_body`、`article_word_count`、`video_url` 和 `video_duration_seconds`；读写 `Document` 时会按 `body` 列自动分发到对应 enum 变体。

如果已有数值枚举或包装类型且不想派生 `FieldType`，可以用 `#[data_type(i32)]` 指定数据库类型。可空字段必须同时使用 `#[data_type(Option<i32>)]`：

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

外键字段只描述数据库约束；需要加载关联模型时，可使用 `#[has_many]`、`#[belongs_to]`、`#[has_one]` 和 `#[through]`：

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

关系字段不会成为数据库列；`#[belongs_to]` 和 `#[has_one]` 字段必须是 `Option<T>`，`#[has_many]` 和常见 `#[through]` 字段使用 `Vec<T>`。`#[through(user_roles.role)]` 会沿用本模型的 `user_roles` 关系和中间模型上的 `role` 关系。

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

通过 `primary_field_names()` 获取 Rust 主键字段名列表，`model.promary_fields()` 获取当前主键字段值元组；通过 `primary_key_columns()` 获取 SQL 主键列名列表。复合主键按字段声明顺序返回。

## 表操作

### 创建表

```rust
db.create_table::<User>().execute().await?;
```

### 验证表

```rust
db.validate_table::<User>().await?;
```

`validate_table` 会检查列数量、顺序、名称、类型、可空性、主键、自增属性、唯一约束、索引和外键；PostgreSQL 模型还会检查 TimescaleDB 超表及时间分片间隔。

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
