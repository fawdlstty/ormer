#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::generate_create_table_sql;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(TestUser, "create_table_users_1");
define_test_complete_types!(TestCompleteTypes, "create_table_complete_types_1");

#[cfg(test)]
mod create_table_tests {
    use super::*;

    async fn test_turso_create_table_sql_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestUser>(config.0);

        // 根据不同的数据库类型进行不同的断言
        #[allow(unreachable_patterns)]
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                assert!(sql.contains("id INTEGER PRIMARY KEY"));
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT")); // Option 类型，不加 NOT NULL
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT")); // Option 类型，不加 NOT NULL
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                assert!(sql.contains("id INT PRIMARY KEY AUTO_INCREMENT")); // 有auto，所以应该有AUTO_INCREMENT
                assert!(sql.contains("name VARCHAR(255) NOT NULL"));
                assert!(sql.contains("age INT NOT NULL"));
                assert!(sql.contains("email VARCHAR(255)")); // Option 类型，不加 NOT NULL
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }

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
        println!("{} SQL: {}", db_type_name, sql);
    }

    async fn test_postgresql_create_table_sql_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestUser>(config.0);

        // 根据不同的数据库类型进行不同的断言
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                // Turso/SQLite 使用 INTEGER
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT"));
                println!("Turso SQL: {}", sql);
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                // PostgreSQL 使用 INTEGER 和 TEXT
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT"));
                println!("PostgreSQL SQL: {}", sql);
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                // MySQL 使用 INT 和 VARCHAR(255)
                assert!(sql.contains("id INT PRIMARY KEY"));
                assert!(sql.contains("name VARCHAR(255) NOT NULL"));
                assert!(sql.contains("age INT NOT NULL"));
                assert!(sql.contains("email VARCHAR(255)"));
                println!("MySQL SQL: {}", sql);
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    async fn test_mysql_create_table_sql_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestUser>(config.0);

        // 根据不同的数据库类型进行不同的断言
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                // Turso/SQLite 使用 INTEGER
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT"));
                println!("Turso SQL: {}", sql);
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                // PostgreSQL 使用 INTEGER 和 TEXT
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("name TEXT NOT NULL"));
                assert!(sql.contains("age INTEGER NOT NULL"));
                assert!(sql.contains("email TEXT"));
                println!("PostgreSQL SQL: {}", sql);
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                // MySQL 使用 INT 和 VARCHAR(255)
                assert!(sql.contains("id INT PRIMARY KEY"));
                assert!(sql.contains("name VARCHAR(255) NOT NULL"));
                assert!(sql.contains("age INT NOT NULL"));
                assert!(sql.contains("email VARCHAR(255)"));
                println!("MySQL SQL: {}", sql);
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    async fn test_different_databases_produce_different_sql_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        // 使用条件编译确保只使用已启用的数据库类型
        #[cfg(all(feature = "turso", feature = "postgresql", feature = "mysql"))]
        {
            // 使用 TestCompleteTypes 来测试，因为它包含 bool 类型，在不同数据库中映射不同
            let turso_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::Turso);
            let pg_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::PostgreSQL);
            let mysql_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::MySQL);

            // 验证三个数据库生成的SQL确实不同
            // Turso: bool -> INTEGER, String -> TEXT
            // PostgreSQL: bool -> BOOLEAN, String -> TEXT
            // MySQL: bool -> TINYINT(1), String -> VARCHAR(255)
            assert_ne!(
                turso_sql, pg_sql,
                "Turso and PostgreSQL SQL should be different"
            );
            assert_ne!(
                turso_sql, mysql_sql,
                "Turso and MySQL SQL should be different"
            );
            assert_ne!(
                pg_sql, mysql_sql,
                "PostgreSQL and MySQL SQL should be different"
            );
        }

        #[cfg(all(feature = "turso", feature = "postgresql", not(feature = "mysql")))]
        {
            let turso_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::Turso);
            let pg_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::PostgreSQL);
            assert_ne!(turso_sql, pg_sql);
        }

        #[cfg(all(feature = "turso", feature = "mysql", not(feature = "postgresql")))]
        {
            let turso_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::Turso);
            let mysql_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::MySQL);
            assert_ne!(turso_sql, mysql_sql);
        }

        #[cfg(all(feature = "postgresql", feature = "mysql", not(feature = "turso")))]
        {
            let pg_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::PostgreSQL);
            let mysql_sql = generate_create_table_sql::<TestCompleteTypes>(ormer::DbType::MySQL);
            assert_ne!(pg_sql, mysql_sql);
        }
    }

    async fn test_postgresql_complete_types_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestCompleteTypes>(config.0);

        // 根据不同的数据库类型进行不同的断言
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                // Turso 使用 INTEGER 和 TEXT
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT"));
                assert!(sql.contains("optional_int INTEGER"));
                assert!(sql.contains("bool_val INTEGER NOT NULL"));
                assert!(sql.contains("optional_bool INTEGER"));
                println!("Turso Complete Types SQL: {}", sql);
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                // PostgreSQL 使用 BIGINT、TEXT、INTEGER、BOOLEAN
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT"));
                assert!(sql.contains("optional_int INTEGER"));
                assert!(sql.contains("bool_val BOOLEAN NOT NULL"));
                assert!(sql.contains("optional_bool BOOLEAN"));
                println!("PostgreSQL Complete Types SQL: {}", sql);
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                // MySQL 使用 BIGINT、VARCHAR(255)、INT、TINYINT(1)
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(sql.contains("text_val VARCHAR(255) NOT NULL"));
                assert!(sql.contains("optional_text VARCHAR(255)"));
                assert!(sql.contains("optional_int INT"));
                assert!(sql.contains("bool_val TINYINT(1) NOT NULL"));
                assert!(sql.contains("optional_bool TINYINT(1)"));
                println!("MySQL Complete Types SQL: {}", sql);
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    async fn test_mysql_complete_types_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestCompleteTypes>(config.0);

        // 根据不同的数据库类型进行不同的断言
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                // Turso 使用 INTEGER 和 TEXT
                assert!(
                    sql.contains("id SERIAL PRIMARY KEY") || sql.contains("id INTEGER PRIMARY KEY")
                );
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT"));
                assert!(sql.contains("optional_int INTEGER"));
                assert!(sql.contains("bool_val INTEGER NOT NULL"));
                assert!(sql.contains("optional_bool INTEGER"));
                println!("Turso Complete Types SQL: {}", sql);
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                // PostgreSQL 使用 BIGINT、TEXT、INTEGER、BOOLEAN
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT"));
                assert!(sql.contains("optional_int INTEGER"));
                assert!(sql.contains("bool_val BOOLEAN NOT NULL"));
                assert!(sql.contains("optional_bool BOOLEAN"));
                println!("PostgreSQL Complete Types SQL: {}", sql);
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                // MySQL 使用 BIGINT、VARCHAR(255)、INT、TINYINT(1)
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(sql.contains("text_val VARCHAR(255) NOT NULL"));
                assert!(sql.contains("optional_text VARCHAR(255)"));
                assert!(sql.contains("optional_int INT"));
                assert!(sql.contains("bool_val TINYINT(1) NOT NULL"));
                assert!(sql.contains("optional_bool TINYINT(1)"));
                println!("MySQL Complete Types SQL: {}", sql);
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    async fn test_turso_complete_types_impl(config: &_test_common::DbConfig) {
        let _ = config; // 避免未使用变量警告
        let sql = generate_create_table_sql::<TestCompleteTypes>(config.0);

        // 根据不同的数据库类型进行不同的断言
        match config.0 {
            #[cfg(feature = "turso")]
            ormer::DbType::Turso => {
                assert!(sql.contains("id INTEGER PRIMARY KEY"));
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT")); // 不加 NOT NULL
                assert!(sql.contains("optional_int INTEGER")); // 不加 NOT NULL
                assert!(sql.contains("bool_val INTEGER NOT NULL"));
                assert!(sql.contains("optional_bool INTEGER")); // 不加 NOT NULL
            }
            #[cfg(feature = "postgresql")]
            ormer::DbType::PostgreSQL => {
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(sql.contains("text_val TEXT NOT NULL"));
                assert!(sql.contains("optional_text TEXT"));
                assert!(sql.contains("optional_int INTEGER"));
                assert!(sql.contains("bool_val BOOLEAN NOT NULL"));
                assert!(sql.contains("optional_bool BOOLEAN"));
            }
            #[cfg(feature = "mysql")]
            ormer::DbType::MySQL => {
                assert!(sql.contains("id BIGINT PRIMARY KEY"));
                assert!(!sql.contains("AUTO_INCREMENT")); // 没有auto，所以不应该有AUTO_INCREMENT
                assert!(sql.contains("text_val VARCHAR(255) NOT NULL"));
                assert!(sql.contains("optional_text VARCHAR(255)"));
                assert!(sql.contains("optional_int INT"));
                assert!(sql.contains("bool_val TINYINT(1) NOT NULL"));
                assert!(sql.contains("optional_bool TINYINT(1)"));
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }

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
        println!("{} Complete Types SQL: {}", db_type_name, sql);
    }

    test_on_all_dbs!(test_turso_create_table_sql_impl);
    test_on_all_dbs!(test_postgresql_create_table_sql_impl);
    test_on_all_dbs!(test_mysql_create_table_sql_impl);
    test_on_all_dbs!(test_different_databases_produce_different_sql_impl);
    test_on_all_dbs!(test_postgresql_complete_types_impl);
    test_on_all_dbs!(test_mysql_complete_types_impl);
    test_on_all_dbs!(test_turso_complete_types_impl);
}
