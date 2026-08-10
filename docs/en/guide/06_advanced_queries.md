# Advanced Queries

## Aggregate Queries

```rust
let count: usize = db.select::<User>().count(|u| u.id).await?;

let total: Option<i32> = db.select::<Product>().sum(|p| p.price).await?;

let avg: Option<f64> = db.select::<User>().avg(|u| u.age).await?;

let max: Option<i32> = db.select::<User>().max(|u| u.age).await?;

let min: Option<i32> = db.select::<User>().min(|u| u.age).await?;
```

With conditions:

```rust
let adult_count: usize = db
    .select::<User>()
    .filter(|u| u.age.ge(18))
    .count(|u| u.id)
    .await?;
```

## GROUP BY and HAVING Queries

### Basic Grouping

```rust
use ormer::Select;

let sql = Select::<User>::new()
    .select_column(|u| u.id.count())
    .group_by(|u| u.age)
    .to_sql();
```

### Multiple Columns + Grouping

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .to_sql();
```

### HAVING Condition Filter

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.id.count()))
    .group_by(|u| u.department)
    .having(|u| u.id.count().gt(5))
    .to_sql();
```

### Multi-Column Grouping

```rust
let sql = Select::<User>::new()
    .select_column(|u| (u.department, u.age, u.id.count(), u.score.avg()))
    .group_by(|u| (u.department, u.age))
    .to_sql();
```

### Complete Query: WHERE + GROUP BY + HAVING + ORDER BY + LIMIT

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

### Supported Aggregate Functions

- `count()` - Count, returns `usize`
- `sum()` - Sum, returns original type (numeric types)
- `avg()` - Average, returns `f64`
- `max()` - Maximum, returns original type (numeric types)
- `min()` - Minimum, returns original type (numeric types)

## Unified Expressions

Fields, projections, filters, ordering, grouping, and selected aggregate features share the same expression AST. Expressions can combine functions, `CASE`, casts, collations, window functions, and backend operators:

```rust
let rows: Vec<(i32, String, String)> = db
    .select::<User>()
    .filter(|u| u.email.to_lower().eq("alice@example.com"))
    .map_to(|u| {
        (
            u.id.alias("user_id"),
            u.email,
            ormer::expr!(match u.status {
                "paid" => "done",
                "new" => "open",
                _ => "other",
            })
            .alias("status_label"),
        )
    })
    .order_by(|u| u.email.collate("nocase").asc())
    .collect()
    .await?;
```

Aggregate expressions support `FILTER`, inner ordering, and `OVER`:

```rust
let rows: Vec<(i32, i32)> = db
    .select::<Order>()
    .map_to(|o| {
        (
            o.user_id,
            o.total
                .sum()
                .filter(|o| o.paid.eq(true))
                .over(|w| w.partition_by(o.user_id)),
        )
    })
    .collect()
    .await?;
```

Grouping supports `ROLLUP`, `CUBE`, and `GROUPING SETS`:

```rust
let sql = Select::<Sale>::new()
    .select_column(|s| (s.region, s.amount.sum()))
    .rollup(|s| (s.region, s.city))
    .to_sql();

let sql = Select::<Sale>::new()
    .select_column(|s| (s.region, s.amount.sum()))
    .grouping_sets(|s| ((s.region,), ()))
    .to_sql();
```

You can also combine row values, JSON paths, full text search, `DISTINCT ON`, and row locks:

```rust
let rows: Vec<User> = db
    .select::<User>()
    .distinct_on(|u| u.org_id)
    .filter(|u| (u.org_id, u.email).eq((org_id, email)))
    .filter(|u| u.profile.json_text("role").eq("admin"))
    .filter(|u| u.bio.matches_text("rust"))
    .for_update()
    .skip_locked()
    .collect()
    .await?;
```

## JOIN Queries

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

### JOIN with Filter

```rust
let admin_users: Vec<(User, Role)> = db
    .select::<User>()
    .inner_join::<Role>(|u, r| u.id.eq(r.user_id))
    .filter(|u| u.name.eq("Alice".to_string()))
    .collect()
    .await?;
```

### JOIN with Right-Table Sorting and Pagination (LATERAL JOIN)

When `order_by` / `order_by_desc` or `range` is used in the JOIN condition, the framework automatically generates **LATERAL JOIN** SQL to enable sorting and pagination on the right table.

```rust
// Right table sorted by role_name desc, take only the first row
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).order_by_desc(r.role_name).range(..1))
    .collect()
    .await?;

// Sort only
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).order_by_desc(r.role_name))
    .collect()
    .await?;

// Pagination only
let user_roles: Vec<(User, Option<Role>)> = db
    .select::<User>()
    .left_join::<Role>(|u, r| u.id.eq(r.user_id).range(..3))
    .collect()
    .await?;
```

Supported JOIN types: `left_join`, `inner_join`, `right_join`.

Can be combined with `filter`, `range`, and other methods on the main query.

## Multi-Table Joins

### Two Tables (from)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from::<User, Role>()
    .filter(|u, r| u.id.eq(r.user_id))
    .filter(|_, r| r.role_name.eq("admin".to_string()))
    .collect()
    .await?;
```

### Three Tables (from3)

```rust
let users: Vec<User> = db
    .select::<User>()
    .from3::<User, Role, Permission>()
    .filter(|u, r, p| u.id.eq(r.user_id).and(r.id.eq(p.role_id)))
    .collect()
    .await?;
```

### Four Tables (from4)

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

## Subqueries

### IN Subquery

```rust
let subquery = db.select::<Role>().map_to(|r| r.user_id);

let users: Vec<User> = db
    .select::<User>()
    .filter(|u| u.id.is_in(subquery))
    .collect()
    .await?;
```

### EXISTS / NOT EXISTS

Use `Select::exists()` and `Select::not_exists()` to build subquery expressions:

```rust
let users_with_roles: Vec<User> = db
    .select::<User>()
    .filter(|_u| {
        Select::<Role>::new()
            .filter(|r| r.name.eq("admin"))
            .exists()  // or .not_exists()
    })
    .collect()
    .await?;
```

Can be combined with outer conditions:

```rust
.filter(|p| p.age.ge(18).or(
    Select::<Role>::new().filter(|r| r.uid.eq(p.id)).exists()
))
```

## Loading Model Relations

After declaring `#[has_many]` or `#[belongs_to]` on a model, related objects can be loaded on demand:

```rust
let user = db.find_by_id::<User>(1).await?.unwrap();
let posts = db
    .find_related(&user, UserWhere::default().posts)
    .await?;
```

Use `preload` to load a relation for a batch of parent models without an N+1 query loop:

```rust
let mut users = db.select::<User>().collect::<Vec<_>>().await?;
db.preload(&mut users, UserWhere::default().posts).await?;
```

Use `include` to load a `belongs_to` relation as part of the query result:

```rust
let posts: Vec<Post> = db
    .select::<Post>()
    .include(|post| post.user)
    .collect()
    .await?;
```

Relation fields are excluded from column mapping. A missing `has_many` relation is an empty `Vec`, while a missing `belongs_to` target is `None`.

## Set Operations

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

Set operations support chained `order_by` and `range`:

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

## Complete Example

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
    
    // Aggregate
    let count: usize = db.select::<User>().count(|u| u.id).await?;
    let avg_age: Option<f64> = db.select::<User>().avg(|u| u.age).await?;
    
    // LEFT JOIN
    let user_roles: Vec<(User, Option<Role>)> = db
        .select::<User>()
        .left_join::<Role>(|u, r| u.id.eq(r.user_id))
        .collect()
        .await?;
    
    // Multi-table
    let admin_users: Vec<User> = db
        .select::<User>()
        .from::<User, Role>()
        .filter(|u, r| u.id.eq(r.user_id))
        .filter(|_, r| r.role_name.eq("admin".to_string()))
        .collect()
        .await?;
    
    // Subquery
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
