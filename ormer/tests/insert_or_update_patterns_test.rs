#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_simple!(TestUserIOU, "test_users_iou1_1");
define_test_role_with_unique_group!(TestRoleIOU, "test_roles_iou1_1");
define_test_user_simple!(TestUserIOU2, "test_users_iou2_1");
define_test_user_simple!(TestUserIOU3, "test_users_iou3_1");
#[allow(dead_code)]
define_test_user_simple!(TestUserIOU4, "test_users_iou4_1");
define_test_user_simple!(TestUserIOU5, "test_users_iou5_1");
define_test_role_with_unique_group!(TestRoleIOU4, "test_roles_iou4_1");

/// 测试 insert_or_update 的所有调用方式
async fn test_insert_or_update_all_patterns_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestRoleIOU>().execute().await;
    let _ = db.drop_table::<TestUserIOU>().execute().await;

    db.create_table::<TestUserIOU>().execute().await?;
    db.create_table::<TestRoleIOU>().execute().await?;

    // 1. 插入或更新单个对象引用 &T
    db.insert_or_update(&TestUserIOU {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
    })
    .await?;

    // 2. 插入或更新 Vec<T> 的引用 &vec![T {...}]
    db.insert_or_update(&vec![TestUserIOU {
        id: 2,
        name: "Bob".to_string(),
        age: 20,
    }])
    .await?;

    // 3. 插入或更新 Vec<T> 的引用 &vec![T {...}]
    db.insert_or_update(&vec![TestUserIOU {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
    }])
    .await?;

    // 4. 插入或更新数组引用 &[T; N]
    db.insert_or_update(&[TestUserIOU {
        id: 4,
        name: "David".to_string(),
        age: 24,
    }])
    .await?;

    // 5. 插入或更新数组引用 &[T; N]
    db.insert_or_update(&[TestUserIOU {
        id: 5,
        name: "Eve".to_string(),
        age: 26,
    }])
    .await?;

    // 6. 插入或更新数组切片 &[T; N][..]
    db.insert_or_update(
        &[TestUserIOU {
            id: 6,
            name: "Frank".to_string(),
            age: 28,
        }][..],
    )
    .await?;

    // 7. 插入或更新数组切片 &[T; N][..]
    db.insert_or_update(
        &[TestUserIOU {
            id: 7,
            name: "Grace".to_string(),
            age: 30,
        }][..],
    )
    .await?;

    // 验证所有数据插入成功
    let users: Vec<TestUserIOU> = db.select::<TestUserIOU>().collect::<Vec<_>>().await?;

    assert_eq!(users.len(), 7);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[1].name, "Bob");
    assert_eq!(users[2].name, "Charlie");
    assert_eq!(users[3].name, "David");
    assert_eq!(users[4].name, "Eve");
    assert_eq!(users[5].name, "Frank");
    assert_eq!(users[6].name, "Grace");

    println!("所有 insert_or_update 插入用法测试通过！");

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRoleIOU>().execute().await?;
    db.drop_table::<TestUserIOU>().execute().await?;

    Ok(())
}

/// 测试 insert_or_update 的更新功能（遇到重复键时更新）
async fn test_insert_or_update_update_behavior_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestUserIOU2>().execute().await;

    db.create_table::<TestUserIOU2>().execute().await?;

    // 第一次插入
    db.insert_or_update(&TestUserIOU2 {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
    })
    .await?;

    let users: Vec<TestUserIOU2> = db.select::<TestUserIOU2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].age, 18);

    // 使用 insert_or_update 更新同一条记录
    db.insert_or_update(&TestUserIOU2 {
        id: 1,
        name: "Alice Updated".to_string(),
        age: 25,
    })
    .await?;

    let users: Vec<TestUserIOU2> = db.select::<TestUserIOU2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice Updated");
    assert_eq!(users[0].age, 25);

    println!("insert_or_update 更新行为测试通过！");

    // 清理测试表
    db.drop_table::<TestUserIOU2>().execute().await?;

    Ok(())
}

/// 测试 insert_or_update 批量更新功能
async fn test_insert_or_update_batch_update_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestUserIOU3>().execute().await;

    db.create_table::<TestUserIOU3>().execute().await?;

    // 批量插入
    db.insert_or_update(&vec![
        TestUserIOU3 {
            id: 1,
            name: "User1".to_string(),
            age: 20,
        },
        TestUserIOU3 {
            id: 2,
            name: "User2".to_string(),
            age: 25,
        },
        TestUserIOU3 {
            id: 3,
            name: "User3".to_string(),
            age: 30,
        },
    ])
    .await?;

    let users: Vec<TestUserIOU3> = db.select::<TestUserIOU3>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 3);

    // 批量更新（部分更新，部分插入）
    db.insert_or_update(&vec![
        TestUserIOU3 {
            id: 1,
            name: "User1 Updated".to_string(),
            age: 21,
        },
        TestUserIOU3 {
            id: 4,
            name: "User4".to_string(),
            age: 35,
        },
    ])
    .await?;

    let users: Vec<TestUserIOU3> = db.select::<TestUserIOU3>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 4);

    // 查找 id=1 的用户（不依赖顺序）
    let user1 = users
        .iter()
        .find(|u| u.id == 1)
        .expect("User with id=1 should exist");
    assert_eq!(user1.name, "User1 Updated"); // id=1 被更新
    assert_eq!(user1.age, 21);

    // 查找 id=4 的用户（新插入）
    let user4 = users
        .iter()
        .find(|u| u.id == 4)
        .expect("User with id=4 should exist");
    assert_eq!(user4.name, "User4"); // id=4 新插入

    println!("insert_or_update 批量更新测试通过！");

    // 清理测试表
    db.drop_table::<TestUserIOU3>().execute().await?;

    Ok(())
}

/// 测试 insert_or_update 使用数组引用
async fn test_insert_or_update_with_array_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestRoleIOU4>().execute().await;

    db.create_table::<TestRoleIOU4>().execute().await?;

    // 使用数组引用插入
    db.insert_or_update(&[TestRoleIOU4 {
        id: 1,
        uid: 100,
        name: "admin".to_string(),
    }])
    .await?;

    // 使用数组切片更新
    db.insert_or_update(
        &[TestRoleIOU4 {
            id: 1,
            uid: 100,
            name: "super_admin".to_string(),
        }][..],
    )
    .await?;

    let roles: Vec<TestRoleIOU4> = db.select::<TestRoleIOU4>().collect::<Vec<_>>().await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "super_admin");

    println!("insert_or_update 数组引用测试通过！");

    // 清理测试表
    db.drop_table::<TestRoleIOU4>().execute().await?;

    Ok(())
}

/// 测试 insert 和 insert_or_update 混合使用
async fn test_insert_and_insert_or_update_mix_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<TestUserIOU5>().execute().await;

    db.create_table::<TestUserIOU5>().execute().await?;

    // 使用 insert 插入
    db.insert(&TestUserIOU5 {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
    })
    .await?;

    db.insert(&vec![TestUserIOU5 {
        id: 2,
        name: "Bob".to_string(),
        age: 20,
    }])
    .await?;

    // 使用 insert_or_update 更新
    db.insert_or_update(&TestUserIOU5 {
        id: 1,
        name: "Alice Updated".to_string(),
        age: 25,
    })
    .await?;

    // 使用 insert_or_update 插入新记录
    db.insert_or_update(&[TestUserIOU5 {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
    }])
    .await?;

    let users: Vec<TestUserIOU5> = db.select::<TestUserIOU5>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 3);

    // 查找特定用户（不依赖顺序）
    let alice = users
        .iter()
        .find(|u| u.id == 1)
        .expect("User with id=1 should exist");
    assert_eq!(alice.name, "Alice Updated");

    let bob = users
        .iter()
        .find(|u| u.id == 2)
        .expect("User with id=2 should exist");
    assert_eq!(bob.name, "Bob");

    let charlie = users
        .iter()
        .find(|u| u.id == 3)
        .expect("User with id=3 should exist");
    assert_eq!(charlie.name, "Charlie");

    println!("insert 和 insert_or_update 混合使用测试通过！");

    // 清理测试表
    db.drop_table::<TestUserIOU5>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_insert_or_update_all_patterns_impl);
test_on_all_dbs_result!(test_insert_or_update_update_behavior_impl);
test_on_all_dbs_result!(test_insert_or_update_batch_update_impl);
test_on_all_dbs_result!(test_insert_or_update_with_array_impl);
test_on_all_dbs_result!(test_insert_and_insert_or_update_mix_impl);
