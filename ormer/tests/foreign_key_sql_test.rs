use ormer::Model;

mod _test_common;

#[derive(Debug, Model)]
#[table = "verify_users"]
struct VerifyUser {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Model)]
#[table = "verify_roles"]
struct VerifyRole {
    #[primary]
    id: i32,
    #[foreign(VerifyUser)]
    user_id: i32,
    role_name: String,
}

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
        sql.contains("verify_user"),
        "SQL should reference the correct table"
    );
    assert!(
        sql.contains("id"),
        "SQL should reference the correct column"
    );

    // 验证完整的约束语句（注意：VerifyUser 的表名是 verify_users）
    assert!(
        sql.contains("FOREIGN KEY (user_id) REFERENCES verify_users (id)"),
        "SQL should contain the complete foreign key constraint"
    );

    println!("Foreign key SQL generation test passed!");
}

test_on_all_dbs!(test_foreign_key_sql_generation_impl);
