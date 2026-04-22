use ormer::{Database, DbType, Model};

#[derive(Debug, Model)]
#[table = "exec_test_users"]
struct ExecTestUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

/// 测试 exec_table 方法 - 执行原生 SQL 查询并返回模型列表
#[tokio::test]
async fn test_exec_table() -> Result<(), Box<dyn std::error::Error>> {
    // 连接到内存数据库
    let db = Database::connect(DbType::Turso, ":memory:").await?;

    // 创建表
    db.create_table::<ExecTestUser>().await?;

    // 插入测试数据
    db.insert(&ExecTestUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .await?;

    db.insert(&ExecTestUser {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    })
    .await?;

    db.insert(&ExecTestUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
        email: None,
    })
    .await?;

    // 测试 exec_table - 查询所有用户
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users;")
        .await?;

    assert_eq!(users.len(), 3);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[2].name, "Charlie");

    // 测试 exec_table - 带 WHERE 条件的查询
    let young_users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users WHERE age < 30;")
        .await?;

    assert_eq!(young_users.len(), 1);
    assert_eq!(young_users[0].name, "Alice");
    assert_eq!(young_users[0].age, 25);

    // 测试 exec_table - 空结果
    let old_users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users WHERE age > 100;")
        .await?;

    assert_eq!(old_users.len(), 0);

    // 测试 exec_table - 排序查询
    let sorted_users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users ORDER BY age DESC;")
        .await?;

    assert_eq!(sorted_users.len(), 3);
    assert_eq!(sorted_users[0].name, "Charlie");
    assert_eq!(sorted_users[0].age, 35);
    assert_eq!(sorted_users[1].name, "Bob");
    assert_eq!(sorted_users[1].age, 30);
    assert_eq!(sorted_users[2].name, "Alice");
    assert_eq!(sorted_users[2].age, 25);

    Ok(())
}

/// 测试 exec_non_query 方法 - 执行原生非查询 SQL 并返回受影响行数
#[tokio::test]
async fn test_exec_non_query() -> Result<(), Box<dyn std::error::Error>> {
    // 连接到内存数据库
    let db = Database::connect(DbType::Turso, ":memory:").await?;

    // 创建表
    db.create_table::<ExecTestUser>().await?;

    // 插入测试数据
    db.insert(&ExecTestUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    })
    .await?;

    db.insert(&ExecTestUser {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
        email: Some("bob@example.com".to_string()),
    })
    .await?;

    db.insert(&ExecTestUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
        email: None,
    })
    .await?;

    // 测试 exec_non_query - UPDATE 语句
    let updated_rows = db
        .exec_non_query("UPDATE exec_test_users SET age = 40 WHERE age >= 30;")
        .await?;

    assert_eq!(updated_rows, 2); // Bob 和 Charlie 的年龄被更新

    // 验证更新结果
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users ORDER BY id;")
        .await?;

    assert_eq!(users[0].age, 25); // Alice 未变
    assert_eq!(users[1].age, 40); // Bob 已更新
    assert_eq!(users[2].age, 40); // Charlie 已更新

    // 测试 exec_non_query - DELETE 语句
    let deleted_rows = db
        .exec_non_query("DELETE FROM exec_test_users WHERE age < 30;")
        .await?;

    assert_eq!(deleted_rows, 1); // 只删除 Alice

    // 验证删除结果
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users;")
        .await?;

    assert_eq!(users.len(), 2); // 只剩下 Bob 和 Charlie

    // 测试 exec_non_query - INSERT 语句
    let inserted_rows = db
        .exec_non_query(
            "INSERT INTO exec_test_users (id, name, age, email) VALUES (4, 'David', 28, 'david@example.com');",
        )
        .await?;

    assert_eq!(inserted_rows, 1); // 插入了一行

    // 验证插入结果
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users;")
        .await?;

    assert_eq!(users.len(), 3); // 现在有 3 个用户

    // 测试 exec_non_query - 不影响任何行的 UPDATE
    let updated_rows = db
        .exec_non_query("UPDATE exec_test_users SET age = 99 WHERE age > 200;")
        .await?;

    assert_eq!(updated_rows, 0); // 没有符合条件的行

    Ok(())
}

/// 测试 exec_table 和 exec_non_query 的组合使用
#[tokio::test]
async fn test_exec_table_and_non_query_combined() -> Result<(), Box<dyn std::error::Error>> {
    // 连接到内存数据库
    let db = Database::connect(DbType::Turso, ":memory:").await?;

    // 创建表
    db.create_table::<ExecTestUser>().await?;

    // 使用 exec_non_query 插入数据
    db.exec_non_query(
        "INSERT INTO exec_test_users (id, name, age, email) VALUES 
         (1, 'Alice', 25, 'alice@example.com'),
         (2, 'Bob', 30, 'bob@example.com'),
         (3, 'Charlie', 35, NULL);",
    )
    .await?;

    // 使用 exec_table 查询数据
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users ORDER BY age;")
        .await?;

    assert_eq!(users.len(), 3);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].age, 25);
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[1].age, 30);
    assert_eq!(users[2].name, "Charlie");
    assert_eq!(users[2].age, 35);

    // 使用 exec_non_query 批量更新
    db.exec_non_query("UPDATE exec_test_users SET age = age + 5;")
        .await?;

    // 验证批量更新
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users ORDER BY age;")
        .await?;

    assert_eq!(users[0].age, 30);
    assert_eq!(users[1].age, 35);
    assert_eq!(users[2].age, 40);

    // 使用 exec_non_query 删除所有数据
    let deleted = db.exec_non_query("DELETE FROM exec_test_users;").await?;

    assert_eq!(deleted, 3);

    // 验证表为空
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users;")
        .await?;

    assert_eq!(users.len(), 0);

    Ok(())
}

/// 测试 exec_table 处理 NULL 值
#[tokio::test]
async fn test_exec_table_with_null_values() -> Result<(), Box<dyn std::error::Error>> {
    // 连接到内存数据库
    let db = Database::connect(DbType::Turso, ":memory:").await?;

    // 创建表
    db.create_table::<ExecTestUser>().await?;

    // 插入包含 NULL 值的数据
    db.exec_non_query(
        "INSERT INTO exec_test_users (id, name, age, email) VALUES 
         (1, 'Alice', 25, NULL),
         (2, 'Bob', 30, 'bob@example.com');",
    )
    .await?;

    // 查询并验证 NULL 值处理
    let users = db
        .exec_table::<ExecTestUser>("SELECT * FROM exec_test_users ORDER BY id;")
        .await?;

    assert_eq!(users.len(), 2);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].email, None); // NULL 应该被转换为 None
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[1].email, Some("bob@example.com".to_string()));

    Ok(())
}
