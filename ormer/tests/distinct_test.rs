#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_direct!(TestDistinctUser, "test_distinct_users_1");

// ---------------------------------------------------------------------------
// SQL 生成测试（不需要真实数据库连接）
// ---------------------------------------------------------------------------

/// 测试 Select::distinct() 生成 SELECT DISTINCT ... FROM ...
#[test]
fn test_distinct_full_select_sql() {
    let sql = ormer::Select::<TestDistinctUser>::new().distinct().to_sql();
    println!("DISTINCT full select SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT DISTINCT "),
        "Expected SQL to start with 'SELECT DISTINCT ', got: {}",
        sql
    );
    assert!(
        sql.contains("FROM test_distinct_users_1"),
        "Expected SQL to contain table name, got: {}",
        sql
    );
}

/// 测试不带 distinct 时仍然是普通 SELECT
#[test]
fn test_normal_select_sql_no_distinct() {
    let sql = ormer::Select::<TestDistinctUser>::new().to_sql();
    println!("Normal select SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT ") && !sql.starts_with("SELECT DISTINCT "),
        "Expected normal SELECT, got: {}",
        sql
    );
}

/// 测试 distinct() + map_to() 生成 SELECT DISTINCT col FROM ...
#[test]
fn test_distinct_map_to_sql() {
    let sql = ormer::Select::<TestDistinctUser>::new()
        .distinct()
        .map_to(|p| p.name)
        .to_sql();
    println!("DISTINCT map_to SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT DISTINCT name FROM test_distinct_users_1"),
        "Expected 'SELECT DISTINCT name FROM ...', got: {}",
        sql
    );
}

/// 测试 distinct() + filter() + map_to() 组合
#[test]
fn test_distinct_with_filter_map_to_sql() {
    let sql = ormer::Select::<TestDistinctUser>::new()
        .distinct()
        .filter(|p| p.age.ge(18))
        .map_to(|p| p.name)
        .to_sql();
    println!("DISTINCT + filter + map_to SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT DISTINCT name FROM test_distinct_users_1"),
        "Expected DISTINCT, got: {}",
        sql
    );
    assert!(
        sql.contains("WHERE age >="),
        "Expected WHERE clause, got: {}",
        sql
    );
}

/// 测试 distinct() + map_to() 多字段元组
#[test]
fn test_distinct_map_to_tuple_sql() {
    let sql = ormer::Select::<TestDistinctUser>::new()
        .distinct()
        .map_to(|p| (p.name, p.age))
        .to_sql();
    println!("DISTINCT map_to tuple SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT DISTINCT name, age FROM test_distinct_users_1"),
        "Expected 'SELECT DISTINCT name, age FROM ...', got: {}",
        sql
    );
}

/// 测试 distinct() + order_by() + range() 组合
#[test]
fn test_distinct_with_order_and_range_sql() {
    let sql = ormer::Select::<TestDistinctUser>::new()
        .distinct()
        .order_by(|p| p.name)
        .range(..10)
        .to_sql();
    println!("DISTINCT + order + range SQL: {}", sql);
    assert!(
        sql.starts_with("SELECT DISTINCT "),
        "Expected DISTINCT, got: {}",
        sql
    );
    assert!(
        sql.contains("ORDER BY name"),
        "Expected ORDER BY, got: {}",
        sql
    );
    assert!(sql.contains("LIMIT 10"), "Expected LIMIT, got: {}", sql);
}

// ---------------------------------------------------------------------------
// 端到端测试（需要数据库连接）
// ---------------------------------------------------------------------------

async fn test_distinct_e2e_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestDistinctUser>().execute().await?;

    // 插入有重复的数据
    db.insert(&TestDistinctUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .execute()
    .await?;
    db.insert(&TestDistinctUser {
        id: 2,
        name: "Alice".to_string(),
        age: 30,
    })
    .execute()
    .await?;
    db.insert(&TestDistinctUser {
        id: 3,
        name: "Bob".to_string(),
        age: 25,
    })
    .execute()
    .await?;
    db.insert(&TestDistinctUser {
        id: 4,
        name: "Charlie".to_string(),
        age: 35,
    })
    .execute()
    .await?;

    // DISTINCT name -> 应得到 3 个不同的名字
    let distinct_names: Vec<String> = db
        .select::<TestDistinctUser>()
        .distinct()
        .map_to(|p| p.name)
        .collect::<Vec<_>>()
        .await?;
    println!("Distinct names: {:?}", distinct_names);
    assert_eq!(distinct_names.len(), 3, "Expected 3 distinct names");

    // 普通 SELECT name -> 应得到 4 条记录
    let all_names: Vec<String> = db
        .select::<TestDistinctUser>()
        .map_to(|p| p.name)
        .collect::<Vec<_>>()
        .await?;
    println!("All names: {:?}", all_names);
    assert_eq!(all_names.len(), 4, "Expected 4 names without DISTINCT");

    // DISTINCT age -> 应得到 3 个不同年龄 (25, 30, 35)
    let distinct_ages: Vec<i32> = db
        .select::<TestDistinctUser>()
        .distinct()
        .map_to(|p| p.age)
        .collect::<Vec<_>>()
        .await?;
    println!("Distinct ages: {:?}", distinct_ages);
    assert_eq!(distinct_ages.len(), 3, "Expected 3 distinct ages");

    // DISTINCT 全字段查询
    let distinct_users: Vec<TestDistinctUser> = db
        .select::<TestDistinctUser>()
        .distinct()
        .collect::<Vec<_>>()
        .await?;
    println!("Distinct users count: {}", distinct_users.len());
    assert_eq!(
        distinct_users.len(),
        4,
        "All rows are distinct on full columns"
    );

    // 清理测试表
    db.drop_table::<TestDistinctUser>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_distinct_e2e_impl);
