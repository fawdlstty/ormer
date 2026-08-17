# 数据操作

## 插入 (Create)

### 单条插入

```rust
db.insert(&User {
    id: 1,
    name: "Alice".to_string(),
    age: 25,
    email: Some("alice@example.com".to_string()),
})
.execute()
.await?;
```

> `execute()` 返回自增ID：若模型主键标注了 `#[primary(auto)]`，`execute()` 返回自动生成的ID（如 `i32`），而非影响行数。

### 插入并返回 (RETURNING)

插入后返回所有插入的行数据（支持 PostgreSQL、SQLite）：

```rust
let users: Vec<User> = db.insert(&vec![user1, user2]).returning().await?;
```

### 批量插入

```rust
db.insert(&vec![user1, user2, user3])
    .execute()
    .await?;

db.insert(&[user1, user2])
    .execute()
    .await?;
```

大批量插入会按后端参数上限自动拆批；PostgreSQL 在无冲突处理、无自增键返回且值类型可安全序列化时会自动使用 `COPY FROM STDIN`。

### 部分插入和表单模型

`insert_partial::<User>()` 只写入通过 `set` 指定的列；`default` 表示 INSERT 时省略该列以使用数据库默认值：

```rust
db.insert_partial::<User>()
    .set(|u| u.name.set("Alice"))
    .set(|u| (u.email, Some("alice@example.com".to_string())))
    .default(|u| u.created_at)
    .execute()
    .await?;
```

表单结构可派生 `InsertModel`，用 `insert_model::<User>(&value)` 插入。`ActiveValue::not_set()` 会省略列，`set(value)` 和 `unchanged(value)` 会写入该值：

```rust
#[derive(ormer::InsertModel)]
#[table = "users"]
struct NewUser {
    name: String,
    email: Option<String>,
    created_at: ormer::ActiveValue<chrono::NaiveDateTime>,
}

let new_user = NewUser {
    name: "Bob".to_string(),
    email: None,
    created_at: ormer::ActiveValue::not_set(),
};

db.insert_model::<User>(&new_user)
    .execute()
    .await?;
```

### 插入或更新

```rust
db.insert_or_update(&user)
    .execute()
    .await?;

db.insert_or_update(&vec![user1, user2])
    .execute()
    .await?;
```

`upsert` 是 `insert_or_update` 的短别名，仍按主键处理冲突：

```rust
db.upsert(&user).execute().await?;
```

### 对象图插入与更新

`insert_graph` 会在一个事务内先插入根对象，再处理非空关系集合：

```rust
db.insert_graph(&mut user).execute().await?;
```

`update_graph` 会更新根对象，并对非空 `has_many`、`has_one`、`through` 关系执行 upsert 或中间表同步：

```rust
db.update_graph(&mut user).execute().await?;
```

空 `Vec` 默认表示本次不处理该关系，不会清空已有关系。

### 可配置插入冲突

在 `insert()` 上指定唯一键冲突目标、更新字段和更新条件：

```rust
db.insert(&user)
    .on_conflict(|u| u.email)
    .do_update_if(|u| u.active.eq(true))
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;

db.insert(&user)
    .on_conflict(|u| u.email)
    .do_nothing()
    .execute()
    .await?;

db.insert(&membership)
    .on_conflict(|m| (m.org_id, m.user_id))
    .do_update()
    .set(|m| m.role = m.role.incoming())
    .execute()
    .await?;

db.insert(&user)
    .on_conflict(|u| u.email)
    .conflict_where(|u| u.deleted_at.is_null())
    .do_update()
    .set(|u| u.name = u.name.incoming())
    .execute()
    .await?;
```

PostgreSQL 可使用命名约束：

```rust
db.insert(&user)
    .on_constraint("users_email_key")
    .do_nothing()
    .execute()
    .await?;
```

MySQL 会映射到 `ON DUPLICATE KEY UPDATE` 或 `INSERT IGNORE`，不能指定具体唯一键、部分索引或 `DO UPDATE WHERE`。

### 插入或忽略

存在重复主键时静默忽略：

```rust
db.insert_or_ignore(&user)
    .execute()
    .await?;

db.insert_or_ignore(&vec![user1, user2])
    .execute()
    .await?;
```

## 查询 (Read)

```rust
let all: Vec<User> = db.select::<User>().collect().await?;

let adults: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .collect()
    .await?;

let page: Vec<User> = db
    .select::<User>()
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// 只取第一条
let first: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

### 根据主键查找

支持单主键和复合主键：

```rust
// 单主键
let user: Option<User> = db.find_by_id::<User>(1).await?;

// 复合主键
let item: Option<OrderItem> = db.find_by_id::<OrderItem>((1, 100)).await?;
```

也可在事务中使用：

```rust
let txn = db.begin().await?;
let user: Option<User> = txn.find_by_id::<User>(1).await?;
txn.commit().await?;
```

## 更新 (Update)

```rust
let count = db
    .update::<User>()
    .filter(|u| u.age.ge(18))
    .set(|u| u.name = u.name.set("Adult".to_string()))
    .execute()
    .await?;

db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.name = u.name.set("New Name".to_string()))
    .set(|u| u.age = u.age.set(26))
    .execute()
    .await?;

db.update::<User>()
    .filter(|u| u.id.eq(1))
    .set(|u| u.age += 1)
    .execute()
    .await?;

// 使用模型实例更新（自动跳过主键字段）
db.update::<User>()
    .set_model(&updated_user)
    .execute()
    .await?;

// 带 #[version(u64)] 的模型会自动校验版本并递增 version

// 只更新模型中的指定字段，避免覆盖其他列
db.update::<User>()
    .set_model_fields(&updated_user, |u| (u.name, u.age))
    .execute()
    .await?;
```

`set_model(&users)` 和 `set_model_fields(&users, ...)` 接收模型集合时，会在无乐观锁冲突定位需求且主键不重复的场景下自动生成单 SQL 差异更新；不满足条件时保持逐条更新语义。

### 变更跟踪保存

从数据库加载模型后调用 `track()` 可记录当前快照，`save()` 只更新发生变化的非主键字段；没有变更时返回 `0` 且不执行 UPDATE：

```rust
let mut user = db.find_by_id::<User>(1).await?.unwrap().track();
user.name = "New Name".to_string();
user.email = Some("new@example.com".to_string());

db.save(&mut user).execute().await?;
```

## 删除 (Delete)

```rust
let count = db
    .delete::<User>()
    .filter(|u| u.age.lt(18))
    .execute()
    .await?;

db.delete::<User>().execute().await?;

// 按模型删除；带 #[version(u64)] 时自动追加主键和版本条件
db.delete::<User>()
    .model(&user)
    .execute()
    .await?;
```

## 表管理

```rust
db.create_table::<User>().execute().await?;

db.drop_table::<User>().execute().await?;
```

## Typed DSL raw 表达式

`filter`、`order_by` / `order_by_desc`、`map_to` 中可以用 `ormer::raw!` 写数据库函数或方言表达式。`{...}` 内的字段会渲染为列引用，普通变量和字面量会走参数绑定；字面量花括号用 `{{` / `}}` 转义。

```rust
let term = "%alice%";

let names: Vec<String> = db
    .select::<User>()
    .filter(|u| ormer::raw!("LOWER({u.name}) LIKE LOWER({term})"))
    .order_by_desc(|u| ormer::raw!("LENGTH({u.name})"))
    .map_to(|u| {
        ormer::raw!("LOWER({u.name})")
            .typed::<String>()
            .alias("name_lower")
    })
    .collect()
    .await?;

db.update::<User>()
    .set(|u| u.name = u.name.set_expr(ormer::raw!("LOWER({u.name})")))
    .execute()
    .await?;
```

## 原生 SQL

```rust
let users: Vec<User> = db
    .select_sql::<User>("SELECT * FROM users WHERE age >= 18")
    .collect::<Vec<User>>()
    .await?;

let affected = db
    .execute_sql("UPDATE users SET name = 'Test' WHERE id = 1")
    .await?;
```

需要参数时使用 `sql!`、`bind` 或 `bind_named`，不要拼接用户输入：

```rust
db.execute_sql(ormer::sql!(
    "UPDATE users SET name = {name} WHERE id = {id}",
    name = "Alice".to_string(),
    id = 1,
))
.await?;

let users: Vec<User> = db
    .select_sql::<User>(
        ormer::sql("SELECT * FROM users WHERE age >= :age")
            .bind_named("age", 18),
    )
    .collect()
    .await?;
```

`{}` 是位置参数，`{name}` 和 `:name` 是命名参数；渲染时会根据后端转换为 `?` 或 `$1` 等占位符，字符串和注释中的类似文本不会被替换。事务和连接池连接也提供相同的 `select_sql` 与 `execute_sql` 方法。

如果 SQL 中包含方言语法或需要保留字面量中的冒号，可使用 `RawSql::plain()` 禁用占位符解析：

```rust
let users: Vec<User> = db
    .select_sql::<User>(ormer::RawSql::plain("SELECT * FROM users WHERE note = ':literal'"))
    .collect()
    .await?;
```

## 完整示例

```rust
use ormer::{Database, DbType, Model};

#[derive(Debug, Model)]
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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .execute()
    .await?;
    
    db.insert(&vec![
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ])
    .execute()
    .await?;
    
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    db.update::<User>()
        .filter(|u| u.id.eq(1))
        .set(|u| u.age = u.age.set(26))
        .execute()
        .await?;
    
    db.delete::<User>()
        .filter(|u| u.id.eq(3))
        .execute()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## 钩子系统 (Hooks)

Ormer 提供了钩子系统，允许您在写操作前后执行可失败的自定义逻辑。插入会自动触发已实现的 Hook；更新和删除需要使用 `execute_with_hooks` 或 `execute_models_with_hooks` 显式提供 Hook 对象。

- `BeforeInsert` / `AfterInsert` - 插入前后
- `BeforeUpdate` / `AfterUpdate` - 更新前后
- `BeforeDelete` / `AfterDelete` - 删除前后

### 使用示例

```rust
use ormer::{AfterInsert, BeforeInsert, BeforeUpdate, HookContext, Model};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        let now = chrono::Utc::now();
        self.created_at = now;
        self.updated_at = now;
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl BeforeUpdate for User {
    async fn before_update(&mut self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        self.updated_at = chrono::Utc::now();
        Ok(())
    }
}
```

详细文档请参考：[钩子系统](09_hooks.md)
