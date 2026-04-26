#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// ==================== 测试Model定义 ====================
// 使用宏定义测试专用模型（唯一表名）
define_test_user_for_map_to!(User, "map_to_users_1");
define_test_role_for_map_to!(Role, "map_to_roles_1");
define_test_user_id_with_eq!(UserId, "map_to_user_ids_1");
define_test_user_name_age!(UserNameAge, "map_to_user_name_age_1");

// ==================== 测试用例 ====================

async fn test_map_to_complete_usage_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========== map_to 完整用法测试 ==========\n");

    // 创建内存数据库
    let db = _test_common::create_db_connection(config).await?;

    // 清理可能存在的旧表
    let _ = db.drop_table::<Role>().execute().await;
    let _ = db.drop_table::<User>().execute().await;

    // 创建表
    db.create_table::<User>().execute().await?;
    db.create_table::<Role>().execute().await?;

    // 插入测试数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        age: 25,
    })
    .await?;
    db.insert(&User {
        id: 2,
        name: "Bob".to_string(),
        email: "bob@example.com".to_string(),
        age: 30,
    })
    .await?;
    db.insert(&User {
        id: 3,
        name: "Charlie".to_string(),
        email: "charlie@example.com".to_string(),
        age: 35,
    })
    .await?;

    db.insert(&Role {
        id: 1,
        user_id: 1,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&Role {
        id: 2,
        user_id: 2,
        role_name: "admin".to_string(),
    })
    .await?;
    db.insert(&Role {
        id: 3,
        user_id: 3,
        role_name: "user".to_string(),
    })
    .await?;

    // ==================== 测试1: 单字段映射 - 收集为基本类型 ====================
    println!("测试1: 单字段映射 - 收集为基本类型");
    let admin_user_ids: Vec<i32> = db
        .select::<Role>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.user_id)
        .collect::<Vec<i32>>()
        .await?;

    println!("  Admin user IDs: {:?}", admin_user_ids);
    assert_eq!(admin_user_ids, vec![1, 2]);
    println!("  ✅ 测试通过\n");

    // ==================== 测试2: 单字段映射 - 使用collect_with转换为单字段Model ====================
    println!("测试2: 单字段映射 - collect_with转换为单字段Model");
    let admin_user_ids_model: Vec<UserId> = db
        .select::<Role>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.user_id)
        .collect_with(|id| UserId { id })
        .await?;

    println!("  Admin user IDs (as Model): {:?}", admin_user_ids_model);
    assert_eq!(admin_user_ids_model.len(), 2);
    assert_eq!(admin_user_ids_model[0], UserId { id: 1 });
    assert_eq!(admin_user_ids_model[1], UserId { id: 2 });
    println!("  ✅ 测试通过\n");

    // ==================== 测试3: 单字段映射 - 收集为String类型 ====================
    println!("测试3: 单字段映射 - 收集为String类型");
    let admin_role_names: Vec<String> = db
        .select::<Role>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.role_name)
        .collect::<Vec<String>>()
        .await?;

    println!("  Admin role names: {:?}", admin_role_names);
    assert_eq!(
        admin_role_names,
        vec!["admin".to_string(), "admin".to_string()]
    );
    println!("  ✅ 测试通过\n");

    // ==================== 测试4: 元组投影 - 二元组 ====================
    println!("测试4: 元组投影 - 二元组");
    let role_pairs: Vec<(i32, String)> = db
        .select::<Role>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| (r.user_id, r.role_name))
        .collect::<Vec<(i32, String)>>()
        .await?;

    println!("  Role pairs (user_id, role_name): {:?}", role_pairs);
    assert_eq!(role_pairs.len(), 2);
    assert_eq!(role_pairs[0], (1, "admin".to_string()));
    assert_eq!(role_pairs[1], (2, "admin".to_string()));
    println!("  ✅ 测试通过\n");

    // ==================== 测试5: 元组投影 - 三元组 ====================
    println!("测试5: 元组投影 - 三元组");
    let user_triples: Vec<(i32, String, i32)> = db
        .select::<User>()
        .filter(|u| u.age.ge(30))
        .map_to(|u| (u.id, u.name, u.age))
        .collect::<Vec<(i32, String, i32)>>()
        .await?;

    println!("  User triples (id, name, age): {:?}", user_triples);
    assert_eq!(user_triples.len(), 2);
    assert_eq!(user_triples[0], (2, "Bob".to_string(), 30));
    assert_eq!(user_triples[1], (3, "Charlie".to_string(), 35));
    println!("  ✅ 测试通过\n");

    // ==================== 测试6: collect_with转换为多字段Model ====================
    println!("测试6: collect_with转换为多字段Model");
    let user_name_ages: Vec<UserNameAge> = db
        .select::<User>()
        .filter(|u| u.age.ge(30))
        .map_to(|u| (u.name, u.age))
        .collect_with(|(name, age)| UserNameAge { name, age })
        .await?;

    println!("  User name&age (as Model): {:?}", user_name_ages);
    assert_eq!(user_name_ages.len(), 2);
    assert_eq!(
        user_name_ages[0],
        UserNameAge {
            name: "Bob".to_string(),
            age: 30
        }
    );
    assert_eq!(
        user_name_ages[1],
        UserNameAge {
            name: "Charlie".to_string(),
            age: 35
        }
    );
    println!("  ✅ 测试通过\n");

    // ==================== 测试7: 结合order_by使用 ====================
    println!("测试7: 结合order_by使用");
    let sorted_ages: Vec<i32> = db
        .select::<User>()
        .order_by(|u| u.age.desc())
        .map_to(|u| u.age)
        .collect::<Vec<i32>>()
        .await?;

    println!("  Sorted ages (desc): {:?}", sorted_ages);
    assert_eq!(sorted_ages, vec![35, 30, 25]);
    println!("  ✅ 测试通过\n");

    // ==================== 测试8: 结合range使用 ====================
    println!("测试8: 结合range使用（limit/offset）");
    let first_two_names: Vec<String> = db
        .select::<User>()
        .order_by(|u| u.id.asc())
        .range(0..2)
        .map_to(|u| u.name)
        .collect::<Vec<String>>()
        .await?;

    println!("  First two user names: {:?}", first_two_names);
    assert_eq!(
        first_two_names,
        vec!["Alice".to_string(), "Bob".to_string()]
    );
    println!("  ✅ 测试通过\n");

    // ==================== 测试9: 多次复用同一个查询 ====================
    println!("测试9: 多次复用同一个查询对象");
    let query = db
        .select::<Role>()
        .filter(|r| r.role_name.eq("admin"))
        .map_to(|r| r.user_id);

    // 第一次collect
    let ids_1: Vec<i32> = query.clone().collect::<Vec<i32>>().await?;
    println!("  First collect: {:?}", ids_1);
    assert_eq!(ids_1, vec![1, 2]);

    // 第二次collect（使用collect_with）
    let ids_2: Vec<UserId> = query.collect_with(|id| UserId { id }).await?;
    println!("  Second collect (as Model): {:?}", ids_2);
    assert_eq!(ids_2.len(), 2);
    println!("  ✅ 测试通过\n");

    // ==================== 测试10: 过滤条件组合 ====================
    println!("测试10: 过滤条件组合");
    let filtered_users: Vec<(i32, String)> = db
        .select::<User>()
        .filter(|u| u.age.ge(25))
        .filter(|u| u.age.le(30))
        .map_to(|user| (user.id, user.name))
        .collect::<Vec<(i32, String)>>()
        .await?;

    println!("  Users with age 25-30: {:?}", filtered_users);
    assert_eq!(filtered_users.len(), 2);
    assert_eq!(filtered_users[0], (1, "Alice".to_string()));
    assert_eq!(filtered_users[1], (2, "Bob".to_string()));
    println!("  ✅ 测试通过\n");

    println!("========== 所有测试通过！==========\n");

    // 清理测试表（先删除有外键的表）
    db.drop_table::<Role>().execute().await?;
    db.drop_table::<User>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_map_to_complete_usage_impl);
