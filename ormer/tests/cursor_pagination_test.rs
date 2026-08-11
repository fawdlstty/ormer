#![cfg(feature = "sqlite")]

mod _test_common;

use ormer::PageCursor;

define_test_user_with_score!(CursorUser, "cursor_pagination_users_1");

#[tokio::test]
async fn sqlite_cursor_pagination_is_stable_across_inserts()
-> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(&_test_common::sqlite_config()).await?;

    let _ = db.drop_table::<CursorUser>().execute().await;
    db.create_table::<CursorUser>().execute().await?;

    for user in [
        CursorUser {
            id: 0,
            name: "Alice".to_string(),
            age: 20,
            score: 100,
        },
        CursorUser {
            id: 0,
            name: "Bob".to_string(),
            age: 21,
            score: 90,
        },
        CursorUser {
            id: 0,
            name: "Carol".to_string(),
            age: 22,
            score: 90,
        },
        CursorUser {
            id: 0,
            name: "Dave".to_string(),
            age: 23,
            score: 80,
        },
        CursorUser {
            id: 0,
            name: "Eve".to_string(),
            age: 24,
            score: 70,
        },
    ] {
        db.insert(&user).execute().await?;
    }

    let page1 = db
        .select::<CursorUser>()
        .order_by_desc(|u| u.score)
        .order_by_desc(|u| u.id)
        .cursor_by(|u| (u.score, u.id))
        .limit(2)
        .fetch_page()
        .await?;

    assert_eq!(
        page1
            .items
            .iter()
            .map(|u| u.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alice", "Carol"]
    );

    let cursor = page1
        .next_cursor()
        .expect("first page should yield a cursor");

    db.insert(&CursorUser {
        id: 0,
        name: "Frank".to_string(),
        age: 25,
        score: 95,
    })
    .execute()
    .await?;

    let page2 = db
        .select::<CursorUser>()
        .order_by_desc(|u| u.score)
        .order_by_desc(|u| u.id)
        .cursor_by(|u| (u.score, u.id))
        .after(PageCursor::from(&cursor))
        .limit(2)
        .fetch_page()
        .await?;

    assert_eq!(
        page2
            .items
            .iter()
            .map(|u| u.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Bob", "Dave"]
    );
    assert!(page2.next_cursor().is_some());

    let _ = db.drop_table::<CursorUser>().execute().await;

    Ok(())
}
