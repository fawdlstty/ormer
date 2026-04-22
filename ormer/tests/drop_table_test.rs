/// drop_table 功能测试
use ormer::{Database, DbType, Model};

#[derive(Debug, Model)]
#[table = "test_drop_users"]
struct TestDropUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model)]
#[table = "test_drop_roles"]
struct TestDropRole {
    #[primary]
    id: i32,
    role_name: String,
}

#[tokio::test]
async fn test_drop_table_basic() {
    let db = Database::connect(DbType::Turso, "test_drop_basic.db")
        .await
        .unwrap();

    // 创建表
    db.create_table::<TestDropUser>().await.unwrap();

    // 验证表存在 - 尝试插入数据
    db.insert(&TestDropUser {
        id: 1,
        name: "Alice".to_string(),
        age: 20,
    })
    .await
    .unwrap();

    // 删除表
    db.drop_table::<TestDropUser>().await.unwrap();

    // 验证表已删除 - 重新创建表应该成功（如果表还存在会验证schema）
    db.create_table::<TestDropUser>().await.unwrap();

    // 清理
    db.drop_table::<TestDropUser>().await.unwrap();
    std::fs::remove_file("test_drop_basic.db").ok();
}

#[tokio::test]
async fn test_drop_table_if_exists() {
    let db = Database::connect(DbType::Turso, "test_drop_exists.db")
        .await
        .unwrap();

    // 删除一个不存在的表（应该不报错）
    db.drop_table::<TestDropRole>().await.unwrap();

    // 创建表
    db.create_table::<TestDropRole>().await.unwrap();

    // 删除表
    db.drop_table::<TestDropRole>().await.unwrap();

    // 再次删除（应该不报错，因为使用了 IF EXISTS）
    db.drop_table::<TestDropRole>().await.unwrap();

    // 清理
    std::fs::remove_file("test_drop_exists.db").ok();
}

#[tokio::test]
async fn test_drop_multiple_tables() {
    let db = Database::connect(DbType::Turso, "test_drop_multi.db")
        .await
        .unwrap();

    // 创建多个表
    db.create_table::<TestDropUser>().await.unwrap();
    db.create_table::<TestDropRole>().await.unwrap();

    // 插入一些数据
    db.insert(&TestDropUser {
        id: 1,
        name: "Bob".to_string(),
        age: 25,
    })
    .await
    .unwrap();

    db.insert(&TestDropRole {
        id: 1,
        role_name: "admin".to_string(),
    })
    .await
    .unwrap();

    // 删除所有表
    db.drop_table::<TestDropUser>().await.unwrap();
    db.drop_table::<TestDropRole>().await.unwrap();

    // 验证表已删除 - 重新创建应该成功
    db.create_table::<TestDropUser>().await.unwrap();
    db.create_table::<TestDropRole>().await.unwrap();

    // 清理
    db.drop_table::<TestDropUser>().await.unwrap();
    db.drop_table::<TestDropRole>().await.unwrap();
    std::fs::remove_file("test_drop_multi.db").ok();
}

#[tokio::test]
async fn test_drop_table_and_recreate() {
    let db = Database::connect(DbType::Turso, "test_drop_recreate.db")
        .await
        .unwrap();

    // 创建表
    db.create_table::<TestDropUser>().await.unwrap();

    // 插入数据
    db.insert(&TestDropUser {
        id: 1,
        name: "Charlie".to_string(),
        age: 30,
    })
    .await
    .unwrap();

    db.insert(&TestDropUser {
        id: 2,
        name: "Diana".to_string(),
        age: 28,
    })
    .await
    .unwrap();

    // 查询验证数据存在
    let users = db
        .select::<TestDropUser>()
        .collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(users.len(), 2);

    // 删除表
    db.drop_table::<TestDropUser>().await.unwrap();

    // 重新创建表
    db.create_table::<TestDropUser>().await.unwrap();

    // 验证表是空的
    let users = db
        .select::<TestDropUser>()
        .collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(users.len(), 0);

    // 清理
    db.drop_table::<TestDropUser>().await.unwrap();
    std::fs::remove_file("test_drop_recreate.db").ok();
}
