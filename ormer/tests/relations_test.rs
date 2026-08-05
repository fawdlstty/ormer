#![cfg(feature = "sqlite")]

use ormer::{Database, Model, RelationKind};

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_users"]
struct RelationUser {
    #[primary(auto)]
    id: i32,
    name: String,
    #[has_many(RelationPost.user_id)]
    posts: Vec<RelationPost>,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_posts"]
struct RelationPost {
    #[primary(auto)]
    id: i32,
    #[foreign(RelationUser.id)]
    user_id: i32,
    #[belongs_to(user_id)]
    user: Option<RelationUser>,
    title: String,
}

#[tokio::test]
async fn relations_support_metadata_find_related_preload_and_include() -> anyhow::Result<()> {
    assert_eq!(RelationUser::COLUMNS, &["id", "name"]);
    assert_eq!(RelationPost::COLUMNS, &["id", "user_id", "title"]);
    assert_eq!(RelationUser::RELATIONS[0].kind, RelationKind::HasMany);
    assert_eq!(RelationPost::RELATIONS[0].kind, RelationKind::BelongsTo);

    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<RelationUser>().execute().await?;
    db.create_table::<RelationPost>().execute().await?;

    db.insert(&RelationUser {
        id: 1,
        name: "Alice".to_string(),
        posts: Vec::new(),
    })
    .execute()
    .await?;
    db.insert(&RelationUser {
        id: 2,
        name: "Bob".to_string(),
        posts: Vec::new(),
    })
    .execute()
    .await?;
    db.insert(&RelationPost {
        id: 1,
        user_id: 1,
        user: None,
        title: "One".to_string(),
    })
    .execute()
    .await?;
    db.insert(&RelationPost {
        id: 2,
        user_id: 1,
        user: None,
        title: "Two".to_string(),
    })
    .execute()
    .await?;
    db.insert(&RelationPost {
        id: 3,
        user_id: 2,
        user: None,
        title: "Three".to_string(),
    })
    .execute()
    .await?;

    let user = db.find_by_id::<RelationUser>(1).await?.unwrap();
    let posts = db
        .find_related(&user, RelationUserWhere::default().posts)
        .await?;
    assert_eq!(posts.len(), 2);

    let mut users = db.select::<RelationUser>().collect::<Vec<_>>().await?;
    db.preload(&mut users, RelationUserWhere::default().posts)
        .await?;
    assert_eq!(
        users.iter().find(|user| user.id == 1).unwrap().posts.len(),
        2
    );
    assert_eq!(
        users.iter().find(|user| user.id == 2).unwrap().posts.len(),
        1
    );

    let posts = db
        .select::<RelationPost>()
        .include(|post| post.user)
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(posts[0].user.as_ref().unwrap().id, 1);
    assert_eq!(posts[2].user.as_ref().unwrap().id, 2);

    db.drop_table::<RelationPost>().execute().await?;
    db.drop_table::<RelationUser>().execute().await?;
    Ok(())
}
