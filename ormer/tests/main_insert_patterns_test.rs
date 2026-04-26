#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 为每个测试使用唯一的表名，避免并发测试冲突
define_test_user!(TestUserInsert, "test_users_insert_1");
define_test_role_with_unique_group!(TestRoleInsert, "test_roles_insert_1");

#[allow(dead_code)]
define_test_user!(TestUserInsert2, "test_users_insert_2");
#[allow(dead_code)]
define_test_role_with_unique_group!(TestRoleInsert2, "test_roles_insert_2");

#[allow(dead_code)]
define_test_user!(TestUserInsert3, "test_users_insert_3");
define_test_role_with_unique_group!(TestRoleInsert3, "test_roles_insert_3");

#[allow(dead_code)]
define_test_user!(TestUserInsert4, "test_users_insert_4");
define_test_role_with_unique_group!(TestRoleInsert4, "test_roles_insert_4");

/// 测试所有 insert 调用方式（基于 main.rs 的用法）
async fn test_all_insert_patterns_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestRoleInsert>().execute().await;
    let _ = db.drop_table::<TestUserInsert>().execute().await;

    db.create_table::<TestUserInsert>().execute().await?;
    db.create_table::<TestRoleInsert>().execute().await?;

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
    db.drop_table::<TestRoleInsert>().execute().await?;
    db.drop_table::<TestUserInsert>().execute().await?;

    Ok(())
}

/// 测试批量插入功能
async fn test_batch_insert_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestUserInsert2>().execute().await;

    db.create_table::<TestUserInsert2>().execute().await?;

    // 批量插入多个用户
    db.insert(&vec![
        TestUserInsert2 {
            id: 1,
            name: "User1".to_string(),
            age: 20,
            email: Some("user1@example.com".to_string()),
        },
        TestUserInsert2 {
            id: 2,
            name: "User2".to_string(),
            age: 25,
            email: Some("user2@example.com".to_string()),
        },
        TestUserInsert2 {
            id: 3,
            name: "User3".to_string(),
            age: 30,
            email: None,
        },
    ])
    .await?;

    let users: Vec<TestUserInsert2> = db.select::<TestUserInsert2>().collect::<Vec<_>>().await?;

    assert_eq!(users.len(), 3);
    assert_eq!(users[0].name, "User1");
    assert_eq!(users[1].name, "User2");
    assert_eq!(users[2].name, "User3");

    println!("批量插入测试通过！");

    // 清理测试表
    db.drop_table::<TestUserInsert2>().execute().await?;

    Ok(())
}

/// 测试 insert_or_update 的所有调用方式
async fn test_insert_or_update_patterns_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestRoleInsert3>().execute().await;

    db.create_table::<TestRoleInsert3>().execute().await?;

    // 1. 插入或更新单个对象引用 &T
    db.insert_or_update(&TestRoleInsert3 {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 2. 插入或更新 Vec<T> 的引用 &vec![T {...}]
    db.insert_or_update(&vec![TestRoleInsert3 {
        id: 2,
        uid: 2,
        name: "editor".to_string(),
    }])
    .await?;

    // 3. 插入或更新数组引用 &[T; N]
    db.insert_or_update(&[TestRoleInsert3 {
        id: 3,
        uid: 3,
        name: "viewer".to_string(),
    }])
    .await?;

    // 4. 插入或更新数组切片 &[T; N][..]
    db.insert_or_update(
        &[TestRoleInsert3 {
            id: 4,
            uid: 4,
            name: "guest".to_string(),
        }][..],
    )
    .await?;

    // 验证所有数据插入成功
    let roles: Vec<TestRoleInsert3> = db.select::<TestRoleInsert3>().collect::<Vec<_>>().await?;

    assert_eq!(roles.len(), 4);

    // 检查所有角色都存在（不依赖顺序）
    let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"admin"));
    assert!(names.contains(&"editor"));
    assert!(names.contains(&"viewer"));
    assert!(names.contains(&"guest"));

    println!("所有 insert_or_update 用法测试通过！");

    // 清理测试表
    db.drop_table::<TestRoleInsert3>().execute().await?;

    Ok(())
}

/// 测试 insert_or_update 的更新功能
async fn test_insert_or_update_update_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestRoleInsert4>().execute().await;

    db.create_table::<TestRoleInsert4>().execute().await?;

    // 第一次插入
    db.insert_or_update(&TestRoleInsert4 {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 使用 insert_or_update 更新同一条记录
    db.insert_or_update(&TestRoleInsert4 {
        id: 1,
        uid: 1,
        name: "super_admin".to_string(),
    })
    .await?;

    let roles: Vec<TestRoleInsert4> = db.select::<TestRoleInsert4>().collect::<Vec<_>>().await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "super_admin");

    println!("insert_or_update 更新功能测试通过！");

    // 清理测试表
    db.drop_table::<TestRoleInsert4>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_all_insert_patterns_impl);
test_on_all_dbs_result!(test_batch_insert_impl);
test_on_all_dbs_result!(test_insert_or_update_patterns_impl);
test_on_all_dbs_result!(test_insert_or_update_update_impl);
