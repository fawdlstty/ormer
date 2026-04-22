// 定义 User 模型
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
struct Role {
    #[primary]
    id: i32,
    #[unique(group = 1)]
    uid: i32,
    #[unique(group = 1)]
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // connect
    let db = ormer::Database::connect(ormer::DbType::Turso, "data.db").await?;
    db.create_table::<User>().await?;
    db.create_table::<Role>().await?;

    // insert
    db.insert(&User {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .await?;
    db.insert(&vec![User {
        id: 2,
        name: "Bob".to_string(),
        age: 20,
        email: Some("bob@example.com".to_string()),
    }])
    .await?;
    db.insert(&vec![User {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        email: Some("charlie@example.com".to_string()),
    }])
    .await?;
    db.insert(&[User {
        id: 4,
        name: "David".to_string(),
        age: 24,
        email: Some("david@example.com".to_string()),
    }])
    .await?;
    db.insert(&[User {
        id: 5,
        name: "Eve".to_string(),
        age: 26,
        email: Some("eve@example.com".to_string()),
    }])
    .await?;
    db.insert(
        &[User {
            id: 6,
            name: "Frank".to_string(),
            age: 28,
            email: Some("frank@example.com".to_string()),
        }][..],
    )
    .await?;
    db.insert(
        &[User {
            id: 7,
            name: "Grace".to_string(),
            age: 30,
            email: Some("grace@example.com".to_string()),
        }][..],
    )
    .await?;
    db.insert_or_update(&Role {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;
    println!("inserted data");

    // query
    let users = db
        .select::<User>()
        //.filter(|p| p.age.ge(18))
        .filter(|p| p.age.is_in(&vec![2, 4, 6, 7, 8]))
        .limit(10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data: {users:?}");

    // aggregate
    let sum: Option<i32> = db.select::<User>().sum(|p| p.age).await?;
    println!("sum: {sum:?}");
    let min: Option<i32> = db.select::<User>().min(|p| p.age).await?;
    println!("min: {min:?}");
    let max: Option<i32> = db.select::<User>().max(|p| p.age).await?;
    println!("max: {max:?}");
    let avg: Option<f64> = db.select::<User>().avg(|p| p.age).await?;
    println!("avg: {avg:?}");
    let count: usize = db.select::<User>().count(|p| p.age).await?;
    println!("count: {count:?}");

    // related query
    let users = db
        .select::<User>()
        .from::<User, Role>()
        .filter(|p, q| p.id.eq(q.uid))
        .filter(|_, q| q.name.eq("admin".to_string()))
        .limit(10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data: {users:?}");

    // join query
    let user_roles: Vec<(User, Option<Role>)> = db
        .select::<User>()
        .left_join::<Role>(|p, q| p.id.eq(q.uid))
        .limit(10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data: {user_roles:?}");

    // update
    let count = db
        .update::<User>()
        .filter(|p| p.age.ge(18))
        .set(|p| p.age, 10)
        .execute()
        .await?;
    println!("updated rows: {count}");

    // delete
    let count = db
        .delete::<User>()
        .filter(|p| p.age.ge(18))
        .execute()
        .await?;
    println!("deleted rows: {count}");

    let t = db.begin().await?;
    t.delete::<User>()
        .filter(|p| p.age.ge(18))
        .execute()
        .await?;
    t.commit().await?;

    // drop table
    db.drop_table::<User>().await?;
    db.drop_table::<Role>().await?;
    Ok(())
}
