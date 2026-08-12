#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_derived_users_1"]
struct DerivedUser {
    #[primary]
    id: i32,
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "test_derived_orders_1"]
struct DerivedOrder {
    #[primary]
    id: i32,
    user_id: i32,
    amount: i32,
}

#[derive(Debug, Clone, ormer::ViewModel)]
struct UserTotal {
    user_id: i32,
    total: i64,
}

#[derive(Debug, Clone, ormer::ViewModel)]
struct UserName {
    id: i32,
    name: String,
}

async fn test_derived_table_from_and_join_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    let _ = db.drop_table::<DerivedOrder>().execute().await;
    let _ = db.drop_table::<DerivedUser>().execute().await;
    db.create_table::<DerivedUser>().execute().await?;
    db.create_table::<DerivedOrder>().execute().await?;

    db.insert(vec![
        DerivedUser {
            id: 1,
            name: "Alice".to_string(),
        },
        DerivedUser {
            id: 2,
            name: "Bob".to_string(),
        },
    ])
    .execute()
    .await?;

    db.insert(vec![
        DerivedOrder {
            id: 1,
            user_id: 1,
            amount: 40,
        },
        DerivedOrder {
            id: 2,
            user_id: 1,
            amount: 70,
        },
        DerivedOrder {
            id: 3,
            user_id: 2,
            amount: 10,
        },
    ])
    .execute()
    .await?;

    let totals = db
        .select::<DerivedOrder>()
        .select_column(|o| (o.user_id, o.amount.sum()))
        .group_by(|o| o.user_id)
        .as_model::<UserTotal>();

    let hot_users: Vec<UserTotal> = db
        .from_derived(totals.clone())
        .filter(|t| t.total.gt(50_i64))
        .order_by_desc(|t| t.total)
        .collect()
        .await?;

    assert_eq!(hot_users.len(), 1);
    assert_eq!(hot_users[0].user_id, 1);
    assert_eq!(hot_users[0].total, 110);

    let rows: Vec<(DerivedUser, Option<UserTotal>)> = db
        .select::<DerivedUser>()
        .left_join_derived(totals, |u, t| u.id.eq(t.user_id))
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(rows.len(), 2);
    let alice = rows
        .iter()
        .find(|(user, _)| user.name == "Alice")
        .expect("Alice row should exist");
    assert_eq!(alice.1.as_ref().map(|total| total.total), Some(110));

    let bob = rows
        .iter()
        .find(|(user, _)| user.name == "Bob")
        .expect("Bob row should exist");
    assert_eq!(bob.1.as_ref().map(|total| total.total), Some(10));

    let filtered_totals = db
        .select::<DerivedOrder>()
        .filter(|o| o.amount.gt(30))
        .select_column(|o| (o.user_id, o.amount.sum()))
        .group_by(|o| o.user_id)
        .as_model::<UserTotal>();

    let filtered_only: Vec<UserTotal> = db
        .from_derived(filtered_totals.clone())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(filtered_only.len(), 1);
    assert_eq!(filtered_only[0].user_id, 1);
    assert_eq!(filtered_only[0].total, 110);

    db.drop_table::<DerivedOrder>().execute().await?;
    db.drop_table::<DerivedUser>().execute().await?;
    Ok(())
}

test_on_all_dbs_result!(test_derived_table_from_and_join_impl);

#[test]
fn test_mapped_and_select_as_model_sql() {
    let mapped = ormer::Select::<DerivedUser>::new()
        .map_to(|u| (u.id, u.name))
        .as_model::<UserName>();
    let mapped_sql = mapped.to_sql();
    assert!(mapped_sql.starts_with("SELECT id AS id, name AS name FROM test_derived_users_1"));

    let select = ormer::Select::<DerivedUser>::new().as_model::<UserName>();
    let select_sql = select.to_sql();
    assert!(select_sql.starts_with("SELECT t0.id AS id, t0.name AS name FROM (SELECT id, name"));

    let filtered_totals = ormer::Select::<DerivedOrder>::new()
        .filter(|o| o.amount.gt(30))
        .select_column(|o| (o.user_id, o.amount.sum()))
        .group_by(|o| o.user_id)
        .as_model::<UserTotal>();
    let (join_sql, join_params) = ormer::Select::<DerivedUser>::new()
        .left_join_derived(filtered_totals, |u, t| u.id.eq(t.user_id))
        .to_sql_with_params(ormer::DbType::Sqlite);
    assert!(join_sql.contains(
        "LEFT JOIN (SELECT user_id AS user_id, SUM(amount) AS total FROM test_derived_orders_1 WHERE amount > ? GROUP BY user_id) AS t1 ON t0.id = t1.user_id"
    ));
    assert!(matches!(
        join_params.as_slice(),
        [ormer::Value::Integer(30)]
    ));
}
