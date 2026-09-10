//! 审查修复批次（W3：模型/派生/迁移）的行为回归测试。
//!
//! 覆盖：M6（迁移外键 diff）、M7（4..=8 元组复合主键）、M8（QuestDB 非
//! String 索引列）、M9（多态枚举列赋值）、M10（DbValue 未配置后端）、
//! L14（派生宏字符串驻留）、L15（版本快照键单射）、L17（宽 repr 数值枚举
//! 的无损转换）、L18（保留字标识符引号化）。
#![cfg(any(feature = "sqlite", feature = "postgresql"))]

pub mod _test_common;

use ormer::model::PrimaryKey;
#[cfg(feature = "sqlite")]
use ormer::Database;
#[cfg(feature = "postgresql")]
use ormer::MigrationStep;
use ormer::{Model, OrmerError, Value};

// ---------------------------------------------------------------------------
// M7：复合主键 PrimaryKey 元组实现扩展到 8 列
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_pk4"]
struct Pk4 {
    #[primary]
    a: i32,
    #[primary]
    b: i32,
    #[primary]
    c: i32,
    #[primary]
    d: i32,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_pk8"]
struct Pk8 {
    #[primary]
    a: i32,
    #[primary]
    b: i32,
    #[primary]
    c: i32,
    #[primary]
    d: i32,
    #[primary]
    e: i32,
    #[primary]
    f: i32,
    #[primary]
    g: i32,
    #[primary]
    h: i32,
}

#[test]
fn composite_primary_key_tuples_expand_to_eight_columns() {
    let values = PrimaryKey::into_values((1_i32, 2, 3, 4));
    assert_eq!(
        values,
        vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(4),
        ]
    );

    let values = PrimaryKey::into_values((1_i32, 2, 3, 4, 5, 6, 7, 8));
    assert_eq!(values.len(), 8);
    assert_eq!(values[7], Value::Integer(8));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn composite_primary_key_find_by_id_accepts_four_and_eight_tuples() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<Pk4>().execute().await?;
    db.create_table::<Pk8>().execute().await?;

    db.insert(&Pk4 {
        a: 1,
        b: 2,
        c: 3,
        d: 4,
    })
    .execute()
    .await?;

    let row = db.find_by_id::<Pk4>((1, 2, 3, 4)).await?;
    assert!(row.is_some());

    db.insert(&Pk8 {
        a: 1,
        b: 2,
        c: 3,
        d: 4,
        e: 5,
        f: 6,
        g: 7,
        h: 8,
    })
    .execute()
    .await?;

    let row = db.find_by_id::<Pk8>((1, 2, 3, 4, 5, 6, 7, 8)).await?;
    assert!(row.is_some());
    let missing = db.find_by_id::<Pk8>((1, 2, 3, 4, 5, 6, 7, 9)).await?;
    assert!(missing.is_none());
    Ok(())
}

// ---------------------------------------------------------------------------
// L15：乐观锁版本快照内容键单射（Null 与 Text("null") 不得共享快照）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_version_snapshots"]
#[version(u64)]
struct VersionedSnapshotUser {
    #[primary]
    id: i32,
    nick: Option<String>,
}

#[test]
fn version_snapshot_key_distinguishes_null_from_text_null() {
    let text_null = VersionedSnapshotUser {
        id: 1,
        nick: Some("null".to_string()),
    };
    let actual_null = VersionedSnapshotUser { id: 2, nick: None };

    ormer::model::clear_version_snapshots::<VersionedSnapshotUser>();
    ormer::model::record_version_snapshot(&text_null, 5);

    // 修复前：两行内容键都编码为 "null"，actual_null 会错误读到 5
    assert_ne!(ormer::model::model_version(&actual_null), 5);
    assert_eq!(ormer::model::model_version(&text_null), 5);
}

// ---------------------------------------------------------------------------
// L18：保留字列名在建表 SQL 中被引号化
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_reserved_words"]
struct ReservedWordColumns {
    #[primary]
    id: i32,
    limit: i32,
    offset: i32,
    left: i32,
    values: i32,
}

#[cfg(feature = "sqlite")]
#[test]
fn reserved_word_columns_are_quoted_in_create_table_sql() {
    let sql = ormer::generate_create_table_sql::<ReservedWordColumns>(ormer::DbType::Sqlite)
        .expect("create table sql");
    for quoted in ["\"limit\"", "\"offset\"", "\"left\"", "\"values\""] {
        assert!(sql.contains(quoted), "expected {quoted} in: {sql}");
    }
    // 非保留字列保持未引用
    assert!(sql.contains("(id INTEGER"), "{sql}");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn reserved_word_columns_roundtrip_on_sqlite() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.drop_table::<ReservedWordColumns>().execute().await?;
    db.create_table::<ReservedWordColumns>().execute().await?;

    db.insert(&ReservedWordColumns {
        id: 1,
        limit: 10,
        offset: 20,
        left: 30,
        values: 40,
    })
    .execute()
    .await?;
    let row = db
        .select::<ReservedWordColumns>()
        .filter(|r| r.limit.eq(10))
        .first()
        .await?;
    assert!(row.is_some());
    Ok(())
}

// ---------------------------------------------------------------------------
// M8：QuestDB 非 String 列的 #[index] 返回 UnsupportedFeature
// ---------------------------------------------------------------------------

#[cfg(feature = "questdb")]
mod questdb_index_gate {
    use ormer::{Model, OrmerError};

    #[derive(Debug, Clone, ormer::Model)]
    #[table = "audit_questdb_events"]
    struct QuestDbNumericIndexEvent {
        #[primary]
        ts: chrono::DateTime<chrono::Utc>,
        #[index]
        severity: i32,
    }

    #[derive(Debug, Clone, ormer::Model)]
    #[table = "audit_questdb_tags"]
    struct QuestDbStringIndexEvent {
        #[primary]
        ts: chrono::DateTime<chrono::Utc>,
        #[index]
        tag: String,
    }

    #[test]
    fn questdb_rejects_index_on_non_string_column() {
        let error = ormer::generate_create_table_sql::<QuestDbNumericIndexEvent>(
            ormer::DbType::QuestDB,
        )
        .expect_err("non-String indexed column must be rejected");
        assert!(matches!(error, OrmerError::UnsupportedFeature { .. }), "{error}");
    }

    #[test]
    fn questdb_keeps_inline_symbol_index_for_string_columns() {
        let sql = ormer::generate_create_table_sql::<QuestDbStringIndexEvent>(
            ormer::DbType::QuestDB,
        )
        .expect("String indexed column renders");
        assert!(sql.contains("SYMBOL"), "{sql}");
        assert!(sql.contains(" INDEX"), "{sql}");
    }
}

// ---------------------------------------------------------------------------
// M10：DbValue 未配置后端建表返回 UnsupportedFeature（而非 panic）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, ormer::DbValue)]
#[db_type(sqlite = "TEXT")]
struct AuditOnlySqliteMoney(rust_decimal::Decimal);

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_db_value_rows"]
struct DbValueRow {
    #[primary]
    id: i32,
    amount: AuditOnlySqliteMoney,
}

#[cfg(feature = "sqlite")]
#[test]
fn db_value_create_table_uses_configured_backend_type() {
    let sql =
        ormer::generate_create_table_sql::<DbValueRow>(ormer::DbType::Sqlite).expect("sqlite sql");
    assert!(sql.contains("TEXT"), "{sql}");
}

#[cfg(feature = "postgresql")]
#[test]
fn db_value_create_table_errors_on_unconfigured_backend() {
    let error =
        ormer::generate_create_table_sql::<DbValueRow>(ormer::DbType::PostgreSQL)
            .expect_err("unconfigured backend must error");
    assert!(matches!(error, OrmerError::UnsupportedFeature { .. }), "{error}");
}

// ---------------------------------------------------------------------------
// M9：多态枚举字段的列赋值
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, ormer::ModelEnum)]
enum AuditAttr {
    Text {
        text_value: String,
    },
    Num {
        num_value: i64,
    },
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_poly_nodes"]
struct PolyNode {
    #[primary]
    id: i32,
    attr: AuditAttr,
}

#[test]
fn polymorphic_enum_assigns_discriminator_and_payload_columns() -> ormer::Result<()> {
    let mut node = PolyNode {
        id: 1,
        attr: AuditAttr::Num { num_value: 7 },
    };

    // discriminator 列（模型字段名）与当前变体一致 → 赋值成功
    <PolyNode as Model>::assign_column_value(&mut node, "attr", Value::Text("num".into()))?;

    // payload 列在当前变体上 → 写入对应字段
    node.assign_column_value("num_value", Value::Integer(9))?;
    assert_eq!(node.attr, AuditAttr::Num { num_value: 9 });

    // discriminator 值与当前变体不一致：仅凭 discriminator 无法切换变体
    let mut text_node = PolyNode {
        id: 2,
        attr: AuditAttr::Text {
            text_value: "x".to_string(),
        },
    };
    let error = text_node
        .assign_column_value("attr", Value::Text("num".into()))
        .expect_err("variant switch must fail");
    assert!(
        error.to_string().contains("cannot switch"),
        "unexpected error: {error}"
    );

    // payload 列属于另一变体：无法在当前变体上赋值
    let error = text_node
        .assign_column_value("num_value", Value::Integer(1))
        .expect_err("cross-variant payload assignment must fail");
    assert!(
        error.to_string().contains("different variant"),
        "unexpected error: {error}"
    );

    // 未声明的列 → 维持既有 not assignable 错误
    assert!(text_node.assign_column_value("missing", Value::Null).is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// L17：数值枚举 repr(i64) 的无损转换
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ormer::ModelEnum)]
#[repr(i64)]
enum AuditI64Repr {
    Disabled = 0,
    Active = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ormer::ModelEnum)]
#[repr(u16)]
enum AuditU16Repr {
    Disabled = 0,
    Active = 1,
}

#[test]
fn numeric_enum_i64_repr_roundtrips_losslessly() -> ormer::Result<()> {
    use ormer::model::{FieldType, FromValue};

    assert!(matches!(
        Value::from(AuditI64Repr::Active),
        Value::Integer(value) if value == 1
    ));
    assert_eq!(
        <AuditI64Repr as FromValue>::from_value(&Value::Integer(1))?,
        AuditI64Repr::Active
    );
    assert_eq!(
        <AuditI64Repr as FieldType>::as_i64(&AuditI64Repr::Active),
        1
    );
    assert_eq!(
        <AuditI64Repr as FieldType>::from_i64(1)?,
        AuditI64Repr::Active
    );

    // 窄 repr 仍提供 From<Enum> for i32（#[data_type(i32)] 依赖）
    assert_eq!(i32::from(AuditU16Repr::Active), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// L14：派生宏字符串驻留（hot path 不再每次 Box::leak）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, ormer::Embed)]
struct AuditAddress {
    city: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_embed_users"]
struct EmbedPrefixUser {
    #[primary]
    id: i32,
    #[embed(prefix = "addr_")]
    address: AuditAddress,
    name: String,
}

#[test]
fn embed_columns_use_interned_static_names() {
    assert_eq!(
        EmbedPrefixUser::columns(),
        vec!["id", "addr_city", "name"]
    );
    // 重复调用拿到同一份 &'static 拷贝（驻留生效），而不是每次新泄漏
    let first = EmbedPrefixUser::columns();
    let second = EmbedPrefixUser::columns();
    let first_addr = first[1];
    let second_addr = second[1];
    assert!(std::ptr::eq(first_addr, second_addr));

    let schema_names: Vec<&'static str> =
        EmbedPrefixUser::column_schema().iter().map(|c| c.name).collect();
    assert_eq!(schema_names, vec!["id", "addr_city", "name"]);
    let first_rust_name = EmbedPrefixUser::column_schema()[1].rust_name;
    let second_rust_name = EmbedPrefixUser::column_schema()[1].rust_name;
    assert!(std::ptr::eq(first_rust_name, second_rust_name));
}

#[test]
fn intern_helpers_return_stable_references() {
    let a = ormer::model::intern_concat("p_", "q");
    let b = ormer::model::intern_concat("p_", "q");
    assert!(std::ptr::eq(a, b));
    let c = ormer::model::intern_prefixed("runtime_", "q");
    let d = ormer::model::intern_prefixed("runtime_", "q");
    assert!(std::ptr::eq(c, d));
    assert_eq!(c, "runtime_q");
}

// ---------------------------------------------------------------------------
// M6：已有列的外键变更进入迁移计划
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_fk_users"]
struct FkDiffUser {
    #[primary(auto)]
    id: i32,
    name: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_fk_orders"]
struct FkDiffOrderNoFk {
    #[primary]
    id: i32,
    user_id: i32,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "audit_fk_orders"]
struct FkDiffOrderWithFk {
    #[primary]
    id: i32,
    #[foreign(FkDiffUser)]
    user_id: i32,
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_foreign_key_add_on_existing_column_reports_unmigratable() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.create_table::<FkDiffUser>().execute().await?;
    db.create_table::<FkDiffOrderNoFk>().execute().await?;

    // 给已有列补 #[foreign]：SQLite 无法 ALTER ADD FOREIGN KEY，计划应给出
    // 明确的 UnmigratableSchema 而不是静默空计划
    let error = db
        .migrate_table::<FkDiffOrderWithFk>()
        .plan()
        .await
        .expect_err("adding a foreign key to an existing SQLite column must fail");
    assert!(matches!(error, OrmerError::UnmigratableSchema { .. }), "{error}");
    assert!(error.to_string().contains("user_id"), "{error}");
    Ok(())
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_foreign_key_drop_records_warning() -> ormer::Result<()> {
    let db = Database::connect(ormer::DbType::Sqlite, ":memory:").await?;
    db.drop_table::<FkDiffOrderWithFk>().execute().await?;
    db.create_table::<FkDiffUser>().execute().await?;
    db.create_table::<FkDiffOrderWithFk>().execute().await?;

    // 从模型移除 #[foreign]：SQLite 无法 DROP CONSTRAINT，应记录 warning
    let plan = db.migrate_table::<FkDiffOrderNoFk>().plan().await?;
    assert!(
        plan.warnings()
            .iter()
            .any(|warning| warning.contains("cannot drop constraints")),
        "warnings: {:?}",
        plan.warnings()
    );
    Ok(())
}

#[cfg(feature = "postgresql")]
#[tokio::test]
async fn postgres_foreign_key_diff_adds_and_drops_constraints() -> ormer::Result<()> {
    let config = _test_common::postgresql_config();
    let db = _test_common::create_db_connection(&config)
        .await
        .map_err(|error| ormer::ormer_error!("connect: {error}"))?;

    db.drop_table::<FkDiffOrderNoFk>().execute().await?;
    db.drop_table::<FkDiffOrderWithFk>().execute().await?;
    db.drop_table::<FkDiffUser>().execute().await?;
    db.create_table::<FkDiffUser>().execute().await?;
    db.create_table::<FkDiffOrderNoFk>().execute().await?;

    // v1：无外键 → v2：给已有列补 #[foreign] → 计划生成 AddForeignKey
    let plan = db.migrate_table::<FkDiffOrderWithFk>().plan().await?;
    let add_step = plan
        .steps()
        .iter()
        .find(|step| matches!(step, MigrationStep::AddForeignKey { column, .. } if column == "user_id"));
    assert!(add_step.is_some(), "steps: {:?}", plan.steps());
    db.migrate_table::<FkDiffOrderWithFk>().execute().await?;

    // 计划幂等：外键已与模型一致，不再生成外键步骤
    let plan = db.migrate_table::<FkDiffOrderWithFk>().plan().await?;
    assert!(
        !plan
            .steps()
            .iter()
            .any(|step| matches!(step, MigrationStep::AddForeignKey { .. })),
        "steps: {:?}",
        plan.steps()
    );

    // v2 → v1：移除 #[foreign] → 计划生成 DROP CONSTRAINT
    let plan = db.migrate_table::<FkDiffOrderNoFk>().plan().await?;
    let sql = plan.to_sql()?;
    assert!(sql.contains("DROP CONSTRAINT"), "sql: {sql}");
    db.migrate_table::<FkDiffOrderNoFk>().execute().await?;

    db.drop_table::<FkDiffOrderNoFk>().execute().await?;
    db.drop_table::<FkDiffUser>().execute().await?;
    Ok(())
}
