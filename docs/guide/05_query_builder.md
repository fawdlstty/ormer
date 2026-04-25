# 查询构建器

Ormer 提供强大的类型安全查询构建器,支持链式调用和编译期类型检查。

## 基本查询

### 查询所有记录

```rust
let users: Vec<User> = db
    .select::<User>()
    .collect::<Vec<_>>()
    .await?;
```

### 单条查询

```rust
// 查询第一个匹配的记录
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.eq(1))
    .range(..1)  // 只取第一条
    .collect()
    .await?;

if let Some(user) = users.into_iter().next() {
    println!("Found: {:?}", user);
}
```

## 过滤条件

### 比较运算符

```rust
// 等于
.filter(|u| u.name.eq("Alice".to_string()))

// 大于等于
.filter(|u| u.age.ge(18))

// 大于
.filter(|u| u.age.gt(18))

// 小于等于
.filter(|u| u.age.le(65))

// 小于
.filter(|u| u.age.lt(65))
```

### IN 查询

```rust
// 在集合中
.filter(|u| u.age.is_in(&vec![18, 20, 22, 25]))

// 字符串 IN
.filter(|u| u.name.is_in(&vec!["Alice".to_string(), "Bob".to_string()]))
```

### 组合条件

```rust
// AND 条件 (多个 filter)
db.select::<User>()
    .filter(|u| u.age.ge(18))
    .filter(|u| u.age.le(65))
    .collect()
    .await?;

// 使用 and() 组合
.filter(|u| u.age.ge(18).and(u.name.eq("Alice".to_string())))

// 使用 or() 组合
.filter(|u| u.age.lt(18).or(u.age.gt(65)))
```

## 排序

### 升序排序

```rust
db.select::<User>()
    .order_by(|u| u.name.asc())
    .collect()
    .await?;
```

### 降序排序

```rust
db.select::<User>()
    .order_by_desc(|u| u.age)
    .collect()
    .await?;
```

### 多字段排序

```rust
db.select::<User>()
    .order_by(|u| u.age.desc())
    .order_by(|u| u.name.asc())
    .collect()
    .await?;
```

## 分页

### 使用 range

```rust
// 前 10 条
.range(0..10)

// 第 2 页 (每页 10 条)
.range(10..20)

// 只要前 5 条
.range(..5)

// 从第 10 条开始到最后
.range(10..)
```

### 分页示例

```rust
fn get_page(db: &Database, page: usize, page_size: usize) {
    let start = page * page_size;
    let end = start + page_size;
    
    db.select::<User>()
        .order_by(|u| u.id.asc())
        .range(start..end)
        .collect::<Vec<_>>()
        .await
}

// 使用
let page1 = get_page(&db, 0, 10).await?;  // 第 1 页
let page2 = get_page(&db, 1, 10).await?;  // 第 2 页
```

## 字段投影 (map_to)

只查询需要的字段,提高性能:

### 单字段投影

```rust
// 只查询名字
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect::<Vec<String>>()
    .await?;

// 只查询 ID
let ids: Vec<i32> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect::<Vec<i32>>()
    .await?;
```

### 元组投影

```rust
// 二元组
let name_age: Vec<(String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.name, u.age))
    .collect()
    .await?;

// 三元组
let user_info: Vec<(i32, String, i32)> = db
    .select::<User>()
    .map_to(|u| (u.id, u.name, u.age))
    .collect()
    .await?;
```

### 转换为自定义 Model

```rust
#[derive(Debug, Model)]
#[table = "user_ids"]
struct UserId {
    #[primary]
    id: i32,
}

let user_ids: Vec<UserId> = db
    .select::<User>()
    .map_to(|u| u.id)
    .collect_with(|id| UserId { id })
    .await?;
```

## 查询构建器组合

### 链式调用

```rust
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .filter(|u| u.name.eq("Alice".to_string()))
    .order_by(|u| u.name.asc())
    .range(0..10)
    .collect()
    .await?;
```

### 复用查询

```rust
// 创建基础查询
let base_query = db
    .select::<User>()
    .filter(|u| u.age.ge(18));

// 复用并添加不同条件
let adults_in_china = base_query.clone()
    .filter(|u| u.country.eq("CN".to_string()))
    .collect::<Vec<_>>()
    .await?;

let adults_in_usa = base_query
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
    db.create_table::<User>().await?;
    
    // 插入测试数据
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25, email: None },
        User { id: 2, name: "Bob".to_string(), age: 30, email: None },
        User { id: 3, name: "Charlie".to_string(), age: 35, email: None },
        User { id: 4, name: "David".to_string(), age: 28, email: None },
    ]).await?;
    
    // 1. 基本查询
    let all: Vec<User> = db.select::<User>().collect().await?;
    println!("All users: {}", all.len());
    
    // 2. 条件查询
    let adults: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(18))
        .collect()
        .await?;
    println!("Adults: {}", adults.len());
    
    // 3. 排序查询
    let sorted: Vec<User> = db
        .select::<User>()
        .order_by_desc(|u| u.age)
        .collect()
        .await?;
    println!("Oldest: {:?}", sorted.first());
    
    // 4. 分页查询
    let page1: Vec<User> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .collect()
        .await?;
    println!("Page 1: {:?}", page1);
    
    // 5. 字段投影
    let names: Vec<String> = db
        .select::<User>()
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;
    println!("Names: {:?}", names);
    
    // 6. 组合查询
    let result: Vec<User> = db
        .select::<User>()
        .filter(|u| u.age.ge(25))
        .filter(|u| u.age.le(35))
        .order_by(|u| u.name.asc())
        .range(0..10)
        .collect()
        .await?;
    println!("Filtered: {:?}", result);
    
    db.drop_table::<User>().await?;
    Ok(())
}
```

## 最佳实践

### 1. 只查询需要的字段

```rust
// ✅ 推荐: 使用 map_to
let names: Vec<String> = db
    .select::<User>()
    .map_to(|u| u.name)
    .collect()
    .await?;

// ❌ 避免: 查询所有字段
let users: Vec<User> = db.select::<User>().collect().await?;
let names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
```

### 2. 合理使用索引

```rust
// 为经常过滤的字段添加索引
#[derive(Debug, Model)]
struct User {
    #[index]
    age: i32,
    
    #[index]
    status: String,
}
```

### 3. 避免 N+1 查询

```rust
// ✅ 推荐: 使用 IN 查询
let ids = vec![1, 2, 3, 4, 5];
let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(&ids))
    .collect()
    .await?;

// ❌ 避免: 循环查询
for id in ids {
    let user: Vec<User> = db
        .select::<User>()
        .filter(|u| u.id.eq(id))
        .collect()
        .await?;
}
```
