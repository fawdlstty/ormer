#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_fk!(VerifyUser, "verify_fk_users_1");
define_test_role_for_fk!(VerifyRole, "verify_fk_roles_1", VerifyUser);

async fn test_foreign_key_sql_generation_impl(config: &_test_common::DbConfig) {
    // 验证生成的 CREATE TABLE SQL 包含外键约束
    let sql = ormer::model::generate_create_table_sql::<VerifyRole>(config.0);

    println!("Generated SQL:\n{}", sql);

    // 验证 SQL 包含外键约束
    assert!(
        sql.contains("FOREIGN KEY"),
        "SQL should contain FOREIGN KEY constraint"
    );
    assert!(
        sql.contains("verify_fk_users_1"),
        "SQL should reference the correct table"
    );
    assert!(
        sql.contains("id"),
        "SQL should reference the correct column"
    );

    // 验证完整的约束语句（注意：VerifyUser 的表名是 verify_fk_users_1）
    assert!(
        sql.contains("FOREIGN KEY (user_id) REFERENCES verify_fk_users_1 (id)"),
        "SQL should contain the complete foreign key constraint"
    );

    println!("Foreign key SQL generation test passed!");
}

test_on_all_dbs!(test_foreign_key_sql_generation_impl);
