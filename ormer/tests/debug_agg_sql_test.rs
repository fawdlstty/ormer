use ormer::Model;

#[derive(Debug, Model, Clone)]
#[table = "test_agg_users"]
struct TestAggUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
    score: i32,
}

#[tokio::test]
async fn test_debug_agg_sql() -> Result<(), Box<dyn std::error::Error>> {
    // 创建 Select 并生成 SQL
    let select = ormer::query::builder::Select::<TestAggUser>::new();
    let agg = select.count(|p| p.id);
    let (sql, params) = agg.to_sql_with_params();

    println!("SQL: '{}'", sql);
    println!("SQL length: {}", sql.len());
    println!("Params: {:?}", params);

    // 打印每个字符及其位置
    for (i, ch) in sql.chars().enumerate() {
        println!("  [{}] '{}'", i, ch);
    }

    Ok(())
}
