use ormer::Model;
use ormer::abstract_layer::DbType;
use ormer::generate_create_table_sql;

// 测试模型定义
#[derive(Model)]
#[table = "users"]
struct TestUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

// 测试更完整类型的模型
#[derive(Model)]
#[table = "complete_types"]
struct TestCompleteTypes {
    #[primary]
    id: i64,
    text_val: String,
    optional_text: Option<String>,
    optional_int: Option<i32>,
    bool_val: bool,
    optional_bool: Option<bool>,
}

#[cfg(test)]
mod create_table_tests {
    use super::*;

    #[cfg(feature = "turso")]
    #[test]
    fn test_turso_create_table_sql() {
        let sql = generate_create_table_sql::<TestUser>(DbType::Turso);

        // Turso/SQLite 应该使用 INTEGER PRIMARY KEY
        assert!(sql.contains("id INTEGER PRIMARY KEY"));
        assert!(sql.contains("name TEXT NOT NULL"));
        assert!(sql.contains("age INTEGER NOT NULL"));
        assert!(sql.contains("email TEXT")); // Option 类型，不加 NOT NULL

        println!("Turso SQL: {}", sql);
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn test_postgresql_create_table_sql() {
        let sql = generate_create_table_sql::<TestUser>(DbType::PostgreSQL);

        // PostgreSQL 没有 auto_increment，主键使用 INTEGER PRIMARY KEY
        assert!(sql.contains("id INTEGER PRIMARY KEY"));
        assert!(sql.contains("name VARCHAR NOT NULL"));
        assert!(sql.contains("age INTEGER NOT NULL"));
        assert!(sql.contains("email VARCHAR")); // Option 类型，不加 NOT NULL

        println!("PostgreSQL SQL: {}", sql);
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn test_mysql_create_table_sql() {
        let sql = generate_create_table_sql::<TestUser>(DbType::MySQL);

        // MySQL 没有 auto_increment 标记，主键使用 INT PRIMARY KEY
        assert!(sql.contains("id INT PRIMARY KEY"));
        assert!(sql.contains("name VARCHAR(255) NOT NULL"));
        assert!(sql.contains("age INT NOT NULL"));
        assert!(sql.contains("email VARCHAR(255)")); // Option 类型，不加 NOT NULL

        println!("MySQL SQL: {}", sql);
    }

    #[cfg(all(feature = "turso", feature = "postgresql", feature = "mysql"))]
    #[test]
    fn test_different_databases_produce_different_sql() {
        let turso_sql = generate_create_table_sql::<TestUser>(DbType::Turso);
        let pg_sql = generate_create_table_sql::<TestUser>(DbType::PostgreSQL);
        let mysql_sql = generate_create_table_sql::<TestUser>(DbType::MySQL);

        // 验证三个数据库生成的SQL确实不同
        assert_ne!(turso_sql, pg_sql);
        assert_ne!(turso_sql, mysql_sql);
        assert_ne!(pg_sql, mysql_sql);
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn test_postgresql_complete_types() {
        let sql = generate_create_table_sql::<TestCompleteTypes>(DbType::PostgreSQL);

        // 验证 PostgreSQL 完整类型映射（没有 auto_increment）
        assert!(sql.contains("id BIGINT PRIMARY KEY"));
        assert!(sql.contains("text_val VARCHAR NOT NULL"));
        assert!(sql.contains("optional_text VARCHAR")); // 不加 NOT NULL
        assert!(sql.contains("optional_int INTEGER")); // 不加 NOT NULL
        assert!(sql.contains("bool_val BOOLEAN NOT NULL"));
        assert!(sql.contains("optional_bool BOOLEAN")); // 不加 NOT NULL

        println!("PostgreSQL Complete Types SQL: {}", sql);
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn test_mysql_complete_types() {
        let sql = generate_create_table_sql::<TestCompleteTypes>(DbType::MySQL);

        // 验证 MySQL 完整类型映射（没有 auto_increment）
        assert!(sql.contains("id BIGINT PRIMARY KEY"));
        assert!(sql.contains("text_val VARCHAR(255) NOT NULL"));
        assert!(sql.contains("optional_text VARCHAR(255)")); // 不加 NOT NULL
        assert!(sql.contains("optional_int INT")); // 不加 NOT NULL
        assert!(sql.contains("bool_val TINYINT(1) NOT NULL"));
        assert!(sql.contains("optional_bool TINYINT(1)")); // 不加 NOT NULL

        println!("MySQL Complete Types SQL: {}", sql);
    }

    #[cfg(feature = "turso")]
    #[test]
    fn test_turso_complete_types() {
        let sql = generate_create_table_sql::<TestCompleteTypes>(DbType::Turso);

        // 验证 Turso 完整类型映射
        assert!(sql.contains("id INTEGER PRIMARY KEY"));
        assert!(sql.contains("text_val TEXT NOT NULL"));
        assert!(sql.contains("optional_text TEXT")); // 不加 NOT NULL
        assert!(sql.contains("optional_int INTEGER")); // 不加 NOT NULL
        assert!(sql.contains("bool_val INTEGER NOT NULL"));
        assert!(sql.contains("optional_bool INTEGER")); // 不加 NOT NULL

        println!("Turso Complete Types SQL: {}", sql);
    }
}
