// 跨 schema 枚举是 Postgres 特有回归，其余后端无需编译本测试
#![cfg(feature = "postgresql")]

//! 回归测试：schema 限定表名 + 枚举列，建表后的结构校验。
//!
//! 现场故障（东莞华勤 2026-09-17）：模型表名带 `collect.` 前缀后，建表的
//! CREATE TYPE 不带 schema 限定，枚举类型落在 search_path 首位（public），
//! 与表不同 schema；表已存在后的校验若按表 schema 反查枚举变体会查到空
//! 列表，误报 "Enum variants mismatch" 导致启动死循环。正确行为是按列的
//! udt_schema 反查枚举类型。

pub mod _test_common;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ormer::ModelEnum)]
pub enum ReproTaskKind {
    Charge,
    #[default]
    Load,
    Unload,
    Transport,
}

#[derive(Debug, ormer::Model, Clone)]
#[table = "collect.schema_enum_cross_tasks_1"]
struct SchemaEnumTask {
    #[primary]
    id: i64,
    name: String,
    kind: ReproTaskKind,
}

async fn test_schema_enum_cross_schema_impl(
    config: &_test_common::DbConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // 跨 schema 枚举类型是 postgres 特有语义，其余后端直接跳过
    if !matches!(config.0, ormer::DbType::PostgreSQL) {
        return Ok(());
    }

    let db = _test_common::create_db_connection(config).await?;

    // 清理历史残留；枚举类型的残留不影响，建类型语句本身幂等
    let _ = db.drop_table::<SchemaEnumTask>().execute().await;

    // 首次创建：CREATE TYPE（落 public）+ CREATE TABLE（collect schema）
    db.create_table::<SchemaEnumTask>()
        .execute()
        .await
        .expect("first create_table on schema-qualified table should succeed");

    // 关键断言：表已存在后再诊断，枚举类型虽在 public 也必须解析成功
    assert!(
        matches!(
            db.plan_table::<SchemaEnumTask>().await?,
            ormer::TableDiagnosis::Ready
        ),
        "plan_table must resolve enum type via column udt_schema, not table schema"
    );

    // 现场等价路径：重启后的建表调用对已存在的表走校验分支，也必须通过
    db.create_table::<SchemaEnumTask>()
        .execute()
        .await
        .expect("second create_table (validate path) must pass");

    let _ = db.drop_table::<SchemaEnumTask>().execute().await;
    Ok(())
}

test_on_all_dbs_result!(test_schema_enum_cross_schema_impl);
