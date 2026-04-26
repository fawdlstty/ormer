#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::generate_create_table_sql;

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(TestUser, "test_new_features_users_1");
define_test_role_with_unique_group!(TestRole, "test_new_features_roles_1");

async fn test_generate_sql_with_new_features_impl(config: &_test_common::DbConfig) {
    let user_sql = generate_create_table_sql::<TestUser>(config.0);
    println!("User SQL: {}", user_sql);

    // 验证 AUTOINCREMENT (Turso/SQLite) 或 SERIAL (PostgreSQL) 或 AUTO_INCREMENT (MySQL)
    let has_auto_increment = user_sql.contains("AUTOINCREMENT")
        || user_sql.contains("SERIAL")
        || user_sql.contains("AUTO_INCREMENT");
    assert!(
        has_auto_increment,
        "Should contain AUTOINCREMENT, SERIAL, or AUTO_INCREMENT"
    );

    // 验证 name 字段的 UNIQUE（不同数据库类型不同：TEXT for Turso/PostgreSQL, VARCHAR for MySQL）
    let has_unique_name = user_sql.contains("name TEXT NOT NULL UNIQUE")
        || user_sql.contains("name VARCHAR(255) NOT NULL UNIQUE");
    assert!(has_unique_name, "name should be UNIQUE");

    // 验证 age 字段的索引（索引是单独的语句）
    assert!(
        user_sql.contains("CREATE INDEX"),
        "Should contain CREATE INDEX"
    );
    assert!(
        user_sql.contains("idx_test_new_features_users_1_age"),
        "Should have index on age"
    );

    let role_sql = generate_create_table_sql::<TestRole>(config.0);
    println!("Role SQL: {}", role_sql);

    // 验证没有 AUTOINCREMENT/SERIAL/AUTO_INCREMENT（因为主键没有 auto）
    let has_auto = role_sql.contains("AUTOINCREMENT")
        || role_sql.contains("SERIAL")
        || role_sql.contains("AUTO_INCREMENT");
    assert!(
        !has_auto,
        "Should NOT contain AUTOINCREMENT, SERIAL, or AUTO_INCREMENT"
    );

    // 验证联合唯一约束
    assert!(
        role_sql.contains("UNIQUE (uid, name)"),
        "Should have UNIQUE (uid, name)"
    );
}

test_on_all_dbs!(test_generate_sql_with_new_features_impl);
