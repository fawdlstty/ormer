#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::Model;
use ormer::generate_create_table_sql;

mod _test_common;

// 定义复合主键模型：用户角色关联表
#[derive(Debug, ormer::Model, Clone)]
#[table = "composite_pk_user_roles_1"]
struct UserRole {
    #[primary]
    user_id: i32,
    #[primary]
    role_id: i32,
    assigned_at: String,
}

// 定义带自增的复合主键模型（只有第一个主键可以是 auto）
#[derive(Debug, ormer::Model, Clone)]
#[table = "composite_pk_auto_test_1"]
struct OrderItem {
    #[primary(auto)]
    id: i32,
    #[primary]
    product_id: i32,
    quantity: i32,
}

#[cfg(test)]
mod composite_primary_key_tests {
    use super::*;

    #[test]
    fn test_composite_pk_sql_generation() {
        // 测试 Turso (SQLite) 的 SQL 生成
        let sql = generate_create_table_sql::<UserRole>(ormer::DbType::Turso);
        println!("UserRole SQL: {}", sql);

        assert!(sql.contains("CREATE TABLE IF NOT EXISTS composite_pk_user_roles_1"));
        assert!(sql.contains("user_id INTEGER NOT NULL"));
        assert!(sql.contains("role_id INTEGER NOT NULL"));
        assert!(sql.contains("assigned_at TEXT NOT NULL"));
        // 应该有复合主键约束
        assert!(sql.contains("PRIMARY KEY (user_id, role_id)"));
    }

    #[test]
    fn test_composite_pk_with_auto_sql_generation() {
        // 测试带 auto 的复合主键
        let sql = generate_create_table_sql::<OrderItem>(ormer::DbType::Turso);
        println!("OrderItem SQL: {}", sql);

        assert!(sql.contains("CREATE TABLE IF NOT EXISTS composite_pk_auto_test_1"));
        // 第一个主键有 auto，但因为是复合主键，所以不在列级别标记
        assert!(sql.contains("id INTEGER NOT NULL"));
        assert!(sql.contains("product_id INTEGER NOT NULL"));
        assert!(sql.contains("quantity INTEGER NOT NULL"));
        // 应该有复合主键约束
        assert!(sql.contains("PRIMARY KEY (id, product_id)"));
    }

    #[test]
    fn test_composite_pk_primary_key_columns() {
        // 测试获取复合主键列名
        let columns = <UserRole as ormer::Model>::primary_key_columns();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0], "user_id");
        assert_eq!(columns[1], "role_id");
    }

    #[test]
    fn test_composite_pk_primary_key_values() {
        // 测试获取复合主键值
        let role = UserRole {
            user_id: 123,
            role_id: 456,
            assigned_at: "2024-01-01".to_string(),
        };

        let values = role.primary_key_values();
        assert_eq!(values.len(), 2);

        // 清理测试表（如果存在）
        // 注意：这里只是单元测试，不需要实际连接数据库
    }

    #[test]
    fn test_composite_pk_multiple_databases() {
        // 测试不同数据库的 SQL 生成
        let sql_turso = generate_create_table_sql::<UserRole>(ormer::DbType::Turso);
        assert!(sql_turso.contains("PRIMARY KEY (user_id, role_id)"));

        #[cfg(feature = "postgresql")]
        {
            let sql_pg = generate_create_table_sql::<UserRole>(ormer::DbType::PostgreSQL);
            assert!(sql_pg.contains("PRIMARY KEY (user_id, role_id)"));
            println!("PostgreSQL SQL: {}", sql_pg);
        }

        #[cfg(feature = "mysql")]
        {
            let sql_mysql = generate_create_table_sql::<UserRole>(ormer::DbType::MySQL);
            assert!(sql_mysql.contains("PRIMARY KEY (user_id, role_id)"));
            println!("MySQL SQL: {}", sql_mysql);
        }
    }
}
