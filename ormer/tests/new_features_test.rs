use ormer::Model;
use ormer::abstract_layer::DbType;
use ormer::generate_create_table_sql;

#[derive(Debug, Model)]
#[table = "test_users"]
struct TestUser {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}

#[derive(Debug, Model)]
#[table = "test_roles"]
struct TestRole {
    #[primary]
    id: i32,
    #[unique(group = 1)]
    uid: i32,
    #[unique(group = 1)]
    name: String,
}

#[test]
fn test_generate_sql_with_new_features() {
    let user_sql = generate_create_table_sql::<TestUser>(DbType::Turso);
    println!("User SQL: {}", user_sql);

    // 验证 AUTOINCREMENT
    assert!(
        user_sql.contains("AUTOINCREMENT"),
        "Should contain AUTOINCREMENT"
    );

    // 验证 name 字段的 UNIQUE
    assert!(
        user_sql.contains("name TEXT NOT NULL UNIQUE"),
        "name should be UNIQUE"
    );

    // 验证 age 字段的索引（索引是单独的语句）
    assert!(
        user_sql.contains("CREATE INDEX"),
        "Should contain CREATE INDEX"
    );
    assert!(
        user_sql.contains("idx_test_users_age"),
        "Should have index on age"
    );

    let role_sql = generate_create_table_sql::<TestRole>(DbType::Turso);
    println!("Role SQL: {}", role_sql);

    // 验证没有 AUTOINCREMENT（因为主键没有 auto）
    assert!(
        !role_sql.contains("AUTOINCREMENT"),
        "Should NOT contain AUTOINCREMENT"
    );

    // 验证联合唯一约束
    assert!(
        role_sql.contains("UNIQUE (uid, name)"),
        "Should have UNIQUE (uid, name)"
    );
}
