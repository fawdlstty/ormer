#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(TestUser2, "step_by_step_users_1");

async fn test_step_by_step_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUser2>().execute().await?;

    // insert
    db.insert(&TestUser2 {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .await?;
    println!("Insert OK");

    // simple query
    let users = db.select::<TestUser2>().collect::<Vec<_>>().await?;
    println!("Simple query OK: {:?}", users);

    // query with filter
    let users = db
        .select::<TestUser2>()
        .filter(|p| p.age.ge(18))
        .collect::<Vec<_>>()
        .await?;
    println!("Filter query OK: {:?}", users);

    // query with order_by
    let users = db
        .select::<TestUser2>()
        .order_by(|p| p.age)
        .collect::<Vec<_>>()
        .await?;
    println!("Order by query OK: {:?}", users);

    // query with range
    let users = db
        .select::<TestUser2>()
        .range(0..10)
        .collect::<Vec<_>>()
        .await?;
    println!("Range query OK: {:?}", users);

    // combined query
    let users = db
        .select::<TestUser2>()
        .filter(|p| p.age.ge(18))
        .order_by(|p| p.age)
        .range(0..10)
        .collect::<Vec<_>>()
        .await?;
    println!("Combined query OK: {:?}", users);

    // 清理测试表
    db.drop_table::<TestUser2>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_step_by_step_impl);
