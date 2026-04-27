# 高级查询

## 聚合查询

```rust
// COUNT
let count: usize = db.select::<User>().count(|u| u.id).await?;

// SUM
let total: Option<i32> = db.select::<Product>().sum(|p| p.price).await?;

// AVG
let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

// MAX
let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

// MIN
let min: Option<i32> = db.select::<User>().min(|u| u.age).await?;
```

带条件:

```rust
let adult_count: usize = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .count(|u| u.id)
    .await?;
```

## GROUP BY 和 HAVING 查询

### 基础分组

```rust
use ormer::Select;

// 统计每个年龄段的用户数量
let sql = Select::<User>::new()
    .select_column(|u| u.id.count())
    .group_by(|u| u.age)
    .to_sql();

// 生成 SQL: SELECT COUNT(id) FROM users GROUP BY age
```

### 多字段选择 + 分组

```rust
// 选择部门和用户数量
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .to_sql();

// 生成 SQL: SELECT department, COUNT(id) FROM users GROUP BY department
```

### HAVING 条件过滤

```rust
// 统计用户数量大于 5 的部门
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(5))
    .to_sql();

// 生成 SQL: SELECT department, COUNT(id) FROM users GROUP BY department HAVING COUNT(id) > ?
```

### 多字段分组

```rust
// 按部门和年龄分组，统计每组的数量和平均分
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.age, u.id.count(), u.score.avg()))
    .group_by(|u| (u.department, u.age))
    .to_sql();

// 生成 SQL: SELECT department, age, COUNT(id), AVG(score) FROM users GROUP BY department, age
```

### 完整查询：WHERE + GROUP BY + HAVING + ORDER BY + LIMIT

```rust
let sql = Select::<User>::new()
    .filter(|u| u.age.ge(18))
    .select_column(|u| (u.department, u.id.count(), u.score.avg()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(0))
    .order_by(|u| u.department)
    .range(0..10)
    .to_sql();

// 生成 SQL:
// SELECT department, COUNT(id), AVG(score)
// FROM users
// WHERE age >= ?
// GROUP BY department
// HAVING COUNT(id) > ?
// ORDER BY department ASC
// LIMIT 10
```

### 支持的聚合函数

- `count()` - 计数，返回 `usize`
- `sum()` - 求和，返回原类型（数值类型）
- `avg()` - 平均值，返回 `f64`
- `max()` - 最大值，返回原类型（数值类型）
- `min()` - 最小值，返回原类型（数值类型）

### 注意事项

- `group_by()` 方法可以在 `having()` 之前或之后调用，但推荐先 `group_by()` 后 `having()`
- `filter()` 添加的是 WHERE 条件（分组前过滤），`having()` 添加的是 HAVING 条件（分组后过滤）
- 所有聚合函数生成的 SQL 使用参数化查询，保证安全性

## JOIN 查询

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
```

### LEFT JOIN

```rust
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### INNER JOIN

```rust
let user_roles: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### RIGHT JOIN

```rust
let user_roles: Vec<(Option<User>, Role)> = db
    .select::<User>()
    .right_join::<Role>(|u, r| u.id.eq(r.user_id))
    .collect()
    .await?;
```

### JOIN 带过滤

```rust
let admin_users: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

## 多表关联

### 两表 (from)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from::<User, Role>()
    .filter(|u, r| u.id.eq(r.user_id))
    .filter(|_, r| r.role_name.eq("admin".to_string()))
    .collect()
    .await?;
```

### 三表 (from3)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from3::<User, Role, Permission>()
    .filter(|u, r, p| u.id.eq(r.user_id).and(r.id.eq(p.role_id)))
    .collect()
    .await?;
```

### 四表 (from4)

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

### IN 子查询

```rust
let subquery = db.select::<Role>().map_to(|r| r.user_id);

let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(subquery))
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
    db.create_table::<User>().execute().await?;
    db.create_table::<Role>().execute().await?;
    
    db.insert(&vec![
        User { id: 1, name: "Alice".to_string(), age: 25 },
        User { id: 2, name: "Bob".to_string(), age: 30 },
    ]).await?;
    
    db.insert(&vec![
        Role { id: 1, user_id: 1, role_name: "admin".to_string() },
        Role { id: 2, user_id: 2, role_name: "user".to_string() },
    ]).await?;
    
    // 聚合查询
    let count: usize = db.select::<User>().count(|u| u.id).await?;
    let avg_age: Option<f64> = db.select::<User>().avg(|u| u.age).await?;
    
    // LEFT JOIN
    let user_roles: Vec<(User, Option<Role>)> = db
        .select::<User>()
        .left_join::<Role>(|u, r| u.id.eq(r.user_id))
        .collect()
        .await?;
    
    // 多表关联
    let admin_users: Vec<User> = db
        .select::<User>()
        .from::<User, Role>()
        .filter(|u, r| u.id.eq(r.user_id))
        .filter(|_, r| r.role_name.eq("admin".to_string()))
        .collect()
        .await?;
    
    // 子查询
    let users_with_roles: Vec<User> = db
        .select::<User>()
        .filter(|u| u.id.is_in(
            db.select::<Role>().map_to(|r| r.user_id)
        ))
        .collect()
        .await?;
    
    db.drop_table::<Role>().execute().await?;
    db.drop_table::<User>().execute().await?;
    Ok(())
}
```

## 说明

### JOIN 类型选择

- **LEFT JOIN**: 返回左表所有记录，右表无匹配时返回 NULL
- **INNER JOIN**: 只返回两表匹配的记录
- **RIGHT JOIN**: 返回右表所有记录，左表无匹配时返回 NULL

### 聚合函数

聚合函数在数据库层面计算，比查询所有数据后在应用层计算更高效：

```rust
let count: usize = db.select::<User>().count(|u| u.id).await?;
```
