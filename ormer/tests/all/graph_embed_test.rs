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

#[cfg(feature = "duckdb")]
#[tokio::test]
async fn duckdb_through_link_primitives_sync_links() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::DuckDB, ":memory:").await?;
    prepare_graph_tables(&db).await?;

    let user = GraphUser {
        id: 7,
        name: "Alice".to_string(),
        profile: None,
        posts: Vec::new(),
        user_roles: Vec::new(),
        roles: Vec::new(),
    };
    db.insert(&user).execute().await?;
    for role_id in 1..=3 {
        db.insert(&GraphRole {
            id: role_id,
            name: format!("role{role_id}"),
        })
        .execute()
        .await?;
    }

    let mut tx = db.begin().await?;
    let via_relation = ormer::model::graph_relation_info::<GraphUser, GraphUserRole>("user_roles")?;
    let target_relation = ormer::model::graph_relation_info::<GraphUserRole, GraphRole>("role")?;
    let stored_user = db.select::<GraphUser>().first().await?.unwrap();
    let owner_key = stored_user.relation_key_value(via_relation)?;

    for role_id in [1, 1, 2, 3] {
        ormer::model::graph_insert_through_link_values::<GraphUserRole>(
            &mut tx,
            via_relation.target_key,
            owner_key.clone(),
            target_relation.local_key,
            ormer::Value::Integer(role_id),
        )
        .await?;
    }
    ormer::model::graph_sync_through_links::<GraphUser, GraphUserRole>(
        &mut tx,
        &stored_user,
        via_relation,
        target_relation,
        &[ormer::Value::Integer(2), ormer::Value::Integer(3)],
    )
    .await?;
    tx.commit().await?;

    let links = db
        .select::<GraphUserRole>()
        .filter(|link| link.user_id.eq(stored_user.id))
        .order_by(|link| link.role_id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        links.iter().map(|link| link.role_id).collect::<Vec<_>>(),
        vec![2, 3]
    );

    Ok(())
}

#[cfg(feature = "duckdb")]
#[tokio::test]
async fn duckdb_update_graph_updates_parent_with_children() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::DuckDB, ":memory:").await?;
    prepare_graph_tables(&db).await?;

    let mut user = GraphUser {
        id: 0,
        name: "Alice".to_string(),
        profile: Some(GraphProfile {
            id: 0,
            user_id: 0,
            bio: "Alice bio".to_string(),
        }),
        posts: vec![GraphPost {
            id: 0,
            user_id: 0,
            title: "First".to_string(),
        }],
        user_roles: Vec::new(),
        roles: Vec::new(),
    };
    db.insert_graph(&mut user).execute().await?;

    user.name = "Alice updated".to_string();
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
    assert_eq!(profile.bio, "Alice bio");

    let posts = db
        .select::<GraphPost>()
        .filter(|post| post.user_id.eq(user.id))
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "First");

    Ok(())
}

#[tokio::test]
async fn sqlite_tracked_save_diffs_included_relations() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    prepare_graph_tables(&db).await?;

    let mut user = GraphUser {
        id: 0,
        name: "Alice".to_string(),
        profile: None,
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

    let mut tracked = db
        .select::<GraphUser>()
        .filter(|model| model.id.eq(user.id))
        .include(|model| model.posts.order_by_desc(|post| post.id))
        .include(|model| model.roles.order_by_desc(|role| role.id))
        .collect::<Vec<_>>()
        .await?
        .into_iter()
        .next()
        .unwrap()
        .track();

    let changed_post_id = tracked.posts[1].id;
    tracked.posts[1].title = "First updated".to_string();
    tracked.posts.remove(0);
    tracked.posts.push(GraphPost {
        id: 0,
        user_id: 0,
        title: "Third".to_string(),
    });
    tracked.posts.reverse();

    tracked.roles[0].name = "viewer updated".to_string();
    tracked.roles.remove(1);
    tracked.roles.push(GraphRole {
        id: 3,
        name: "auditor".to_string(),
    });
    tracked.roles.reverse();

    let affected = db.save(&mut tracked).execute().await?;
    assert!(affected > 0);

    let posts = db
        .select::<GraphPost>()
        .filter(|post| post.user_id.eq(user.id))
        .order_by(|post| post.id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(posts.len(), 2);
    assert!(!posts.iter().any(|post| post.title == "Second"));
    assert_eq!(
        posts
            .iter()
            .find(|post| post.id == changed_post_id)
            .unwrap()
            .title,
        "First updated"
    );
    assert!(tracked.posts.iter().any(|post| post.title == "Third"));
    assert!(tracked.posts.iter().all(|post| post.id > 0));

    let links = db
        .select::<GraphUserRole>()
        .filter(|link| link.user_id.eq(user.id))
        .order_by(|link| link.role_id.asc())
        .collect::<Vec<_>>()
        .await?;
    assert_eq!(
        links.iter().map(|link| link.role_id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    let mut roles = db.select::<GraphRole>().collect::<Vec<_>>().await?;
    roles.sort_by_key(|role| role.id);
    assert_eq!(roles[0].name, "viewer updated");
    assert_eq!(roles[1].name, "viewer");
    assert_eq!(roles[2].name, "auditor");

    tracked.posts.clear();
    tracked.roles.clear();
    let affected = db.save(&mut tracked).execute().await?;
    assert!(affected > 0);
    assert_eq!(
        db.select::<GraphPost>()
            .filter(|post| post.user_id.eq(user.id))
            .collect::<Vec<_>>()
            .await?
            .len(),
        0
    );
    assert_eq!(
        db.select::<GraphUserRole>()
            .filter(|link| link.user_id.eq(user.id))
            .collect::<Vec<_>>()
            .await?
            .len(),
        0
    );

    Ok(())
}
