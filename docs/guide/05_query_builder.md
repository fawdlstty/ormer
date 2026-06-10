# 查询构建器

## 基本查询

```rust
let users: Vec<User> = db.select::<User>().collect().await?;

let user: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.eq(1))
    .range(..1)
    .collect()
    .await?;
```

## 过滤条件

### 比较运算

```rust
.filter(|u| u.name.eq("Alice".to_string()))
.filter(|u| u.age.ne(18))          // != 不等
.filter(|u| u.age.ge(18))          // >= 大于等于
.filter(|u| u.age.gt(18))          // > 大于
.filter(|u| u.age.le(65))          // <= 小于等于
.filter(|u| u.age.lt(65))          // < 小于
```

### LIKE 模糊匹配

```rust
.filter(|u| u.name.like("Al%"))           // 自定义模式
.filter(|u| u.name.contains("alice"))      // 包含
.filter(|u| u.name.starts_with("Al"))      // 以...开头
.filter(|u| u.name.ends_with("ce"))        // 以...结尾
```

可与其他条件组合：

```rust
.filter(|u| u.name.contains("li").and(u.age.gt(29)))
```

### NULL 判断

```rust
.filter(|u| u.email.is_null())       // IS NULL
.filter(|u| u.email.is_not_null())   // IS NOT NULL
```

### BETWEEN 范围

```rust
.filter(|u| u.age.between(18, 30))  // age BETWEEN 18 AND 30
```

### IN 与 NOT IN

```rust
.filter(|u| u.age.is_in(&vec![18, 20, 22]))
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))

.filter(|u| u.age.is_not_in(&vec![18, 20]))   // NOT IN
```

`is_in` 和 `is_not_in` 也支持子查询：

```rust
.filter(|u| u.id.is_in(db.select::<Role>().map_to(|r| r.user_id)))
.filter(|u| u.id.is_not_in(db.select::<Role>().map_to(|r| r.user_id)))
```

### 组合条件

```rust
.filter(|u| u.age.ge(18))
.filter(|u| u.age.le(65))

.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## 排序

```rust
.order_by(|u| u.name.asc())

.order_by_desc(|u| u.age)

.order_by(|u| u.age.desc())
.order_by(|u| u.name.asc())
```

## 分页

```rust
.range(0..10)
.range(10..20)
.range(..5)
.range(10..)
```

## 去重查询

```rust
// SELECT DISTINCT *
let users = db.select::<User>().distinct().collect().await?;

// SELECT DISTINCT name
let names: Vec<String> = db.select::<User>().distinct().map_to(|u| u.name).collect().await?;
```

可与 `filter`、`order_by`、`range` 等组合使用。

## 单条查询

```rust
// 等价于 range(..1)，只取第一条
let user: Option<User> = db.select::<User>().filter(|u| u.age.ge(18)).first().await?;
```

## 流式查询 (stream)

```rust
let mut stream = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .stream()
    .into_iter()
    .await?;

while let Some(user_result) = stream.next().await {
    let user = user_result?;
    println!("{:?}", user);
}
```

## 字段投影 (map_to)

```rust
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## 查询组合

```rust
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

let base_query = db.select::<User>().filter(|u| u.age.ge(18));

let adults_cn = base_query.clone()
    .filter(|u| u.country.eq("CN".to_string()))
    .collect::<Vec<_>>()
    .await?;

let adults_us = base_query
    .filter(|u| u.country.eq("US".to_string()))
    .collect::<Vec<_>>()
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
    
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, email: None },
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    let adults: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .collect()
        .await?;
    
    let sorted: Vec<User> = db
        .select::<User>()
        .order_by_desc(|u| u.age)
        .collect()
        .await?;
    
    let page: Vec<User> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .collect()
        .await?;
    
    let names: Vec<String> = db
        .select::<User>()
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```
