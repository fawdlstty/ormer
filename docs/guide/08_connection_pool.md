# 连接池

连接池是管理数据库连接的重要机制,可以显著提高应用性能和资源利用率。

## 什么是连接池

连接池维护一组预先创建的数据库连接,应用可以复用这些连接,避免频繁创建和销毁连接的开销。

### 优势

- **性能提升** - 避免每次操作都创建新连接
- **资源控制** - 限制最大连接数,防止数据库过载
- **连接复用** - 自动管理连接的生命周期
- **故障恢复** - 自动检测和替换失效连接

## 创建连接池

### 基本用法

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)  // 最大连接数
    .build()
    .await?;
```

### 配置选项

```rust
let pool = Database::create_pool(DbType::PostgreSQL, connection_string)
    .max_size(20)        // 最大连接数 (默认: 10)
    .min_size(5)         // 最小空闲连接数 (可选)
    .idle_timeout(300)   // 空闲超时时间 (秒) (可选)
    .build()
    .await?;
```

## 使用连接池

### 获取连接

```rust
// 从连接池获取连接
let conn = pool.get().await?;

// 使用连接
let users: Vec<User> = conn
    .select::<User>()
    .collect()
    .await?;

// 连接会自动返回连接池
```

### 自动连接管理

连接池会自动管理连接:

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    // 获取连接
    let conn = pool.get().await?;
    
    // 执行操作
    conn.insert(&user).await?;
    
    // conn 超出作用域后自动返回连接池
    Ok(())
}
```

## 完整示例

### Web 应用示例

```rust
use ormer::{Database, DbType, ConnectionPool, Model};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    email: String,
}

struct AppState {
    pool: ConnectionPool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建连接池
    let pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/mydb"
    )
    .max_size(20)
    .build()
    .await?;
    
    let state = Arc::new(AppState { pool });
    
    // 模拟处理多个请求
    let mut handles = vec![];
    
    for i in 0..10 {
        let state = state.clone();
        let handle = tokio::spawn(async move {
            // 从连接池获取连接
            let conn = state.pool.get().await.unwrap();
            
            // 执行数据库操作
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
    
    // 等待所有请求完成
    for handle in handles {
        handle.await.unwrap();
    }
    
    Ok(())
}
```

### 连接池配置示例

```rust
use ormer::{Database, DbType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 开发环境: 少量连接
    let dev_pool = Database::create_pool(
        DbType::Turso,
        "file:dev.db"
    )
    .max_size(5)
    .build()
    .await?;
    
    // 生产环境: 更多连接
    let prod_pool = Database::create_pool(
        DbType::PostgreSQL,
        "postgresql://user:pass@localhost/prod_db"
    )
    .max_size(50)
    .min_size(10)
    .idle_timeout(600)  // 10 分钟
    .build()
    .await?;
    
    Ok(())
}
```

## 连接池最佳实践

### 1. 合理设置连接数

```rust
// ✅ 推荐: 根据服务器配置设置
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(num_cpus::get() * 4)  // CPU 核心数的 4 倍
    .build()
    .await?;

// ❌ 避免: 连接数过大或过小
.max_size(1000)  // 太大,浪费资源
.max_size(1)     // 太小,性能差
```

### 2. 使用连接池而非单个连接

```rust
// ✅ 推荐: 使用连接池
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .max_size(20)
    .build()
    .await?;

async fn handle_request(pool: &ConnectionPool) {
    let conn = pool.get().await?;
    // 使用 conn...
}

// ❌ 避免: 每次创建新连接
async fn handle_request_bad(db_url: &str) {
    let db = Database::connect(DbType::PostgreSQL, db_url).await?;
    // 使用 db...
} // 连接被丢弃
```

### 3. 及时释放连接

```rust
// ✅ 推荐: 连接在使用后自动返回池
{
    let conn = pool.get().await?;
    conn.insert(&user).await?;
} // conn 在这里被释放

// ❌ 避免: 长时间持有连接
let conn = pool.get().await?;
tokio::time::sleep(Duration::from_secs(60)).await;  // 不好!
conn.insert(&user).await?;
```

### 4. 错误处理

```rust
async fn safe_query(pool: &ConnectionPool) -> Result<Vec<User>, Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    
    let users = conn
        .select::<User>()
        .collect::<Vec<_>>()
        .await?;
    
    Ok(users)
}
```

## 监控连接池

### 连接池状态

```rust
// 某些连接池实现提供状态监控
println!("Active connections: {}", pool.active_connections());
println!("Idle connections: {}", pool.idle_connections());
println!("Total connections: {}", pool.total_connections());
```

### 性能指标

监控以下指标:

- **等待时间** - 获取连接的等待时间
- **活跃连接数** - 正在使用的连接数
- **空闲连接数** - 可用的连接数
- **连接错误率** - 连接失败的次数

## 不同数据库的连接池配置

### Turso/SQLite

```rust
let pool = Database::create_pool(DbType::Turso, "file:test.db")
    .max_size(10)  // SQLite 通常不需要太多连接
    .build()
    .await?;
```

### PostgreSQL

```rust
let pool = Database::create_pool(
    DbType::PostgreSQL,
    "postgresql://user:pass@localhost/dbname"
)
.max_size(20)  // PostgreSQL 支持较多并发连接
.min_size(5)
.build()
.await?;
```

### MySQL

```rust
let pool = Database::create_pool(
    DbType::MySQL,
    "mysql://user:pass@localhost/dbname"
)
.max_size(20)
.min_size(5)
.build()
.await?;
```

## 常见问题

### 1. 连接池耗尽

当所有连接都在使用时,新的请求会等待:

```rust
// 连接池已满,这里会阻塞直到有连接可用
let conn = pool.get().await?;
```

**解决方法**:
- 增加 `max_size`
- 优化查询,减少连接占用时间
- 使用超时机制

```rust
// 带超时的获取连接
let conn = tokio::time::timeout(
    Duration::from_secs(5),
    pool.get()
).await??;
```

### 2. 连接泄漏

忘记释放连接会导致连接泄漏:

```rust
// ❌ 错误: 连接未释放
let conn = pool.get().await?;
// 函数返回但没有释放 conn

// ✅ 正确: 使用作用域
{
    let conn = pool.get().await?;
    // 使用 conn
} // conn 自动释放
```

### 3. 连接失效

数据库重启或网络问题会导致连接失效:

```rust
// 连接池通常会自动处理
let conn = pool.get().await?;

// 如果连接失效,重新获取
match conn.select::<User>().collect().await {
    Ok(users) => println!("Got {} users", users.len()),
    Err(_) => {
        // 连接池会自动替换失效连接
        let conn = pool.get().await?;
        let users: Vec<User> = conn.select::<User>().collect().await?;
    }
}
```

## 性能调优

### 1. 根据负载调整连接数

```rust
// 低负载应用
.max_size(5)

// 中等负载
.max_size(20)

// 高负载应用
.max_size(50)
```

### 2. 设置合理的超时

```rust
// 连接获取超时
.max_wait_time(5)  // 5 秒

// 空闲超时
.idle_timeout(300)  // 5 分钟
```

### 3. 预创建连接

```rust
// 启动时创建最小连接数
let pool = Database::create_pool(DbType::PostgreSQL, &db_url)
    .min_size(10)  // 预创建 10 个连接
    .build()
    .await?;
```
