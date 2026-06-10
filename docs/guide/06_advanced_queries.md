# 高级查询

## 聚合查询

```rust
let count: usize = db.select::<User>().count(|u| u.id).await?;

let total: Option<i32> = db.select::<Product>().sum(|p| p.price).await?;

let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

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

let sql = Select::<User>::new()
    .select_column(|u| u.id.count())
    .group_by(|u| u.age)
    .to_sql();
```

### 多字段选择 + 分组

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .to_sql();
```

### HAVING 条件过滤

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(5))
    .to_sql();
```

### 多字段分组

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.age, u.id.count(), u.score.avg()))
    .group_by(|u| (u.department, u.age))
    .to_sql();
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
```

### 支持的聚合函数

- `count()` - 计数，返回 `usize`
- `sum()` - 求和，返回原类型（数值类型）
- `avg()` - 平均值，返回 `f64`
- `max()` - 最大值，返回原类型（数值类型）
- `min()` - 最小值，返回原类型（数值类型）

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

### JOIN 右表排序与分页 (LATERAL JOIN)

当 JOIN 条件中使用了 `order_by` / `order_by_desc` 或 `range` 时，框架会自动生成 **LATERAL JOIN** SQL，实现对右表的排序和分页。

```rust
// 右表按 role_name 降序，只取第1条
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).order_by_desc(r.role_name).range(..1))
    .collect()
    .await?;

// 仅排序
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).order_by_desc(r.role_name))
    .collect()
    .await?;

// 仅分页
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).range(..3))
    .collect()
    .await?;
```

支持的 JOIN 类型：`left_join`、`inner_join`、`right_join`。

可与主查询的 `filter`、`range` 等方法组合使用。

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

### EXISTS / NOT EXISTS

使用 `Select::exists()` 和 `Select::not_exists()` 构建子查询表达式：

```rust
let users_with_roles: Vec<User> = db
    .select::<User>()
    .filter(|_u| {
        Select::<Role>::new()
            .filter(|r| r.name.eq("admin"))
            .exists()  // 或 .not_exists()
    })
    .collect()
    .await?;
```

可与外层条件组合：

```rust
.filter(|p| p.age.ge(18).or(
    Select::<Role>::new().filter(|r| r.uid.eq(p.id)).exists()
))
```

## 集合操作

### UNION / UNION ALL

```rust
// UNION
let sql = Select::<User>::new()
    .filter(|u| u.age.gt(30))
    .union(Select::<User>::new().filter(|u| u.age.lt(18)))
    .to_sql();

// UNION ALL
let sql = Select::<User>::new()
    .filter(|u| u.age.gt(30))
    .union_all(Select::<User>::new().filter(|u| u.age.lt(18)))
    .to_sql();
```

### INTERSECT / EXCEPT

```rust
let sql = Select::<User>::new()
    .filter(|u| u.age.gt(18))
    .intersect(Select::<User>::new().filter(|u| u.age.lt(65)))
    .to_sql();

let sql = Select::<User>::new()
    .filter(|u| u.age.gt(18))
    .except(Select::<User>::new().filter(|u| u.name.eq("admin")))
    .to_sql();
```

集合操作支持链式 `order_by` 和 `range`：

```rust
let sql = Select::<User>::new()
    .filter(|u| u.age.gt(30))
    .order_by(|u| u.name)
    .range(..10)
    .union(
        Select::<User>::new()
            .filter(|u| u.age.lt(18))
            .order_by_desc(|u| u.age)
            .range(..5),
    )
    .to_sql();
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
    let db = Database::connect(DbType::Sqlite, "file:test.db").await?;
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
