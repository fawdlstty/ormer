// 定义 User 模型
#[derive(Debug, ormer::Model)]
#[table = "users"]
struct User {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // connect
    let db = ormer::Database::connect(ormer::DbType::Turso, "data.db").await?;
    db.create_table::<User>().await?;

    // insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .await?;
    println!("inserted data");

    // query
    let users = db
        .select::<User>()
        .filter(|p| p.age.ge(18))
        .limit(10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data: {users:?}");

    // // update
    // let users = db
    //     .update::<User>()
    //     .filter(|p| p.age.ge(18))
    //     .set(|p| p.age, 10)
    //     .await?;
    // println!("queryed data: {users:?}");

    // // delete
    // db.delete::<User>().filter(|p| p.age.ge(18)).await?;
    // println!("deleted data");

    Ok(())
}
