#[allow(unused_imports)]
use ormer::{FieldType, Model};

#[cfg(feature = "postgresql")]
mod _test_common;

#[derive(Debug, Clone, FieldType, PartialEq)]
enum UserStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, FieldType, PartialEq)]
pub struct ExceptionType(pub u16);

// 注意: 枚举类型需要实现 ColumnValueType 才能用于 filter
// 这需要额外的实现,暂时跳过 filter 测试

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
#[derive(Debug, Model, PartialEq)]
#[table = "test_enum_users_1"]
struct TestEnumUser {
    #[primary(auto)]
    id: i32,
    status: UserStatus,
    name: String,
}

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
#[derive(Debug, Model, PartialEq)]
#[table = "test_enum_users_optional_1"]
struct TestEnumUserOptional {
    #[primary(auto)]
    id: i32,
    status: Option<UserStatus>,
    name: String,
}

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
#[derive(Debug, Model, PartialEq)]
#[table = "test_field_type_exceptions"]
struct TestException {
    #[primary]
    id: i32,
    exception_type: ExceptionType,
    optional_exception_type: Option<ExceptionType>,
}

#[tokio::test]
async fn test_enum_basic() {
    #[cfg(feature = "sqlite")]
    {
        use ormer::Database;
        let db = Database::connect(ormer::DbType::Sqlite, ":memory:")
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
        let _ = db.insert(&user1).execute().await.unwrap();

        let user2 = TestEnumUser {
            id: 2,
            status: UserStatus::Inactive,
            name: "Bob".to_string(),
        };
        let _ = db.insert(&user2).execute().await.unwrap();

        let user3 = TestEnumUser {
            id: 3,
            status: UserStatus::Banned,
            name: "Charlie".to_string(),
        };
        let _ = db.insert(&user3).execute().await.unwrap();

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
    #[cfg(feature = "sqlite")]
    {
        use ormer::Database;
        let db = Database::connect(ormer::DbType::Sqlite, ":memory:")
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
        let _ = db.insert(&user1).execute().await.unwrap();

        let user2 = TestEnumUserOptional {
            id: 2,
            status: None,
            name: "Bob".to_string(),
        };
        let _ = db.insert(&user2).execute().await.unwrap();

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

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
#[test]
fn test_enum_column_schema_metadata() {
    let status_col = TestEnumUser::COLUMN_SCHEMA
        .iter()
        .find(|col| col.name == "status")
        .unwrap();
    assert_eq!(status_col.rust_type, "UserStatus");
    assert_eq!(status_col.enum_variants, Some(UserStatus::VARIANTS));

    let optional_status_col = TestEnumUserOptional::COLUMN_SCHEMA
        .iter()
        .find(|col| col.name == "status")
        .unwrap();
    assert_eq!(optional_status_col.rust_type, "UserStatus");
    assert_eq!(
        optional_status_col.enum_variants,
        Some(UserStatus::VARIANTS)
    );
}

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]
#[test]
fn test_tuple_struct_field_type() -> ormer::Result<()> {
    let exception_col = TestException::COLUMN_SCHEMA
        .iter()
        .find(|col| col.name == "exception_type")
        .unwrap();
    assert_eq!(exception_col.rust_type, "u16");
    assert_eq!(exception_col.enum_variants, None);
    assert_eq!(exception_col.data_type, None);

    let model = TestException {
        id: 1,
        exception_type: ExceptionType(42),
        optional_exception_type: Some(ExceptionType(7)),
    };
    let values = model.field_values();
    assert_eq!(values.len(), 3);
    assert_integer_value(&values[1], 42);
    assert_integer_value(&values[2], 7);

    let from_values = <TestException as Model>::from_row_values(&[
        ormer::Value::Integer(2),
        ormer::Value::Integer(9),
        ormer::Value::Null,
    ])?;
    assert_eq!(from_values.exception_type, ExceptionType(9));
    assert_eq!(from_values.optional_exception_type, None);

    #[cfg(feature = "sqlite")]
    {
        let (sql, params) = ormer::Select::<TestException>::new()
            .filter(|e| e.exception_type.eq(ExceptionType(12)))
            .to_sql_with_params(ormer::DbType::Sqlite);
        assert!(sql.contains("exception_type = ?"), "SQL: {sql}");
        assert_eq!(params.len(), 1);
        assert_integer_value(&params[0], 12);

        let create_sql = ormer::generate_create_table_sql::<TestException>(ormer::DbType::Sqlite)?;
        assert!(
            create_sql.contains("exception_type INTEGER NOT NULL"),
            "SQL: {create_sql}"
        );
    }

    Ok(())
}

fn assert_integer_value(value: &ormer::Value, expected: i64) {
    match value {
        ormer::Value::Integer(value) => assert_eq!(*value, expected),
        other => panic!("expected integer value {expected}, got {other:?}"),
    }
}

#[cfg(feature = "postgresql")]
#[test]
fn test_postgresql_enum_create_sql() {
    let sql = ormer::generate_create_table_sql::<TestEnumUser>(ormer::DbType::PostgreSQL).unwrap();
    assert!(sql.contains("status user_status NOT NULL"));
    assert!(!sql.contains("status TEXT"));

    let optional_sql =
        ormer::generate_create_table_sql::<TestEnumUserOptional>(ormer::DbType::PostgreSQL)
            .unwrap();
    assert!(optional_sql.contains("status user_status"));
    assert!(!optional_sql.contains("status TEXT"));
}

#[cfg(feature = "mysql")]
#[test]
fn test_mysql_enum_create_sql() {
    let sql = ormer::generate_create_table_sql::<TestEnumUser>(ormer::DbType::MySQL).unwrap();
    assert!(sql.contains("status ENUM('Active', 'Inactive', 'Banned') NOT NULL"));

    let optional_sql =
        ormer::generate_create_table_sql::<TestEnumUserOptional>(ormer::DbType::MySQL).unwrap();
    assert!(optional_sql.contains("status ENUM('Active', 'Inactive', 'Banned')"));
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn test_postgresql_enum_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let config = _test_common::postgresql_config();
    let db = _test_common::create_db_connection(&config).await?;
    let _ = db.drop_table::<TestEnumUserOptional>().execute().await;
    let _ = db.drop_table::<TestEnumUser>().execute().await;

    db.create_table::<TestEnumUser>().execute().await?;
    db.create_table::<TestEnumUserOptional>().execute().await?;

    db.insert(&TestEnumUser {
        id: 1,
        status: UserStatus::Active,
        name: "Alice".to_string(),
    })
    .execute()
    .await?;
    db.insert(&TestEnumUserOptional {
        id: 1,
        status: Some(UserStatus::Banned),
        name: "Bob".to_string(),
    })
    .execute()
    .await?;
    db.insert(&TestEnumUserOptional {
        id: 2,
        status: None,
        name: "Carol".to_string(),
    })
    .execute()
    .await?;

    let users = db.select::<TestEnumUser>().collect::<Vec<_>>().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].status, UserStatus::Active);

    let optional_users = db
        .select::<TestEnumUserOptional>()
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(optional_users.len(), 2);
    assert_eq!(optional_users[0].status, Some(UserStatus::Banned));
    assert_eq!(optional_users[1].status, None);

    let returned = db
        .insert(&TestEnumUser {
            id: 2,
            status: UserStatus::Inactive,
            name: "Dave".to_string(),
        })
        .returning()
        .await?;
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].status, UserStatus::Inactive);

    db.drop_table::<TestEnumUserOptional>().execute().await?;
    db.drop_table::<TestEnumUser>().execute().await?;
    Ok(())
}
