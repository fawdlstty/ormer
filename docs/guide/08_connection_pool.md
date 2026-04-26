# 连接池

## 什么是连接池

连接池复用数据库连接，避免频繁创建销毁。

### 优势

- 性能提升 - 避免每次创建新连接
- 资源控制 - 限制最大连接数
- 连接复用 - 自动管理生命周期
- 故障恢复 - 自动替换失效连接

## 创建连接池

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)        // 最大连接数
    .min_size(5)         // 最小空闲连接 (可选)
    .idle_timeout(300)   // 空闲超时秒 (可选)
    .build()
    .await?;
```

## 使用连接池

```rust
// 获取连接
let conn = pool.get().await?;

// 使用
let users: Vec<User> = conn.select::<User>().collect().await?;

// 连接自动返回池
```

### 自动管理

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    conn.insert(&user).await?;
    // conn 超出作用域后自动返回池
    Ok(())
}
```

## 完整示例

```rust
use ormer::{Database, DbType, ConnectionPool, Model};
use std::sync::Arc;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/mydb"
    )
    .max_size(20)
    .build()
    .await?;
    
    let state = Arc::new(pool);
    
    // 并发请求
    let mut handles = vec![];
    for i in 0..10 {
        let state = state.clone();
        let handle = tokio::spawn(async move {
            let conn = state.get().await.unwrap();
            let users: Vec<User> = conn
                .select::<User>()
                .range(0..10)
                .collect()
                .await
                .unwrap();
            println!("Request {}: {} users", i, users.len());
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    Ok(())
}
```

## 配置建议

### 开发环境

```rust
.max_size(5)
```

### 生产环境

```rust
.max_size(num_cpus::get() * 4)  // CPU核心数4倍
.min_size(10)
.idle_timeout(600)
```

### 不同数据库

```rust
// SQLite - 少量连接
.max_size(10)

// PostgreSQL/MySQL - 支持较多并发
.max_size(20)
.min_size(5)
```

## 最佳实践

### 使用连接池

```rust
// ✅ 推荐
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

// ❌ 避免 - 每次创建新连接
let db = Database::connect(DbType::PostgreSQL, db_url).await?;
```

### 及时释放连接

```rust
// ✅ 推荐 - 使用作用域
{
    let conn = pool.get().await?;
    conn.insert(&user).await?;
} // 自动释放

// ❌ 避免 - 长时间持有
let conn = pool.get().await?;
tokio::time::sleep(Duration::from_secs(60)).await;
```

### 带超时获取

```rust
let conn = tokio::time::timeout(
    Duration::from_secs(5),
    pool.get()
).await??;
```

## 常见问题

### 连接池耗尽

增加 `max_size` 或优化查询减少占用时间。

### 连接泄漏

使用作用域确保连接自动释放。

### 连接失效

连接池自动替换失效连接，重新获取即可。

## 性能调优

- 根据负载调整连接数 (5-50)
- 设置合理超时 (获取5秒，空闲5分钟)
- 预创建连接 (`min_size`)
