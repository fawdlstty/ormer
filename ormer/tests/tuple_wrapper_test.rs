#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

use ormer::Model;
use ormer::generate_create_table_sql;

mod _test_common;

// 基础模型定义 - 使用宏定义（无表名）
#[derive(Debug, ormer::Model, Clone)]
struct BaseTestUser {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}

// 使用元组结构体包装，复用 BaseTestUser 的结构，但使用不同的表名
#[derive(Debug, Model)]
#[table = "tuple_wrapper_test_users_1"]
struct TestUser(BaseTestUser);

#[derive(Debug, Model)]
#[table = "tuple_wrapper_archive_users_1"]
struct ArchiveUser(BaseTestUser);

#[derive(Debug, Model)]
#[table = "tuple_wrapper_temp_users_1"]
struct TempUser(BaseTestUser);

#[cfg(test)]
mod tuple_wrapper_tests {
    use super::*;

    #[tokio::test]
    async fn test_tuple_wrapper_sql_generation() {
        // 测试基础模型
        let user_sql = generate_create_table_sql::<TestUser>(ormer::DbType::Turso);
        println!("TestUser SQL: {}", user_sql);
        assert!(user_sql.contains("CREATE TABLE IF NOT EXISTS tuple_wrapper_test_users_1"));
        assert!(user_sql.contains("id INTEGER PRIMARY KEY AUTOINCREMENT"));
        assert!(user_sql.contains("name TEXT NOT NULL UNIQUE"));
        assert!(user_sql.contains("age INTEGER NOT NULL"));
        assert!(user_sql.contains("email TEXT"));

        // 测试元组包装器模型 - 应该使用新的表名，但字段结构相同
        let archive_sql = generate_create_table_sql::<ArchiveUser>(ormer::DbType::Turso);
        println!("ArchiveUser SQL: {}", archive_sql);
        assert!(archive_sql.contains("CREATE TABLE IF NOT EXISTS tuple_wrapper_archive_users_1"));
        assert!(!archive_sql.contains("tuple_wrapper_test_users_1"));
        // 验证字段结构相同
        assert!(archive_sql.contains("id INTEGER PRIMARY KEY AUTOINCREMENT"));
        assert!(archive_sql.contains("name TEXT NOT NULL UNIQUE"));
        assert!(archive_sql.contains("age INTEGER NOT NULL"));
        assert!(archive_sql.contains("email TEXT"));

        // 测试另一个包装器
        let temp_sql = generate_create_table_sql::<TempUser>(ormer::DbType::Turso);
        println!("TempUser SQL: {}", temp_sql);
        assert!(temp_sql.contains("CREATE TABLE IF NOT EXISTS tuple_wrapper_temp_users_1"));
        assert!(!temp_sql.contains("tuple_wrapper_test_users_1"));
        assert!(temp_sql.contains("id INTEGER PRIMARY KEY AUTOINCREMENT"));
    }

    async fn test_tuple_wrapper_crud_impl(
        config: &_test_common::DbConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = _test_common::create_db_connection(config).await?;

        // 创建基础表
        db.create_table::<TestUser>().execute().await?;
        println!("✓ Created test_users table");

        // 使用包装器创建归档表
        db.create_table::<ArchiveUser>().execute().await?;
        println!("✓ Created archive_users table");

        // 插入数据到基础表
        db.insert(&TestUser(BaseTestUser {
            id: 0,
            name: "Alice".to_string(),
            age: 25,
            email: Some("alice@example.com".to_string()),
        }))
        .await?;
        println!("✓ Inserted into test_users");

        // 查询基础表
        let users: Vec<TestUser> = db.select::<TestUser>().collect::<Vec<_>>().await?;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].0.name, "Alice");
        println!("✓ Queried test_users: {:?}", users);

        // 使用包装器类型插入数据到归档表
        let archive_user = ArchiveUser(BaseTestUser {
            id: 0,
            name: "Bob".to_string(),
            age: 30,
            email: Some("bob@example.com".to_string()),
        });
        db.insert(&archive_user).await?;
        println!("✓ Inserted into archive_users");

        // 使用包装器类型查询归档表
        let archived: Vec<ArchiveUser> = db.select::<ArchiveUser>().collect::<Vec<_>>().await?;
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].0.name, "Bob");
        println!("✓ Queried archive_users: {:?}", archived);

        // 测试内部类型的访问
        let inner_user: BaseTestUser = archived[0].0.clone();
        assert_eq!(inner_user.name, "Bob");
        assert_eq!(inner_user.age, 30);
        println!("✓ Accessed inner TestUser from ArchiveUser");

        // 测试过滤查询
        let filtered: Vec<ArchiveUser> = db
            .select::<ArchiveUser>()
            .filter(|p| p.age.ge(25))
            .collect::<Vec<_>>()
            .await?;
        assert_eq!(filtered.len(), 1);
        println!("✓ Filtered query on archive_users");

        // 测试更新操作
        let count = db
            .update::<ArchiveUser>()
            .filter(|p| p.name.eq("Bob"))
            .set(|p| p.age, 35)
            .execute()
            .await?;
        assert_eq!(count, 1);
        println!("✓ Updated archive_users");

        // 验证更新结果
        let updated: Vec<ArchiveUser> = db
            .select::<ArchiveUser>()
            .filter(|p| p.name.eq("Bob"))
            .collect::<Vec<_>>()
            .await?;
        assert_eq!(updated[0].0.age, 35);
        println!("✓ Verified update result");

        // 清理
        db.drop_table::<TestUser>().execute().await?;
        db.drop_table::<ArchiveUser>().execute().await?;
        println!("✓ Cleaned up tables");

        Ok(())
    }

    #[tokio::test]
    async fn test_tuple_wrapper_crud_turso() {
        let config: _test_common::DbConfig = (ormer::DbType::Turso, ":memory:");
        match test_tuple_wrapper_crud_impl(&config).await {
            Ok(_) => println!("✓ All tuple wrapper CRUD tests passed"),
            Err(e) => panic!("✗ Test failed: {}", e),
        }
    }
}
