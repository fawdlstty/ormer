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

#[derive(Debug, ormer::Model)]
#[table = "roles"]
#[allow(dead_code)]
struct Role {
    #[primary]
    id: i32,
    #[foreign(User.id)]
    #[unique(group = 1)]
    uid: i32,
    #[unique(group = 1)]
    name: String,
}

#[derive(Debug, ormer::Model)]
#[table = "new_users"]
#[allow(dead_code)]
struct NewUser(User);

#[derive(Debug, ormer::Model)]
#[table = "new_roles"]
#[allow(dead_code)]
struct NewRole(Role);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    // db.create_table::<User>().execute().await?;
    // db.create_table::<NewRole>().execute().await?;

    // // insert
    // db.insert(&User {
    //     id: 1,
    //     name: "Alice".to_string(),
    //     age: 18,
    //     email: None,
    // })
    // .execute()
    // .await?;

    Ok(())
}
