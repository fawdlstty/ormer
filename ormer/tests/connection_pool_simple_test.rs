#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::Database;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_minimal!(SimpleUser, "test_pool_simple_users_1");

/// 测试连接池基本创建和获取
async fn test_pool_basic_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating connection pool...");

    // 创建连接池（不设置 min_idle，避免初始连接）
    // Turso 后端限制最大连接数为 1，其他数据库可以使用更大的连接池
    let pool = match config.0 {
        ormer::DbType::Turso => {
            Database::create_pool(config.0, config.1)
                .range(0..1) // Turso: max=1
                .build()
                .await?
        }
        _ => {
            Database::create_pool(config.0, config.1)
                .range(0..3) // 其他数据库: max=3
                .build()
                .await?
        }
    };

    println!("Pool created successfully");

    // 从池中获取连接
    println!("Getting connection from pool...");
    let conn = pool.get().await?;
    println!("Connection acquired");

    // 创建表
    conn.create_table::<SimpleUser>().execute().await?;
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

    // 清理测试表
    conn.drop_table::<SimpleUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_pool_basic_impl);
