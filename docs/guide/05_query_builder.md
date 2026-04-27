# 查询构建器

## 基本查询

```rust
// 查询所有
let users: Vec<User> = db.select::<User>().collect().await?;

// 单条查询
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
.filter(|u| u.name.eq("Alice".to_string()))  // 等于
.filter(|u| u.age.ge(18))                    // 大于等于
.filter(|u| u.age.gt(18))                    // 大于
.filter(|u| u.age.le(65))                    // 小于等于
.filter(|u| u.age.lt(65))                    // 小于
```

### IN 查询

```rust
.filter(|u| u.age.is_in(&vec![18, 20, 22]))
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))
```

### 组合条件

```rust
// AND
.filter(|u| u.age.ge(18))
.filter(|u| u.age.le(65))

// 或使用 and()/or()
.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## 排序

```rust
// 升序
.order_by(|u| u.name.asc())

// 降序
.order_by_desc(|u| u.age)

// 多字段
.order_by(|u| u.age.desc())
.order_by(|u| u.name.asc())
```

## 分页

```rust
.range(0..10)    // 前10条
.range(10..20)   // 第2页
.range(..5)      // 前5条
.range(10..)     // 从第10条开始
```

## 流式查询 (stream)

当需要处理大量数据时，流式查询可以逐行获取结果，避免一次性加载到内存：

```rust
// 基础流式查询
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

### 流式查询特点

- **内存友好**: 逐行获取数据，适合大数据集查询
- **异步迭代**: 支持异步逐行处理，不会阻塞线程
- **支持所有查询选项**: 可配合 filter、order_by、range 等使用

### 带过滤和排序的流式查询

```rust
let mut stream = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by_desc(|u| u.age)
    .range(0..100)
    .stream()
    .into_iter()
    .await?;

while let Some(user_result) = stream.next().await {
    let user = user_result?;
    // 处理每一行数据
}
```

## 字段投影 (map_to)

```rust
// 单字段
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

// 元组
let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

// 转换为自定义类型
let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## 查询组合

```rust
// 链式调用
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;

// 复用查询
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
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().execute().await?;
    
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, email: None },
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
    ]).await?;
    
    // 基本查询
    let all: Vec<User> = db.select::<User>().collect().await?;
    
    // 条件查询
    let adults: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .collect()
        .await?;
    
    // 排序
    let sorted: Vec<User> = db
        .select::<User>()
        .order_by_desc(|u| u.age)
        .collect()
        .await?;
    
    // 分页
    let page: Vec<User> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .collect()
        .await?;
    
    // 字段投影
    let names: Vec<String> = db
        .select::<User>()
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;
    
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## 性能提示

### 字段投影

只查询需要的字段可减少数据传输：

```rust
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect()
    .await?;
```

### 避免 N+1 查询

使用 IN 查询替代循环查询：

```rust
// 使用 IN 查询
let ids = vec![1, 2, 3, 4, 5];
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(&ids))
    .collect()
    .await?;
```
