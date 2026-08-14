#![cfg(feature = "sqlite")]

use ormer::{Database, Model, Value};

#[derive(Debug, Clone, PartialEq, ormer::Embed)]
struct GraphAddress {
    city: String,
    street: String,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_embed_users"]
struct EmbedUser {
    #[primary(auto)]
    id: i32,
    name: String,
    #[embed(prefix = "addr_")]
    address: GraphAddress,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_users"]
struct GraphUser {
    #[primary(auto)]
    id: i32,
    name: String,
    #[has_one(GraphProfile.user_id)]
    profile: Option<GraphProfile>,
    #[has_many(GraphPost.user_id)]
    posts: Vec<GraphPost>,
    #[has_many(GraphUserRole.user_id)]
    user_roles: Vec<GraphUserRole>,
    #[through(user_roles.role)]
    roles: Vec<GraphRole>,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_profiles"]
struct GraphProfile {
    #[primary(auto)]
    id: i32,
    #[foreign(GraphUser.id)]
    user_id: i32,
    bio: String,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_posts"]
struct GraphPost {
    #[primary(auto)]
    id: i32,
    #[foreign(GraphUser.id)]
    user_id: i32,
    title: String,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_roles"]
struct GraphRole {
    #[primary]
    id: i32,
    name: String,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "graph_user_roles"]
struct GraphUserRole {
    #[primary]
    #[foreign(GraphUser.id)]
    user_id: i32,
    #[primary]
    #[foreign(GraphRole.id)]
    role_id: i32,
    #[belongs_to(role_id)]
    role: Option<GraphRole>,
}

async fn prepare_graph_tables(db: &Database) -> ormer::Result<()> {
    db.create_table::<GraphUser>().execute().await?;
    db.create_table::<GraphProfile>().execute().await?;
    db.create_table::<GraphPost>().execute().await?;
    db.create_table::<GraphRole>().execute().await?;
    db.create_table::<GraphUserRole>().execute().await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_embed_expands_columns_and_roundtrips() -> ormer::Result<()> {
    assert_eq!(
        EmbedUser::columns(),
        vec!["id", "name", "addr_city", "addr_street"]
    );
    assert_eq!(
        EmbedUser::column_schema()
            .iter()
            .map(|column| column.name)
            .collect::<Vec<_>>(),
        vec!["id", "name", "addr_city", "addr_street"]
    );

    let sql = ormer::generate_create_table_sql::<EmbedUser>(ormer::DbType::Sqlite)?;
    assert!(sql.contains("addr_city TEXT NOT NULL"));
    assert!(sql.contains("addr_street TEXT NOT NULL"));

    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<EmbedUser>().execute().await?;

    let user = EmbedUser {
        id: 0,
        name: "Alice".to_string(),
        address: GraphAddress {
            city: "Shanghai".to_string(),
            street: "Century Ave".to_string(),
        },
    };
    db.insert(&user).execute().await?;

    let users = db
        .select::<EmbedUser>()
        .filter(|u| u.address.city.eq("Shanghai"))
        .collect::<Vec<_>>()
        .await?;

    assert_eq!(users.len(), 1);
    assert_eq!(users[0].address, user.address);
    assert!(matches!(
        users[0].column_value("addr_city"),
        Some(Value::Text(value)) if value == "Shanghai"
    ));
    Ok(())
}

#[tokio::test]
async fn sqlite_insert_graph_inserts_relations_in_one_call() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    prepare_graph_tables(&db).await?;

    let mut user = GraphUser {
        id: 0,
        name: "Alice".to_string(),
        profile: Some(GraphProfile {
            id: 0,
            user_id: 0,
            bio: "Alice bio".to_string(),
        }),
        posts: vec![
            GraphPost {
                id: 0,
                user_id: 0,
                title: "First".to_string(),
            },
            GraphPost {
                id: 0,
                user_id: 0,
                title: "Second".to_string(),
            },
        ],
        user_roles: Vec::new(),
        roles: vec![
            GraphRole {
                id: 1,
                name: "admin".to_string(),
            },
            GraphRole {
                id: 2,
                name: "viewer".to_string(),
            },
        ],
    };

    db.insert_graph(&mut user).execute().await?;

    assert!(user.id > 0);
    assert_eq!(user.profile.as_ref().unwrap().user_id, user.id);
    assert!(user.profile.as_ref().unwrap().id > 0);
    assert!(user.posts.iter().all(|post| post.user_id == user.id));
    assert!(user.posts.iter().all(|post| post.id > 0));

    let posts = db
        .select::<GraphPost>()
        .filter(|post| post.user_id.eq(user.id))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(posts.len(), 2);

    let links = db
        .select::<GraphUserRole>()
        .filter(|link| link.user_id.eq(user.id))
        .order_by(|link| link.role_id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        links.iter().map(|link| link.role_id).collect::<Vec<_>>(),
        vec![1, 2]
    );

    Ok(())
}

#[tokio::test]
async fn sqlite_update_graph_upserts_children_and_syncs_through_links() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    prepare_graph_tables(&db).await?;

    let mut user = GraphUser {
        id: 0,
        name: "Alice".to_string(),
        profile: Some(GraphProfile {
            id: 0,
            user_id: 0,
            bio: "Old bio".to_string(),
        }),
        posts: vec![GraphPost {
            id: 0,
            user_id: 0,
            title: "Old title".to_string(),
        }],
        user_roles: Vec::new(),
        roles: vec![
            GraphRole {
                id: 1,
                name: "admin".to_string(),
            },
            GraphRole {
                id: 2,
                name: "viewer".to_string(),
            },
        ],
    };
    db.insert_graph(&mut user).execute().await?;

    user.name = "Alice updated".to_string();
    user.profile.as_mut().unwrap().bio = "New bio".to_string();
    user.posts[0].title = "New title".to_string();
    user.roles = vec![
        GraphRole {
            id: 2,
            name: "viewer".to_string(),
        },
        GraphRole {
            id: 3,
            name: "auditor".to_string(),
        },
    ];

    let affected = db.update_graph(&mut user).execute().await?;
    assert_eq!(affected, 1);

    let stored_user = db.find_by_id::<GraphUser>(user.id).await?.unwrap();
    assert_eq!(stored_user.name, "Alice updated");

    let profile = db
        .select::<GraphProfile>()
        .filter(|profile| profile.user_id.eq(user.id))
        .first()
        .await?
        .unwrap();
    assert_eq!(profile.bio, "New bio");

    let post = db
        .select::<GraphPost>()
        .filter(|post| post.user_id.eq(user.id))
        .first()
        .await?
        .unwrap();
    assert_eq!(post.title, "New title");

    let links = db
        .select::<GraphUserRole>()
        .filter(|link| link.user_id.eq(user.id))
        .order_by(|link| link.role_id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        links.iter().map(|link| link.role_id).collect::<Vec<_>>(),
        vec![2, 3]
    );

    Ok(())
}
