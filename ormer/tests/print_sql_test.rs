use ormer::Model;
use ormer::abstract_layer::DbType;

#[derive(Debug, Model, Clone)]
#[table = "test_sql_users"]
struct TestSqlUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[tokio::test]
async fn test_print_aggregate_sql() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestSqlUser>().await?;

    // 测试不带filter的聚合 - 直接从 Select 获取 AggregateSelect
    let select1 = ormer::query::builder::Select::<TestSqlUser>::new();
    let agg1 = select1.count(|p| p.id);
    let (sql, params) = agg1.to_sql_with_params(DbType::Turso);
    println!("COUNT SQL: {}", sql);
    println!("COUNT params: {:?}", params);

    // 测试带filter的聚合
    let select2 = ormer::query::builder::Select::<TestSqlUser>::new();
    let filtered = select2.filter(|p| p.age.ge(18));
    let agg2 = filtered.count(|p| p.id);
    let (sql2, params2) = agg2.to_sql_with_params(DbType::Turso);
    println!("COUNT with filter SQL: {}", sql2);
    println!("COUNT with filter params: {:?}", params2);

    // 测试 MAX
    let select3 = ormer::query::builder::Select::<TestSqlUser>::new();
    let agg3 = select3.max(|p| p.age);
    let (sql3, params3) = agg3.to_sql_with_params(DbType::Turso);
    println!("MAX SQL: {}", sql3);
    println!("MAX params: {:?}", params3);

    Ok(())
}
