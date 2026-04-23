use ormer::{Database, DbType, Model};

#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
mod connection_pool_simple_tests {
    use super::*;

    #[derive(Debug, Model)]
    #[table = "simple_users"]
    struct SimpleUser {
        #[primary(auto)]
        id: i32,
        name: String,
    }

    /// 测试连接池基本创建和获取
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_basic_turso() -> Result<(), Box<dyn std::error::Error>> {
        println!("Creating connection pool...");

        // 创建连接池（不设置 min_idle，避免初始连接）
        let pool = Database::create_pool(DbType::Turso, ":memory:")
            .range(0..3) // min=0, 不会立即创建连接
            .build()
            .await?;

        println!("Pool created successfully");

        // 从池中获取连接
        println!("Getting connection from pool...");
        let conn = pool.get().await?;
        println!("Connection acquired");

        // 创建表
        conn.create_table::<SimpleUser>().await?;
        println!("Table created");

        // 插入数据
        conn.insert(&SimpleUser {
            id: 1,
            name: "Alice".to_string(),
        })
        .await?;
        println!("Data inserted");

        // 查询数据
        let users = conn.select::<SimpleUser>().collect::<Vec<_>>().await?;
        println!("Queried {} users", users.len());
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "Alice");

        // conn 离开作用域后自动归还
        println!("Test passed!");
        Ok(())
    }
}
