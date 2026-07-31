#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

#[derive(Debug, Clone, ormer::Model)]
#[table = "auth.schema_table_users_1"]
struct SchemaTableUser {
    #[primary]
    id: i32,
    #[index]
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "auth.schema_table_parents_1"]
struct SchemaTableParent {
    #[primary]
    id: i32,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "auth.schema_table_children_1"]
struct SchemaTableChild {
    #[primary]
    id: i32,
    #[foreign(SchemaTableParent)]
    parent_id: i32,
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_strips_schema_prefix_from_table_names() {
    let db_type = ormer::DbType::Sqlite;

    assert_eq!(
        <SchemaTableUser as ormer::Model>::table_name_for_db(db_type),
        "schema_table_users_1"
    );

    let create_sql = ormer::generate_create_table_sql::<SchemaTableUser>(db_type).unwrap();
    assert!(create_sql.contains("CREATE TABLE IF NOT EXISTS schema_table_users_1"));
    assert!(create_sql.contains(
        "CREATE INDEX IF NOT EXISTS idx_schema_table_users_1_name ON schema_table_users_1 (name)"
    ));
    assert!(!create_sql.contains("auth.schema_table_users_1"));

    let child_sql = ormer::generate_create_table_sql::<SchemaTableChild>(db_type).unwrap();
    assert!(child_sql.contains("FOREIGN KEY (parent_id) REFERENCES schema_table_parents_1 (id)"));

    let (select_sql, _) =
        ormer::query::builder::Select::<SchemaTableUser>::new().to_sql_with_params(db_type);
    assert_eq!(select_sql, "SELECT id, name FROM schema_table_users_1");
}

#[cfg(feature = "mysql")]
#[test]
fn mysql_strips_schema_prefix_from_table_names() {
    let db_type = ormer::DbType::MySQL;

    assert_eq!(
        <SchemaTableUser as ormer::Model>::table_name_for_db(db_type),
        "schema_table_users_1"
    );

    let create_sql = ormer::generate_create_table_sql::<SchemaTableUser>(db_type).unwrap();
    assert!(create_sql.contains("CREATE TABLE IF NOT EXISTS schema_table_users_1"));
    assert!(
        create_sql
            .contains("CREATE INDEX idx_schema_table_users_1_name ON schema_table_users_1 (name)")
    );
    assert!(!create_sql.contains("auth.schema_table_users_1"));

    let child_sql = ormer::generate_create_table_sql::<SchemaTableChild>(db_type).unwrap();
    assert!(child_sql.contains("FOREIGN KEY (parent_id) REFERENCES schema_table_parents_1 (id)"));

    let (select_sql, _) =
        ormer::query::builder::Select::<SchemaTableUser>::new().to_sql_with_params(db_type);
    assert_eq!(select_sql, "SELECT id, name FROM schema_table_users_1");
}

#[cfg(feature = "postgresql")]
#[test]
fn postgresql_keeps_schema_prefix_in_table_names() {
    let db_type = ormer::DbType::PostgreSQL;

    assert_eq!(
        <SchemaTableUser as ormer::Model>::table_name_for_db(db_type),
        "auth.schema_table_users_1"
    );

    let create_sql = ormer::generate_create_table_sql::<SchemaTableUser>(db_type).unwrap();
    assert!(create_sql.contains("CREATE TABLE IF NOT EXISTS auth.schema_table_users_1"));
    assert!(create_sql.contains(
        "CREATE INDEX IF NOT EXISTS idx_auth_schema_table_users_1_name ON auth.schema_table_users_1 (name)"
    ));

    let child_sql = ormer::generate_create_table_sql::<SchemaTableChild>(db_type).unwrap();
    assert!(
        child_sql.contains("FOREIGN KEY (parent_id) REFERENCES auth.schema_table_parents_1 (id)")
    );

    let (select_sql, _) =
        ormer::query::builder::Select::<SchemaTableUser>::new().to_sql_with_params(db_type);
    assert_eq!(select_sql, "SELECT id, name FROM auth.schema_table_users_1");
}

#[cfg(feature = "mssql")]
#[test]
fn mssql_keeps_schema_prefix_in_table_names() {
    let db_type = ormer::DbType::MSSQL;

    assert_eq!(
        <SchemaTableUser as ormer::Model>::table_name_for_db(db_type),
        "auth.schema_table_users_1"
    );

    let create_sql = ormer::generate_create_table_sql::<SchemaTableUser>(db_type).unwrap();
    assert!(create_sql.contains("CREATE TABLE IF NOT EXISTS auth.schema_table_users_1"));
    assert!(create_sql.contains(
        "CREATE INDEX IF NOT EXISTS idx_auth_schema_table_users_1_name ON auth.schema_table_users_1 (name)"
    ));

    let child_sql = ormer::generate_create_table_sql::<SchemaTableChild>(db_type).unwrap();
    assert!(
        child_sql.contains("FOREIGN KEY (parent_id) REFERENCES auth.schema_table_parents_1 (id)")
    );

    let (select_sql, _) =
        ormer::query::builder::Select::<SchemaTableUser>::new().to_sql_with_params(db_type);
    assert_eq!(select_sql, "SELECT id, name FROM auth.schema_table_users_1");
}
