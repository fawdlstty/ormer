#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

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

// ==================== 公共测试模型宏定义 ====================
// 用于在测试文件中快速定义具有唯一表名的测试模型

/// 定义基础User模型（带自增主键、唯一约束、索引）
#[macro_export]
macro_rules! define_test_user {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            #[unique]
            name: String,
            #[index]
            age: i32,
            email: Option<String>,
        }
    };
}

/// 定义基础Role模型
#[macro_export]
macro_rules! define_test_role {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            #[index]
            uid: i32,
            name: String,
        }
    };
}

/// 定义带score的User模型（用于聚合测试）
#[macro_export]
macro_rules! define_test_user_with_score {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            name: String,
            age: i32,
            score: i32,
        }
    };
}

/// 定义简单User模型（仅用于基本测试）
#[macro_export]
macro_rules! define_test_user_simple {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            name: String,
            age: i32,
        }
    };
}

/// 定义带Option id的User模型（用于事务测试）
#[macro_export]
macro_rules! define_test_user_with_option_id {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: Option<i64>,
            name: String,
            email: String,
        }
    };
}

/// 定义带联合唯一的Role模型（用于main_usage测试）
#[macro_export]
macro_rules! define_test_role_with_unique_group {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            #[unique(group = 1)]
            uid: i32,
            #[unique(group = 1)]
            name: String,
        }
    };
}

/// 定义Join测试用的User模型（无email）
#[macro_export]
macro_rules! define_test_user_for_join {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            #[unique]
            name: String,
            age: i32,
        }
    };
}

/// 定义Join测试用的Role模型（有role_name）
#[macro_export]
macro_rules! define_test_role_for_join {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            #[index]
            uid: i32,
            role_name: String,
        }
    };
}

/// 定义带score的User模型（用于aggregate_type测试，id为i64）
#[macro_export]
macro_rules! define_test_user_for_aggregate_type {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i64,
            name: String,
            age: i32,
            score: f64,
        }
    };
}

/// 定义带完整类型的模型（用于create_table测试）
#[macro_export]
macro_rules! define_test_complete_types {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i64,
            text_val: String,
            optional_text: Option<String>,
            optional_int: Option<i32>,
            bool_val: bool,
            optional_bool: Option<bool>,
        }
    };
}

/// 定义直接创建测试用的User模型（无email）
#[macro_export]
macro_rules! define_test_user_direct {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            name: String,
            age: i32,
        }
    };
}

/// 定义collect测试用的Role模型（有user_id字段）
#[macro_export]
macro_rules! define_test_role_for_collect {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            user_id: i32,
            role_name: String,
        }
    };
}

/// 定义单字段ID模型（用于collect测试）
#[macro_export]
macro_rules! define_test_user_id {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
        }
    };
}

/// 定义最简User模型（只有id和name）
#[macro_export]
macro_rules! define_test_user_minimal {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            name: String,
        }
    };
}

/// 定义外键测试用的User模型
#[macro_export]
macro_rules! define_test_user_for_fk {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: i32,
            name: String,
        }
    };
}

/// 定义外键测试用的Role模型（带user_id和role_name）
#[macro_export]
macro_rules! define_test_role_for_fk {
    ($struct_name:ident, $table_name:literal, $foreign_type:ty) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            #[foreign($foreign_type)]
            user_id: i32,
            role_name: String,
        }
    };
}

/// 定义简单Role模型（只有id和name）
#[macro_export]
macro_rules! define_test_role_simple {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            name: String,
        }
    };
}

/// 定义连接池测试用的User模型（无auto）
#[macro_export]
macro_rules! define_test_user_for_pool {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            name: String,
            age: i32,
            email: Option<String>,
        }
    };
}

/// 定义map_to测试用的User模型（无auto，有email为String）
#[macro_export]
macro_rules! define_test_user_for_map_to {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            name: String,
            email: String,
            age: i32,
        }
    };
}

/// 定义map_to测试用的Role模型（有user_id和role_name）
#[macro_export]
macro_rules! define_test_role_for_map_to {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            user_id: i32,
            role_name: String,
        }
    };
}

/// 定义单字段ID模型（用于map_to测试，有PartialEq）
#[macro_export]
macro_rules! define_test_user_id_with_eq {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone, PartialEq)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
        }
    };
}

/// 定义双字段模型（用于map_to测试，有PartialEq）
#[macro_export]
macro_rules! define_test_user_name_age {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone, PartialEq)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            name: String,
            age: i32,
        }
    };
}

/// 定义range测试用的User模型（无email，无auto）
#[macro_export]
macro_rules! define_test_user_for_range {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary]
            id: i32,
            name: String,
            age: i32,
        }
    };
}

/// 定义简单事务测试用的User模型（Option<i64> id，只有name）
#[macro_export]
macro_rules! define_test_user_for_simple_txn {
    ($struct_name:ident, $table_name:literal) => {
        #[derive(Debug, ormer::Model, Clone)]
        #[table = $table_name]
        struct $struct_name {
            #[primary(auto)]
            id: Option<i64>,
            name: String,
        }
    };
}
