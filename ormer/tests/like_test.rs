#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 定义测试模型（包含 email: Option<String> 字段）
define_test_user!(TestUser, "test_like_users");

// ==================== SQL 生成测试 ====================

async fn test_like_sql_impl(config: &_test_common::DbConfig) {
    let _config = config;
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.like("%alice%"))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name LIKE ?"));
    assert!(sql.contains("WHERE"));
}

async fn test_contains_sql_impl(config: &_test_common::DbConfig) {
    let _config = config;
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.contains("alice"))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name LIKE ?"));
}

async fn test_starts_with_sql_impl(config: &_test_common::DbConfig) {
    let _config = config;
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.starts_with("al"))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name LIKE ?"));
}

async fn test_ends_with_sql_impl(config: &_test_common::DbConfig) {
    let _config = config;
    let sql = ormer::Select::<TestUser>::new()
        .filter(|p| p.name.ends_with("ce"))
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.contains("name LIKE ?"));
}

test_on_all_dbs!(test_like_sql_impl);
test_on_all_dbs!(test_contains_sql_impl);
test_on_all_dbs!(test_starts_with_sql_impl);
test_on_all_dbs!(test_ends_with_sql_impl);

// ==================== 端到端查询测试 ====================

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_like_e2e_sqlite() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::get_sqlite_config();
    let db = _test_common::create_db_connection(&config[0]).await?;
    db.create_table::<TestUser>().execute().await?;

    // 插入测试数据
    db.insert(&TestUser {
        id: 0,
        name: "Alice".to_string(),
        age: 30,
        email: None,
    })
    .execute()
    .await?;
    db.insert(&TestUser {
        id: 0,
        name: "Bob".to_string(),
        age: 25,
        email: None,
    })
    .execute()
    .await?;
    db.insert(&TestUser {
        id: 0,
        name: "Alicia".to_string(),
        age: 28,
        email: None,
    })
    .execute()
    .await?;
    db.insert(&TestUser {
        id: 0,
        name: "Charlie".to_string(),
        age: 35,
        email: None,
    })
    .execute()
    .await?;

    // 测试 like: 以 "Al" 开头
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.like("Al%"))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|u| u.name.starts_with("Al")));

    // 测试 contains: 包含 "lic"
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.contains("lic"))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 2); // Alice, Alicia
    assert!(result.iter().all(|u| u.name.contains("lic")));

    // 测试 starts_with: 以 "Al" 开头
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.starts_with("Al"))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 2); // Alice, Alicia
    assert!(result.iter().all(|u| u.name.starts_with("Al")));

    // 测试 ends_with: 以 "e" 结尾
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.ends_with("e"))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 2); // Alice, Charlie
    assert!(result.iter().all(|u| u.name.ends_with('e')));

    // 测试无匹配
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.starts_with("Z"))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 0);

    // 测试 like 与其他过滤条件组合
    let result = db
        .select::<TestUser>()
        .filter(|p| p.name.contains("li").and(p.age.gt(29)))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(result.len(), 2); // Alice (age=30, 包含 "li") 和 Charlie (age=35, 包含 "li")
    assert!(result.iter().any(|u| u.name == "Alice"));
    assert!(result.iter().any(|u| u.name == "Charlie"));

    Ok(())
}
