/// 测试数据库配置模块
/// 提供统一的数据库后端配置，支持在多个数据库上运行测试
use ormer::DbType;

/// 数据库后端配置类型
/// 每个配置包含 (DbType, 连接字符串)
pub type DbConfig = (DbType, &'static str);

/// 获取所有可用的数据库后端配置
///
/// 根据编译时启用的 feature，返回可用的数据库配置列表
/// 测试函数应该遍历此列表，对每个数据库后端执行测试
#[allow(dead_code)]
#[allow(unused_mut)]
pub fn get_all_db_configs() -> Vec<DbConfig> {
    let mut configs = Vec::new();

    // Turso 仅在启用 turso feature 时可用
    #[cfg(feature = "turso")]
    configs.push((DbType::Turso, ":memory:"));

    // PostgreSQL 仅在启用 postgresql feature 时可用
    #[cfg(feature = "postgresql")]
    configs.push((
        DbType::PostgreSQL,
        "postgres://postgres:postgres@localhost:5432/ormer_test",
    ));

    // MySQL 仅在启用 mysql feature 时可用
    #[cfg(feature = "mysql")]
    configs.push((DbType::MySQL, "mysql://root:root@localhost:3306/ormer_test"));

    configs
}

/// 获取仅包含 Turso 的配置（用于快速测试或不支持多数据库的场景）
#[cfg(feature = "turso")]
#[allow(dead_code)]
pub fn get_turso_config() -> Vec<DbConfig> {
    vec![(DbType::Turso, ":memory:")]
}

/// 辅助函数：创建数据库连接
/// 从配置创建数据库连接
#[allow(dead_code)]
pub async fn create_db_connection(
    config: &DbConfig,
) -> Result<ormer::Database, Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(config.0, config.1).await?;
    Ok(db)
}

/// 宏：为所有数据库后端生成测试（用于返回 () 的函数）
///
/// 使用方法：
/// ```rust
/// test_on_all_dbs!(my_test_fn);
/// ```
///
/// 这将为每个可用的数据库后端生成一个测试函数
#[macro_export]
macro_rules! test_on_all_dbs {
    ($test_fn:ident) => {
        paste::paste! {
            #[tokio::test]
            async fn [<test_on_all_dbs_ $test_fn>]() {
                let configs = $crate::_test_common::get_all_db_configs();

                for (idx, config) in configs.iter().enumerate() {
                    let db_type_name = match config.0 {
                        #[cfg(feature = "turso")]
                        ormer::DbType::Turso => "Turso",
                        #[cfg(feature = "postgresql")]
                        ormer::DbType::PostgreSQL => "PostgreSQL",
                        #[cfg(feature = "mysql")]
                        ormer::DbType::MySQL => "MySQL",
                        #[allow(unreachable_patterns)]
                        _ => "Unknown",
                    };

                    println!("\n=== Testing on {} (config {}) ===", db_type_name, idx);

                    // 调用实际的测试函数（返回 ()）
                    $test_fn(config).await;
                }
            }
        }
    };
}

/// 宏：为所有数据库后端生成测试（使用 Result 返回类型）
///
/// 使用方法：
/// ```rust
/// test_on_all_dbs_result!(my_test_fn);
/// ```
#[macro_export]
macro_rules! test_on_all_dbs_result {
    ($test_fn:ident) => {
        paste::paste! {
            #[tokio::test]
            async fn [<test_on_all_dbs_ $test_fn>]() -> Result<(), Box<dyn std::error::Error>> {
                let configs = $crate::_test_common::get_all_db_configs();

                for (idx, config) in configs.iter().enumerate() {
                    let db_type_name = match config.0 {
                        #[cfg(feature = "turso")]
                        ormer::DbType::Turso => "Turso",
                        #[cfg(feature = "postgresql")]
                        ormer::DbType::PostgreSQL => "PostgreSQL",
                        #[cfg(feature = "mysql")]
                        ormer::DbType::MySQL => "MySQL",
                        #[allow(unreachable_patterns)]
                        _ => "Unknown",
                    };

                    println!("\n=== Testing on {} (config {}) ===", db_type_name, idx);

                    // 调用实际的测试函数
                    $test_fn(config).await?;
                }

                Ok(())
            }
        }
    };
}
