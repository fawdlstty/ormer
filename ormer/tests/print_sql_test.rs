#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_direct!(TestSqlUser, "test_sql_users_1");

async fn test_print_aggregate_sql_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;
    db.create_table::<TestSqlUser>().execute().await?;

    // 测试不带filter的聚合 - 直接从 Select 获取 AggregateSelect
    let select1 = ormer::query::builder::Select::<TestSqlUser>::new();
    let agg1 = select1.count(|p| p.id);
    let (sql, params) = agg1.to_sql_with_params(config.0);
    println!("COUNT SQL: {}", sql);
    println!("COUNT params: {:?}", params);

    // 测试带filter的聚合
    let select2 = ormer::query::builder::Select::<TestSqlUser>::new();
    let filtered = select2.filter(|p| p.age.ge(18));
    let agg2 = filtered.count(|p| p.id);
    let (sql2, params2) = agg2.to_sql_with_params(config.0);
    println!("COUNT with filter SQL: {}", sql2);
    println!("COUNT with filter params: {:?}", params2);

    // 测试 MAX
    let select3 = ormer::query::builder::Select::<TestSqlUser>::new();
    let agg3 = select3.max(|p| p.age);
    let (sql3, params3) = agg3.to_sql_with_params(config.0);
    println!("MAX SQL: {}", sql3);
    println!("MAX params: {:?}", params3);

    // 清理测试表
    db.drop_table::<TestSqlUser>().execute().await?;

    Ok(())
}

test_on_all_dbs_result!(test_print_aggregate_sql_impl);
