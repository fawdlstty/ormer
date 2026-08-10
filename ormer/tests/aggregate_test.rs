#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_with_score!(TestAggCountUser, "test_agg_count_users_1");
define_test_user_with_score!(TestAggSumUser, "test_agg_sum_users_1");
define_test_user_with_score!(TestAggAvgUser, "test_agg_avg_users_1");
define_test_user_with_score!(TestAggMaxUser, "test_agg_max_users_1");
define_test_user_with_score!(TestAggMinUser, "test_agg_min_users_1");
define_test_user_with_score!(TestAggFilterUser, "test_agg_filter_users_1");

/// 测试 COUNT 聚合函数
async fn test_count_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggCountUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggCountUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let count: usize = db.select::<TestAggCountUser>().count(|p| p.id).await?;
    println!("COUNT result: {:?}", count);

    assert_eq!(count, 3);

    _test_common::clean_table::<TestAggCountUser>(&db).await?;

    Ok(())
}

/// 测试 SUM 聚合函数
async fn test_sum_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggSumUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggSumUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let sum: Option<i32> = db.select::<TestAggSumUser>().sum(|p| p.age).await?;
    println!("SUM result: {:?}", sum);

    assert_eq!(sum, Some(67)); // 20 + 25 + 22

    _test_common::clean_table::<TestAggSumUser>(&db).await?;

    Ok(())
}

/// 测试 AVG 聚合函数
async fn test_avg_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggAvgUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggAvgUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let avg: Option<f64> = db.select::<TestAggAvgUser>().avg(|p| p.score).await?;
    println!("AVG result: {:?}", avg);

    assert!((avg.unwrap() - 85.0).abs() < 0.01);

    _test_common::clean_table::<TestAggAvgUser>(&db).await?;

    Ok(())
}

/// 测试 MAX 聚合函数
async fn test_max_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggMaxUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggMaxUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let max: Option<i32> = db.select::<TestAggMaxUser>().max(|p| p.age).await?;
    println!("MAX result: {:?}", max);

    assert_eq!(max, Some(25));

    _test_common::clean_table::<TestAggMaxUser>(&db).await?;

    Ok(())
}

/// 测试 MIN 聚合函数
async fn test_min_aggregate_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggMinUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggMinUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let min: Option<i32> = db.select::<TestAggMinUser>().min(|p| p.age).await?;
    println!("MIN result: {:?}", min);

    assert_eq!(min, Some(20));

    _test_common::clean_table::<TestAggMinUser>(&db).await?;

    Ok(())
}

/// 测试带过滤条件的聚合函数
async fn test_aggregate_with_filter_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    _test_common::prepare_table::<TestAggFilterUser>(&db).await?;
    _test_common::seed_score_users(&db, |id, name, age, score| TestAggFilterUser {
        id,
        name: name.to_string(),
        age,
        score,
    })
    .await?;

    let count: usize = db
        .select::<TestAggFilterUser>()
        .filter(|p| p.age.ge(22))
        .count(|p| p.id)
        .await?;
    println!("COUNT with filter result: {:?}", count);

    assert_eq!(count, 2);

    let max: Option<i32> = db
        .select::<TestAggFilterUser>()
        .filter(|p| p.age.ge(22))
        .max(|p| p.score)
        .await?;
    println!("MAX with filter result: {:?}", max);

    assert_eq!(max, Some(92));

    _test_common::clean_table::<TestAggFilterUser>(&db).await?;

    Ok(())
}

test_on_all_dbs_result!(test_count_aggregate_impl);
test_on_all_dbs_result!(test_sum_aggregate_impl);
test_on_all_dbs_result!(test_avg_aggregate_impl);
test_on_all_dbs_result!(test_max_aggregate_impl);
test_on_all_dbs_result!(test_min_aggregate_impl);
test_on_all_dbs_result!(test_aggregate_with_filter_impl);
