mod _test_common;

#[derive(Debug, ormer::Model)]
#[table = "test_users_batch_1"]
struct TestUser1 {
    #[primary]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, ormer::Model)]
#[table = "test_users_batch_2"]
struct TestUser2 {
    #[primary]
    id: i32,
    name: String,
    age: i32,
}

async fn test_insert_single_and_batch_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 连接数据库
    let db = _test_common::create_db_connection(config).await?;

    // 先删除表（如果存在）
    let _ = db.drop_table::<TestUser1>().await;

    db.create_table::<TestUser1>().await?;

    // 测试插入单个对象
    db.insert(&TestUser1 {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
    })
    .await?;
    println!("插入单个对象成功");

    // 查询验证
    let users = db.select::<TestUser1>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");
    println!("第一次查询: {:?}", users);

    // 测试插入 Vec（使用 &vec![...]）
    db.insert(&vec![
        TestUser1 {
            id: 2,
            name: "Bob".to_string(),
            age: 25,
        },
        TestUser1 {
            id: 3,
            name: "Charlie".to_string(),
            age: 30,
        },
    ])
    .await?;
    println!("插入 Vec 成功");

    // 查询验证
    let users = db.select::<TestUser1>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 3);
    println!("第二次查询: {:?}", users);

    // 测试插入数组切片
    let users_array = vec![
        TestUser1 {
            id: 4,
            name: "David".to_string(),
            age: 35,
        },
        TestUser1 {
            id: 5,
            name: "Eve".to_string(),
            age: 28,
        },
    ];
    db.insert(&users_array).await?;
    println!("插入数组切片成功");

    // 查询验证
    let users = db.select::<TestUser1>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 5);
    println!("第三次查询: {:?}", users);

    println!("\n测试通过！insert 方法支持单个对象和数组");

    // 清理测试表
    db.drop_table::<TestUser1>().await?;

    Ok(())
}

async fn test_insert_or_update_single_and_batch_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 连接数据库
    let db = _test_common::create_db_connection(config).await?;

    // 先删除表（如果存在）
    let _ = db.drop_table::<TestUser2>().await;

    db.create_table::<TestUser2>().await?;

    // 测试插入或更新单个对象
    db.insert_or_update(&TestUser2 {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
    })
    .await?;
    println!("第一次 insert_or_update 单个对象成功");

    // 查询验证
    let users = db.select::<TestUser2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");
    println!("第一次查询: {:?}", users);

    // 使用 insert_or_update 更新同一条记录
    db.insert_or_update(&TestUser2 {
        id: 1,
        name: "Alice Updated".to_string(),
        age: 21,
    })
    .await?;
    println!("第二次 insert_or_update 单个对象成功（更新操作）");

    // 查询验证
    let users = db.select::<TestUser2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice Updated");
    assert_eq!(users[0].age, 21);
    println!("第二次查询: {:?}", users);

    // 测试批量 insert_or_update
    db.insert_or_update(&vec![
        TestUser2 {
            id: 1,
            name: "Alice Again".to_string(),
            age: 22,
        },
        TestUser2 {
            id: 2,
            name: "Bob".to_string(),
            age: 25,
        },
        TestUser2 {
            id: 3,
            name: "Charlie".to_string(),
            age: 30,
        },
    ])
    .await?;
    println!("批量 insert_or_update 成功");

    // 查询验证
    let users = db.select::<TestUser2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 3);
    assert_eq!(users[0].name, "Alice Again"); // id=1 被更新
    assert_eq!(users[1].name, "Bob"); // id=2 新插入
    assert_eq!(users[2].name, "Charlie"); // id=3 新插入
    println!("第三次查询: {:?}", users);

    println!("\n测试通过！insert_or_update 方法支持单个对象和数组");

    // 清理测试表
    db.drop_table::<TestUser2>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_insert_single_and_batch_impl);
test_on_all_dbs_result!(test_insert_or_update_single_and_batch_impl);
