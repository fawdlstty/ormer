#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 定义单主键测试模型
#[derive(Debug, ormer::Model, Clone, PartialEq)]
#[table = "test_find_by_id_users"]
struct FindByIdUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
}

// 定义复合主键测试模型
#[derive(Debug, ormer::Model, Clone, PartialEq)]
#[table = "test_find_by_id_order_items"]
struct FindByIdOrderItem {
    #[primary]
    order_id: i32,
    #[primary]
    product_id: i32,
    quantity: i32,
}

async fn test_find_by_id_single_pk_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理并创建表
    let _ = db.drop_table::<FindByIdUser>().execute().await;
    db.create_table::<FindByIdUser>().execute().await?;

    // 插入测试数据
    db.insert(&FindByIdUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .execute()
    .await?;
    db.insert(&FindByIdUser {
        id: 2,
        name: "Bob".to_string(),
        age: 30,
    })
    .execute()
    .await?;
    db.insert(&FindByIdUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 35,
    })
    .execute()
    .await?;

    // 测试 find_by_id 找到记录
    let user = db.find_by_id::<FindByIdUser>(1).await?;
    assert!(user.is_some());
    let user = user.unwrap();
    assert_eq!(user.id, 1);
    assert_eq!(user.name, "Alice");
    assert_eq!(user.age, 25);

    // 测试 find_by_id 找到另一条记录
    let user = db.find_by_id::<FindByIdUser>(2).await?;
    assert!(user.is_some());
    let user = user.unwrap();
    assert_eq!(user.id, 2);
    assert_eq!(user.name, "Bob");

    // 测试 find_by_id 找不到记录
    let user = db.find_by_id::<FindByIdUser>(999).await?;
    assert!(user.is_none());

    // 清理
    db.drop_table::<FindByIdUser>().execute().await?;

    Ok(())
}

async fn test_find_by_id_composite_pk_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理并创建表
    let _ = db.drop_table::<FindByIdOrderItem>().execute().await;
    db.create_table::<FindByIdOrderItem>().execute().await?;

    // 插入测试数据
    db.insert(&FindByIdOrderItem {
        order_id: 1,
        product_id: 100,
        quantity: 5,
    })
    .execute()
    .await?;
    db.insert(&FindByIdOrderItem {
        order_id: 1,
        product_id: 101,
        quantity: 3,
    })
    .execute()
    .await?;
    db.insert(&FindByIdOrderItem {
        order_id: 2,
        product_id: 100,
        quantity: 2,
    })
    .execute()
    .await?;

    // 测试复合主键 find_by_id 找到记录
    let item = db.find_by_id::<FindByIdOrderItem>((1, 100)).await?;
    assert!(item.is_some());
    let item = item.unwrap();
    assert_eq!(item.order_id, 1);
    assert_eq!(item.product_id, 100);
    assert_eq!(item.quantity, 5);

    // 测试复合主键 find_by_id 找到另一条记录
    let item = db.find_by_id::<FindByIdOrderItem>((1, 101)).await?;
    assert!(item.is_some());
    let item = item.unwrap();
    assert_eq!(item.order_id, 1);
    assert_eq!(item.product_id, 101);
    assert_eq!(item.quantity, 3);

    // 测试复合主键 find_by_id 找不到记录
    let item = db.find_by_id::<FindByIdOrderItem>((999, 999)).await?;
    assert!(item.is_none());

    // 清理
    db.drop_table::<FindByIdOrderItem>().execute().await?;

    Ok(())
}

async fn test_find_by_id_in_transaction_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理并创建表
    let _ = db.drop_table::<FindByIdUser>().execute().await;
    db.create_table::<FindByIdUser>().execute().await?;

    // 插入测试数据
    db.insert(&FindByIdUser {
        id: 1,
        name: "Alice".to_string(),
        age: 25,
    })
    .execute()
    .await?;

    // 在事务中测试 find_by_id
    let txn = db.begin().await?;
    let user = txn.find_by_id::<FindByIdUser>(1).await?;
    assert!(user.is_some());
    let user = user.unwrap();
    assert_eq!(user.id, 1);
    assert_eq!(user.name, "Alice");
    txn.commit().await?;

    // 清理
    db.drop_table::<FindByIdUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_find_by_id_single_pk_impl);
test_on_all_dbs_result!(test_find_by_id_composite_pk_impl);
test_on_all_dbs_result!(test_find_by_id_in_transaction_impl);
