use ormer::Model;

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
    #[foreign(VerifyUser.id)]
    user_id: i32,
    role_name: String,
}

#[test]
fn test_foreign_key_sql_generation() {
    // 验证生成的 CREATE TABLE SQL 包含外键约束
    let sql = ormer::model::generate_create_table_sql::<VerifyRole>(ormer::DbType::Turso);

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

    // 验证完整的约束语句（注意：VerifyUser 转换为 verify_user）
    assert!(
        sql.contains("FOREIGN KEY (user_id) REFERENCES verify_user (id)"),
        "SQL should contain the complete foreign key constraint"
    );

    println!("Foreign key SQL generation test passed!");
}
