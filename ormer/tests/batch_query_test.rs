#![cfg(feature = "sqlite")]

use ormer::{Database, DbType};

#[derive(Debug, Clone, ormer::Model)]
#[table = "batch_query_users"]
struct BatchUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    #[has_many(BatchOrder.user_id)]
    orders: Vec<BatchOrder>,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "batch_query_orders"]
struct BatchOrder {
    #[primary]
    id: i32,
    #[foreign(BatchUser.id)]
    user_id: i32,
    total: i32,
}

async fn setup_db() -> ormer::Result<Database> {
    let db = Database::connect(DbType::Sqlite, ":memory:").await?;
    db.create_table::<BatchUser>().execute().await?;
    db.create_table::<BatchOrder>().execute().await?;

    db.insert(&[
        BatchUser {
            id: 1,
            name: "Alice".to_string(),
            age: 20,
            orders: Vec::new(),
        },
        BatchUser {
            id: 2,
            name: "Bob".to_string(),
            age: 25,
            orders: Vec::new(),
        },
    ])
    .execute()
    .await?;

    db.insert(&[
        BatchOrder {
            id: 1,
            user_id: 1,
            total: 100,
        },
        BatchOrder {
            id: 2,
            user_id: 1,
            total: 120,
        },
        BatchOrder {
            id: 3,
            user_id: 2,
            total: 80,
        },
    ])
    .execute()
    .await?;

    Ok(db)
}

#[tokio::test]
async fn sqlite_batch_returns_tuple_results() -> ormer::Result<()> {
    let db = setup_db().await?;

    let (users, orders, order_count): (Vec<BatchUser>, Vec<BatchOrder>, usize) = db
        .batch((
            db.select::<BatchUser>()
                .order_by(|u| u.id)
                .include(|u| u.orders),
            db.select::<BatchOrder>()
                .filter(|o| o.user_id.eq(1))
                .order_by(|o| o.id),
            db.select::<BatchOrder>().count(|o| o.id),
        ))
        .await?;

    assert_eq!(users.len(), 2);
    assert_eq!(users[0].name, "Alice");
    assert_eq!(users[0].orders.len(), 2);
    assert_eq!(users[1].orders.len(), 1);
    assert_eq!(
        orders.iter().map(|order| order.total).collect::<Vec<_>>(),
        vec![100, 120]
    );
    assert_eq!(order_count, 3);

    Ok(())
}

#[tokio::test]
async fn sqlite_batch_many_returns_vec_results() -> ormer::Result<()> {
    let db = setup_db().await?;
    let order_queries = vec![
        db.select::<BatchOrder>()
            .filter(|o| o.user_id.eq(1))
            .order_by(|o| o.id),
        db.select::<BatchOrder>()
            .filter(|o| o.user_id.eq(2))
            .order_by(|o| o.id),
    ];

    let orders: Vec<Vec<BatchOrder>> = db.batch_many(order_queries).await?;

    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].len(), 2);
    assert_eq!(orders[1].len(), 1);
    assert_eq!(orders[1][0].total, 80);

    Ok(())
}

#[tokio::test]
async fn sqlite_transaction_batch_uses_transaction_executor() -> ormer::Result<()> {
    let db = setup_db().await?;

    let (users, orders): (Vec<BatchUser>, Vec<BatchOrder>) = db
        .transaction(|tx| {
            Box::pin(async move {
                tx.batch((
                    tx.select::<BatchUser>().order_by(|u| u.id),
                    tx.select::<BatchOrder>()
                        .filter(|o| o.user_id.eq(1))
                        .order_by(|o| o.id),
                ))
                .await
            })
        })
        .await?;

    assert_eq!(
        users.iter().map(|user| user.age).collect::<Vec<_>>(),
        vec![20, 25]
    );
    assert_eq!(
        orders.iter().map(|order| order.id).collect::<Vec<_>>(),
        vec![1, 2]
    );

    Ok(())
}
