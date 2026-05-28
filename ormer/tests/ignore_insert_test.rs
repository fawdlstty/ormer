#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

define_test_user_simple!(TestUserIgnore, "test_users_ignore_1");
define_test_user_simple!(TestUserIgnore2, "test_users_ignore_2");

async fn test_insert_or_ignore_basic_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserIgnore>().execute().await?;

    // 第一次插入
    db.insert_or_ignore(&TestUserIgnore {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
    })
    .execute()
    .await?;

    let users: Vec<TestUserIgnore> = db.select::<TestUserIgnore>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice");

    // 重复插入应被忽略
    db.insert_or_ignore(&TestUserIgnore {
        id: 1,
        name: "Bob".to_string(),
        age: 20,
    })
    .execute()
    .await?;

    let users: Vec<TestUserIgnore> = db.select::<TestUserIgnore>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "Alice"); // 未被覆盖

    db.drop_table::<TestUserIgnore>().execute().await?;
    Ok(())
}

async fn test_insert_or_ignore_batch_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUserIgnore2>().execute().await?;

    // 先插入一条
    db.insert(&TestUserIgnore2 {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
    })
    .execute()
    .await?;

    // 批量插入，部分重复
    db.insert_or_ignore(&vec![
        TestUserIgnore2 {
            id: 1,
            name: "Bob".to_string(),
            age: 20,
        },
        TestUserIgnore2 {
            id: 2,
            name: "Charlie".to_string(),
            age: 22,
        },
    ])
    .execute()
    .await?;

    let users: Vec<TestUserIgnore2> = db.select::<TestUserIgnore2>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 2);

    let user1 = users.iter().find(|u| u.id == 1).unwrap();
    assert_eq!(user1.name, "Alice"); // 未被覆盖

    let user2 = users.iter().find(|u| u.id == 2).unwrap();
    assert_eq!(user2.name, "Charlie");

    db.drop_table::<TestUserIgnore2>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_insert_or_ignore_basic_impl);
test_on_all_dbs_result!(test_insert_or_ignore_batch_impl);
