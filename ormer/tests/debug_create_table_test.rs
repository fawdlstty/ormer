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
async fn test_debug_create_table_sql() -> Result<(), Box<dyn std::error::Error>> {
    // 生成 CREATE TABLE SQL
    let sql = ormer::generate_create_table_sql::<TestAggUser>(ormer::DbType::Turso);

    println!("CREATE TABLE SQL: '{}'", sql);
    println!("SQL length: {}", sql.len());

    // 打印每个字符及其位置
    for (i, ch) in sql.chars().enumerate() {
        if i >= 60 && i <= 80 {
            println!("  [{}] '{}'", i, ch);
        }
    }

    Ok(())
}
