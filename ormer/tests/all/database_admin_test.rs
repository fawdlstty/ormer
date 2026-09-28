#![cfg(feature = "postgresql")]

//! 数据库级管理四 API（db_access.md 3.2）测试：
//! - SQL 形状：纯函数精确匹配生成文本（无 PG 实例也能跑）；
//! - e2e：直连 PG 走 create/exists/apply_extension/plug 幂等与清库闭环。

use crate::_test_common::postgresql_config;
use ormer::abstract_layer::postgresql_backend::{
    create_database_sql, create_extension_sql, drop_database_sql,
};

/// 形状：合法标识符按 `quote_identifier` 规则生成精确文本。
#[test]
fn test_database_admin_sql_shape() {
    // 纯小写 + 下划线：无需引号化
    assert_eq!(
        create_database_sql("ormer_tmp_test").unwrap(),
        "CREATE DATABASE ormer_tmp_test"
    );
    assert_eq!(
        drop_database_sql("ormer_tmp_test").unwrap(),
        "DROP DATABASE IF EXISTS ormer_tmp_test WITH (FORCE)"
    );
    assert_eq!(
        create_extension_sql("plpgsql").unwrap(),
        "CREATE EXTENSION IF NOT EXISTS plpgsql CASCADE"
    );

    // 大写 / `$`：需引号化时双引号包裹
    assert_eq!(
        create_database_sql("OrmerTmpDB").unwrap(),
        "CREATE DATABASE \"OrmerTmpDB\""
    );
    assert_eq!(
        drop_database_sql("OrmerTmpDB").unwrap(),
        "DROP DATABASE IF EXISTS \"OrmerTmpDB\" WITH (FORCE)"
    );
    assert_eq!(
        create_extension_sql("My$Ext").unwrap(),
        "CREATE EXTENSION IF NOT EXISTS \"My$Ext\" CASCADE"
    );

    // 保留字同样引号化
    assert_eq!(
        create_database_sql("select").unwrap(),
        "CREATE DATABASE \"select\""
    );
}

/// 形状：非法标识符（空串、数字开头、含 `-` / `;` / 引号 / 空格）一律报错。
#[test]
fn test_database_admin_sql_rejects_unsafe_identifiers() {
    let bad = ["", "1abc", "bad-name", "bad;name", "bad\"name", "bad name", "drop--x"];
    for name in bad {
        assert!(create_database_sql(name).is_err(), "create: {name:?}");
        assert!(drop_database_sql(name).is_err(), "drop: {name:?}");
        assert!(create_extension_sql(name).is_err(), "extension: {name:?}");
    }
}

/// e2e：四 API 闭环（需本机 PG，惯例同 postgresql_json_test）。
#[tokio::test]
async fn test_database_admin_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let config = postgresql_config();
    let db = ormer::Database::connect(config.0, config.1).await?;

    // 清残留，保证起点干净
    db.drop_database("ormer_tmp_test").await?;

    assert!(!db.database_exists("ormer_tmp_test").await?);
    db.create_database("ormer_tmp_test").await?;
    assert!(db.database_exists("ormer_tmp_test").await?);
    // 已存在时同样 Ok(())，库仍在
    db.create_database("ormer_tmp_test").await?;
    assert!(db.database_exists("ormer_tmp_test").await?);

    // plpgsql 内置任何库都可装；幂等跑两遍
    db.apply_extension("ormer_tmp_test", "plpgsql").await?;
    db.apply_extension("ormer_tmp_test", "plpgsql").await?;

    // 目标库不存在：按连接失败报错
    assert!(db
        .apply_extension("ormer_tmp_test_missing", "plpgsql")
        .await
        .is_err());

    db.drop_database("ormer_tmp_test").await?;
    assert!(!db.database_exists("ormer_tmp_test").await?);
    // 再删一遍仍 Ok（幂等）
    db.drop_database("ormer_tmp_test").await?;

    // 非法库名：统一层校验拦截
    assert!(db.database_exists("bad-name").await.is_err());
    assert!(db.create_database("bad;name").await.is_err());
    assert!(db.drop_database("1abc").await.is_err());
    Ok(())
}
