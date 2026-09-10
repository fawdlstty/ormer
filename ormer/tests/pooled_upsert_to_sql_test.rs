//! L12：池化 InsertOrUpdate / InsertOrIgnore 执行器的 to_sql 必须与后端
//! 执行器 to_sql 同源（同一 common_helpers 渲染入口），本测试逐字比对
//! 两条路径产出的 SQL、参数与语句分块。
#![cfg(feature = "sqlite")]

pub mod _test_common;

define_test_user_minimal!(UpsertSqlUser, "test_pool_upsert_sql_users_1");

use ormer::{Database, DbType, SqlStatement};

fn assert_same_sql(pooled: &SqlStatement, direct: &SqlStatement) {
    assert_eq!(pooled.db_type, direct.db_type);
    assert_eq!(
        pooled.statements.len(),
        direct.statements.len(),
        "statement count mismatch"
    );
    for (index, (pooled, direct)) in pooled
        .statements
        .iter()
        .zip(direct.statements.iter())
        .enumerate()
    {
        assert_eq!(pooled.sql, direct.sql, "statement {index} sql mismatch");
        assert_eq!(
            pooled.params, direct.params,
            "statement {index} params mismatch"
        );
    }
}

#[tokio::test]
async fn pooled_upsert_and_ignore_to_sql_match_backend_executor() {
    let pool = _test_common::sqlite_pool().await.expect("build sqlite pool");
    let conn = pool.get().await.expect("acquire pooled connection");
    let db = Database::connect(DbType::Sqlite, ":memory:")
        .await
        .expect("connect sqlite database");

    let user = |id: i32, name: &str| UpsertSqlUser {
        id,
        name: name.to_string(),
    };

    // 自增主键已设置 / 未设置混合（upsert 按主键是否已设置拆分语句）
    let mixed = vec![user(7, "a"), user(0, "b")];
    assert_same_sql(
        &conn.insert_or_update(&mixed).to_sql().unwrap(),
        &db.insert_or_update(&mixed).to_sql().unwrap(),
    );

    // 自增主键全部未设置（排除自增列的纯插入路径）
    let unset = vec![user(0, "c"), user(0, "d")];
    assert_same_sql(
        &conn.insert_or_update(&unset).to_sql().unwrap(),
        &db.insert_or_update(&unset).to_sql().unwrap(),
    );

    // insert_or_ignore（INSERT OR IGNORE，排除自增列）
    assert_same_sql(
        &conn.insert_or_ignore(&mixed).to_sql().unwrap(),
        &db.insert_or_ignore(&mixed).to_sql().unwrap(),
    );

    // 超过 SQLite 999 绑定参数上限，验证两条路径按相同尺寸分块
    let bulk: Vec<UpsertSqlUser> = (0..1500)
        .map(|index| user(0, &format!("u{index}")))
        .collect();
    assert_same_sql(
        &conn.insert_or_update(&bulk).to_sql().unwrap(),
        &db.insert_or_update(&bulk).to_sql().unwrap(),
    );
    assert_same_sql(
        &conn.insert_or_ignore(&bulk).to_sql().unwrap(),
        &db.insert_or_ignore(&bulk).to_sql().unwrap(),
    );
    assert!(
        conn.insert_or_ignore(&bulk).to_sql().unwrap().statements.len() > 1,
        "bulk insert_or_ignore should be chunked"
    );
}
