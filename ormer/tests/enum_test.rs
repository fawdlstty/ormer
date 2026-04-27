use ormer::{Model, ModelEnum};

#[derive(Debug, Clone, ModelEnum, PartialEq)]
enum UserStatus {
    Active,
    Inactive,
    Banned,
}

// 注意: 枚举类型需要实现 ColumnValueType 才能用于 filter
// 这需要额外的实现,暂时跳过 filter 测试

#[derive(Debug, Model, PartialEq)]
#[table = "test_enum_users_1"]
struct TestEnumUser {
    #[primary(auto)]
    id: i32,
    status: UserStatus,
    name: String,
}

#[derive(Debug, Model, PartialEq)]
#[table = "test_enum_users_optional_1"]
struct TestEnumUserOptional {
    #[primary(auto)]
    id: i32,
    status: Option<UserStatus>,
    name: String,
}

#[tokio::test]
async fn test_enum_basic() {
    #[cfg(feature = "turso")]
    {
        use ormer::Database;
        let db = Database::connect(ormer::DbType::Turso, ":memory:")
            .await
            .unwrap();
        let _ = db.drop_table::<TestEnumUser>().execute().await;
        db.create_table::<TestEnumUser>().execute().await.unwrap();

        // 插入测试数据 - 验证枚举可以正确插入和读取
        let user1 = TestEnumUser {
            id: 1,
            status: UserStatus::Active,
            name: "Alice".to_string(),
        };
        db.insert(&user1).await.unwrap();

        let user2 = TestEnumUser {
            id: 2,
            status: UserStatus::Inactive,
            name: "Bob".to_string(),
        };
        db.insert(&user2).await.unwrap();

        let user3 = TestEnumUser {
            id: 3,
            status: UserStatus::Banned,
            name: "Charlie".to_string(),
        };
        db.insert(&user3).await.unwrap();

        // 查询所有用户 - 验证枚举可以正确读取
        let users = db
            .select::<TestEnumUser>()
            .collect::<Vec<_>>()
            .await
            .unwrap();

        assert_eq!(users.len(), 3);
        assert_eq!(users[0].status, UserStatus::Active);
        assert_eq!(users[0].name, "Alice");
        assert_eq!(users[1].status, UserStatus::Inactive);
        assert_eq!(users[1].name, "Bob");
        assert_eq!(users[2].status, UserStatus::Banned);
        assert_eq!(users[2].name, "Charlie");

        println!("✓ Enum basic test passed!");
    }
}

#[tokio::test]
async fn test_enum_optional() {
    #[cfg(feature = "turso")]
    {
        use ormer::Database;
        let db = Database::connect(ormer::DbType::Turso, ":memory:")
            .await
            .unwrap();
        let _ = db.drop_table::<TestEnumUserOptional>().execute().await;
        db.create_table::<TestEnumUserOptional>()
            .execute()
            .await
            .unwrap();

        // 插入测试数据 - 验证 Option<Enum> 可以正确插入和读取
        let user1 = TestEnumUserOptional {
            id: 1,
            status: Some(UserStatus::Active),
            name: "Alice".to_string(),
        };
        db.insert(&user1).await.unwrap();

        let user2 = TestEnumUserOptional {
            id: 2,
            status: None,
            name: "Bob".to_string(),
        };
        db.insert(&user2).await.unwrap();

        // 查询所有用户 - 验证 Option<Enum> 可以正确读取
        let users = db
            .select::<TestEnumUserOptional>()
            .collect::<Vec<_>>()
            .await
            .unwrap();

        assert_eq!(users.len(), 2);
        assert_eq!(users[0].status, Some(UserStatus::Active));
        assert_eq!(users[0].name, "Alice");
        assert_eq!(users[1].status, None);
        assert_eq!(users[1].name, "Bob");

        println!("✓ Enum optional test passed!");
    }
}

#[test]
fn test_enum_variants() {
    // 测试枚举变体常量
    assert_eq!(UserStatus::VARIANTS, &["Active", "Inactive", "Banned"]);

    // 测试 name() 方法
    assert_eq!(UserStatus::Active.name(), "Active");
    assert_eq!(UserStatus::Inactive.name(), "Inactive");
    assert_eq!(UserStatus::Banned.name(), "Banned");

    println!("✓ Enum variants test passed!");
}
