#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user!(TestUser, "test_main_users_1");
define_test_role_with_unique_group!(TestRole, "test_main_roles_1");

async fn test_main_rs_usage_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // connect
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestUser>().execute().await?;
    db.create_table::<TestRole>().execute().await?;

    // insert
    db.insert(&TestUser {
        id: 1,
        name: "Alice".to_string(),
        age: 18,
        email: None,
    })
    .await?;
    db.insert(&vec![TestUser {
        id: 2,
        name: "Bob".to_string(),
        age: 20,
        email: Some("bob@example.com".to_string()),
    }])
    .await?;
    db.insert(&vec![TestUser {
        id: 3,
        name: "Charlie".to_string(),
        age: 22,
        email: Some("charlie@example.com".to_string()),
    }])
    .await?;
    db.insert(&[TestUser {
        id: 4,
        name: "David".to_string(),
        age: 24,
        email: Some("david@example.com".to_string()),
    }])
    .await?;
    db.insert(&[TestUser {
        id: 5,
        name: "Eve".to_string(),
        age: 26,
        email: Some("eve@example.com".to_string()),
    }])
    .await?;
    db.insert(
        &[TestUser {
            id: 6,
            name: "Frank".to_string(),
            age: 28,
            email: Some("frank@example.com".to_string()),
        }][..],
    )
    .await?;
    db.insert(
        &[TestUser {
            id: 7,
            name: "Grace".to_string(),
            age: 30,
            email: Some("grace@example.com".to_string()),
        }][..],
    )
    .await?;
    db.insert_or_update(&TestRole {
        id: 1,
        uid: 1,
        name: "admin".to_string(),
    })
    .await?;
    println!("inserted data");

    // query with order_by
    let users = db
        .select::<TestUser>()
        .filter(|p| p.age.is_in(&vec![2, 4, 6, 7, 8]))
        .order_by(|p| p.age)
        .range(0..10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data with order_by: {users:?}");

    // query with order_by_desc
    let users = db
        .select::<TestUser>()
        .filter(|p| p.age.is_in(&vec![2, 4, 6, 7, 8]))
        .order_by_desc(|p| p.age)
        .range(0..10)
        .collect::<Vec<_>>()
        .await?;
    println!("queryed data with order_by_desc: {users:?}");

    // aggregate
    let sum: Option<i32> = db.select::<TestUser>().sum(|p| p.age).await?;
    println!("sum: {sum:?}");
    let min: Option<i32> = db.select::<TestUser>().min(|p| p.age).await?;
    println!("min: {min:?}");
    let max: Option<i32> = db.select::<TestUser>().max(|p| p.age).await?;
    println!("max: {max:?}");
    let avg: Option<f64> = db.select::<TestUser>().avg(|p| p.age).await?;
    println!("avg: {avg:?}");
    let count: usize = db.select::<TestUser>().count(|p| p.age).await?;
    println!("count: {count:?}");

    // related query
    let users = db
        .select::<TestUser>()
        .from::<TestUser, TestRole>()
        .filter(|p, q| p.id.eq(q.uid))
        .filter(|_, q| q.name.eq("admin".to_string()))
        .range(..10)
        .collect::<Vec<_>>()
        .await?;
    println!("related query data: {users:?}");

    // join query
    let user_roles: Vec<(TestUser, Option<TestRole>)> = db
        .select::<TestUser>()
        .left_join::<TestRole>(|p, q| p.id.eq(q.uid))
        .range(10..20)
        .collect::<Vec<_>>()
        .await?;
    println!("join query data: {user_roles:?}");

    // update
    let count = db
        .update::<TestUser>()
        .filter(|p| p.age.ge(18))
        .set(|p| p.age, 10)
        .execute()
        .await?;
    println!("updated rows: {count}");

    // delete
    let count = db
        .delete::<TestUser>()
        .filter(|p| p.age.ge(18))
        .execute()
        .await?;
    println!("deleted rows: {count}");

    let t = db.begin().await?;
    t.delete::<TestUser>()
        .filter(|p| p.age.ge(18))
        .execute()
        .await?;
    t.commit().await?;

    // drop table
    db.drop_table::<TestUser>().execute().await?;
    db.drop_table::<TestRole>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_main_rs_usage_impl);
