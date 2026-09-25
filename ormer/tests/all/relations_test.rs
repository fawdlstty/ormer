#![cfg(feature = "sqlite")]

use ormer::{Database, Model, RelationKind};

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_users"]
struct RelationUser {
    #[primary(auto)]
    id: i32,
    name: String,
    #[has_one(RelationProfile.user_id)]
    profile: Option<RelationProfile>,
    #[has_many(RelationPost.user_id)]
    posts: Vec<RelationPost>,
    #[has_many(RelationUserRole.user_id)]
    user_roles: Vec<RelationUserRole>,
    #[through(user_roles.role)]
    roles: Vec<RelationRole>,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_profiles"]
struct RelationProfile {
    #[primary(auto)]
    id: i32,
    #[foreign(RelationUser.id)]
    user_id: i32,
    bio: String,
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

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_roles"]
struct RelationRole {
    #[primary(auto)]
    id: i32,
    name: String,
    enabled: bool,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "relation_user_roles"]
struct RelationUserRole {
    #[primary]
    #[foreign(RelationUser.id)]
    user_id: i32,
    #[primary]
    #[foreign(RelationRole.id)]
    role_id: i32,
    #[belongs_to(role_id)]
    role: Option<RelationRole>,
}

#[tokio::test]
async fn relations_support_metadata_find_related_preload_and_include() -> ormer::Result<()> {
    assert_eq!(RelationUser::COLUMNS, &["id", "name"]);
    assert_eq!(RelationPost::COLUMNS, &["id", "user_id", "title"]);
    assert!(
        RelationUser::RELATIONS
            .iter()
            .any(|relation| relation.kind == RelationKind::HasMany)
    );
    assert!(
        RelationUser::RELATIONS
            .iter()
            .any(|relation| relation.kind == RelationKind::HasOne)
    );
    assert!(
        RelationUser::RELATIONS
            .iter()
            .any(|relation| relation.kind == RelationKind::Through)
    );
    assert_eq!(RelationPost::RELATIONS[0].kind, RelationKind::BelongsTo);

    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<RelationUser>().execute().await?;
    db.create_table::<RelationProfile>().execute().await?;
    db.create_table::<RelationPost>().execute().await?;
    db.create_table::<RelationRole>().execute().await?;
    db.create_table::<RelationUserRole>().execute().await?;

    db.insert(&RelationUser {
        id: 1,
        name: "Alice".to_string(),
        profile: None,
        posts: Vec::new(),
        user_roles: Vec::new(),
        roles: Vec::new(),
    })
    .execute()
    .await?;
    db.insert(&RelationUser {
        id: 2,
        name: "Bob".to_string(),
        profile: None,
        posts: Vec::new(),
        user_roles: Vec::new(),
        roles: Vec::new(),
    })
    .execute()
    .await?;
    db.insert(&RelationProfile {
        id: 1,
        user_id: 1,
        bio: "Alice bio".to_string(),
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
    db.insert(&RelationRole {
        id: 1,
        name: "admin".to_string(),
        enabled: true,
    })
    .execute()
    .await?;
    db.insert(&RelationRole {
        id: 2,
        name: "viewer".to_string(),
        enabled: true,
    })
    .execute()
    .await?;
    db.insert(&RelationRole {
        id: 3,
        name: "disabled".to_string(),
        enabled: false,
    })
    .execute()
    .await?;
    db.insert(&RelationUserRole {
        user_id: 1,
        role_id: 1,
        role: None,
    })
    .execute()
    .await?;
    db.insert(&RelationUserRole {
        user_id: 1,
        role_id: 2,
        role: None,
    })
    .execute()
    .await?;
    db.insert(&RelationUserRole {
        user_id: 2,
        role_id: 3,
        role: None,
    })
    .execute()
    .await?;

    let user = db.find_by_id::<RelationUser>(1).await?.unwrap();
    let posts = db
        .find_related(&user, RelationUserWhere::default().posts)
        .await?;
    assert_eq!(posts.len(), 2);
    let roles = db
        .find_related(&user, RelationUserWhere::default().roles)
        .await?;
    assert_eq!(roles.len(), 2);

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

    let users = db
        .select::<RelationUser>()
        .filter(|user| {
            user.roles
                .any(|role| role.name.eq("admin").and(role.enabled.eq(true)))
        })
        .include(|user| user.roles.order_by(|role| role.name.asc()).range(..20))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].roles.len(), 2);

    let users = db
        .select::<RelationUser>()
        .order_by(|user| user.id.asc())
        .include(|user| user.profile)
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users[0].profile.as_ref().unwrap().bio, "Alice bio");
    assert!(users[1].profile.is_none());

    let users = db
        .select::<RelationUser>()
        .filter(|user| user.id.eq(1))
        .include(|user| user.roles.order_by(|role| role.name.asc()).range(..20))
        .include(|user| user.user_roles.include(|user_role| user_role.role))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(users[0].roles.len(), 2);
    assert_eq!(users[0].user_roles.len(), 2);
    assert!(
        users[0]
            .user_roles
            .iter()
            .any(|user_role| user_role.role.as_ref().unwrap().name == "admin")
    );

    let posts = db
        .select::<RelationPost>()
        .include(|post| post.user)
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(posts[0].user.as_ref().unwrap().id, 1);
    assert_eq!(posts[2].user.as_ref().unwrap().id, 2);

    db.drop_table::<RelationUserRole>().execute().await?;
    db.drop_table::<RelationRole>().execute().await?;
    db.drop_table::<RelationPost>().execute().await?;
    db.drop_table::<RelationProfile>().execute().await?;
    db.drop_table::<RelationUser>().execute().await?;
    Ok(())
}
