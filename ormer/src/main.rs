#[cfg(feature = "sqlite")]
#[derive(Debug, ormer::Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}

#[cfg(feature = "sqlite")]
#[derive(Debug, ormer::Model)]
#[table = "roles"]
struct Role {
    #[primary]
    id: i32,
    #[foreign(User.id)]
    #[unique(group = 1)]
    uid: i32,
    #[unique(group = 1)]
    name: String,
}

#[cfg(feature = "sqlite")]
#[derive(Debug, ormer::Model)]
#[table = "new_roles"]
struct NewRole(Role);

#[cfg(feature = "sqlite")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<User>().execute().await?;
    db.create_table::<NewRole>().execute().await?;

    // insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .execute()
    .await?;

    println!("Demo completed successfully!");
    Ok(())
}

#[cfg(not(feature = "sqlite"))]
fn main() {
    println!("Please enable the sqlite feature to run the demo.");
}
