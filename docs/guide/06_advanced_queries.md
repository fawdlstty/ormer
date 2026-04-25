# 高级查询

本文档介绍 Ormer 的高级查询功能,包括聚合查询、JOIN 查询、多表关联和子查询。

## 聚合查询

聚合查询用于计算汇总数据,如计数、求和、平均值等。

### COUNT - 计数

```rust
let count: usize = db
    .select::<User>()
    .count(|u| u.id)
    .await?;

println!("Total users: {}", count);
```

带条件的计数:

```rust
let adult_count: usize = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .count(|u| u.id)
    .await?;
```

### SUM - 求和

```rust
let total_age: Option<i32> = db
    .select::<User>()
    .sum(|u| u.age)
    .await?;

if let Some(sum) = total_age {
    println!("Total age: {}", sum);
}
```

### AVG - 平均值

```rust
let avg_age: Option<f64> = db
    .select::<User>()
    .avg(|u| u.age)
    .await?;

if let Some(avg) = avg_age {
    println!("Average age: {:.2}", avg);
}
```

### MAX - 最大值

```rust
let max_age: Option<i32> = db
    .select::<User>()
    .max(|u| u.age)
    .await?;

if let Some(max) = max_age {
    println!("Max age: {}", max);
}
```

### MIN - 最小值

```rust
let min_age: Option<i32> = db
    .select::<User>()
    .min(|u| u.age)
    .await?;

if let Some(min) = min_age {
    println!("Min age: {}", min);
}
```

### 聚合查询示例

```rust
#[derive(Debug, Model)]
#[table = "products"]
struct Product {
    #[primary(auto)]
    id: i32,
    name: String,
    price: f64,
    stock: i32,
}

// 统计产品数量
let count: usize = db.select::<Product>().count(|p| p.id).await?;

// 计算总库存
let total_stock: Option<i32> = db.select::<Product>().sum(|p| p.stock).await?;

// 计算平均价格
let avg_price: Option<f64> = db.select::<Product>().avg(|p| p.price).await?;

// 找出最高价格
let max_price: Option<f64> = db.select::<Product>().max(|p| p.price).await?;

// 找出最低价格
let min_price: Option<f64> = db.select::<Product>().min(|p| p.price).await?;
```

## JOIN 查询

JOIN 查询用于从多个关联表中获取数据。

### LEFT JOIN

返回左表的所有记录,即使右表没有匹配:

```rust
#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Model)]
#[table = "roles"]
struct Role {
    #[primary]
    id: i32,
    user_id: i32,
    role_name: String,
}

// LEFT JOIN: 获取所有用户及其角色 (可能没有角色)
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

for (user, role) in &user_roles {
    match role {
        Some(r) => println!("{} has role: {}", user.name, r.role_name),
        None => println!("{} has no role", user.name),
    }
}
```

### INNER JOIN

只返回两个表都有匹配的记录:

```rust
let user_roles: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// 只有有角色的用户会被返回
for (user, role) in &user_roles {
    println!("{} has role: {}", user.name, r.role_name);
}
```

### RIGHT JOIN

返回右表的所有记录,即使左表没有匹配:

```rust
let user_roles: Vec<(Option<User>, Role)> = db
    .select::<User>()
    .right_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// 注意: SQLite 不支持 RIGHT JOIN
```

### JOIN 带过滤条件

```rust
let admin_users: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

## 多表关联查询

### 两表关联 (from)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from::<User, Role>()
    .filter(|u, r| u.id.eq(r.user_id))
    .filter(|_, r| r.role_name.eq("admin".to_string()))
    .collect()
    .await?;
```

### 三表关联 (from3)

```rust
#[derive(Debug, Model)]
#[table = "permissions"]
struct Permission {
    #[primary]
    id: i32,
    role_id: i32,
    permission_name: String,
}

let users: Vec<User> = db
    .select::<User>()
    .from3::<User, Role, Permission>()
    .filter(|u, r, p| u.id.eq(r.user_id).and(r.id.eq(p.role_id)))
    .filter(|_, _, p| p.permission_name.eq("read".to_string()))
    .collect()
    .await?;
```

### 四表关联 (from4)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from4::<User, Role, Permission, Department>()
    .filter(|u, r, p, d| {
        u.id.eq(r.user_id)
            .and(r.id.eq(p.role_id))
            .and(u.department_id.eq(d.id))
    })
    .collect()
    .await?;
```

## 子查询

子查询允许在一个查询中嵌套另一个查询。

### IN 子查询

```rust
// 查找有角色的用户
let subquery = db
    .select::<Role>()
    .map_to(|r| r.user_id);

let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(subquery))
    .collect()
    .await?;
```

### 复杂子查询

```rust
// 查找年龄大于平均年龄的用户
let avg_age_query = db
    .select::<User>()
    .avg(|u| u.age);

let older_users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.age.gt(avg_age_query.await? as i32))
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
}

#[derive(Debug, Model)]
#[table = "roles"]
struct Role {
    #[primary(auto)]
    id: i32,
    user_id: i32,
    role_name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect(DbType::Turso, "file:test.db").await?;
    db.create_table::<User>().await?;
    db.create_table::<Role>().await?;
    
    // 插入数据
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25 },
        User { id: 2, name: "Bob".to_string(), age: 30 },
        User { id: 3, name: "Charlie".to_string(), age: 35 },
    ]).await?;
    
    db.insert(&vec![
        Role { id: 1, user_id: 1, role_name: "admin".to_string() },
        Role { id: 2, user_id: 2, role_name: "user".to_string() },
    ]).await?;
    
    // 1. 聚合查询
    let count: usize = db.select::<User>().count(|u| u.id).await?;
    let avg_age: Option<f64> = db.select::<User>().avg(|u| u.age).await?;
    println!("Users: {}, Avg Age: {:.2}", count, avg_age.unwrap_or(0.0));
    
    // 2. LEFT JOIN
    let user_roles: Vec<(User, Option<Role>)> = db
        .select::<User>()
        .left_join::<Role>(|u, r| u.id.eq(r.user_id))
        .collect()
        .await?;
    
    for (user, role) in &user_roles {
        match role {
            Some(r) => println!("{}: {}", user.name, r.role_name),
            None => println!("{}: no role", user.name),
        }
    }
    
    // 3. 多表关联
    let admin_users: Vec<User> = db
        .select::<User>()
        .from::<User, Role>()
        .filter(|u, r| u.id.eq(r.user_id))
        .filter(|_, r| r.role_name.eq("admin".to_string()))
        .collect()
        .await?;
    
    println!("Admin users: {:?}", admin_users);
    
    // 4. 子查询
    let users_with_roles: Vec<User> = db
        .select::<User>()
        .filter(|u| u.id.is_in(
            db.select::<Role>().map_to(|r| r.user_id)
        ))
        .collect()
        .await?;
    
    println!("Users with roles: {:?}", users_with_roles);
    
    db.drop_table::<Role>().await?;
    db.drop_table::<User>().await?;
    
    Ok(())
}
```

## 最佳实践

### 1. 选择合适的 JOIN 类型

- **LEFT JOIN**: 需要左表所有记录,右表可选
- **INNER JOIN**: 只需要两个表都有匹配的记录
- **RIGHT JOIN**: 需要右表所有记录,左表可选

### 2. 避免过度使用多表关联

```rust
// ✅ 推荐: 使用 JOIN
let result = db.select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;

// ❌ 避免: 多次查询
let users = db.select::<User>().collect().await?;
for user in &users {
    let roles = db.select::<Role>()
        .filter(|r| r.user_id.eq(user.id))
        .collect()
        .await?;
}
```

### 3. 合理使用聚合查询

```rust
// ✅ 推荐: 使用聚合函数
let count: usize = db.select::<User>().count(|u| u.id).await?;

// ❌ 避免: 查询所有数据再计数
let users: Vec<User> = db.select::<User>().collect().await?;
let count = users.len();
```
