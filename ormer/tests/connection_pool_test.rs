use ormer::{Database, DbType, Model};

#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
mod connection_pool_tests {
    use super::*;

    // 为 Turso 测试创建临时数据库路径
    #[cfg(feature = "turso")]
    #[allow(dead_code)]
    fn temp_db_path() -> String {
        format!(
            "/tmp/ormer_pool_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[derive(Debug, Model)]
    #[table = "pool_test_users"]
    struct PoolTestUser {
        #[primary(auto)]
        id: i32,
        name: String,
        age: i32,
        email: Option<String>,
    }

    /// 测试连接池创建和基本配置
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_creation_turso() -> Result<(), Box<dyn std::error::Error>> {
        // 创建连接池，min=0 表示创建时不建立连接
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // 最小连接数为 0
        .build()
        .await?;

        // 从池中获取连接（此时才会真正创建连接）
        let conn = pool.get().await?;

        // 验证连接可以使用
        conn.create_table::<PoolTestUser>().await?;

        // conn 离开作用域后自动归还
        Ok(())
    }

    /// 测试连接池的插入操作
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_insert_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..5) // min=0
        .build()
        .await?;

        let conn = pool.get().await?;
        conn.create_table::<PoolTestUser>().await?;

        // 插入单条记录
        conn.insert(&PoolTestUser {
            id: 1,
            name: "Alice".to_string(),
            age: 25,
            email: Some("alice@example.com".to_string()),
        })
        .await?;

        // 插入多条记录
        conn.insert(&vec![
            PoolTestUser {
                id: 2,
                name: "Bob".to_string(),
                age: 30,
                email: Some("bob@example.com".to_string()),
            },
            PoolTestUser {
                id: 3,
                name: "Charlie".to_string(),
                age: 35,
                email: None,
            },
        ])
        .await?;

        Ok(())
    }

    /// 测试连接池的查询操作
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_select_turso() -> Result<(), Box<dyn std::error::Error>> {
        let db_path = format!("/tmp/ormer_test_{}.db", std::process::id());
        let pool = Database::create_pool(DbType::Turso, &db_path)
            .range(0..3) // min=0
            .build()
            .await?;

        // 插入测试数据
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
            conn.insert(&PoolTestUser {
                id: 1,
                name: "Alice".to_string(),
                age: 25,
                email: Some("alice@example.com".to_string()),
            })
            .await?;
            conn.insert(&PoolTestUser {
                id: 2,
                name: "Bob".to_string(),
                age: 30,
                email: Some("bob@example.com".to_string()),
            })
            .await?;
        }

        // 查询数据
        {
            let conn = pool.get().await?;
            let users = conn.select::<PoolTestUser>().collect::<Vec<_>>().await?;

            assert_eq!(users.len(), 2);
            assert_eq!(users[0].name, "Alice");
            assert_eq!(users[1].name, "Bob");
        }

        // 清理测试文件
        let _ = std::fs::remove_file(&db_path);

        Ok(())
    }

    /// 测试连接池的过滤查询
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_filter_select_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        // 插入测试数据
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
            for i in 1..=5 {
                conn.insert(&PoolTestUser {
                    id: i,
                    name: format!("User{}", i),
                    age: 20 + i * 5,
                    email: Some(format!("user{}@example.com", i)),
                })
                .await?;
            }
        }

        // 带过滤条件的查询
        {
            let conn = pool.get().await?;
            let users = conn
                .select::<PoolTestUser>()
                .filter(|p| p.age.ge(35))
                .collect::<Vec<_>>()
                .await?;

            assert_eq!(users.len(), 3); // age >= 35 的有 3 个
        }

        Ok(())
    }

    /// 测试连接池的更新操作
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_update_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        // 插入测试数据
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
            conn.insert(&PoolTestUser {
                id: 1,
                name: "Alice".to_string(),
                age: 25,
                email: Some("alice@example.com".to_string()),
            })
            .await?;
        }

        // 更新数据
        {
            let conn = pool.get().await?;
            let count = conn
                .update::<PoolTestUser>()
                .filter(|p| p.name.eq("Alice".to_string()))
                .set(|p| p.age, 30)
                .execute()
                .await?;

            assert_eq!(count, 1);
        }

        // 验证更新结果
        {
            let conn = pool.get().await?;
            let users = conn
                .select::<PoolTestUser>()
                .filter(|p| p.name.eq("Alice".to_string()))
                .collect::<Vec<_>>()
                .await?;

            assert_eq!(users.len(), 1);
            assert_eq!(users[0].age, 30);
        }

        Ok(())
    }

    /// 测试连接池的删除操作
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_delete_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        // 插入测试数据
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
            conn.insert(&PoolTestUser {
                id: 1,
                name: "Alice".to_string(),
                age: 25,
                email: None,
            })
            .await?;
            conn.insert(&PoolTestUser {
                id: 2,
                name: "Bob".to_string(),
                age: 30,
                email: None,
            })
            .await?;
        }

        // 删除数据
        {
            let conn = pool.get().await?;
            let count = conn
                .delete::<PoolTestUser>()
                .filter(|p| p.age.lt(28))
                .execute()
                .await?;

            assert_eq!(count, 1);
        }

        // 验证删除结果
        {
            let conn = pool.get().await?;
            let users = conn.select::<PoolTestUser>().collect::<Vec<_>>().await?;
            assert_eq!(users.len(), 1);
            assert_eq!(users[0].name, "Bob");
        }

        Ok(())
    }

    /// 测试连接池的事务操作
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_transaction_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        // 在同一个连接中创建表和执行事务
        let conn = pool.get().await?;
        conn.create_table::<PoolTestUser>().await?;

        // 使用事务插入数据
        let mut txn = conn.begin().await?;

        txn.insert(&PoolTestUser {
            id: 1,
            name: "Alice".to_string(),
            age: 25,
            email: Some("alice@example.com".to_string()),
        })
        .await?;

        txn.insert(&PoolTestUser {
            id: 2,
            name: "Bob".to_string(),
            age: 30,
            email: Some("bob@example.com".to_string()),
        })
        .await?;

        txn.commit().await?;

        // 验证事务提交成功
        let users = conn.select::<PoolTestUser>().collect::<Vec<_>>().await?;
        assert_eq!(users.len(), 2);

        Ok(())
    }

    /// 测试连接池的聚合查询
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_aggregate_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        // 插入测试数据
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
            for i in 1..=5 {
                conn.insert(&PoolTestUser {
                    id: i,
                    name: format!("User{}", i),
                    age: 20 + i * 5,
                    email: None,
                })
                .await?;
            }
        }

        // 聚合查询
        {
            let conn = pool.get().await?;

            let count: usize = conn.select::<PoolTestUser>().count(|p| p.id).await?;
            assert_eq!(count, 5);

            let sum: Option<i32> = conn.select::<PoolTestUser>().sum(|p| p.age).await?;
            assert_eq!(sum, Some(175)); // 25+30+35+40+45 = 175

            let avg: Option<f64> = conn.select::<PoolTestUser>().avg(|p| p.age).await?;
            assert!((avg.unwrap() - 35.0).abs() < 0.01);

            let min: Option<i32> = conn.select::<PoolTestUser>().min(|p| p.age).await?;
            assert_eq!(min, Some(25));

            let max: Option<i32> = conn.select::<PoolTestUser>().max(|p| p.age).await?;
            assert_eq!(max, Some(45));
        }

        Ok(())
    }

    /// 测试多次获取和归还连接
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_multiple_get_return_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..5) // min=0
        .build()
        .await?;

        // 第一次获取连接并创建表
        {
            let conn = pool.get().await?;
            conn.create_table::<PoolTestUser>().await?;
        }

        // 第二次获取连接并插入数据
        {
            let conn = pool.get().await?;
            conn.insert(&PoolTestUser {
                id: 1,
                name: "Alice".to_string(),
                age: 25,
                email: None,
            })
            .await?;
        }

        // 第三次获取连接并查询数据
        {
            let conn = pool.get().await?;
            let users = conn.select::<PoolTestUser>().collect::<Vec<_>>().await?;
            assert_eq!(users.len(), 1);
        }

        // 第四次获取连接并删除表
        {
            let conn = pool.get().await?;
            conn.drop_table::<PoolTestUser>().await?;
        }

        Ok(())
    }

    /// 测试连接池的原生 SQL 执行
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_exec_sql_turso() -> Result<(), Box<dyn std::error::Error>> {
        let pool = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..3) // min=0
        .build()
        .await?;

        let conn = pool.get().await?;
        conn.create_table::<PoolTestUser>().await?;

        // 执行原生插入 SQL
        conn.exec_non_query(
            "INSERT INTO pool_test_users (id, name, age, email) VALUES (1, 'Alice', 25, 'alice@example.com')",
        )
        .await?;

        // 执行原生查询 SQL
        let users = conn
            .exec_table::<PoolTestUser>("SELECT * FROM pool_test_users")
            .await?;

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "Alice");

        Ok(())
    }

    /// 测试连接池配置的范围参数
    #[cfg(feature = "turso")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pool_range_config_turso() -> Result<(), Box<dyn std::error::Error>> {
        // 测试不同的范围配置
        let pool1 = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..2) // min=0
        .build()
        .await?;

        let pool2 = Database::create_pool(
            DbType::Turso,
            &format!("/tmp/ormer_{}_{}.db", std::process::id(), line!()),
        )
        .range(0..10) // min=0
        .build()
        .await?;

        // 验证两个池都可以正常工作
        let conn1 = pool1.get().await?;
        conn1.create_table::<PoolTestUser>().await?;

        let conn2 = pool2.get().await?;
        conn2.create_table::<PoolTestUser>().await?;

        Ok(())
    }
}
