#[derive(Debug, ormer::Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 连接数据库并创建表
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<User>().execute().await?;

    // 插入数据
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .execute()
    .await?;

    // 查询数据
    let users = db
        .select::<User>()
        .filter(|p| p.age.ge(18))
        .collect::<Vec<_>>()
        .await?;
    println!("users: {users:?}");

    Ok(())
}
