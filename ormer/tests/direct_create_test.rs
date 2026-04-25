use ormer::Model;

mod _test_common;

#[derive(Debug, Model, Clone)]
#[table = "test_direct_users"]
struct TestDirectUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

async fn test_direct_create_table_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    println!("Creating table...");
    db.create_table::<TestDirectUser>().await?;
    println!("Table created successfully!");

    // 打印 INSERT SQL
    let user = TestDirectUser {
        id: 0, // 会被 AUTOINCREMENT 覆盖
        name: "Test".to_string(),
        age: 25,
    };
    let _values = user.field_values();
    let columns = <TestDirectUser as ormer::Model>::COLUMNS.join(", ");
    let placeholders: Vec<String> = (1..=<TestDirectUser as ormer::Model>::COLUMNS.len())
        .map(|_| "?".to_string())
        .collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        <TestDirectUser as ormer::Model>::TABLE_NAME,
        columns,
        placeholders.join(", ")
    );
    println!("INSERT SQL: '{}'", sql);
    println!("SQL length: {}", sql.len());

    // 插入一条数据测试
    db.insert(&user).await?;
    println!("Data inserted successfully!");

    // 查询
    let users: Vec<TestDirectUser> = db.select::<TestDirectUser>().collect().await?;
    println!("Users: {:?}", users);

    // 清理测试表
    db.drop_table::<TestDirectUser>().await?;

    Ok(())
}

test_on_all_dbs_result!(test_direct_create_table_impl);
