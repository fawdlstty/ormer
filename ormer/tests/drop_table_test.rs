#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

/// drop_table 功能测试
mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_direct!(TestDropBasicUser, "test_drop_basic_users_1");
define_test_user_direct!(TestDropIfExistsUser, "test_drop_if_exists_users_1");
define_test_user_direct!(TestDropMultipleUser, "test_drop_multiple_users_1");
define_test_user_minimal!(TestDropMultipleRole, "test_drop_multiple_roles_1");
define_test_user_direct!(TestDropRecreateUser, "test_drop_recreate_users_1");

async fn test_drop_table_basic_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropBasicUser>().execute().await;

    // 创建表
    db.create_table::<TestDropBasicUser>().execute().await?;

    // 删除表
    db.drop_table::<TestDropBasicUser>().execute().await?;

    // 再次删除（MySQL使用IF EXISTS，所以不会报错）
    db.drop_table::<TestDropBasicUser>().execute().await?;

    Ok(())
}

async fn test_drop_table_if_exists_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropIfExistsUser>().execute().await;

    // 创建表
    db.create_table::<TestDropIfExistsUser>().execute().await?;

    // 删除表
    db.drop_table::<TestDropIfExistsUser>().execute().await?;

    // 再次删除（应该不报错，因为使用了 IF EXISTS）
    db.drop_table::<TestDropIfExistsUser>().execute().await?;

    Ok(())
}

async fn test_drop_multiple_tables_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropMultipleUser>().execute().await;
    let _ = db.drop_table::<TestDropMultipleRole>().execute().await;

    // 创建多个表
    db.create_table::<TestDropMultipleUser>().execute().await?;
    db.create_table::<TestDropMultipleRole>().execute().await?;

    // 插入一些数据
    db.insert(&TestDropMultipleUser {
        id: 1,
        name: "Bob".to_string(),
        age: 25,
    })
    .await?;

    db.insert(&TestDropMultipleRole {
        id: 1,
        name: "admin".to_string(),
    })
    .await?;

    // 删除所有表
    db.drop_table::<TestDropMultipleUser>().execute().await?;
    db.drop_table::<TestDropMultipleRole>().execute().await?;

    // 验证表已删除 - 重新创建应该成功
    db.create_table::<TestDropMultipleUser>().execute().await?;
    db.create_table::<TestDropMultipleRole>().execute().await?;

    // 清理
    db.drop_table::<TestDropMultipleUser>().execute().await?;
    db.drop_table::<TestDropMultipleRole>().execute().await?;

    Ok(())
}

async fn test_drop_table_and_recreate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropRecreateUser>().execute().await;

    // 创建表
    db.create_table::<TestDropRecreateUser>().execute().await?;

    // 插入数据
    db.insert(&TestDropRecreateUser {
        id: 1,
        name: "Charlie".to_string(),
        age: 30,
    })
    .await?;

    db.insert(&TestDropRecreateUser {
        id: 2,
        name: "Diana".to_string(),
        age: 28,
    })
    .await?;

    // 查询验证数据存在
    let users = db
        .select::<TestDropRecreateUser>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 2);

    // 删除表
    db.drop_table::<TestDropRecreateUser>().execute().await?;

    // 重新创建表
    db.create_table::<TestDropRecreateUser>().execute().await?;

    // 验证表是空的
    let users = db
        .select::<TestDropRecreateUser>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 0);

    // 清理
    db.drop_table::<TestDropRecreateUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_drop_table_basic_impl);
test_on_all_dbs_result!(test_drop_table_if_exists_impl);
test_on_all_dbs_result!(test_drop_multiple_tables_impl);
test_on_all_dbs_result!(test_drop_table_and_recreate_impl);
