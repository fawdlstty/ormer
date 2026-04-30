# 连接池

## 创建连接池

```rust
use ormer::{Database, DbType, ConnectionPool};

let pool = Database::create_pool(DbType::PostgreSQL, "postgresql://user:pass@localhost/dbname")
    .max_size(10)
    .min_size(5)
    .idle_timeout(300)
    .build()
    .await?;
```

## 使用连接池

```rust
let conn = pool.get().await?;

let users: Vec<User> = conn.select::<User>().collect().await?;
```

### 自动管理

```rust
async fn handle_request(pool: &ConnectionPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    conn.insert(&user).execute().await?;
    Ok(())
}
```

## SQLite 后端注意事项

SQLite (turso) 后端由于其嵌入式特性，官方不支持多线程共享连接。建议：

1. **连接池配置**: 设置 `max_size=1`，使用单连接池
   ```rust
   let pool = Database::create_pool(DbType::Sqlite, "path/to/database.db")
       .range(0..1)  // 建议使用单连接
       .build()
       .await?;
   ```

2. **并发场景**: 如需高并发读写，考虑启用 MVCC 模式
   ```rust
   let conn = pool.get().await?;
   conn.exec_non_query("PRAGMA journal_mode = 'mvcc'").await?;
   // 使用 BEGIN CONCURRENT 实现并发写入
   ```

3. **事务处理**: 避免长时间持有连接，及时归还到池中
   ```rust
   {
       let conn = pool.get().await?;
       // 执行操作
       // conn 离开作用域后自动归还
   }
   ```

4. **多进程访问**: SQLite 不支持多进程同时访问同一数据库文件，如需多进程场景请考虑使用 PostgreSQL 或 MySQL

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
