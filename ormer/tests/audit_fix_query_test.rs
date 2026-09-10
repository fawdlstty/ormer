//! 全库审查修复的查询层回归测试（M1 查询层部分、L1-L9）。
//!
//! 只依赖 SQL 生成与 SQLite 内存库，PostgreSQL/MSSQL 相关断言按 feature 门控。

#![cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "mssql"
))]

pub mod _test_common;

use ormer::query::builder::Select;
use ormer::{DbType, OrderBy};
#[cfg(feature = "postgresql")]
use ormer::FullTextMode;
#[cfg(any(feature = "sqlite", feature = "questdb"))]
use ormer::TimeUnit;
#[cfg(feature = "sqlite")]
use ormer::FullTextRank;

#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_users"]
struct AfqUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_orders"]
struct AfqOrder {
    #[primary]
    id: i32,
    user_id: i32,
    status: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_articles"]
struct AfqArticle {
    #[primary]
    id: i32,
    #[index(method = "fulltext", columns = "(title, body)")]
    title: String,
    body: String,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_products"]
struct AfqProduct {
    #[primary]
    id: i32,
    price: rust_decimal::Decimal,
}

#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_amounts"]
struct AfqAmount {
    #[primary]
    id: i32,
    user_id: i32,
    amount: i32,
}

#[derive(Debug, Clone, ormer::ViewModel)]
struct AfqUserTotal {
    user_id: i32,
    total: i64,
}

#[cfg(feature = "sqlite")]
fn decimal(value: &str) -> rust_decimal::Decimal {
    use std::str::FromStr;
    rust_decimal::Decimal::from_str(value).unwrap()
}

// ==================== M1：聚合/派生表校验出口与渲染层兜底 ====================

/// M1：AggregateSelect 的 try_ 版本对错误动态字段返回 Err 而不是渲染期 panic。
#[cfg(feature = "sqlite")]
#[test]
fn aggregate_try_to_sql_rejects_invalid_dynamic_field() {
    let aggregate = Select::<AfqUser>::new()
        .filter_dynamic(|p| p.field("typo").eq(1))
        .count(|u| u.id);
    let error = aggregate
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("invalid dynamic field must be rejected");
    assert!(
        error.to_string().contains("does not exist"),
        "unexpected error: {error}"
    );
}

/// M1：合法过滤器的聚合查询 try_ 版本正常出 SQL。
#[cfg(feature = "sqlite")]
#[test]
fn aggregate_try_to_sql_accepts_valid_filter() {
    let aggregate = Select::<AfqUser>::new()
        .filter(|u| u.age.gt(18))
        .count(|u| u.id);
    let (sql, params) = aggregate
        .try_to_sql_with_params(DbType::Sqlite)
        .expect("valid aggregate must render");
    assert!(sql.contains("COUNT("), "{sql}");
    assert_eq!(params.len(), 1);
}

/// M1：非 try_ 渲染路径遇到错误动态字段不再 panic，而是渲染错误占位引用。
#[cfg(feature = "sqlite")]
#[test]
fn invalid_dynamic_field_renders_error_marker_instead_of_panic() {
    let (sql, params) = Select::<AfqUser>::new()
        .filter_dynamic(|p| p.field("typo").eq(1))
        .to_sql_with_params(DbType::Sqlite);
    assert!(
        sql.contains("__ormer_invalid_filter__"),
        "expected error marker in SQL: {sql}"
    );
    assert!(params.is_empty());
}

/// M1：聚合执行器改为前置校验，无效动态字段在执行期返回指明字段的 Err，
/// 不再等数据库报错（错误信息来自 InvalidDynamicField 校验而非渲染占位）。
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn aggregate_executor_validates_invalid_dynamic_field() {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:")
        .await
        .unwrap();
    let _ = db.drop_table::<AfqUser>().execute().await;
    db.create_table::<AfqUser>().execute().await.unwrap();

    let result: ormer::Result<usize> = db
        .select::<AfqUser>()
        .filter_dynamic(|p| p.field("typo").eq(1))
        .count(|u| u.id)
        .await;
    let error = result.expect_err("invalid dynamic field must surface as an error");
    assert!(
        error.to_string().contains("does not exist"),
        "error must name the invalid field: {error}"
    );
    assert!(
        error.to_string().contains("typo"),
        "error must name the invalid field: {error}"
    );
}

/// M1：RelatedSelect 执行器路径同样前置校验（impl_multi_table_select! 宏
/// 生成的 try_ 版本），无效动态字段返回指明字段的 Err。
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn related_executor_validates_invalid_dynamic_field() {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:")
        .await
        .unwrap();
    let _ = db.drop_table::<AfqUser>().execute().await;
    let _ = db.drop_table::<AfqOrder>().execute().await;
    db.create_table::<AfqUser>().execute().await.unwrap();
    db.create_table::<AfqOrder>().execute().await.unwrap();

    let result: ormer::Result<Vec<AfqUser>> = db
        .select::<AfqUser>()
        .filter_dynamic(|p| p.field("typo").eq(1))
        .from::<AfqOrder>()
        .collect::<Vec<AfqUser>>()
        .await;
    let error = result.expect_err("invalid dynamic field must surface as an error");
    assert!(
        error.to_string().contains("does not exist"),
        "error must name the invalid field: {error}"
    );

    // 合法过滤器照常出结果，防止校验误伤正常路径
    let naive = chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc();
    db.insert(vec![
        AfqUser { id: 1, name: "a".into(), age: 20, created_at: naive },
        AfqUser { id: 2, name: "b".into(), age: 30, created_at: naive },
    ])
    .execute()
    .await
    .unwrap();
    db.insert(vec![AfqOrder { id: 1, user_id: 1, status: "paid".into() }])
        .execute()
        .await
        .unwrap();

    let rows: Vec<AfqUser> = db
        .select::<AfqUser>()
        .from::<AfqOrder>()
        .filter(|u, o| u.id.eq(o.user_id))
        .collect::<Vec<AfqUser>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "valid related query must still work");
}

/// M1：DerivedTableSelect 的 try_ 版本与 Select 对称（校验 + 渲染）。
#[cfg(feature = "sqlite")]
#[test]
fn derived_table_try_to_sql_validates_and_renders() {
    let derived = Select::<AfqAmount>::new()
        .select_column(|a| (a.user_id, a.amount.sum()))
        .group_by(|a| a.user_id)
        .as_model::<AfqUserTotal>();
    let table = ormer::from_derived(derived)
        .filter(|t| t.total.gt(50_i64))
        .order_by_desc(|t| t.total);
    let (sql, params) = table
        .try_to_sql_with_params(DbType::Sqlite)
        .expect("valid derived table query must render");
    assert!(sql.contains("FROM ("), "{sql}");
    assert!(sql.contains("AS t0"), "{sql}");
    assert_eq!(params.len(), 1);
}

// ==================== L1：UnionSelect 占位符重编号 ====================

/// L1：PostgreSQL 上 UNION 右侧占位符重编号为 $2，参数顺序对齐。
#[cfg(feature = "postgresql")]
#[test]
fn union_renumbers_placeholders_on_postgresql() {
    let union = Select::<AfqUser>::new()
        .filter(|u| u.age.gt(30))
        .union(Select::<AfqUser>::new().filter(|u| u.name.eq("admin")));

    let (sql, params) = union.to_sql_with_params(DbType::PostgreSQL);
    assert!(sql.contains("> $1"), "{sql}");
    assert!(sql.contains("= $2"), "{sql}");
    assert_eq!(sql.matches("$1").count(), 1, "{sql}");
    assert_eq!(params.len(), 2);

    let (sql, _) = union
        .try_to_sql_with_params(DbType::PostgreSQL)
        .expect("valid union must render");
    assert!(sql.contains("= $2"), "{sql}");
}

/// L1：MSSQL 不加括号路径的占位符同样重编号为 @P2。
#[cfg(feature = "mssql")]
#[test]
fn union_unparenthesized_renumbers_placeholders_on_mssql() {
    let union = Select::<AfqUser>::new()
        .filter(|u| u.age.gt(30))
        .union(Select::<AfqUser>::new().filter(|u| u.name.eq("admin")));

    let (sql, params) = union.to_sql_with_params_unparenthesized(DbType::MSSQL);
    assert!(sql.contains("> @P1"), "{sql}");
    assert!(sql.contains("= @P2"), "{sql}");
    assert_eq!(sql.matches("@P1").count(), 1, "{sql}");
    assert_eq!(params.len(), 2);
}

// ==================== L2：SQLite 全文检索 relevance 单份尾部 ====================

/// L2：relevance 与用户 order_by/range 组合只生成一份 ORDER BY/LIMIT。
#[cfg(feature = "sqlite")]
#[test]
fn sqlite_fulltext_relevance_with_order_and_range_renders_single_tail() {
    let (sql, _) = Select::<AfqArticle>::new()
        .fields(|a| (a.title,))
        .query("rust")
        .rank(FullTextRank::Relevance)
        .order_by(|a| a.id)
        .range(0..5)
        .to_sql_with_params(DbType::Sqlite);
    assert_eq!(sql.matches("ORDER BY").count(), 1, "{sql}");
    assert_eq!(sql.matches("LIMIT").count(), 1, "{sql}");
    assert!(sql.contains("__ormer_hits.__ormer_rank"), "{sql}");

    // 无用户排序时，relevance 仍提供唯一的 ORDER BY / LIMIT
    let (sql, _) = Select::<AfqArticle>::new()
        .fields(|a| (a.title,))
        .query("rust")
        .rank(FullTextRank::Relevance)
        .limit(3)
        .to_sql_with_params(DbType::Sqlite);
    assert_eq!(sql.matches("ORDER BY").count(), 1, "{sql}");
    assert_eq!(sql.matches("LIMIT").count(), 1, "{sql}");
    assert!(sql.contains("__ormer_hits.__ormer_rank"), "{sql}");
}

// ==================== L3：query() 未搭配 fields() ====================

/// L3：query() 单独调用（NULL 占位）被校验拒绝，而不是生成恒假/非法 SQL。
#[cfg(feature = "sqlite")]
#[test]
fn query_without_fields_is_rejected_on_sqlite() {
    let error = Select::<AfqArticle>::new()
        .query("rust")
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("query without fields must be rejected");
    assert!(
        error.to_string().contains("at least one field"),
        "unexpected error: {error}"
    );
}

/// L3：PostgreSQL 上同样拒绝。
#[cfg(feature = "postgresql")]
#[test]
fn query_without_fields_is_rejected_on_postgresql() {
    let error = Select::<AfqArticle>::new()
        .query("rust")
        .try_to_sql_with_params(DbType::PostgreSQL)
        .expect_err("query without fields must be rejected");
    assert!(
        error.to_string().contains("at least one field"),
        "unexpected error: {error}"
    );
}

// ==================== L4：mode/language/rank 先于 query/fields ====================

/// L4：rank() 在 fields()/query() 之前调用不再被丢弃。
#[cfg(feature = "sqlite")]
#[test]
fn fulltext_rank_before_fields_and_query_is_kept() {
    let (sql, _) = Select::<AfqArticle>::new()
        .rank(FullTextRank::Relevance)
        .fields(|a| (a.title,))
        .query("rust")
        .to_sql_with_params(DbType::Sqlite);
    assert!(sql.contains("bm25("), "{sql}");
}

/// L4：mode/language 在 fields()/query() 之前调用不再被丢弃。
#[cfg(feature = "postgresql")]
#[test]
fn fulltext_mode_and_language_before_fields_query_are_kept() {
    let (sql, params) = Select::<AfqArticle>::new()
        .mode(FullTextMode::Boolean)
        .language("english")
        .fields(|a| (a.title,))
        .query("rust +ormer")
        .to_sql_with_params(DbType::PostgreSQL);
    assert!(sql.contains("to_tsquery"), "{sql}");
    assert!(!sql.contains("plainto_tsquery"), "{sql}");
    // language 作为绑定参数出现（to_tsvector/to_tsquery 共用同一个占位符）
    assert!(
        params.iter().any(|value| matches!(value, ormer::Value::Text(language) if language == "english")),
        "params: {params:?}"
    );
}

// ==================== L5：DateAdd 校验内层表达式 ====================

/// L5：DateAdd 的 validate_for_db 递归校验被操作表达式，
/// SQLite 上内层 AtTimeZone 不支持时在校验期返回 Err。
#[cfg(feature = "sqlite")]
#[test]
fn date_add_validates_inner_expr() {
    let error = Select::<AfqUser>::new()
        .filter(|u| {
            u.created_at
                .at_time_zone("UTC")
                .add(TimeUnit::Day, 1)
                .gt(ormer::value(String::new()))
        })
        .try_to_sql_with_params(DbType::Sqlite)
        .expect_err("unsupported inner expr must be rejected by validation");
    assert!(
        error.to_string().contains("timezone conversion"),
        "unexpected error: {error}"
    );
}

/// L5：QuestDB 上 JSON 提取 + DateAdd 组合在校验期被拒绝（todo 原始场景）。
#[cfg(feature = "questdb")]
#[test]
fn date_add_validates_inner_expr_on_questdb() {
    let error = Select::<AfqUser>::new()
        .filter(|u| {
            u.name
                .json_text("k")
                .add(TimeUnit::Day, 1)
                .gt(ormer::raw::<String>("''"))
        })
        .try_to_sql_with_params(DbType::QuestDB)
        .expect_err("QuestDB JSON text extraction inside DateAdd must be rejected");
    assert!(
        error.to_string().contains("JSON text extraction"),
        "unexpected error: {error}"
    );
}

// ==================== L6：and/or 丢弃右操作数 LATERAL 配置 ====================

/// L6：右操作数携带 LATERAL 排序/分页配置时组合期报错，
/// 不再静默丢弃子查询限制。
#[test]
#[should_panic(expected = "LATERAL")]
fn where_expr_and_rejects_lateral_config_on_right_operand() {
    let _ = Select::<AfqUser>::new().left_join::<AfqOrder>(|u, o| {
        u.id.eq(o.user_id)
            .and(o.status.eq("paid").order_by(o.id).range(0..3))
    });
}

#[test]
#[should_panic(expected = "LATERAL")]
fn where_expr_or_rejects_lateral_config_on_right_operand() {
    let _ = Select::<AfqUser>::new().left_join::<AfqOrder>(|u, o| {
        u.id.eq(o.user_id)
            .or(o.status.eq("paid").order_by(o.id).range(0..3))
    });
}

// ==================== L7：SQLite Decimal BETWEEN/IN 的 CAST ====================

/// L7：SQLite 上 Decimal 的 BETWEEN/IN 与比较运算符一样走 CAST AS NUMERIC。
#[cfg(feature = "sqlite")]
#[test]
fn sqlite_decimal_between_and_in_apply_numeric_cast() {
    let (sql, params) = Select::<AfqProduct>::new()
        .filter(|p| p.price.between(decimal("9.90"), decimal("10.10")))
        .to_sql_with_params(DbType::Sqlite);
    assert!(
        sql.contains("AS NUMERIC) BETWEEN CAST("),
        "BETWEEN must cast both bounds: {sql}"
    );
    assert_eq!(params.len(), 2);

    let (sql, params) = Select::<AfqProduct>::new()
        .filter(|p| p.price.is_in(vec![decimal("9.90"), decimal("10.10")]))
        .to_sql_with_params(DbType::Sqlite);
    assert!(
        sql.contains("AS NUMERIC) IN (CAST("),
        "IN must cast every placeholder: {sql}"
    );
    assert_eq!(params.len(), 2);
}

/// L7：行为验证——TEXT 存储的 Decimal 列 BETWEEN 按数值命中（修复前漏行）。
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_decimal_between_matches_numerically() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(DbType::Sqlite, ":memory:").await?;
    let _ = db.drop_table::<AfqProduct>().execute().await;
    db.create_table::<AfqProduct>().execute().await?;

    let _ = db
        .insert(vec![
            AfqProduct { id: 1, price: decimal("9.50") },
            AfqProduct { id: 2, price: decimal("10.00") },
            AfqProduct { id: 3, price: decimal("10.50") },
        ])
        .execute()
        .await?;

    let rows: Vec<AfqProduct> = db
        .select::<AfqProduct>()
        .filter(|p| p.price.between(decimal("9.90"), decimal("10.10")))
        .collect()
        .await?;
    assert_eq!(rows.len(), 1, "numeric BETWEEN must match the 10.00 row");
    assert_eq!(rows[0].id, 2);

    // 同一列 ge 命中集合应与 between 一致（对照 comparison_sql 的 CAST 分支）
    let ge_rows: Vec<AfqProduct> = db
        .select::<AfqProduct>()
        .filter(|p| p.price.ge(decimal("9.90")))
        .collect()
        .await?;
    assert_eq!(ge_rows.len(), 2);
    Ok(())
}

// ==================== L8：map_to_model 列数断言 ====================

/// L8：单列投影映射到多列模型在构造期报错（单列模型不受影响）。
#[derive(Debug, Clone, ormer::Model)]
#[table = "afq_single_col_models"]
struct AfqSingleColumn {
    #[primary]
    uid: i32,
}

#[test]
#[should_panic(expected = "expects 4 columns")]
fn map_to_model_rejects_column_count_mismatch() {
    let _ = Select::<AfqUser>::new().map_to_model::<AfqUser, _, _>(|u| u.id);
}

#[cfg(feature = "sqlite")]
#[test]
fn map_to_model_accepts_single_column_target() {
    let mapped = Select::<AfqUser>::new().map_to_model::<AfqSingleColumn, _, _>(|u| u.id);
    let (sql, _) = mapped.to_sql_with_params(DbType::Sqlite);
    assert!(sql.contains("AS uid"), "{sql}");
}

// ==================== L9：OrderBy::to_sql 与 to_sql_for 一致 ====================

/// L9：无参 to_sql() 与 to_sql_for(default_db_type()) 输出一致（含列名引用）。
#[test]
fn order_by_to_sql_matches_to_sql_for_default_dialect() {
    let order = OrderBy::asc("Order".into());
    #[cfg(feature = "sqlite")]
    assert_eq!(order.to_sql(), order.to_sql_for(DbType::Sqlite));
    assert_ne!(order.to_sql(), "Order ASC");
    assert!(order.to_sql().ends_with("ASC"));
}
