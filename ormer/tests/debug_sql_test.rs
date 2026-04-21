use ormer::Model;

#[derive(Debug, Model, Clone)]
#[table = "test_debug_users"]
struct TestDebugUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[tokio::test]
async fn test_debug_sql() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;
    db.create_table::<TestDebugUser>().await?;

    // 测试不带filter的聚合
    let select1 = db.select::<TestDebugUser>();
    let _agg1 = select1.count(|p| p.id);

    // 测试带filter的聚合
    let select2 = db.select::<TestDebugUser>();
    let filtered = select2.filter(|p| p.age.ge(18));
    let _agg2 = filtered.count(|p| p.id);

    println!("Tests completed");
    Ok(())
}
