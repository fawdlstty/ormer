#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

/// Grouped Select 端到端测试
/// 验证聚合查询在实际数据库中的执行情况
mod _test_common;

// 使用宏定义测试专用模型（每个测试使用唯一表名）
define_test_user_with_score!(TestGroupedE2EUser, "test_grouped_e2e_basic_1");

use ormer::model::{FromRowValues, Value};

// 定义聚合结果类型（为未来扩展保留）
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct AgeGroupStats {
    age: i32,
    user_count: i64,
    avg_score: f64,
}

impl FromRowValues for AgeGroupStats {
    fn from_row_values(values: &[Value]) -> Result<Self, ormer::Error> {
        if values.len() < 3 {
            return Err(ormer::Error::Database(
                "Expected at least 3 values".to_string(),
            ));
        }

        let age = match &values[0] {
            Value::Integer(i) => *i as i32,
            _ => {
                return Err(ormer::Error::Database(
                    "Expected integer for age".to_string(),
                ));
            }
        };

        let user_count = match &values[1] {
            Value::Integer(i) => *i,
            _ => {
                return Err(ormer::Error::Database(
                    "Expected integer for user_count".to_string(),
                ));
            }
        };

        let avg_score = match &values[2] {
            Value::Real(f) => *f,
            _ => {
                return Err(ormer::Error::Database(
                    "Expected real for avg_score".to_string(),
                ));
            }
        };

        Ok(AgeGroupStats {
            age,
            user_count,
            avg_score,
        })
    }
}

/// 测试基本的 GROUP BY 聚合查询
async fn test_grouped_select_basic_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = _test_common::create_db_connection(config).await?;

    // 清理旧表并创建新表
    let _ = db.drop_table::<TestGroupedE2EUser>().execute().await;
    db.create_table::<TestGroupedE2EUser>().execute().await?;

    // 插入测试数据
    let users = vec![
        TestGroupedE2EUser {
            id: 1,
            name: "Alice".to_string(),
            age: 20,
            score: 85,
        },
        TestGroupedE2EUser {
            id: 2,
            name: "Bob".to_string(),
            age: 20,
            score: 90,
        },
        TestGroupedE2EUser {
            id: 3,
            name: "Charlie".to_string(),
            age: 25,
            score: 78,
        },
        TestGroupedE2EUser {
            id: 4,
            name: "Diana".to_string(),
            age: 25,
            score: 92,
        },
        TestGroupedE2EUser {
            id: 5,
            name: "Eve".to_string(),
            age: 20,
            score: 88,
        },
    ];

    for user in users {
        db.insert(&user).await?;
    }

    // 执行分组聚合查询 - 按年龄分组，统计每组人数
    let count: Vec<ormer::query::builder::TypedColumn<usize>> = db
        .select::<TestGroupedE2EUser>()
        .select_column(|u| u.id.count())
        .group_by(|u| u.age)
        .collect()
        .await?;

    // 验证结果
    println!("GROUP BY count result: {:?} groups", count.len());
    // 总共有5条记录，按年龄分组后应该是2组（20岁3人，25岁2人）
    assert_eq!(count.len(), 2);

    // 清理
    let _ = db.drop_table::<TestGroupedE2EUser>().execute().await;

    Ok(())
}

test_on_all_dbs_result!(test_grouped_select_basic_impl);

/// 测试 GROUP BY + HAVING
async fn test_grouped_select_with_having_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 使用唯一表名避免冲突
    define_test_user_with_score!(TestGroupedE2EUserHaving, "test_grouped_e2e_having_2");

    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    let _ = db.drop_table::<TestGroupedE2EUserHaving>().execute().await;
    db.create_table::<TestGroupedE2EUserHaving>()
        .execute()
        .await?;

    // 插入测试数据
    let users = vec![
        TestGroupedE2EUserHaving {
            id: 1,
            name: "Alice".to_string(),
            age: 20,
            score: 85,
        },
        TestGroupedE2EUserHaving {
            id: 2,
            name: "Bob".to_string(),
            age: 20,
            score: 90,
        },
        TestGroupedE2EUserHaving {
            id: 3,
            name: "Charlie".to_string(),
            age: 25,
            score: 78,
        },
    ];

    for user in users {
        db.insert(&user).await?;
    }

    // 执行分组聚合查询，只返回用户数 >= 2 的年龄组
    let count: Vec<ormer::query::builder::TypedColumn<usize>> = db
        .select::<TestGroupedE2EUserHaving>()
        .select_column(|u| u.id.count())
        .group_by(|u| u.age)
        .having(|u| u.id.count().ge(2))
        .collect()
        .await?;

    // 验证结果 - 只有 age=20 的组有 2 个或以上用户
    println!("GROUP BY HAVING result: {:?} groups", count.len());
    assert_eq!(count.len(), 1);

    // 清理
    let _ = db.drop_table::<TestGroupedE2EUser>().execute().await;

    Ok(())
}

test_on_all_dbs_result!(test_grouped_select_with_having_impl);

/// 测试 GROUP BY + WHERE filter
async fn test_grouped_select_with_filter_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 使用唯一表名避免冲突
    define_test_user_with_score!(TestGroupedE2EUserFilter, "test_grouped_e2e_filter_3");

    let db = _test_common::create_db_connection(config).await?;

    // 创建表
    let _ = db.drop_table::<TestGroupedE2EUserFilter>().execute().await;
    db.create_table::<TestGroupedE2EUserFilter>()
        .execute()
        .await?;

    // 插入测试数据
    let users = vec![
        TestGroupedE2EUserFilter {
            id: 1,
            name: "Alice".to_string(),
            age: 20,
            score: 85,
        },
        TestGroupedE2EUserFilter {
            id: 2,
            name: "Bob".to_string(),
            age: 20,
            score: 90,
        },
        TestGroupedE2EUserFilter {
            id: 3,
            name: "Charlie".to_string(),
            age: 25,
            score: 78,
        },
        TestGroupedE2EUserFilter {
            id: 4,
            name: "Diana".to_string(),
            age: 25,
            score: 95,
        },
    ];

    for user in users {
        db.insert(&user).await?;
    }

    // 执行分组聚合查询，只查询分数 > 80 的用户
    let count: Vec<ormer::query::builder::TypedColumn<usize>> = db
        .select::<TestGroupedE2EUserFilter>()
        .select_column(|u| u.id.count())
        .filter(|u| u.score.gt(80))
        .group_by(|u| u.age)
        .collect()
        .await?;

    // 验证结果
    println!("GROUP BY FILTER result: {:?} groups", count.len());
    // age=20 的组有 2 个用户分数 > 80
    // age=25 的组只有 1 个用户分数 > 80 (Diana)
    assert_eq!(count.len(), 2);

    // 清理
    let _ = db.drop_table::<TestGroupedE2EUser>().execute().await;

    Ok(())
}

test_on_all_dbs_result!(test_grouped_select_with_filter_impl);
