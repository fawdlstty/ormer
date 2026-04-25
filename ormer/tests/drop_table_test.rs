/// drop_table 功能测试
use ormer::Model;

mod _test_common;

// 为每个测试使用不同的表名以避免并发冲突
#[derive(Debug, Model, Clone)]
#[table = "test_drop_basic_users"]
struct TestDropBasicUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_drop_if_exists_users"]
struct TestDropIfExistsUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_drop_multiple_users"]
struct TestDropMultipleUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model, Clone)]
#[table = "test_drop_multiple_roles"]
struct TestDropMultipleRole {
    #[primary]
    id: i32,
    role_name: String,
}

#[derive(Debug, Model, Clone)]
#[table = "test_drop_recreate_users"]
struct TestDropRecreateUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

async fn test_drop_table_basic_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropBasicUser>().await;

    // 创建表
    db.create_table::<TestDropBasicUser>().await?;

    // 删除表
    db.drop_table::<TestDropBasicUser>().await?;

    // 再次删除（MySQL使用IF EXISTS，所以不会报错）
    db.drop_table::<TestDropBasicUser>().await?;

    Ok(())
}

async fn test_drop_table_if_exists_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropIfExistsUser>().await;

    // 创建表
    db.create_table::<TestDropIfExistsUser>().await?;

    // 删除表
    db.drop_table::<TestDropIfExistsUser>().await?;

    // 再次删除（应该不报错，因为使用了 IF EXISTS）
    db.drop_table::<TestDropIfExistsUser>().await?;

    Ok(())
}

async fn test_drop_multiple_tables_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropMultipleUser>().await;
    let _ = db.drop_table::<TestDropMultipleRole>().await;

    // 创建多个表
    db.create_table::<TestDropMultipleUser>().await?;
    db.create_table::<TestDropMultipleRole>().await?;

    // 插入一些数据
    db.insert(&TestDropMultipleUser {
        id: 1,
        name: "Bob".to_string(),
        age: 25,
    })
    .await?;

    db.insert(&TestDropMultipleRole {
        id: 1,
        role_name: "admin".to_string(),
    })
    .await?;

    // 删除所有表
    db.drop_table::<TestDropMultipleUser>().await?;
    db.drop_table::<TestDropMultipleRole>().await?;

    // 验证表已删除 - 重新创建应该成功
    db.create_table::<TestDropMultipleUser>().await?;
    db.create_table::<TestDropMultipleRole>().await?;

    // 清理
    db.drop_table::<TestDropMultipleUser>().await?;
    db.drop_table::<TestDropMultipleRole>().await?;

    Ok(())
}

async fn test_drop_table_and_recreate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 先清理可能存在的旧表
    let _ = db.drop_table::<TestDropRecreateUser>().await;

    // 创建表
    db.create_table::<TestDropRecreateUser>().await?;

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
    db.drop_table::<TestDropRecreateUser>().await?;

    // 重新创建表
    db.create_table::<TestDropRecreateUser>().await?;

    // 验证表是空的
    let users = db
        .select::<TestDropRecreateUser>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 0);

    // 清理
    db.drop_table::<TestDropRecreateUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_drop_table_basic_impl);
test_on_all_dbs_result!(test_drop_table_if_exists_impl);
test_on_all_dbs_result!(test_drop_multiple_tables_impl);
test_on_all_dbs_result!(test_drop_table_and_recreate_impl);
