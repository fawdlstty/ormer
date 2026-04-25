use ormer::Model;

mod _test_common;

// 定义测试模型
#[derive(Debug, Model, Clone)]
#[table = "test_users_join"]
struct TestUserJoin {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    age: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_roles_join"]
struct TestRoleJoin {
    #[primary]
    id: i32,
    #[index]
    uid: i32,
    role_name: String,
}

/// 测试 INNER JOIN 查询
async fn test_inner_join_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserJoin>().await?;
    db.create_table::<TestRoleJoin>().await?;

    // 插入测试数据
    db.insert(&TestUserJoin {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .await?;
    db.insert(&TestUserJoin {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
    })
    .await?;

    // 只为 Alice 插入角色
    db.insert(&TestRoleJoin {
        id: 1,
        uid: 1,
        role_name: "admin".to_string(),
    })
    .await?;

    // 测试 INNER JOIN - 只返回有匹配的记录
    let user_roles: Vec<(TestUserJoin, TestRoleJoin)> = db
        .select::<TestUserJoin>()
        .inner_join::<TestRoleJoin>(|u, r| u.id.eq(r.uid))
        .collect::<Vec<_>>()
        .await?;

    // INNER JOIN 应该只返回 Alice（有角色的用户）
    assert_eq!(user_roles.len(), 1);
    assert_eq!(user_roles[0].0.name, "Alice");
    assert_eq!(user_roles[0].1.role_name, "admin");

    println!("✓ Inner join test passed: {} records", user_roles.len());

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRoleJoin>().await?;
    db.drop_table::<TestUserJoin>().await?;

    Ok(())
}

/// 测试 RIGHT JOIN 查询
/// 注意:SQLite/Turso 不支持 RIGHT JOIN,在Turso上跳过测试
#[cfg(not(feature = "turso"))]
async fn test_right_join_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserJoin>().await?;
    db.create_table::<TestRoleJoin>().await?;

    // 插入测试数据
    db.insert(&TestUserJoin {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .await?;
    // Bob 没有插入

    // 插入角色,包括一个没有对应用户的角色
    db.insert(&TestRoleJoin {
        id: 1,
        uid: 1,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&TestRoleJoin {
        id: 2,
        uid: 99, // 这个 uid 没有对应的用户
        role_name: "orphan_role".to_string(),
    })
    .await?;

    // 测试 RIGHT JOIN - 返回所有角色,即使没有匹配的用户
    let user_roles: Vec<(Option<TestUserJoin>, TestRoleJoin)> = db
        .select::<TestUserJoin>()
        .right_join::<TestRoleJoin>(|u, r| u.id.eq(r.uid))
        .collect::<Vec<_>>()
        .await?;

    // RIGHT JOIN 应该返回所有角色
    assert_eq!(user_roles.len(), 2);

    // 找到 admin 角色(有对应用户)
    let admin_role = user_roles
        .iter()
        .find(|(_, role)| role.role_name == "admin")
        .expect("Should find admin role");
    assert!(admin_role.0.is_some());
    assert_eq!(admin_role.0.as_ref().unwrap().name, "Alice");

    // 找到 orphan_role(没有对应用户)
    let orphan_role = user_roles
        .iter()
        .find(|(_, role)| role.role_name == "orphan_role")
        .expect("Should find orphan role");
    assert!(orphan_role.0.is_none());

    println!("✓ Right join test passed: {} records", user_roles.len());

    // 清理测试表(先删除有外键的表)
    db.drop_table::<TestRoleJoin>().await?;
    db.drop_table::<TestUserJoin>().await?;

    Ok(())
}

/// Turso版本:跳过RIGHT JOIN测试
#[cfg(feature = "turso")]
async fn test_right_join_impl(
    _config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("⊘ Right join test skipped on Turso (SQLite doesn't support RIGHT JOIN)");
    Ok(())
}

/// 测试 LEFT JOIN 查询（对比验证）
async fn test_left_join_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserJoin>().await?;
    db.create_table::<TestRoleJoin>().await?;

    // 插入测试数据
    db.insert(&TestUserJoin {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .await?;
    db.insert(&TestUserJoin {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
    })
    .await?;

    // 只为 Alice 插入角色
    db.insert(&TestRoleJoin {
        id: 1,
        uid: 1,
        role_name: "admin".to_string(),
    })
    .await?;

    // 测试 LEFT JOIN - 返回所有用户，即使没有角色
    let user_roles: Vec<(TestUserJoin, Option<TestRoleJoin>)> = db
        .select::<TestUserJoin>()
        .left_join::<TestRoleJoin>(|u, r| u.id.eq(r.uid))
        .collect::<Vec<_>>()
        .await?;

    // 打印结果用于调试
    println!("Left join results: {:?}", user_roles);

    // LEFT JOIN 应该返回所有用户
    assert_eq!(user_roles.len(), 2);

    // 找到 Alice（有角色）
    let alice = user_roles
        .iter()
        .find(|(user, _)| user.name == "Alice")
        .expect("Should find Alice");
    assert!(alice.1.is_some());
    assert_eq!(alice.1.as_ref().unwrap().role_name, "admin");

    // 找到 Bob（没有角色）
    let bob = user_roles
        .iter()
        .find(|(user, _)| user.name == "Bob")
        .expect("Should find Bob");
    assert!(bob.1.is_none());

    println!("✓ Left join test passed: {} records", user_roles.len());

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRoleJoin>().await?;
    db.drop_table::<TestUserJoin>().await?;

    Ok(())
}

/// 测试带条件的 INNER JOIN
async fn test_inner_join_with_filter_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserJoin>().await?;
    db.create_table::<TestRoleJoin>().await?;

    // 插入测试数据
    db.insert(&TestUserJoin {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .await?;
    db.insert(&TestUserJoin {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
    })
    .await?;

    db.insert(&TestRoleJoin {
        id: 1,
        uid: 1,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&TestRoleJoin {
        id: 2,
        uid: 2,
        role_name: "user".to_string(),
    })
    .await?;

    // 测试带 range 的 INNER JOIN
    let user_roles: Vec<(TestUserJoin, TestRoleJoin)> = db
        .select::<TestUserJoin>()
        .inner_join::<TestRoleJoin>(|u, r| u.id.eq(r.uid))
        .range(..1)
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(user_roles.len(), 1);

    println!(
        "✓ Inner join with range test passed: {} records",
        user_roles.len()
    );

    // 清理测试表（先删除有外键的表）
    db.drop_table::<TestRoleJoin>().await?;
    db.drop_table::<TestUserJoin>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_inner_join_impl);
test_on_all_dbs_result!(test_right_join_impl);
test_on_all_dbs_result!(test_left_join_impl);
test_on_all_dbs_result!(test_inner_join_with_filter_impl);
