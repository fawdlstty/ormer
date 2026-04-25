use ormer::Model;

mod _test_common;

// 定义与 main.rs 相同的测试模型
#[derive(Debug, Model, Clone)]
#[table = "test_users_insert"]
struct TestUserInsert {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}

#[derive(Debug, Model, Clone)]
#[table = "test_roles_insert"]
struct TestRoleInsert {
    #[primary]
    id: i32,
    #[unique(group = 1)]
    uid: i32,
    #[unique(group = 1)]
    name: String,
}

/// 测试所有 insert 调用方式（基于 main.rs 的用法）
async fn test_all_insert_patterns_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserInsert>().await?;
    db.create_table::<TestRoleInsert>().await?;

    // 1. 插入单个对象引用 &T
    db.insert(&TestUserInsert {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .await?;

    // 2. 插入 Vec<T> 的引用 &vec![T {...}]
    db.insert(&vec![TestUserInsert {
        id: 2,
        name: "Bob".to_string(),
        age: 20,
        email: Some("bob@example.com".to_string()),
    }])
    .await?;

    // 3. 插入 Vec<T> 的引用（原来是 &vec![&T {...}]，改为 &vec![T {...}]）
    db.insert(&vec![TestUserInsert {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        email: Some("charlie@example.com".to_string()),
    }])
    .await?;

    // 4. 插入数组引用 &[T; N]
    db.insert(&[TestUserInsert {
        id: 4,
        name: "David".to_string(),
        age: 24,
        email: Some("david@example.com".to_string()),
    }])
    .await?;

    // 5. 插入数组引用 &[T; N]
    db.insert(&[TestUserInsert {
        id: 5,
        name: "Eve".to_string(),
        age: 26,
        email: Some("eve@example.com".to_string()),
    }])
    .await?;

    // 6. 插入数组切片 &[T; N][..]
    db.insert(
        &[TestUserInsert {
            id: 6,
            name: "Frank".to_string(),
            age: 28,
            email: Some("frank@example.com".to_string()),
        }][..],
    )
    .await?;

    // 7. 插入数组切片 &[T; N][..]
    db.insert(
        &[TestUserInsert {
            id: 7,
            name: "Grace".to_string(),
            age: 30,
            email: Some("grace@example.com".to_string()),
        }][..],
    )
    .await?;

    // 测试 insert_or_update
    db.insert_or_update(&TestRoleInsert {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 验证所有数据插入成功
    let users: Vec<TestUserInsert> = db.select::<TestUserInsert>().collect::<Vec<_>>().await?;

    assert_eq!(users.len(), 7);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[2].name, "Charlie");
    assert_eq!(users[3].name, "David");
    assert_eq!(users[4].name, "Eve");
    assert_eq!(users[5].name, "Frank");
    assert_eq!(users[6].name, "Grace");

    let roles: Vec<TestRoleInsert> = db.select::<TestRoleInsert>().collect::<Vec<_>>().await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "admin");

    println!("所有 insert 用法测试通过！");

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRoleInsert>().await?;
    db.drop_table::<TestUserInsert>().await?;

    Ok(())
}

/// 测试批量插入功能
async fn test_batch_insert_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserInsert>().await?;

    // 批量插入多个用户
    db.insert(&vec![
        TestUserInsert {
            id: 1,
            name: "User1".to_string(),
            age: 20,
            email: Some("user1@example.com".to_string()),
        },
        TestUserInsert {
            id: 2,
            name: "User2".to_string(),
            age: 25,
            email: Some("user2@example.com".to_string()),
        },
        TestUserInsert {
            id: 3,
            name: "User3".to_string(),
            age: 30,
            email: None,
        },
    ])
    .await?;

    let users: Vec<TestUserInsert> = db.select::<TestUserInsert>().collect::<Vec<_>>().await?;

    assert_eq!(users.len(), 3);
    assert_eq!(users[0].name, "User1");
    assert_eq!(users[1].name, "User2");
    assert_eq!(users[2].name, "User3");

    println!("批量插入测试通过！");

    // 清理测试表
    db.drop_table::<TestUserInsert>().await?;

    Ok(())
}

/// 测试 insert_or_update 的所有调用方式
async fn test_insert_or_update_patterns_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestRoleInsert>().await?;

    // 1. 插入或更新单个对象引用 &T
    db.insert_or_update(&TestRoleInsert {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 2. 插入或更新 Vec<T> 的引用 &vec![T {...}]
    db.insert_or_update(&vec![TestRoleInsert {
        id: 2,
        uid: 2,
        name: "editor".to_string(),
    }])
    .await?;

    // 3. 插入或更新数组引用 &[T; N]
    db.insert_or_update(&[TestRoleInsert {
        id: 3,
        uid: 3,
        name: "viewer".to_string(),
    }])
    .await?;

    // 4. 插入或更新数组切片 &[T; N][..]
    db.insert_or_update(
        &[TestRoleInsert {
            id: 4,
            uid: 4,
            name: "guest".to_string(),
        }][..],
    )
    .await?;

    // 验证所有数据插入成功
    let roles: Vec<TestRoleInsert> = db.select::<TestRoleInsert>().collect::<Vec<_>>().await?;

    assert_eq!(roles.len(), 4);

    // 检查所有角色都存在（不依赖顺序）
    let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"admin"));
    assert!(names.contains(&"editor"));
    assert!(names.contains(&"viewer"));
    assert!(names.contains(&"guest"));

    println!("所有 insert_or_update 用法测试通过！");

    // 清理测试表
    db.drop_table::<TestRoleInsert>().await?;

    Ok(())
}

/// 测试 insert_or_update 的更新功能
async fn test_insert_or_update_update_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestRoleInsert>().await?;

    // 第一次插入
    db.insert_or_update(&TestRoleInsert {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 使用 insert_or_update 更新同一条记录
    db.insert_or_update(&TestRoleInsert {
        id: 1,
        uid: 1,
        name: "super_admin".to_string(),
    })
    .await?;

    let roles: Vec<TestRoleInsert> = db.select::<TestRoleInsert>().collect::<Vec<_>>().await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "super_admin");

    println!("insert_or_update 更新功能测试通过！");

    // 清理测试表
    db.drop_table::<TestRoleInsert>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_all_insert_patterns_impl);
test_on_all_dbs_result!(test_batch_insert_impl);
test_on_all_dbs_result!(test_insert_or_update_patterns_impl);
test_on_all_dbs_result!(test_insert_or_update_update_impl);
