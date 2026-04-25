# 模型定义

模型是 Ormer 的核心,它定义了数据库表的结构和 Rust 类型之间的映射关系。

## 基本模型定义

使用 `#[derive(Model)]` 宏将 Rust 结构体定义为数据库模型:

```rust
use ormer::Model;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: String,
}
```

### 属性说明

- `#[table = "表名"]` - 指定数据库表名
- `#[primary]` - 标记主键字段
- `#[primary(auto)]` - 标记自增主键
- `#[unique]` - 唯一约束
- `#[index]` - 创建索引

## 字段属性详解

### 1. 主键字段

#### 普通主键

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary]
    id: i32,  // 需要手动指定值
    name: String,
}
```

#### 自增主键

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,  // 数据库自动生成
    name: String,
}
```

使用自增主键插入数据时:

```rust
// 方式1: 使用 Option<i32>
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: Option<i32>,
    name: String,
}

db.insert(&User {
    id: None,  // 数据库自动生成
    name: "Alice".to_string(),
}).await?;

// 方式2: 使用 i32,手动指定
db.insert(&User {
    id: 1,  // 手动指定,但数据库会忽略
    name: "Alice".to_string(),
}).await?;
```

### 2. 唯一约束

#### 单列唯一

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    
    #[unique]
    email: String,  // 邮箱必须唯一
}
```

#### 联合唯一约束

使用 `group` 参数创建联合唯一索引:

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
    // (user_id, role_id) 组合必须唯一
}
```

### 3. 索引

为经常查询的字段创建索引以提高性能:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    
    #[index]
    age: i32,  // 经常按年龄查询时添加索引
    
    #[index]
    created_at: String,
}
```

### 4. 可空字段

使用 `Option<T>` 表示可空字段:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    
    // 可选字段
    email: Option<String>,
    phone: Option<String>,
    age: Option<i32>,
}
```

插入数据时:

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    email: Some("alice@example.com".to_string()),
    phone: None,  // 没有手机号
    age: Some(25),
}).await?;
```

## 支持的字段类型

### 基本类型

| Rust 类型 | SQL 类型 (SQLite) | SQL 类型 (PostgreSQL) | SQL 类型 (MySQL) |
|-----------|-------------------|----------------------|------------------|
| `i32` | INTEGER | INTEGER | INT |
| `i64` | INTEGER | BIGINT | BIGINT |
| `f64` | REAL | DOUBLE | DOUBLE |
| `String` | TEXT | TEXT | TEXT |
| `bool` | INTEGER (0/1) | BOOLEAN | BOOLEAN |

### Option 类型

所有基本类型都可以包装在 `Option` 中:

```rust
Option<i32>
Option<i64>
Option<f64>
Option<String>
Option<bool>
```

## 完整示例

```rust
use ormer::Model;

#[derive(Debug, Model, Clone)]
#[table = "products"]
struct Product {
    #[primary(auto)]
    id: i32,
    
    #[unique]
    sku: String,  // 产品SKU,唯一
    
    name: String,
    price: f64,
    
    #[index]
    category_id: i32,  // 分类ID,添加索引
    
    #[index]
    stock: i32,  // 库存
    
    description: Option<String>,  // 可选描述
    is_active: bool,  // 是否上架
}
```

## 外键关系

虽然 Ormer 不强制使用外键约束,但你可以在模型中定义外键关系:

```rust
#[derive(Debug, Model)]
#[table = "posts"]
struct Post {
    #[primary(auto)]
    id: i32,
    
    #[foreign_key = "users(id)"]
    user_id: i32,  // 外键引用 users 表的 id
    
    title: String,
    content: String,
}
```

## 模型 Trait 方法

定义模型后,`#[derive(Model)]` 宏会自动实现以下方法:

### query() / select()

创建查询构建器:

```rust
let query = User::query();
let select = User::select();
```

### from_row()

从数据库行构建模型:

```rust
let user = User::from_row(&row)?;
```

### field_values()

获取字段值列表 (用于 INSERT/UPDATE):

```rust
let values = user.field_values();
```

### primary_key_column() / primary_key_value()

获取主键信息:

```rust
let pk_column = User::primary_key_column();  // "id"
let pk_value = user.primary_key_value();     // 实际值
```

## 创建表

使用 `create_table` 方法创建表：

```rust
db.create_table::<User>().await?;
```

这会生成类似如下的 SQL：

```sql
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    age INTEGER,
    email TEXT UNIQUE
)
```

**注意**：`create_table` 方法只负责创建表，不会验证表结构。如果表已存在，可能会报错。

## 验证表结构

使用 `validate_table` 方法验证表结构是否与模型定义匹配：

```rust
// 验证表结构
db.validate_table::<User>().await?;
```

该方法会检查：
- 表是否存在
- 列数量是否匹配
- 列名是否匹配
- 列类型是否兼容
- NOT NULL 约束是否匹配
- 主键约束是否匹配

如果验证失败，会返回 `SchemaMismatch` 错误。

## 删除表

```rust
db.drop_table::<User>().await?;
```

生成 SQL:

```sql
DROP TABLE IF EXISTS users
```

## 最佳实践

### 1. 为模型派生常用 Trait

```rust
#[derive(Debug, Model, Clone, PartialEq)]
struct User {
    // ...
}
```

- `Debug` - 调试输出
- `Clone` - 克隆支持
- `PartialEq` - 比较支持 (测试时有用)

### 2. 使用有意义的表名

```rust
// ✅ 推荐: 复数形式
#[table = "users"]
#[table = "blog_posts"]
#[table = "user_roles"]

// ❌ 避免: 单数或不一致
#[table = "user"]
#[table = "User"]
```

### 3. 合理使用索引

```rust
// ✅ 为经常查询的字段添加索引
#[index]
status: String,

#[index]
created_at: String,

// ❌ 不要为所有字段添加索引
```

### 4. 使用 Option 表示可选数据

```rust
// ✅ 明确表达字段可以为空
email: Option<String>,

// ❌ 使用空字符串表示无值
email: String,  // 可能需要 "" 表示空
```
