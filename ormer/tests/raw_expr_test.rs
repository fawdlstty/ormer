#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

#[derive(ormer::Model)]
#[table = "raw_expr_users_1"]
struct RawExprUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
}

fn raw_expr_query(db_type: ormer::DbType) -> (String, Vec<ormer::Value>) {
    let term = "%alice%";
    let min_age = 18;
    let boost = 2;

    ormer::Select::<RawExprUser>::new()
        .filter(|u| {
            ormer::raw!("/* {{raw}} */ LOWER({u.name}) LIKE LOWER({term}) AND {u.age} > {min_age}")
        })
        .order_by_desc(|u| ormer::raw!("LENGTH({u.name}) + {boost}"))
        .map_to(|u| {
            ormer::raw!("COALESCE({u.name}, {term})")
                .typed::<String>()
                .alias("fallback_name")
        })
        .to_sql_with_params(db_type)
}

fn assert_raw_params(params: &[ormer::Value]) {
    assert!(matches!(
        params,
        [
            ormer::Value::Text(first_term),
            ormer::Value::Text(second_term),
            ormer::Value::Integer(min_age),
            ormer::Value::Integer(boost),
        ] if first_term == "%alice%"
            && second_term == "%alice%"
            && *min_age == 18
            && *boost == 2
    ));
}

fn raw_update_sql(db_type: ormer::DbType) -> (String, Vec<ormer::Value>) {
    let fallback = "anonymous";
    let mut update = <RawExprUser as ormer::Model>::Update::default();
    (|u: &mut <RawExprUser as ormer::Model>::Update| {
        u.name = u
            .name
            .set_expr(ormer::raw!("COALESCE({u.name}, {fallback})"));
    })(&mut update);
    let assignments =
        <<RawExprUser as ormer::Model>::Update as ormer::UpdateFields>::assignments(&update);

    ormer::abstract_layer::common::common_helpers::build_update_sql::<RawExprUser>(
        db_type,
        &assignments,
        &[],
    )
    .expect("raw update SQL should render")
}

fn assert_raw_update_params(params: &[ormer::Value]) {
    assert!(matches!(
        params,
        [ormer::Value::Text(fallback)] if fallback == "anonymous"
    ));
}

#[cfg(feature = "sqlite")]
#[test]
fn raw_expr_interpolates_sqlite_placeholders() {
    let (sql, params) = raw_expr_query(ormer::DbType::Sqlite);

    assert!(sql.contains("COALESCE(name, ?) AS fallback_name"));
    assert!(sql.contains("/* {raw} */ LOWER(name) LIKE LOWER(?) AND age > ?"));
    assert!(sql.contains("ORDER BY LENGTH(name) + ? DESC"));
    assert_raw_params(&params);

    let (update_sql, update_params) = raw_update_sql(ormer::DbType::Sqlite);
    assert!(update_sql.contains("SET name = COALESCE(name, ?)"));
    assert_raw_update_params(&update_params);
}

#[cfg(feature = "postgresql")]
#[test]
fn raw_expr_interpolates_postgresql_placeholders() {
    let (sql, params) = raw_expr_query(ormer::DbType::PostgreSQL);

    assert!(sql.contains("COALESCE(name, $1) AS fallback_name"));
    assert!(sql.contains("/* {raw} */ LOWER(name) LIKE LOWER($2) AND age > $3"));
    assert!(sql.contains("ORDER BY LENGTH(name) + $4 DESC"));
    assert_raw_params(&params);

    let (update_sql, update_params) = raw_update_sql(ormer::DbType::PostgreSQL);
    assert!(update_sql.contains("SET name = COALESCE(name, $1)"));
    assert_raw_update_params(&update_params);
}

#[cfg(feature = "mysql")]
#[test]
fn raw_expr_interpolates_mysql_placeholders() {
    let (sql, params) = raw_expr_query(ormer::DbType::MySQL);

    assert!(sql.contains("COALESCE(name, ?) AS fallback_name"));
    assert!(sql.contains("/* {raw} */ LOWER(name) LIKE LOWER(?) AND age > ?"));
    assert!(sql.contains("ORDER BY LENGTH(name) + ? DESC"));
    assert_raw_params(&params);

    let (update_sql, update_params) = raw_update_sql(ormer::DbType::MySQL);
    assert!(update_sql.contains("SET name = COALESCE(name, ?)"));
    assert_raw_update_params(&update_params);
}

#[cfg(feature = "mssql")]
#[test]
fn raw_expr_interpolates_mssql_placeholders() {
    let (sql, params) = raw_expr_query(ormer::DbType::MSSQL);

    assert!(sql.contains("COALESCE(name, @P1) AS fallback_name"));
    assert!(sql.contains("/* {raw} */ LOWER(name) LIKE LOWER(@P2) AND age > @P3"));
    assert!(sql.contains("ORDER BY LENGTH(name) + @P4 DESC"));
    assert_raw_params(&params);

    let (update_sql, update_params) = raw_update_sql(ormer::DbType::MSSQL);
    assert!(update_sql.contains("SET name = COALESCE(name, @P1)"));
    assert_raw_update_params(&update_params);
}
