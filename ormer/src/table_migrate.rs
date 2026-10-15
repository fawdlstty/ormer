//! 表迁移统一入口：`migrate_table`（内部诊断路径 `plan_table`）。
//!
//! 设计原则（规格书 auto.md 第四章）：
//! ① 判断与执行各只有一个入口：`plan_table` 诊断（内部）、`migrate_table` 执行；
//! ② 内省数据不出 ormer：所有 pg_* 自省由本模块内部消费，不设细粒度公开 API；
//! ③ 项目只声明模型：迁移策略全部内化为固定语义（多余列删除、推不动即报错、
//!    索引并发建、扩展从模型推导）；
//! ④ 全部幂等：migrate 可被启动重试循环反复调用，CONCURRENTLY 残留在 migrate 内自愈；
//! ⑤ plan 与 migrate 同源：migrate 内部就是重新诊断（`diagnose_table`）后按计划执行。

use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{Database, Transaction};
#[cfg_attr(not(feature = "postgresql"), allow(unused_imports))]
use crate::migration::split_qualified_table_name;
use crate::migration::{
    execute_steps, execute_steps_nontransactional, is_schema_rebuild_error, table_name,
    ExpectedIndexDef, MigrationPlan, MigrationStep, TableMigration,
};
use crate::model::WritableModel;
#[cfg_attr(not(feature = "postgresql"), allow(unused_imports))]
use crate::model::Value;
use std::collections::BTreeSet;

/// [`Database::plan_table`] 产出的执行计划（内部诊断路径）。与版本化迁移的
/// [`MigrationPlan`] 同一载体（同源设计）：`steps()` / `to_sql()` 可在执行前审查。
pub type TablePlan = MigrationPlan;

/// 表迁移诊断结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableDiagnosis {
    /// 库与模型一致，无需任何动作（等价替代旧 validate_table 通过）。
    Ready,
    /// 可保数据就地迁移：附执行计划（建表/AddColumn/AlterColumn/ChangePrimaryKey/
    /// 索引与 CHECK 约束对齐…）。
    Migratable(TablePlan),
    /// 保数据推不动，只有删表重建才能对齐；附原因（后端不支持原地改主键、
    /// 类型收窄、超表分区约束冲突等）。
    NeedsRebuild(RebuildCause),
}

/// 保数据推不动、只能删表重建的原因说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildCause {
    /// 基础表名（带 schema 前缀，与模型声明一致）。
    pub table: String,
    /// 人读的原因描述。
    pub reason: String,
}

impl RebuildCause {
    pub(crate) fn new(table: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            reason: reason.into(),
        }
    }
}

/// [`Database::migrate_table`] 的执行结果。
#[derive(Debug)]
pub struct TableMigrateOutcome {
    /// 执行前的诊断结论。成功返回时只可能是 Ready / Migratable（推不动
    /// 或复验不收敛在执行前就直接返回 Err）。
    pub diagnosis: TableDiagnosis,
    /// 本次调用是否执行了基础表的 CREATE TABLE（仅源于表原本不存在的新建）。
    pub created_table: bool,
    /// 实际执行的步骤（含建索引/清漂移索引/ChangePrimaryKey）。
    pub executed: Vec<MigrationStep>,
    /// 保数据重建路径本次回灌的行数；未走重建路径时为 None。
    pub migrated_rows: Option<i64>,
    /// 前置检查清理掉的遗留备份表名（`{schema}.{table}_migrate_old`，拍板 B：
    /// 正常路径的备份表在阶段一事务内校验通过后即删、不存续，仅异常/历史
    /// 残留会触发清理并记录在此）；未清理时为 None。
    pub backup_table: Option<String>,
}

// ---------------------------------------------------------------------------
// 纯函数区：SQL 片段归一化、索引语义签名、CHECK 表达式归一化。
// 不触库，全部可单测。
// ---------------------------------------------------------------------------

/// 折叠空白为单空格并 trim。
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 去掉包裹整个表达式的冗余外层括号（循环、顶层逗号感知）。
/// `(a, b)` 的括号包裹多列，不属于冗余括号，保留。
fn trim_redundant_parens(value: &str) -> String {
    let mut current = collapse_whitespace(value);
    for _ in 0..64 {
        let trimmed = current.trim();
        if !(trimmed.starts_with('(') && trimmed.ends_with(')')) || trimmed.len() < 2 {
            return trimmed.to_string();
        }
        // 首括号必须配对到末字符（排除 `(a) + (b)`），且括号内部不得出现
        // 相对深度 1 的逗号（`(a, b)` 是列表/元组而非冗余包裹）
        let mut depth = 0usize;
        let mut closes_at_end = false;
        let mut inner_top_comma = false;
        for (idx, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        closes_at_end = idx + ch.len_utf8() == trimmed.len();
                    }
                }
                ',' if depth == 1 => inner_top_comma = true,
                _ => {}
            }
        }
        if !closes_at_end || inner_top_comma || depth != 0 {
            return trimmed.to_string();
        }
        let inner = trimmed[1..trimmed.len() - 1].trim();
        if inner.is_empty() {
            return trimmed.to_string();
        }
        current = inner.to_string();
    }
    current
}

/// 归一化 SQL 片段用于比对：折叠空白、去标识符引号、去冗余外层括号。
pub(crate) fn norm_sql_fragment(value: &str) -> String {
    let no_quotes = value.replace(['"', '`'], "");
    trim_redundant_parens(&no_quotes)
}

/// 去掉 `::` 类型注解：`::` 后连续的标识符字符/方括号/点。
/// 字符串字面量内的 `::` 不受影响（扫描时跳过单引号区段）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn strip_type_casts(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            in_string = !in_string;
            out.push(ch);
            continue;
        }
        if !in_string && ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            // 吃掉类型 token：标识符字符、[]、点（`::text[]`、`::pg_catalog.tsl`）。
            // 不吃圆括号：`::decimal(10,2)` 处理不到属可接受（保守方向是
            // 误判"定义漂移"触发 Drop+Add，而不是误判相等漏改）。
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' || next == '[' || next == ']' {
                    chars.next();
                } else {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// 括号树归一（CHECK 表达式比对的核心）：
/// - 紧跟标识符字符的 `(` 是函数调用括号，保留（`cardinality(roles)`）；
/// - 其余 `(` 若内部无顶层逗号（非元组/列表）则视为冗余包裹，剥除
///   （`((x))` 与 `(x)` 与 `x` 判等）；
/// - 字符串字面量整体保留。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn normalize_parens(input: &str) -> String {
    let chars: Vec<char> = collapse_whitespace(input).chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\'' {
            // 字符串字面量原样复制（含成对引号）
            out.push(ch);
            index += 1;
            while index < chars.len() {
                out.push(chars[index]);
                if chars[index] == '\'' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if ch == '(' {
            // 配对右括号
            let mut depth = 0usize;
            let mut close = None;
            for (k, &matched) in chars.iter().enumerate().skip(index) {
                match matched {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else {
                // 括号不配对（异常输入）：原样收尾
                out.extend(chars[index..].iter());
                break;
            };
            let inner: String = chars[index + 1..close].iter().collect();
            let prev = if index == 0 {
                ' '
            } else {
                chars[index - 1]
            };
            let prev_is_ident =
                prev.is_ascii_alphanumeric() || prev == '_' || prev == ')' || prev == ']';
            let has_top_level_comma = {
                let mut depth = 0usize;
                let mut comma = false;
                for inner_ch in inner.chars() {
                    match inner_ch {
                        '(' => depth += 1,
                        ')' => depth = depth.saturating_sub(1),
                        ',' if depth == 0 => {
                            comma = true;
                            break;
                        }
                        _ => {}
                    }
                }
                comma
            };
            let inner_normalized = normalize_parens(&inner);
            if !prev_is_ident && !has_top_level_comma {
                out.push_str(&inner_normalized);
            } else {
                out.push('(');
                out.push_str(&inner_normalized);
                out.push(')');
            }
            index = close + 1;
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// 归一化 CHECK 表达式：剥 CHECK 关键字前缀（pg_get_constraintdef 形态
/// `CHECK ((expr))`）、去 `::` 类型注解、括号树归一。
/// `CHECK ((cardinality((roles)::text[])) > 0)` 与
/// `cardinality(roles) > 0` 判等。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
pub(crate) fn normalize_check_expression(value: &str) -> String {
    let fragment = collapse_whitespace(value);
    let without_keyword = if fragment.to_ascii_uppercase().starts_with("CHECK ") {
        fragment[6..].trim()
    } else {
        fragment.as_str()
    };
    let without_cast = strip_type_casts(without_keyword);
    normalize_parens(&without_cast)
}

/// PostgreSQL `pg_get_indexdef` 定义文本的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PgIndexDefinition {
    /// 索引方法（btree/gin/gist/...）。pg_get_indexdef 总是输出 USING 子句。
    pub method: String,
    /// 括号内列表达式文本（原样，未归一）。
    pub columns: String,
    /// WHERE 谓词文本（原样；部分索引才有）。
    pub predicate: Option<String>,
}

/// 解析 `CREATE [UNIQUE] INDEX name ON schema.table USING method (cols) WHERE pred`。
/// 列文本可能包含任意括号嵌套（表达式索引），按括号深度扫描提取。
pub(crate) fn parse_pg_index_definition(definition: &str) -> Option<PgIndexDefinition> {
    let upper = definition.to_ascii_uppercase();
    let using_pos = upper.find(" USING ")?;
    let after_using = &definition[using_pos + " USING ".len()..];
    let method = after_using
        .split_whitespace()
        .next()?
        .to_ascii_lowercase();
    let rest = &after_using[method.len()..];

    // 第一个顶层 '(' 起的配对括号组为列表达式
    let open = rest.find('(')?;
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut close = None;
    for (offset, ch) in bytes[open..].iter().enumerate() {
        match ch {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let columns = rest[open + 1..close].to_string();
    let tail = &rest[close + 1..];
    let tail_upper = tail.to_ascii_uppercase();
    let predicate = tail_upper
        .find(" WHERE ")
        .map(|pos| tail[pos + " WHERE ".len()..].trim().to_string())
        .filter(|value| !value.is_empty());
    Some(PgIndexDefinition {
        method,
        columns,
        predicate,
    })
}

/// 自省索引的统一视图（各后端收敛到同一结构供语义 diff 消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActualIndexFacts {
    pub name: String,
    /// (列名, 是否降序)。definition 可解析时来自定义文本，否则来自自省结构。
    pub columns: Vec<(String, bool)>,
    pub unique: bool,
    /// PG indisvalid；其余后端恒 true。
    pub valid: bool,
    /// 索引方法（PG pg_am）；其余后端 None（按 btree 处理）。
    pub method: Option<String>,
    /// PG pg_get_indexdef 全文；其余后端 None。
    pub definition: Option<String>,
    /// 部分索引谓词（definition 可解析时提取）。
    pub predicate: Option<String>,
}

impl ActualIndexFacts {
    /// 从无定义文本的自省数据构造（SQLite/MySQL/MSSQL/DuckDB 降级路径）。
    pub(crate) fn from_plain(
        name: impl Into<String>,
        columns: Vec<(String, bool)>,
        unique: bool,
    ) -> Self {
        Self {
            name: name.into(),
            columns,
            unique,
            valid: true,
            method: None,
            definition: None,
            predicate: None,
        }
    }

    /// 语义 key（方法归一）：列序列 + 降序 + 唯一性。
    fn plain_key(&self) -> (bool, Vec<(String, bool)>) {
        (self.unique, self.columns.clone())
    }
}

/// 索引语义比对（规格书 §4.3）：按 列集合+唯一性+method/expression/where
/// 定义文本 比对而非按名字。
///
/// 返回值第一项：`actual` 与 `expected[i]` 语义相等时取 `Some(actual 下标)`。
pub(crate) fn match_index_semantics(
    expected_unique: bool,
    expected_method: Option<&str>,
    expected_columns: &[(String, bool)],
    expected_expression: Option<&str>,
    expected_predicate: Option<&str>,
    actual: &ActualIndexFacts,
) -> bool {
    if !actual.valid {
        return false;
    }
    // INCLUDE（存储覆盖）列不属于模型声明的键语义：带 INCLUDE 的实际索引
    // 一律不视为任何模型声明的实现，交由删除侧清理、按模型重建纯索引，
    // 确保库结构与模型严格一致（2026-10-09 拍板：外部建的结构一律撤销）。
    if actual
        .definition
        .as_deref()
        .is_some_and(|definition| definition.to_ascii_uppercase().contains(") INCLUDE ("))
    {
        return false;
    }
    if expected_unique != actual.unique {
        return false;
    }
    // 方法比对：期望声明了 method 或实际侧有非 btree 方法时要求一致
    let actual_method = actual
        .method
        .as_deref()
        .unwrap_or("btree");
    let expected_method_value = expected_method.unwrap_or("btree");
    if !expected_method_value.eq_ignore_ascii_case(actual_method) {
        return false;
    }
    // 特殊索引（表达式/显式列清单）：必须按实际定义文本比对才可靠；
    // 实际侧无定义文本（非 PG 降级路径）时视为不可匹配，由调用方按
    // "无法确认漂移"的保守策略处理（缺→建仍由 IF NOT EXISTS 保证幂等）。
    if let Some(expected_expression_value) = expected_expression {
        let Some(definition) = &actual.definition else {
            return false;
        };
        let Some(parsed) = parse_pg_index_definition(definition) else {
            return false;
        };
        let actual_columns_norm = norm_sql_fragment(&parsed.columns);
        let expected_columns_norm = norm_sql_fragment(expected_expression_value);
        if actual_columns_norm != expected_columns_norm {
            return false;
        }
        let actual_predicate = parsed.predicate.as_deref().map(norm_sql_fragment);
        let expected_predicate_value = expected_predicate.map(norm_sql_fragment);
        return actual_predicate == expected_predicate_value;
    }
    // 普通索引：实际带谓词定义即属定义差异
    if actual.predicate.is_some() {
        return false;
    }
    compare_plain_columns(expected_columns, actual)
}

/// 普通索引列序列比对：顺序敏感 + 降序标志。实际侧有定义文本时用文本归一
/// 比对（能识别 `col DESC` 等），否则用结构化列。
fn compare_plain_columns(expected: &[(String, bool)], actual: &ActualIndexFacts) -> bool {
    if let Some(definition) = &actual.definition {
        if let Some(parsed) = parse_pg_index_definition(definition) {
            // 文本归一比对：`a, b DESC` 形态
            let actual_norm = norm_sql_fragment(&parsed.columns);
            let expected_text = expected
                .iter()
                .map(|(name, descending)| {
                    if *descending {
                        format!("{name} DESC")
                    } else {
                        name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            return actual_norm == norm_sql_fragment(&expected_text);
        }
    }
    actual.plain_key().1 == expected.to_vec()
}

// ---------------------------------------------------------------------------
// CHECK 约束闭环（规格书 §4.3）
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CHECK 约束闭环（规格书 §4.3）
// ---------------------------------------------------------------------------

/// 解析索引定义中的列清单文本为 (列名, 是否降序) 列表（顶层逗号分割，
/// 括号感知；标识符引号与空白归一）。表达式索引的精确比对走定义全文，
/// 本函数仅提供普通列场景的结构化视图。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
pub(crate) fn parse_plain_index_column_list(columns: &str) -> Vec<(String, bool)> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in columns.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
        .iter()
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            let upper = trimmed.to_ascii_uppercase();
            let (name, descending) = if upper.ends_with(" DESC") {
                (trimmed[..trimmed.len() - 5].trim(), true)
            } else if upper.ends_with(" ASC") {
                (trimmed[..trimmed.len() - 4].trim(), false)
            } else {
                (trimmed, false)
            };
            Some((norm_sql_fragment(name), descending))
        })
        .collect()
}

/// 模型声明的一个 CHECK 约束（diff 的期望集合元素）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpectedCheck {
    /// 约束名（显式声明或 ormer 自动命名 `ck_{table}_{column}`）。
    pub name: String,
    /// 约束表达式（模型声明原文，如 `cardinality(roles) > 0`）。
    pub expr: String,
}

/// 自省得到的一个存量 CHECK 约束（PG：conname + pg_get_constraintdef）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActualCheckFacts {
    pub name: String,
    /// pg_get_constraintdef 输出（如 `CHECK ((cardinality((roles)::text[])) > 0)`）。
    pub definition: String,
}

/// CHECK diff（纯函数）：产出 Drop+Add / Add 步骤。
///
/// - 同名且归一化表达式一致 → 跳过；
/// - 存量中存在归一化表达式相同但名字不同的约束 → 视为同一约束，跳过
///   （不重复加；名字归一交由唯一化原则：ormer 不新增按名操作入口，存量
///   异名约束保守不动）；
/// - 同名但表达式不一致 → Drop 旧 + Add 新（定义演进）；
/// - 完全缺失 → Add。
///
/// 模型未声明的存量 CHECK 一律不动（保守）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
pub(crate) fn plan_check_diff(
    table_name: &str,
    expected: &[ExpectedCheck],
    actual: &[ActualCheckFacts],
    db_type: DbType,
) -> Vec<MigrationStep> {
    let mut steps = Vec::new();
    for check in expected {
        let expected_norm = normalize_check_expression(&check.expr);
        let same_name = actual
            .iter()
            .find(|candidate| candidate.name == check.name);
        match same_name {
            Some(existing) => {
                let existing_expr = extract_check_expr(&existing.definition);
                if normalize_check_expression(&existing_expr) == expected_norm {
                    continue;
                }
                // 定义漂移：Drop + Add（先删后建保证定义可演进）
                steps.push(drop_check_constraint_step(db_type, table_name, &check.name));
                steps.push(add_check_constraint_step(db_type, table_name, check));
            }
            None => {
                // 同表达式异名的存量约束视为同一约束
                let semantic_match = actual.iter().any(|candidate| {
                    normalize_check_expression(&extract_check_expr(&candidate.definition))
                        == expected_norm
                });
                if semantic_match {
                    continue;
                }
                steps.push(add_check_constraint_step(db_type, table_name, check));
            }
        }
    }
    steps
}

/// 从 pg_get_constraintdef 输出中取出 CHECK 括号内表达式。
/// `CHECK ((expr))` → `(expr)`（外层归一交给 normalize_check_expression）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn extract_check_expr(definition: &str) -> String {
    let trimmed = definition.trim();
    let body = trimmed
        .strip_prefix("CHECK")
        .map_or(trimmed, |rest| rest.trim());
    body.to_string()
}

#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn drop_check_constraint_step(db_type: DbType, table_name: &str, constraint: &str) -> MigrationStep {
    MigrationStep::Sql {
        sql: format!(
            "ALTER TABLE {} DROP CONSTRAINT {}",
            crate::model::quote_qualified_identifier(db_type, table_name),
            crate::model::quote_identifier(db_type, constraint)
        ),
    }
}

#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn add_check_constraint_step(
    db_type: DbType,
    table_name: &str,
    check: &ExpectedCheck,
) -> MigrationStep {
    MigrationStep::AddConstraint {
        table: table_name.to_string(),
        definition: format!(
            "CONSTRAINT {} CHECK ({})",
            crate::model::quote_identifier(db_type, &check.name),
            check.expr
        ),
    }
}

/// ormer 自动 CHECK 约束名：`ck_{schema.表.列 点换下划线}_{列}`。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
pub(crate) fn default_check_name(table_name: &str, column: &str) -> String {
    format!(
        "ck_{}_{}",
        table_name.replace('.', "_"),
        column
    )
}

// ---------------------------------------------------------------------------
// Database API：migrate_table 与内部编排（plan_table 为内部诊断路径）
// ---------------------------------------------------------------------------

/// 把 ormer 渲染的 CREATE [UNIQUE] INDEX 语句改写为 CONCURRENTLY 形态
/// （PG 专用；ormer 自渲染的受控文本，前缀改写安全）。
pub(crate) fn to_concurrent_create_index(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    if let Some(offset) = upper.find("CREATE UNIQUE INDEX") {
        let mut out = sql.to_string();
        out.insert_str(offset + "CREATE UNIQUE INDEX".len(), " CONCURRENTLY");
        out
    } else if let Some(offset) = upper.find("CREATE INDEX") {
        let mut out = sql.to_string();
        out.insert_str(offset + "CREATE INDEX".len(), " CONCURRENTLY");
        out
    } else {
        sql.to_string()
    }
}

/// 判断步骤是否为建索引步骤（CreateIndex 变体或 Sql 形态的 CREATE INDEX）。
fn is_index_creation_step(step: &MigrationStep) -> bool {
    match step {
        MigrationStep::CreateIndex { .. } => true,
        MigrationStep::Sql { sql } => {
            let upper = sql.trim_start().to_ascii_uppercase();
            upper.starts_with("CREATE UNIQUE INDEX") || upper.starts_with("CREATE INDEX")
        }
        _ => false,
    }
}

/// 建索引步骤的日志标签：CreateIndex 变体取声明的索引名；Sql 形态（带
/// 顺序/谓词的特殊索引）从 ormer 自渲染的 `CREATE [UNIQUE] INDEX
/// [CONCURRENTLY] [IF NOT EXISTS] <name>` 受控文本前缀中提取索引名。
fn index_step_label(step: &MigrationStep) -> String {
    match step {
        MigrationStep::CreateIndex { name, .. } => name.clone(),
        MigrationStep::Sql { sql } => {
            let upper = sql.to_ascii_uppercase();
            let Some(offset) = upper.find(" INDEX") else {
                return String::new();
            };
            let mut rest = sql[offset + " INDEX".len()..].trim_start();
            for keyword in ["CONCURRENTLY ", "IF ", "NOT ", "EXISTS "] {
                if rest.len() >= keyword.len()
                    && rest[..keyword.len()].eq_ignore_ascii_case(keyword)
                {
                    rest = rest[keyword.len()..].trim_start();
                }
            }
            let name = match rest.strip_prefix('"') {
                Some(quoted) => quoted.split('"').next().unwrap_or_default().to_string(),
                None => rest
                    .split([' ', '(', '\t', '\n', '\r'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            };
            if name.is_empty() {
                String::new()
            } else {
                name
            }
        }
        _ => String::new(),
    }
}

/// 校验标识符是合法"纯标识符"（扩展名/库名等不支持参数绑定的拼接前防御）。
/// 供 ensure_extensions 与数据库级管理 API（database_exists 等）共用。
pub(crate) fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
        && !name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
}

/// 从模型自动推导迁移所需的库级扩展，与建表路径（
/// [`crate::migration::create_table_bootstrap_steps`] 的 EXTENSION 前置步骤）
/// 同源：PG 且声明了 hypertable 的模型需要 timescaledb；非 PG 或无超表
/// 声明返回空。
#[allow(unused_variables)]
fn model_required_extensions<T: WritableModel>(db_type: DbType) -> Vec<String> {
    #[cfg(feature = "postgresql")]
    {
        if matches!(db_type, DbType::PostgreSQL) && T::hypertable_info().is_some() {
            return vec!["timescaledb".to_string()];
        }
    }
    #[cfg(not(feature = "postgresql"))]
    let _ = db_type;
    Vec::new()
}

impl Database {
    /// 诊断（内部路径）：一次性内省这张表（含 PG 路由子表）的全部现状
    /// （schema/列/主键/索引/约束/触发器），与模型比对。
    ///
    /// 内部诊断说明：ormer 不提供任何细粒度内省 API，本方法是诊断与执行
    /// 同源的内部载体——[`Database::migrate_table`] 内部就是重新调用同一套
    /// 诊断（`diagnose_family_merged`）后按计划执行（含执行后的复验）。
    /// 保留为 crate 内诊断入口供内部排查复用。
    #[allow(dead_code)]
    pub(crate) async fn plan_table<T: WritableModel>(&self) -> crate::Result<TableDiagnosis> {
        self.diagnose_family_merged::<T>().await
    }

    /// 校验：表结构与模型是否完全一致（只读，不执行任何 DDL）。
    ///
    /// 一致返回 `Ok(())`；有任何差异（表不存在、列/索引/约束/默认值漂移、
    /// 多余列、主键演进等）返回 `Err`，错误信息说明差异性质（表缺失 /
    /// 结构漂移 / 保数据推不动）。供"表存在即校验，不一致再
    /// [`Database::migrate_table`]，迁移失败再删表重建"的调用方决策链使用；
    /// 一步到位的建表/迁移请直接调 [`Database::migrate_table`]。
    ///
    /// 口径与执行同源：内部走与 migrate_table 相同的内省与期望集构造
    /// （严格视图——模型已删、库里还在的多余列也算差异）。
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        let db_type = self.db_type();
        if !crate::abstract_layer::capabilities::Capabilities::of(db_type).schema_introspection {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "validate_table",
            });
        }
        let diagnosis = self.diagnose_family_merged::<T>().await?;
        let table_name = T::table_name_for_db(db_type);
        match diagnosis {
            TableDiagnosis::Ready => Ok(()),
            TableDiagnosis::Migratable(plan) => {
                let missing = plan.steps().iter().any(|step| {
                    matches!(step, MigrationStep::CreateTable { table, .. } if table == table_name)
                });
                if missing {
                    Err(crate::ormer_error!(
                        "validate_table: table {table_name} does not exist \
                         (create it with migrate_table)"
                    ))
                } else {
                    Err(crate::ormer_error!(
                        "validate_table: schema mismatch on table {table_name}: {} pending \
                         step(s) to reconcile with the model (run migrate_table)",
                        plan.steps().len()
                    ))
                }
            }
            TableDiagnosis::NeedsRebuild(cause) => Err(crate::OrmerError::unmigratable_schema(
                cause.table, cause.reason,
            )),
        }
    }

    /// 执行：把库弄到与模型一致。内部重新诊断后按计划执行（天然幂等，
    /// 适配启动重试循环）。
    ///
    /// 覆盖：扩展确保（从模型推导，如超表声明的 timescaledb）→ schema
    /// 隐式创建 → 建表（表不存在直接建全）→ 补列 → 主键原地变更（PG）→
    /// 列类型/默认值对齐 → 多余列删除 → 索引语义 diff（补缺/清漂移/特殊
    /// 索引纳入，PG 非 QuestDB 并发建索引+INVALID 自愈）→ CHECK 约束闭环
    /// （PG）→ 外键对齐 → 普通表→超表原地转换（create_hypertable
    /// migrate_data，存量数据随迁、原表触发器不随迁）。键、列及配置与
    /// 模型完全一致：推不动的差异一律直接返回
    /// [`crate::OrmerError::unmigratable_schema`]，不做任何丢数据的重建。
    /// 两个自动兜底例外（均为零/保数据损失）：
    /// - 空表兜底——表存在但（连同路由子表）没有任何行时，删表重建是零
    ///   数据损失的自愈（如超表时间维列与模型不一致的空壳表），自动重建
    ///   后重跑一轮迁移（见 [`Database::rebuild_empty_table`]）；
    /// - 非空表保数据重建——差异属"可搬迁类"（超表分区列变更/主键换轨）
    ///   且前置检查可满足时，老表改名作备份、按新模型重建并回灌，行数
    ///   对账通过后提交（见
    ///   [`Database::rebuild_nonempty_table_preserving_data`]）。
    ///
    /// PG 路由子表与基础表走同一套诊断与执行（子表推不动同样直接报错）。
    pub async fn migrate_table<T: WritableModel>(&self) -> crate::Result<TableMigrateOutcome> {
        match self.migrate_table_once::<T>(false).await {
            Err(err) if err.is_unmigratable_schema() => {
                if self.rebuild_empty_table::<T>().await {
                    return self.migrate_table_once::<T>(false).await;
                }
                // 非空表保数据重建（"可搬迁类"NeedsRebuild 的兜底，§3.2）：
                // 成功则照 rebuild_empty_table 模式重跑一轮迁移承接阶段二
                // （模型声明索引走事务外 CONCURRENTLY，超表 0A000 自动退回
                // 普通建索引，逐索引打进度日志）；不适用（Ok(None)）维持原
                // 拒绝路径，前置检查拒绝（Err）返回附诊断 SQL 的
                // UnmigratableSchema
                match self.rebuild_nonempty_table_preserving_data::<T>().await {
                    Ok(Some(info)) => {
                        let mut outcome = self.migrate_table_once::<T>(true).await?;
                        outcome.migrated_rows = Some(info.migrated_rows);
                        outcome.backup_table = info.dropped_legacy_backup;
                        Ok(outcome)
                    }
                    Ok(None) => Err(err),
                    Err(rebuild_err) => Err(rebuild_err),
                }
            }
            result => result,
        }
    }

    /// [`Database::migrate_table`] 的单轮执行（诊断 → 增量执行 → 复验），
    /// 基础表推不动时按序尝试空表兜底与非空表保数据重建。
    ///
    /// `index_progress_log`：本计划内每建完一个索引是否打一行进度日志
    /// （索引名/耗时，§3.8 阶段二观测点）。仅保数据重建的承接轮传 true，
    /// 普通迁移轮保持静默（共用索引执行器的输出不被污染）。
    async fn migrate_table_once<T: WritableModel>(
        &self,
        index_progress_log: bool,
    ) -> crate::Result<TableMigrateOutcome> {
        let db_type = self.db_type();
        if !crate::abstract_layer::capabilities::Capabilities::of(db_type).schema_introspection {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "migrate_table",
            });
        }
        self.ensure_extensions(&model_required_extensions::<T>(db_type))
            .await?;

        let diagnosis = self.diagnose_family_merged::<T>().await?;
        let family = self.diagnose_family::<T>().await?;
        let table_name = T::table_name_for_db(db_type);

        let mut executed: Vec<MigrationStep> = Vec::new();
        let mut created_table = false;

        // 基础表
        match family.base {
            TableDiagnosis::Ready => {}
            TableDiagnosis::Migratable(plan) => {
                created_table |= plan.steps().iter().any(|step| {
                    matches!(step, MigrationStep::CreateTable { table, .. } if table == table_name)
                });
                executed.extend(self.execute_plan_steps(&plan, index_progress_log).await?);
            }
            // 保数据推不动：直接报错，不托管删表重建（空表兜底与"可搬迁类"
            // 非空表保数据重建由外层 migrate_table 兜底承接）
            TableDiagnosis::NeedsRebuild(cause) => {
                return Err(crate::OrmerError::unmigratable_schema(
                    cause.table,
                    cause.reason,
                ));
            }
        }

        // 路由子表：与基础表同一套 diff 逻辑；推不动同样直接报错
        #[cfg(feature = "postgresql")]
        let pg = self.as_postgresql();
        for (child, child_diagnosis) in family.children {
            match child_diagnosis {
                TableDiagnosis::Ready => {}
                TableDiagnosis::Migratable(plan) => {
                    executed.extend(self.execute_plan_steps(&plan, index_progress_log).await?);
                }
                TableDiagnosis::NeedsRebuild(cause) => {
                    return Err(crate::OrmerError::unmigratable_schema(
                        cause.table,
                        cause.reason,
                    ));
                }
            }
            // schema diff 收敛后为旧版本创建的子表幂等补挂列存压缩（写入
            // 触达路径只覆盖本次进程写入过的子表，迁移必须全覆盖）；
            // DO 块内部吞掉后端不支持类错误，只会传播连接级错误。
            #[cfg(feature = "postgresql")]
            if matches!(db_type, DbType::PostgreSQL)
                && let Some(pg) = pg
            {
                pg.ensure_existing_columnstore_for_model::<T>(&child).await?;
            }
        }

        // 复验（同源：整体重新诊断）。仍有差异说明增量迁移未收敛，
        // 保数据路径已无计可施，直接报错。
        match self.diagnose_family_merged::<T>().await? {
            TableDiagnosis::Ready => Ok(TableMigrateOutcome {
                diagnosis,
                created_table,
                executed,
                migrated_rows: None,
                backup_table: None,
            }),
            TableDiagnosis::Migratable(_) => Err(crate::OrmerError::unmigratable_schema(
                table_name,
                "incremental migration did not reconcile the schema; \
                 fixing it requires dropping and recreating the table",
            )),
            TableDiagnosis::NeedsRebuild(cause) => {
                Err(crate::OrmerError::unmigratable_schema(cause.table, cause.reason))
            }
        }
    }

    /// 空表自动重建判定与执行（`migrate_table` 的 unmigratable 兜底）：
    /// 仅当**基础表自身**诊断为推不动（NeedsRebuild，如超表时间维列与
    /// 模型不一致）且基础表没有任何行时，DROP 基础表并返回 true 交由
    /// 调用方重跑一轮迁移重建；其余情形一律返回 false 维持原错误路径：
    /// - 基础表非空（保数据拒绝重建的既有语义不变）；
    /// - 推不动的是路由子表或复验不收敛（半迁移态，均不适用空表重建，
    ///   且子表不存在视为就绪，删掉会跳过自适应补挂等执行路径）；
/// - 基础表带路由子表时 DROP 父表失败，同样返回 false。
///
/// 判空用 `SELECT 1 .. LIMIT 1` 探测，不做全表计数；表名来自模型
/// 声明与库内自省，非用户输入，无注入面。
    async fn rebuild_empty_table<T: WritableModel>(&self) -> bool {
        #[cfg(feature = "postgresql")]
        {
            let Some(pg) = self.as_postgresql() else {
                return false;
            };
            let Ok(family) = self.diagnose_family::<T>().await else {
                return false;
            };
            if !matches!(family.base, TableDiagnosis::NeedsRebuild(_)) {
                return false;
            }
            let table = T::table_name_for_db(self.db_type());
            let empty = pg
                .select_raw::<i64, Vec<i64>>(
                    &format!("SELECT 1 FROM {table} LIMIT 1"),
                    Vec::new(),
                )
                .await
                .map(|rows| rows.is_empty())
                .unwrap_or(false);
            if !empty {
                return false;
            }
            pg.execute_sql(crate::raw_sql::RawSql::plain(format!(
                "DROP TABLE IF EXISTS {table}"
            )))
            .await
            .is_ok()
        }
        #[cfg(not(feature = "postgresql"))]
        {
            let _ = self;
            false
        }
    }

    /// 非空表保数据重建（方案 §3.2 两段式；[`Database::rebuild_empty_table`]
    /// 的互补路径，migrate_table 的"可搬迁类"NeedsRebuild 兜底）。
    ///
    /// 触发条件（§3.1）：NeedsRebuild 原因属于超表时间/空间分区列变更或
    /// 主键换轨（新主键不含全部分区键）；复验不收敛（半迁移态）不适用
    /// 重建，返回 `Ok(None)` 维持原拒绝语义。
    ///
    /// 前提约束：前置检查与回灌之间老表数据不可修改——该闸门由调用方
    /// （DataStorer 迁移期间阻塞全部经 get_conn() 的读写）保证，ormer 端
    /// 不加锁；事务内行数对账（count(old) == count(new)）是前置检查结论
    /// 仍然成立的最终防线，不等即 ROLLBACK，老表原名原样恢复、零残留。
    ///
    /// 返回 `Ok(None)`：不适用重建（非 PG 后端、原因不属于可搬迁类、老表
    /// 已消失等），调用方走原拒绝路径；`Ok(Some)`：阶段一已提交（备份表
    /// 已随事务删除，阶段二索引由调用方重跑一轮迁移承接）；`Err`：前置
    /// 检查拒绝或阶段一失败（事务已回滚）。
    async fn rebuild_nonempty_table_preserving_data<T: WritableModel>(
        &self,
    ) -> crate::Result<Option<PreservedDataRebuild>> {
        #[cfg(feature = "postgresql")]
        {
            let Some(pg) = self.as_postgresql() else {
                return Ok(None);
            };
            // 复诊断门禁：仅当前"基础表"诊断为 NeedsRebuild 且原因属
            // "可搬迁类"时适用（路由子表推不动、复验不收敛半迁移态等
            // 场景一律维持原拒绝语义，不做重建）
            let Ok(family) = self.diagnose_family::<T>().await else {
                return Ok(None);
            };
            let TableDiagnosis::NeedsRebuild(cause) = family.base else {
                return Ok(None);
            };
            if !is_rebuild_eligible_cause(&cause.reason) {
                return Ok(None);
            }
            let db_type = self.db_type();
            let table = T::table_name_for_db(db_type);
            let (schema, bare) = crate::model::split_schema_table_name(table, "public");
            let qualified = crate::model::quote_qualified_identifier(db_type, table);
            // 备份表名与索引名同受 NAMEDATALEN 63 字节截断：套用既有截断规则
            let backup_bare = postgres_truncated_identifier(&format!("{bare}_migrate_old"));
            let backup_qualified = format!(
                "{}.{}",
                crate::model::quote_identifier(db_type, schema),
                crate::model::quote_identifier(db_type, &backup_bare)
            );

            // 前置检查 a：遗留备份表存在即 DROP（现行流程备份表不跨事务存续，
            // 残留只可能来自历史版本或异常现场，与本轮迁移数据无关，发现即删）
            let dropped_legacy_backup =
                if pg.schema_columns(&backup_qualified).await?.is_some() {
                    eprintln!(
                        "[ormer] migrate_table: {qualified}: dropping legacy backup table \
                         {backup_qualified} before data-preserving rebuild"
                    );
                    self.execute_sql(crate::raw_sql::RawSql::plain(format!(
                        "DROP TABLE IF EXISTS {backup_qualified}"
                    )))
                    .await?;
                    Some(backup_qualified.clone())
                } else {
                    None
                };

            // 老表自省列（诊断刚确认过存在；消失属并发干预，交由外层重试）
            let Some(old_columns) = pg.schema_columns(&qualified).await? else {
                return Ok(None);
            };

            // 列回灌映射（§3.3；前置检查 c 在映射产出后于 b 之后统一拒绝）
            let mapping =
                crate::migration::build_rebuild_column_mapping::<T>(db_type, &old_columns);

            // 新主键列的回灌来源表达式（§3.4：预检分组键与回灌映射表达式
            // 同构，保证冲突/NULL 检测与回灌逐行等价）。主键集合以建表 DDL
            // 实际写出的有效主键为准：声明主键 + PG 超表自动补齐的空间分区列
            let effective_primary_keys =
                crate::model::effective_primary_key_columns::<T>(db_type);
            let primary_key_sources: Vec<&crate::migration::RebuildColumnEntry> =
                effective_primary_keys
                    .iter()
                    .filter_map(|name| mapping.entries.iter().find(|entry| entry.column == *name))
                    .collect();

            // 前置检查 b-1：新主键列 NULL 预检（回灌 INSERT 会因 NOT NULL 失败）
            for entry in &primary_key_sources {
                if pg
                    .expression_has_null(schema, bare, &entry.expression)
                    .await?
                {
                    return Err(crate::OrmerError::unmigratable_schema(
                        table,
                        format!(
                            "data-preserving rebuild precondition failed: new primary key \
                             column {} (sourced as {}) contains NULL rows in the old table; \
                             detection SQL: SELECT 1 FROM {qualified} WHERE {} IS NULL LIMIT 1; \
                             resolve NULLs and retry",
                            entry.column, entry.expression, entry.expression
                        ),
                    ));
                }
            }

            // 前置检查 b-2：新主键冲突自动去重（拍板 2026-10-14：不再拒绝
            // 迁移等人工去重——对齐运行期 insert 的 ON CONFLICT DO NOTHING，
            // 每组保留最早写入的一行（(tableoid, ctid) 最小），其余重复行
            // 直接丢弃）。检测在这里先行（提示日志）；真正的 DELETE 在阶段一
            // 事务内、LOCK TABLE 与 RENAME 之间执行——去重若留在事务外，外部
            // 写者（2026-10-14 jxgc 现场：data_sync 回放）可在"去重完成后到
            // RENAME 拿锁前"的窗口塞入与既有行同键的新行，回灌必撞唯一约束
            // 而无限重试；锁内去重保证检测结论对回灌始终成立。
            let group_expressions: Vec<String> = primary_key_sources
                .iter()
                .map(|entry| entry.expression.clone())
                .collect();
            let samples = pg
                .duplicate_key_samples(schema, bare, &group_expressions, 1)
                .await?;
            if !samples.is_empty() {
                eprintln!(
                    "[ormer] migrate_table: {qualified}: new primary key duplicates \
                     detected; they will be deduplicated inside the rebuild transaction \
                     (keeping the earliest row per key, same semantics as runtime insert \
                     ON CONFLICT DO NOTHING)"
                );
            }

            // 前置检查 c：非空新列填充来源检查
            if !mapping.unsourced_required.is_empty() {
                return Err(crate::OrmerError::unmigratable_schema(
                    table,
                    format!(
                        "data-preserving rebuild precondition failed: required column(s) [{}] \
                         are absent from the old table and declare no default, so no value \
                         can be derived; backfill the old table or add defaults to the model",
                        mapping.unsourced_required.join(", ")
                    ),
                ));
            }

            // 表级开始日志（拍板 E：表名、老表总行数）
            let total = pg.table_row_count(schema, bare).await?;
            let started = std::time::Instant::now();
            eprintln!(
                "[ormer] migrate_table: {qualified}: data-preserving rebuild started, \
                 {total} row(s) to carry over"
            );

            // 阶段一（单事务，原子）
            let mut transaction = self.begin().await?;
            // 首语句锁表：EXCLUSIVE 级挡住外部 INSERT/UPDATE/DELETE（只读
            // SELECT 不受影响），持有至 COMMIT——覆盖去重 → RENAME → 回灌 →
            // 对账全程，封死"上一轮 ROLLBACK 后到本轮 RENAME 拿锁前"的外部
            // 写入窗口；锁外排队的外部写在 COMMIT 后落到新表，行数对账不受
            // 干扰，数据零丢失。
            transaction
                .execute_sql(crate::raw_sql::RawSql::plain(format!(
                    "LOCK TABLE {qualified} IN EXCLUSIVE MODE"
                )))
                .await?;
            // 事务内去重（语义见前置检查 b-2 注释；ROLLBACK 时去重一并撤销，
            // 老表在原重复状态下原样恢复，重试轮重新检测）。先 GROUP BY 探测
            // （单遍 O(n)，无重复的干净大表避免白吃一次删除 join）；探测命中
            // 才删除，删除用 row_number 窗口函数单遍定序——每组保留
            // (tableoid, ctid) 最小（最早写入）行，PARTITION BY 分组与
            // IS NOT DISTINCT FROM 等价（NULL 同组），执行计划不依赖统计信息。
            let group_list = group_expressions.join(", ");
            let has_duplicates = !transaction
                .select_sql::<i64>(crate::raw_sql::RawSql::plain(format!(
                    "SELECT 1 FROM {qualified} GROUP BY {group_list} \
                     HAVING count(*) > 1 LIMIT 1"
                )))
                .collect::<Vec<i64>>()
                .await?
                .is_empty();
            let removed = if has_duplicates {
                // (tableoid, ctid) 双列定位：分区表上裸 ctid 会被谓词下推在
                // 每个 chunk 内各自匹配（各 chunk 首行同为 (0,1)），跨 chunk
                // 误删保留行；tableoid 限定 chunk 后 ctid 才是行精确坐标。
                transaction
                    .execute_sql(crate::raw_sql::RawSql::plain(format!(
                        "DELETE FROM {qualified} a WHERE (a.tableoid, a.ctid) IN (\
                         SELECT s.tableoid, s.ctid FROM (\
                           SELECT tableoid, ctid, row_number() OVER (\
                             PARTITION BY {group_list} ORDER BY tableoid, ctid) AS rn \
                           FROM {qualified}) s WHERE s.rn > 1)"
                    )))
                    .await?
            } else {
                0
            };
            if removed > 0 {
                eprintln!(
                    "[ormer] migrate_table: {qualified}: new primary key duplicates \
                     auto-deduplicated inside the rebuild transaction, kept the earliest \
                     row per key and dropped the rest (same semantics as runtime insert \
                     ON CONFLICT DO NOTHING), {removed} row(s) removed"
                );
            }
            match Self::copy_table_preserving_data::<T>(
                &mut transaction,
                db_type,
                &qualified,
                &backup_qualified,
                &backup_bare,
                &mapping,
                total,
            )
            .await
            {
                Ok(migrated_rows) => {
                    transaction.commit().await?;
                    eprintln!(
                        "[ormer] migrate_table: {qualified}: data-preserving rebuild finished, \
                         {migrated_rows} row(s) in {:?}; index build follows via re-run",
                        started.elapsed()
                    );
                    Ok(Some(PreservedDataRebuild {
                        migrated_rows,
                        dropped_legacy_backup,
                    }))
                }
                Err(error) => {
                    // 阶段一失败 ROLLBACK = 老表原名原样恢复，零残留
                    let _ = transaction.rollback().await;
                    Err(error)
                }
            }
        }
        #[cfg(not(feature = "postgresql"))]
        {
            Ok(None)
        }
    }

    /// 阶段一主体（须在单事务内调用）：RENAME 老表为备份 → 按新模型建新表
    /// （跳过建表 DDL 内联的索引语句，回灌后由阶段二 CONCURRENTLY 补建）→
    /// INSERT INTO 新表(显式列清单) SELECT 映射表达式 FROM 备份表 按业务时间
    /// 列水位分批（每批 [`REBUILD_COPY_BATCH_SIZE`] 行，批内打进度日志）→
    /// 事务内行数对账 → 校验通过即 DROP 备份表（拍板 B）。任一步失败由
    /// 调用方 ROLLBACK。
    /// 事务内执行一个迁移步骤（含 SQL 语句拆分；阶段一 bootstrap 步骤用）。
    #[cfg(feature = "postgresql")]
    async fn execute_step_statements(
        transaction: &mut Transaction<'_>,
        db_type: DbType,
        step: &MigrationStep,
    ) -> crate::Result<()> {
        let sql = step.sql(db_type)?;
        for statement in crate::migration::split_sql_statements(&sql) {
            transaction
                .execute_sql(crate::raw_sql::RawSql::plain(statement))
                .await?;
        }
        Ok(())
    }

    #[cfg(feature = "postgresql")]
    async fn copy_table_preserving_data<T: WritableModel>(
        transaction: &mut Transaction<'_>,
        db_type: DbType,
        qualified_table: &str,
        backup_qualified: &str,
        backup_bare: &str,
        mapping: &crate::migration::RebuildColumnMapping,
        expected_rows: i64,
    ) -> crate::Result<i64> {
        // 老表整体改名作备份（超表 chunk 数据随行）
        transaction
            .execute_sql(crate::raw_sql::RawSql::plain(format!(
                "ALTER TABLE {qualified_table} RENAME TO {}",
                crate::model::quote_identifier(db_type, backup_bare)
            )))
            .await?;

        // 事务内老表行数（对账基准；前置检查与本事务之间由调用方闸门保证
        // 无写入，此处复核）
        let old_rows = transaction
            .select_sql::<i64>(crate::raw_sql::RawSql::plain(format!(
                "SELECT count(*) FROM {backup_qualified}"
            )))
            .collect::<Vec<i64>>()
            .await?
            .into_iter()
            .next()
            .unwrap_or(0);

        // 按新模型建新表：前置（枚举/扩展）→ CREATE TABLE → 后置
        // （create_hypertable / add_dimension，create_default_indexes => FALSE
        // 由 bootstrap 生成），DDL 序列与 fresh-table bootstrap 路径一致。
        // 建表 DDL 内联的索引语句跳过，避免 insert 前维护二级索引。
        let (pre_steps, post_steps) =
            crate::migration::create_table_bootstrap_steps::<T>(db_type, qualified_table);
        for step in pre_steps.iter() {
            Self::execute_step_statements(transaction, db_type, step).await?;
        }
        let create_definition = crate::generate_create_table_sql::<T>(db_type)?;
        for statement in crate::migration::split_sql_statements(&create_definition) {
            if is_inline_index_statement(&statement) {
                continue;
            }
            transaction
                .execute_sql(crate::raw_sql::RawSql::plain(statement))
                .await?;
        }
        for step in post_steps.iter() {
            Self::execute_step_statements(transaction, db_type, step).await?;
        }

        // 回灌：显式列清单防列序漂移（§3.2）
        let insert_columns = mapping
            .entries
            .iter()
            .map(|entry| crate::model::quote_identifier(db_type, &entry.column))
            .collect::<Vec<_>>()
            .join(", ");
        let select_expressions = mapping
            .entries
            .iter()
            .map(|entry| entry.expression.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_prefix = format!(
            "INSERT INTO {qualified_table} ({insert_columns}) \
             SELECT {select_expressions} FROM {backup_qualified}"
        );

        // 分批水位（拍板 E）：按业务时间列推进；时间列的回灌来源必须是
        // 老表真实列，否则（时间列为合成列）退化为单批全量回灌。
        let time_expression = T::hypertable_info()
            .and_then(|(time_column, _)| {
                mapping
                    .entries
                    .iter()
                    .find(|entry| entry.column == time_column && entry.from_old_table)
            })
            .map(|entry| entry.expression.clone());

        let mut migrated: i64 = 0;
        match time_expression {
            Some(time_expression) => {
                let mut watermark: Option<Value> = None;
                loop {
                    // 批边界：自水位起第 REBUILD_COPY_BATCH_SIZE 行的时间值
                    //（同事务快照）
                    let boundary_sql = match &watermark {
                        Some(_) => format!(
                            "SELECT {time_expression} FROM {backup_qualified} \
                             WHERE {time_expression} >= {{wm}} \
                             ORDER BY {time_expression} \
                             OFFSET {} LIMIT 1",
                            REBUILD_COPY_BATCH_SIZE - 1
                        ),
                        None => format!(
                            "SELECT {time_expression} FROM {backup_qualified} \
                             ORDER BY {time_expression} \
                             OFFSET {} LIMIT 1",
                            REBUILD_COPY_BATCH_SIZE - 1
                        ),
                    };
                    let mut boundary_query = crate::raw_sql::RawSql::new(boundary_sql);
                    if let Some(value) = &watermark {
                        boundary_query = boundary_query.bind_named("wm", value.clone());
                    }
                    let boundary = transaction
                        .select_sql::<Option<Value>>(boundary_query)
                        .collect::<Vec<Option<Value>>>()
                        .await?
                        .into_iter()
                        .flatten()
                        .next();

                    // 边界为空（剩余不足一批）或与水位相等（剩余行同一时刻）
                    // → 一次性收尾；否则按 [水位, 边界) 半开区间回灌并推进
                    // 水位，行不重不漏。
                    let final_batch = match &boundary {
                        None => true,
                        Some(value) => Some(value) == watermark.as_ref(),
                    };
                    let inserted = if final_batch {
                        let sql = match &watermark {
                            Some(_) => {
                                format!("{insert_prefix} WHERE {time_expression} >= {{wm}}")
                            }
                            None => insert_prefix.clone(),
                        };
                        let mut statement = crate::raw_sql::RawSql::new(sql);
                        if let Some(value) = &watermark {
                            statement = statement.bind_named("wm", value.clone());
                        }
                        transaction.execute_sql(statement).await?
                    } else {
                        let bound = boundary.as_ref().expect("boundary checked above");
                        let sql = match &watermark {
                            Some(_) => format!(
                                "{insert_prefix} WHERE {time_expression} >= {{wm}} \
                                 AND {time_expression} < {{bound}}"
                            ),
                            None => {
                                format!("{insert_prefix} WHERE {time_expression} < {{bound}}")
                            }
                        };
                        let mut statement = crate::raw_sql::RawSql::new(sql);
                        if let Some(value) = &watermark {
                            statement = statement.bind_named("wm", value.clone());
                        }
                        statement = statement.bind_named("bound", bound.clone());
                        transaction.execute_sql(statement).await?
                    };
                    migrated += inserted as i64;
                    if old_rows > 0 {
                        eprintln!(
                            "[ormer] migrate_table: {qualified_table}: {migrated}/{old_rows} \
                             row(s) migrated ({:.1}%)",
                            (migrated as f64 / old_rows as f64) * 100.0
                        );
                    }
                    if final_batch {
                        break;
                    }
                    watermark = boundary;
                }
            }
            None => {
                migrated += transaction
                    .execute_sql(crate::raw_sql::RawSql::plain(insert_prefix))
                    .await? as i64;
                eprintln!(
                    "[ormer] migrate_table: {qualified_table}: {migrated}/{old_rows} row(s) \
                     migrated (single batch)"
                );
            }
        }

        // 行数对账（最终防线）：count(old) == count(new)，不等 ROLLBACK
        let new_rows = transaction
            .select_sql::<i64>(crate::raw_sql::RawSql::plain(format!(
                "SELECT count(*) FROM {qualified_table}"
            )))
            .collect::<Vec<i64>>()
            .await?
            .into_iter()
            .next()
            .unwrap_or(0);
        if new_rows != old_rows {
            return Err(crate::OrmerError::unmigratable_schema(
                qualified_table,
                format!(
                    "data-preserving rebuild aborted: row count mismatch after copy \
                     (old table {old_rows} row(s), new table {new_rows} row(s), model \
                     expectation {expected_rows} row(s)); transaction rolled back and the \
                     old table is restored unchanged"
                ),
            ));
        }

        // 校验通过即删备份（拍板 B）：备份表仅存续于本事务，COMMIT 后无残留
        transaction
            .execute_sql(crate::raw_sql::RawSql::plain(format!(
                "DROP TABLE {backup_qualified}"
            )))
            .await?;
        Ok(migrated)
    }

    /// 扩展确保：逐个幂等 `CREATE EXTENSION IF NOT EXISTS`。仅 PostgreSQL
    /// 支持扩展；其他后端传入非空列表返回 UnsupportedFeature（迁移路径的
    /// 列表由 [`model_required_extensions`] 从模型推导，非 PG 后端恒为空，
    /// 不会触达该错误）。
    pub(crate) async fn ensure_extensions(&self, extensions: &[String]) -> crate::Result<()> {
        if extensions.is_empty() {
            return Ok(());
        }
        let db_type = self.db_type();
        for name in extensions {
            if !is_safe_identifier(name) {
                return Err(crate::ormer_error!(
                    "invalid extension name {name:?}: must be a plain identifier"
                ));
            }
        }
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                for name in extensions {
                    db.ensure_extension(name).await?;
                }
                return Ok(());
            }
        }
        let _ = db_type;
        Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "model-required extensions (CREATE EXTENSION is PostgreSQL-only)",
        })
    }

    /// 连接层探活：确认数据库连接可用。不可达/鉴权失败返回 Err。
    pub async fn ping(&self) -> crate::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Database::Sqlite(db) => db.ping().await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => db.ping().await,
            #[cfg(feature = "mysql")]
            Database::MySQL(db) => db.ping().await,
            #[cfg(feature = "mssql")]
            Database::MSSQL(db) => db.ping().await,
            #[cfg(feature = "duckdb")]
            Database::DuckDB(db) => db.ping().await,
            #[cfg(feature = "clickhouse")]
            Database::ClickHouse(db) => db.ping().await,
            #[cfg(feature = "influxdb")]
            Database::InfluxDB(db) => db.ping().await,
        }
    }

    /// 数据库级管理 API 的标识符前置校验（与 ensure_extensions 同规则：
    /// 这些 DDL 不支持参数绑定，拼接前防御）。
    fn validate_admin_identifier(name: &str, what: &str) -> crate::Result<()> {
        if !is_safe_identifier(name) {
            return Err(crate::ormer_error!(
                "invalid {what} {name:?}: must be a plain identifier"
            ));
        }
        Ok(())
    }

    /// 查询库是否存在（内部查 pg_database，绑定参数防注入）。非 PG 后端返回
    /// [`crate::OrmerError::UnsupportedFeature`]。
    pub async fn database_exists(&self, name: &str) -> crate::Result<bool> {
        Self::validate_admin_identifier(name, "database name")?;
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                return db.database_exists(name).await;
            }
        }
        #[cfg(not(feature = "postgresql"))]
        let _ = name;
        Err(crate::OrmerError::UnsupportedFeature {
            backend: self.db_type(),
            feature: "database_exists (database-level administration is PostgreSQL-only)",
        })
    }

    /// 建库（只要不存在即新建；结束时目标库存在即返回 Ok(())，已存在时同样
    /// 成功）。标识符由 ormer 内部校验并加引号。PG 的 CREATE DATABASE 不能
    /// 进事务块，本 API 只挂 Database（Transaction 不提供）。
    pub async fn create_database(&self, name: &str) -> crate::Result<()> {
        Self::validate_admin_identifier(name, "database name")?;
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                return db.create_database(name).await;
            }
        }
        #[cfg(not(feature = "postgresql"))]
        let _ = name;
        Err(crate::OrmerError::UnsupportedFeature {
            backend: self.db_type(),
            feature: "create_database (database-level administration is PostgreSQL-only)",
        })
    }

    /// 删库。始终 WITH (FORCE) 先终止该库全部连接；库不存在视为已删（幂等）。
    /// 非 PG 后端返回 [`crate::OrmerError::UnsupportedFeature`]。
    pub async fn drop_database(&self, name: &str) -> crate::Result<()> {
        Self::validate_admin_identifier(name, "database name")?;
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                return db.drop_database(name).await;
            }
        }
        #[cfg(not(feature = "postgresql"))]
        let _ = name;
        Err(crate::OrmerError::UnsupportedFeature {
            backend: self.db_type(),
            feature: "drop_database (database-level administration is PostgreSQL-only)",
        })
    }

    /// 在指定库上应用扩展（幂等，已应用则跳过）。目标库由参数点名，与连接串
    /// 挂的库分离。PG 的 CREATE EXTENSION 只作用于连接所在库，内部以自身
    /// 端点/凭据另开一条到 `database` 的连接执行、用完即关；目标库不存在按
    /// 连接失败报错。仅直连（`Database::connect`）支持：池化连接（bb8）取不到
    /// 连接配置，返回错误。非 PG 后端返回
    /// [`crate::OrmerError::UnsupportedFeature`]。
    pub async fn apply_extension(&self, database: &str, name: &str) -> crate::Result<()> {
        Self::validate_admin_identifier(database, "database name")?;
        Self::validate_admin_identifier(name, "extension name")?;
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                return db.apply_extension(database, name).await;
            }
        }
        #[cfg(not(feature = "postgresql"))]
        let _ = (database, name);
        Err(crate::OrmerError::UnsupportedFeature {
            backend: self.db_type(),
            feature: "apply_extension (database-level administration is PostgreSQL-only)",
        })
    }

    /// 诊断单张表（基础表或某个路由子表）。`table_name` 为 None 时取模型
    /// 基础表名（含超表元数据校验）；Some 时为子表名（不重复做超表校验）。
    pub(crate) async fn diagnose_single_table<T: WritableModel>(
        &self,
        table_name: Option<&str>,
    ) -> crate::Result<TableDiagnosis> {
        let db_type = self.db_type();
        let base_name = T::table_name_for_db(db_type);
        let target = table_name.unwrap_or(base_name);
        let is_base = table_name.is_none();
        let mut plan = MigrationPlan::new(target, db_type);

        let exists = self.schema_columns(target).await?.is_some();
        if !exists {
            if !is_base {
                // 路由子表不预建：不存在时视为就绪（写入路径按当前 DDL 自动建）。
                return Ok(TableDiagnosis::Ready);
            }
            // 建表配套步骤对齐 CreateTableExecutor 的 DDL 序列：
            // 枚举/EXTENSION 前置，create_hypertable 后置（缺了前者建表
            // 直接报缺类型，缺了后者建出的是普通表，下一轮诊断会生成
            // 原地转换步骤补救而非一次建到位）。
            let (pre, post) =
                crate::migration::create_table_bootstrap_steps::<T>(db_type, target);
            for step in pre {
                plan.push(step);
            }
            plan.push(MigrationStep::CreateTable {
                table: target.to_string(),
                definition: crate::generate_create_table_sql::<T>(db_type)?,
            });
            for step in post {
                plan.push(step);
            }
            return Ok(TableDiagnosis::Migratable(plan));
        }

        let migration = TableMigration::<T>::for_diagnosis(self);
        match migration.plan_for_table(target).await {
            Ok(diff) => {
                if diff.is_empty() {
                    Ok(TableDiagnosis::Ready)
                } else {
                    plan.steps.extend(diff.steps);
                    plan.warnings.extend(diff.warnings);
                    Ok(TableDiagnosis::Migratable(plan))
                }
            }
            Err(err) if err.is_unmigratable_schema() || is_schema_rebuild_error(&err) => {
                Ok(TableDiagnosis::NeedsRebuild(RebuildCause::new(
                    target,
                    err.to_string(),
                )))
            }
            // QuestDB 无法 ALTER COLUMN 类型，但删表重建可以对齐：
            // 诊断层转 NeedsRebuild 而不是硬错误。
            Err(err @ crate::OrmerError::UnsupportedFeature { .. }) if db_type.is_questdb() => {
                Ok(TableDiagnosis::NeedsRebuild(RebuildCause::new(
                    target,
                    err.to_string(),
                )))
            }
            Err(err) => Err(err),
        }
    }

    /// 诊断（内部）：基础表 + 路由子表的结构化结论（migrate 按表分派执行）。
    async fn diagnose_family<T: WritableModel>(&self) -> crate::Result<FamilyDiagnosis> {
        let db_type = self.db_type();
        if !crate::abstract_layer::capabilities::Capabilities::of(db_type).schema_introspection {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "plan_table",
            });
        }
        let base = self.diagnose_single_table::<T>(None).await?;
        #[cfg(feature = "postgresql")]
        let mut children = Vec::new();
        #[cfg(not(feature = "postgresql"))]
        let children = Vec::new();
        #[cfg(feature = "postgresql")]
        {
            if matches!(db_type, DbType::PostgreSQL) && T::hypertable_route_key().is_some() {
                for child in self.existing_routed_child_tables::<T>().await? {
                    // 枚举与执行之间子表可能被并发删除：跳过而非预建
                    if self.schema_columns(&child).await?.is_none() {
                        continue;
                    }
                    let child_diagnosis = self
                        .diagnose_single_table::<T>(Some(&child))
                        .await?;
                    children.push((child, child_diagnosis));
                }
            }
        }
        Ok(FamilyDiagnosis { base, children })
    }

    /// 诊断（内部）：基础表 + 路由子表合并为整体结论（plan_table 内部路径
    /// 与复验用）。
    pub(crate) async fn diagnose_family_merged<T: WritableModel>(
        &self,
    ) -> crate::Result<TableDiagnosis> {
        let family = self.diagnose_family::<T>().await?;
        let mut merged = family.base;
        for (_, child) in family.children {
            merged = merge_diagnoses(merged, child);
        }
        Ok(merged)
    }

    /// 执行一份迁移计划并返回实际执行的步骤。
    ///
    /// - 非事务后端：逐条执行（现状语义）；
    /// - PG（非 QuestDB）：建索引步骤拆到事务外 CONCURRENTLY 执行，
    ///   执行前与失败重试前自动清理同表 INVALID 残留索引（自愈），
    ///   其余步骤保持单事务；
    /// - 其余后端：单事务整体执行。
    ///
    /// `index_progress_log`：逐索引进度日志开关（见
    /// [`Database::migrate_table_once`]），由调用轮次决定。
    #[cfg_attr(not(feature = "postgresql"), allow(unused_variables))]
    pub(crate) async fn execute_plan_steps(
        &self,
        plan: &MigrationPlan,
        index_progress_log: bool,
    ) -> crate::Result<Vec<MigrationStep>> {
        if plan.is_empty() {
            return Ok(Vec::new());
        }
        let db_type = self.db_type();
        #[cfg(any(feature = "sqlite", feature = "postgresql"))]
        let concurrent = {
            #[cfg(feature = "postgresql")]
            {
                matches!(db_type, DbType::PostgreSQL) && !db_type.is_questdb()
            }
            #[cfg(not(feature = "postgresql"))]
            {
                false
            }
        };
        #[cfg(not(any(feature = "sqlite", feature = "postgresql")))]
        let concurrent = false;
        let all: Vec<MigrationStep> = plan.steps().to_vec();

        if !db_type.is_transactional() {
            execute_steps_nontransactional(self, db_type, plan.steps()).await?;
            return Ok(all);
        }
        if !concurrent {
            let mut transaction = self.begin().await?;
            let result = execute_steps(&mut transaction, db_type, plan.steps()).await;
            match result {
                Ok(()) => transaction.commit().await?,
                Err(error) => {
                    let _ = transaction.rollback().await;
                    return Err(error);
                }
            }
            return Ok(all);
        }

        // PG 并发分段：非索引步骤在事务内，索引步骤在事务外 CONCURRENTLY
        let mut executed = Vec::new();
        let mut transaction: Option<Transaction<'_>> = Some(self.begin().await?);
        for step in &all {
            if is_index_creation_step(step) {
                if let Some(open) = transaction.take() {
                    open.commit().await?;
                }
                self.create_index_concurrently(step, db_type, index_progress_log)
                    .await?;
            } else {
                if transaction.is_none() {
                    transaction = Some(self.begin().await?);
                }
                let open = transaction
                    .as_mut()
                    .expect("transaction re-opened above");
                if let Err(error) = execute_steps(open, db_type, std::slice::from_ref(step)).await
                {
                    if let Some(open) = transaction.take() {
                        let _ = open.rollback().await;
                    }
                    return Err(error);
                }
            }
            executed.push(step.clone());
        }
        if let Some(open) = transaction.take() {
            open.commit().await?;
        }
        Ok(executed)
    }

    /// 事务外并发建索引：先清理同表 INVALID 残留，失败后再清理并重试一次。
    /// TimescaleDB hypertable 不支持 CREATE INDEX CONCURRENTLY（0A000）：
    /// 目标表是 hypertable 时退回普通建索引（事务化执行、失败即回滚，
    /// 不会留下 INVALID 残留，无需并发路径与自愈重试）。
    ///
    /// `index_progress_log` 为 true 时（保数据重建承接轮，§3.8 阶段二），
    /// 每建完一个索引用一行输出索引名与耗时。
    async fn create_index_concurrently(
        &self,
        step: &MigrationStep,
        db_type: DbType,
        index_progress_log: bool,
    ) -> crate::Result<()> {
        let started = std::time::Instant::now();
        let result = self.create_index_concurrently_inner(step, db_type).await;
        if index_progress_log && result.is_ok() {
            eprintln!(
                "[ormer] migrate_table: {}: index {} built in {:?}",
                table_name(step),
                index_step_label(step),
                started.elapsed()
            );
        }
        result
    }

    /// 索引构建主体（不含进度日志计时包装）。
    async fn create_index_concurrently_inner(
        &self,
        step: &MigrationStep,
        db_type: DbType,
    ) -> crate::Result<()> {
        let sql = step.sql(db_type)?;
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                let (schema, bare) = split_qualified_table_name(table_name(step));
                let schema = schema.unwrap_or("public");
                if db.is_hypertable_table(schema, bare).await? {
                    self.execute_sql(sql.as_str()).await?;
                    return Ok(());
                }
            }
        }
        let concurrent_sql = to_concurrent_create_index(&sql);
        if concurrent_sql == sql {
            return Err(crate::ormer_error!(
                "cannot render CREATE INDEX CONCURRENTLY for step: {sql}"
            ));
        }
        self.drop_invalid_indexes_for_table(db_type, table_name(step))
            .await?;
        match self.execute_sql(concurrent_sql.as_str()).await {
            Ok(_) => Ok(()),
            Err(first_error) => {
                // 失败残留 INVALID 索引：清理后重试一次
                self.drop_invalid_indexes_for_table(db_type, table_name(step))
                    .await?;
                match self.execute_sql(concurrent_sql.as_str()).await {
                    Ok(_) => Ok(()),
                    Err(retry_error) => Err(crate::ormer_error!(
                        "CREATE INDEX CONCURRENTLY failed after INVALID cleanup retry: {retry_error} (first attempt: {first_error})"
                    )),
                }
            }
        }
    }

    /// 清理表上所有 INVALID（indisvalid=false）残留索引。仅 PG 有此概念，
    /// 其余后端为空操作。
    pub(crate) async fn drop_invalid_indexes_for_table(
        &self,
        db_type: DbType,
        table_name: &str,
    ) -> crate::Result<()> {
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                let (schema, bare) = split_qualified_table_name(table_name);
                let schema = schema.unwrap_or("public");
                let names = db.invalid_index_names(schema, bare).await?;
                for name in names {
                    let qualified = format!(
                        "{}.{}",
                        crate::model::quote_identifier(db_type, schema),
                        crate::model::quote_identifier(db_type, &name)
                    );
                    self.execute_sql(format!("DROP INDEX IF EXISTS {qualified}").as_str())
                        .await?;
                }
            }
        }
        let _ = (db_type, table_name);
        Ok(())
    }

    /// PG 专用视图（QuestDB 连接排除）：供内部自省/运维能力分派。
    #[cfg(feature = "postgresql")]
    pub(crate) fn as_postgresql(&self) -> Option<&crate::abstract_layer::postgresql_backend::Database> {
        match self {
            Database::PostgreSQL(db) if !self.db_type().is_questdb() => Some(db),
            _ => None,
        }
    }

    /// 构造索引自省统一视图：PG 走增强内省（定义文本/方法/valid），
    /// 其余后端从既有 DbFirstIndex 结构降级构造（无定义文本）。
    pub(crate) async fn actual_index_facts(
        &self,
        table_name: &str,
        indexes: &[crate::db_first::DbFirstIndex],
    ) -> crate::Result<Vec<ActualIndexFacts>> {
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                let (schema, bare) = split_qualified_table_name(table_name);
                let schema = schema.unwrap_or("public");
                return db.index_facts(schema, bare).await;
            }
        }
        let _ = table_name;
        Ok(indexes
            .iter()
            .map(|index| {
                ActualIndexFacts::from_plain(
                    index.name.clone(),
                    index
                        .columns
                        .iter()
                        .map(|column| (column.name.clone(), column.descending))
                        .collect(),
                    index.unique,
                )
            })
            .collect())
    }

    /// 现有主键约束名（仅 PG 有可靠自省；其余后端返回 None，调用方在
    /// 非 PG 上不会生成 ChangePrimaryKey 步骤）。
    pub(crate) async fn primary_key_constraint_name(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<String>> {
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                let (schema, bare) = split_qualified_table_name(table_name);
                let schema = schema.unwrap_or("public");
                return db.primary_key_constraint_name(schema, bare).await;
            }
        }
        let _ = table_name;
        Ok(None)
    }

    /// 表上存量 CHECK 约束（仅 PG；调用点由 PostgreSQL 判定守卫，其余
    /// 后端不会到达——返回空集合仅为类型完整性）。
    #[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
    pub(crate) async fn check_constraint_facts(
        &self,
        schema: &str,
        table: &str,
    ) -> crate::Result<Vec<ActualCheckFacts>> {
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                return db.check_constraint_facts(schema, table).await;
            }
        }
        let _ = (schema, table);
        Ok(Vec::new())
    }
}

/// 保数据重建回灌批大小（行/批，拍板 E：同事务分批推进，每批打进度日志）。
#[cfg(feature = "postgresql")]
const REBUILD_COPY_BATCH_SIZE: i64 = 50_000;

/// 保数据重建的一次成功执行结果（[`Database::rebuild_nonempty_table_preserving_data`])。
struct PreservedDataRebuild {
    /// 本次回灌进新表的行数（事务内对账通过值）。
    migrated_rows: i64,
    /// 前置检查 a 清理掉的遗留备份表名（已限定 schema）；正常路径为 None。
    dropped_legacy_backup: Option<String>,
}

/// NeedsRebuild 原因是否属于"可搬迁类"（§3.1 触发条件）：超表时间/空间分区
/// 列变更（TimescaleDB 报 `Hypertable dimension mismatch`）与主键换轨（新
/// 主键不含全部分区键，报 `rebuilding the table is the only safe path`）。
/// 复验不收敛（增量迁移半迁移态）等其他原因不适用重建，维持原拒绝语义
/// （0924 拍板保留，防半迁移态二次伤害）。
#[cfg_attr(not(feature = "postgresql"), allow(dead_code))]
fn is_rebuild_eligible_cause(reason: &str) -> bool {
    reason.contains("Hypertable dimension mismatch")
        || reason.contains("rebuilding the table is the only safe path")
}

/// CREATE TABLE 渲染产物中的内联建索引语句识别（保数据重建跳过用：
/// insert 前不建二级索引，阶段二由重跑迁移 CONCURRENTLY 补建）。
#[cfg(feature = "postgresql")]
fn is_inline_index_statement(statement: &str) -> bool {
    let upper = statement.trim_start().to_ascii_uppercase();
    upper.starts_with("CREATE UNIQUE INDEX") || upper.starts_with("CREATE INDEX")
}

/// 一次诊断的结构化结论：基础表与各路由子表各自独立（migrate 按表分派
/// 执行；plan_table 内部路径合并为单一 [`TableDiagnosis`]）。
struct FamilyDiagnosis {
    base: TableDiagnosis,
    children: Vec<(String, TableDiagnosis)>,
}

/// 合并基础表与子表的诊断结论：任一 NeedsRebuild → NeedsRebuild（取第一个
/// 原因）；否则任一 Migratable → Migratable（计划合并）；全 Ready → Ready。
fn merge_diagnoses(left: TableDiagnosis, right: TableDiagnosis) -> TableDiagnosis {
    use TableDiagnosis::*;
    match (left, right) {
        (NeedsRebuild(cause), _) | (_, NeedsRebuild(cause)) => NeedsRebuild(cause),
        (Migratable(mut plan), Migratable(other)) => {
            plan.steps.extend(other.steps);
            plan.warnings.extend(other.warnings);
            Migratable(plan)
        }
        (Migratable(plan), Ready) => Migratable(plan),
        (Ready, Migratable(plan)) => Migratable(plan),
        (Ready, Ready) => Ready,
    }
}

// ---------------------------------------------------------------------------
// 主键原地变更（规格书 §4.3 / 任务 6）
// ---------------------------------------------------------------------------

/// ExpectedIndexDef 的语义参数提取（method / expression / 谓词）。
impl ExpectedIndexDef<'_> {
    /// 声明的索引方法（gin/gist/fulltext/...）；未声明视为 btree。
    pub(crate) fn index_method(&self) -> Option<&'static str> {
        self.columns.iter().find_map(|column| column.index_method)
    }

    /// 表达式/显式列清单定义文本（全文向量、函数索引、列清单覆盖）。
    pub(crate) fn index_expression(&self) -> Option<String> {
        self.columns
            .iter()
            .find_map(|column| column.index_expression)
            .map(str::to_string)
            .or_else(|| {
                self.columns
                    .iter()
                    .find_map(|column| column.index_columns)
                    .map(str::to_string)
            })
    }

    /// 部分索引谓词。
    pub(crate) fn index_predicate(&self) -> Option<&'static str> {
        self.columns.iter().find_map(|column| column.index_where)
    }

    /// 是否声明了 method/expression/列清单覆盖（无法按列集合比对的特殊索引）。
    pub(crate) fn is_special(&self) -> bool {
        self.index_method().is_some() || self.index_expression().is_some()
    }
}

/// 索引语义 diff（规格书 §4.3，纯函数）：按 列集合+唯一性+method/expression/
/// where 定义文本 比对而非按名字。
///
/// - 期望有实际没有 → 建缺失（ormer 自动命名）；
/// - 语义相等但名字不同（旧命名事故 `idx_{裸表名}_{列}` 等）→ 删旧 +
///   按 ormer 命名重建（命名归一）；
/// - 列序/定义漂移 → 同上（先删后建，防同名冲突）；
/// - INVALID（indisvalid=false）残留 → 直接删除；
/// - 外键后备索引、无法确认定义的降级路径按保守策略保留并记 warning。
///
/// 返回 (steps, warnings)；steps 内部先 DropIndex 后 CreateIndex。
/// PG 标识符 NAMEDATALEN-1=63 字节截断：超出部分由 PG 在创建索引时静默丢弃，
/// 内省读回的必然是截断后的名字。长表名+长列名组合生成的 ormer 索引名
/// （如 `idx_collect_collect_report_task_event_exceptions_report_start_time`）
/// 恰好越界，若按字面比对会永远判为"外来名"陷入 drop+recreate 循环，
/// 因此 PG 比对时须把期望名按同一规则截断后再比。
#[cfg(feature = "postgresql")]
fn postgres_truncated_identifier(name: &str) -> String {
    if name.len() <= 63 {
        return name.to_string();
    }
    let mut end = 63;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

pub(crate) fn plan_index_semantic_diff(
    db_type: DbType,
    table_name: &str,
    expected: &[ExpectedIndexDef<'_>],
    actual: &[ActualIndexFacts],
    available_columns: &BTreeSet<&str>,
    foreign_key_columns: &BTreeSet<&str>,
    unique_name_reliable: bool,
) -> crate::Result<(Vec<MigrationStep>, Vec<String>)> {
    let mut drops: Vec<MigrationStep> = Vec::new();
    let mut creates: Vec<MigrationStep> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut consumed = vec![false; actual.len()];

    for expected_def in expected {
        let special = expected_def.is_special();
        let method = expected_def.index_method();
        let expression = expected_def.index_expression();
        let predicate = expected_def.index_predicate();
        let plain_columns: Vec<(String, bool)> = if special {
            Vec::new()
        } else {
            expected_def
                .columns
                .iter()
                .map(|column| {
                    (
                        column.name.to_string(),
                        column.index_order == Some("DESC"),
                    )
                })
                .collect()
        };

        let matched = actual.iter().enumerate().find(|(position, candidate)| {
            !consumed[*position]
                && match_index_semantics(
                    expected_def.unique,
                    method,
                    &plain_columns,
                    expression.as_deref(),
                    predicate,
                    candidate,
                )
        });

        match matched {
            Some((position, actual_index)) => {
                consumed[position] = true;
                // 名字归一：语义相等但名字不是 ormer 命名（显式名或自动名）
                // 时，删旧按 ormer 名重建。解析器名称不可靠的组合豁免。
                let ormer_name = expected_def
                    .name
                    .map(ToString::to_string)
                    .unwrap_or_else(|| default_index_name_fallback(table_name, expected_def));
                let name_checks = unique_name_reliable || !expected_def.unique;
                #[cfg(feature = "postgresql")]
                let name_matches = actual_index.name == ormer_name
                    || (db_type == DbType::PostgreSQL
                        && actual_index.name == postgres_truncated_identifier(&ormer_name));
                #[cfg(not(feature = "postgresql"))]
                let name_matches = actual_index.name == ormer_name;
                if name_checks && !name_matches {
                    warnings.push(format!(
                        "index {} on {table_name} matches the model semantically but uses a \
                         foreign name; renaming to {ormer_name} (drop + recreate)",
                        actual_index.name
                    ));
                    drops.push(MigrationStep::DropIndex {
                        name: actual_index.name.clone(),
                        table: table_name.to_string(),
                    });
                    creates.push(create_index_step(
                        db_type,
                        &ormer_name,
                        table_name,
                        expected_def,
                        expression.as_deref(),
                    )?);
                }
            }
            None => {
                // 缺失 → 建。索引列必须全部存在于表中（含本次新增列）。
                if !special
                    && !expected_def
                        .columns
                        .iter()
                        .all(|column| available_columns.contains(column.name))
                {
                    warnings.push(format!(
                        "skipping index on ({}) because some columns are missing from table {table_name}",
                        expected_def
                            .columns
                            .iter()
                            .map(|column| column.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    continue;
                }
                // 特殊索引在"无定义自省 + 无 IF NOT EXISTS"的后端不自动建，
                // 否则重复启动会因同名索引已存在而报错（破坏幂等）。
                if special && actual_definition_unavailable(db_type) {
                    warnings.push(format!(
                        "special index on ({}) cannot be verified against this backend; \
                         its presence is not diffed automatically",
                        expected_def
                            .columns
                            .iter()
                            .map(|column| column.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    continue;
                }
                let name = expected_def
                    .name
                    .map(ToString::to_string)
                    .unwrap_or_else(|| default_index_name_fallback(table_name, expected_def));
                creates.push(create_index_step(
                    db_type,
                    &name,
                    table_name,
                    expected_def,
                    expression.as_deref(),
                )?);
            }
        }
    }

    // 删除侧：未语义匹配的存量索引
    let special_columns: BTreeSet<&str> = expected
        .iter()
        .filter(|expected_def| expected_def.is_special())
        .flat_map(|expected_def| {
            expected_def
                .columns
                .iter()
                .map(|column| column.name)
                .collect::<Vec<_>>()
        })
        .collect();
    for (position, index) in actual.iter().enumerate() {
        if consumed[position] || index.name.is_empty() {
            continue;
        }
        // INVALID 残留（CONCURRENTLY 失败产物）：无条件清除
        if !index.valid {
            drops.push(MigrationStep::DropIndex {
                name: index.name.clone(),
                table: table_name.to_string(),
            });
            continue;
        }
        let leading_column = index
            .columns
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        // 外键后备索引（MySQL 为 FK 列自动创建）保留
        if !leading_column.is_empty() && foreign_key_columns.contains(leading_column.as_str()) {
            warnings.push(format!(
                "keeping index {} because it backs a foreign key on column {leading_column}",
                index.name
            ));
            continue;
        }
        // 降级路径（无定义文本自省）无法确认某索引是否实现了特殊声明，
        // 保守保留（PG 有定义文本，走语义匹配，不需要该豁免）。
        if index.definition.is_none()
            && index
                .columns
                .iter()
                .any(|(name, _)| special_columns.contains(name.as_str()))
        {
            warnings.push(format!(
                "keeping index {} because it may implement a method/expression index declaration",
                index.name
            ));
            continue;
        }
        // 模型未声明的非 btree 索引（业务手工 GIN/BRIN 等）保守保留
        if index
            .method
            .as_deref()
            .is_some_and(|method| !method.eq_ignore_ascii_case("btree"))
        {
            warnings.push(format!(
                "keeping non-btree index {} because it is not declared in the model",
                index.name
            ));
            continue;
        }
        warnings.push(format!(
            "dropping index {} because it is not declared in the model",
            index.name
        ));
        drops.push(MigrationStep::DropIndex {
            name: index.name.clone(),
            table: table_name.to_string(),
        });
    }

    // 先删后建：漂移索引与重建索引同名（历史组合索引恰好占用 ormer 名）
    // 时避免撞名
    let mut steps = drops;
    steps.extend(creates);
    Ok((steps, warnings))
}

/// 非默认命名时 `default_index_name` 的转发（位于 migration.rs，未显式
/// 命名时与建表路径命名一致）。
fn default_index_name_fallback(table_name: &str, expected: &ExpectedIndexDef<'_>) -> String {
    crate::migration::default_index_name(table_name, expected)
}

/// 该后端是否既无索引定义自省、又不支持 CREATE INDEX IF NOT EXISTS
/// （此组合下特殊索引不自动建，防重复建索引报错）。
fn actual_definition_unavailable(db_type: DbType) -> bool {
    let has_definition_introspection = {
        #[cfg(feature = "postgresql")]
        {
            matches!(db_type, DbType::PostgreSQL)
        }
        #[cfg(not(feature = "postgresql"))]
        {
            false
        }
    };
    !has_definition_introspection && !crate::model::index_supports_if_not_exists(db_type)
}

/// 用 ormer 渲染器生成建索引步骤（普通索引走 CreateIndex 变体，带
/// 顺序/谓词的特殊索引走 Sql 形态）。
fn create_index_step(
    db_type: DbType,
    name: &str,
    table_name: &str,
    expected: &ExpectedIndexDef<'_>,
    expression: Option<&str>,
) -> crate::Result<MigrationStep> {
    if expression.is_some() {
        // 特殊索引：复用建表路径渲染（method/expression/predicate 全支持）
        let columns_sql = expected
            .columns
            .iter()
            .map(|column| {
                let mut value = crate::model::quote_identifier(db_type, column.name);
                if let Some(order) = column.index_order {
                    value.push(' ');
                    value.push_str(order);
                }
                value
            })
            .collect::<Vec<_>>();
        // 表达式索引：用声明的表达式文本替代列名
        let columns_body = match expression {
            Some(expression_text) => expression_text.to_string(),
            None => columns_sql.join(", "),
        };
        let unique_sql = if expected.unique { "UNIQUE " } else { "" };
        let predicate = expected
            .index_predicate()
            .map(|where_clause| format!(" WHERE {where_clause}"))
            .unwrap_or_default();
        let base = crate::model::render_create_index(db_type, name, table_name, &columns_body, expected.unique);
        let _ = (unique_sql, columns_sql);
        return Ok(MigrationStep::Sql {
            sql: format!("{base}{predicate}"),
        });
    }
    crate::migration::index_migration_step(db_type, name.to_string(), table_name, &expected.columns, expected.unique)
}

/// 主键差异的原地变更判定（纯函数）。
///
/// - 集合一致 → `Ok(None)`；
/// - PostgreSQL 且变更后的主键包含全部分区键（超表约束）→
///   `Ok(Some(ChangePrimaryKey 步骤))`；
/// - 其余 → `Err(rebuild 原因)`。
#[cfg_attr(not(feature = "postgresql"), allow(unused_variables))]
pub(crate) fn plan_primary_key_change(
    db_type: DbType,
    table_name: &str,
    actual_pk: &BTreeSet<String>,
    expected_pk: &[&str],
    expected_pk_constraint: Option<String>,
    is_hypertable: bool,
    partition_columns: &[&str],
) -> Result<Option<MigrationStep>, String> {
    let expected_set: BTreeSet<&str> = expected_pk.iter().copied().collect();
    if actual_pk.len() == expected_set.len()
        && actual_pk.iter().all(|name| expected_set.contains(name.as_str()))
    {
        return Ok(None);
    }
    #[cfg(feature = "postgresql")]
    if matches!(db_type, DbType::PostgreSQL) {
        // 超表主键必须包含全部分区键；变更后（期望）主键不含分区键时不可原地
        if is_hypertable
            && partition_columns
                .iter()
                .any(|column| !expected_set.contains(column))
        {
            return Err(format!(
                "hypertable primary key must contain partition column(s) [{}]; \
                 rebuilding the table is the only safe path",
                partition_columns.join(", ")
            ));
        }
        return Ok(Some(MigrationStep::ChangePrimaryKey {
            table: table_name.to_string(),
            columns: expected_pk.iter().map(|column| column.to_string()).collect(),
            drop_constraint: expected_pk_constraint,
        }));
    }
    let _ = table_name;
    Err("primary key change cannot be migrated in place on this backend".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_sql_fragment_collapses_whitespace_and_quotes() {
        assert_eq!(norm_sql_fragment("\"a\" ,  \"b\""), "a , b");
        assert_eq!(norm_sql_fragment("( a )"), "a");
        assert_eq!(norm_sql_fragment("(a, b)"), "(a, b)");
    }

    #[test]
    fn parse_pg_index_definition_extracts_parts() {
        let parsed = parse_pg_index_definition(
            "CREATE INDEX idx ON public.t USING btree (a, b DESC) WHERE (deleted = false)",
        )
        .unwrap();
        assert_eq!(parsed.method, "btree");
        assert_eq!(parsed.columns, "a, b DESC");
        assert_eq!(parsed.predicate.as_deref(), Some("(deleted = false)"));

        let parsed = parse_pg_index_definition(
            "CREATE UNIQUE INDEX uq ON public.t USING gin (to_tsvector('english'::regconfig, title))",
        )
        .unwrap();
        assert_eq!(parsed.method, "gin");
        assert_eq!(
            parsed.columns,
            "to_tsvector('english'::regconfig, title)"
        );
        assert_eq!(parsed.predicate, None);
    }

    #[test]
    fn normalize_check_expression_strips_casts_and_identifier_parens() {
        // pg_get_constraintdef 对 cardinality(roles) 的存储形态
        assert_eq!(
            normalize_check_expression("CHECK ((cardinality((roles)::text[])) > 0)"),
            "cardinality(roles) > 0"
        );
        assert_eq!(
            normalize_check_expression("CHECK ((cardinality(roles)) > 0)"),
            "cardinality(roles) > 0"
        );
        // 字符串字面量内的内容不受类型注解剥离影响
        assert_eq!(
            normalize_check_expression("CHECK ((status)::text = 'a::b')"),
            "status = 'a::b'"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn check_diff_renders_on_sqlite() {
        let expected = vec![ExpectedCheck {
            name: "ck_t_roles".to_string(),
            expr: "cardinality(roles) > 0".to_string(),
        }];
        // 缺失 → Add（渲染为 SQLite 方言的约束步骤）
        let steps = plan_check_diff("t", &expected, &[], DbType::Sqlite);
        assert_eq!(steps.len(), 1);
        match &steps[0] {
            MigrationStep::AddConstraint { definition, .. } => {
                assert!(definition.contains("CHECK (cardinality(roles) > 0)"), "{definition}");
            }
            other => panic!("unexpected step: {other:?}"),
        }
        // 同名同义 → 跳过（幂等）
        let actual = vec![ActualCheckFacts {
            name: "ck_t_roles".to_string(),
            definition: "CHECK ((cardinality((roles)::text[])) > 0)".to_string(),
        }];
        assert!(plan_check_diff("t", &expected, &actual, DbType::Sqlite).is_empty());
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn check_diff_skips_semantically_equal_constraints() {
        let expected = vec![ExpectedCheck {
            name: "ck_t_roles".to_string(),
            expr: "cardinality(roles) > 0".to_string(),
        }];
        // 同名同义 → 跳过
        let actual = vec![ActualCheckFacts {
            name: "ck_t_roles".to_string(),
            definition: "CHECK ((cardinality((roles)::text[])) > 0)".to_string(),
        }];
        assert!(plan_check_diff("t", &expected, &actual, DbType::PostgreSQL).is_empty());
        // 异名同义 → 跳过（视为同一约束）
        let renamed = vec![ActualCheckFacts {
            name: "auth_event_roles_check".to_string(),
            definition: "CHECK ((cardinality((roles)::text[])) > 0)".to_string(),
        }];
        assert!(plan_check_diff("t", &expected, &renamed, DbType::PostgreSQL).is_empty());
        // 同名不同义 → Drop + Add
        let drifted = vec![ActualCheckFacts {
            name: "ck_t_roles".to_string(),
            definition: "CHECK ((cardinality(roles)) >= 0)".to_string(),
        }];
        let steps = plan_check_diff("t", &expected, &drifted, DbType::PostgreSQL);
        assert_eq!(steps.len(), 2);
        assert!(matches!(steps[0], MigrationStep::Sql { .. }));
        assert!(matches!(steps[1], MigrationStep::AddConstraint { .. }));
        // 完全缺失 → Add
        let steps = plan_check_diff("t", &expected, &[], DbType::PostgreSQL);
        assert_eq!(steps.len(), 1);
        assert!(matches!(steps[0], MigrationStep::AddConstraint { .. }));
    }

    #[test]
    fn index_semantics_matches_by_columns_ignoring_name() {
        let actual = ActualIndexFacts::from_plain(
            "idx_auth_auth_users_project".to_string(),
            vec![("project_id".to_string(), false)],
            false,
        );
        // 期望无名（ormer 自动命名），列一致 → 语义相等
        assert!(match_index_semantics(
            false,
            None,
            &[("project_id".to_string(), false)],
            None,
            None,
            &actual
        ));
        // 列序矛盾 → 不匹配
        assert!(!match_index_semantics(
            false,
            None,
            &[("update_time".to_string(), false), ("project_id".to_string(), false)],
            None,
            None,
            &actual
        ));
    }

    #[test]
    fn index_semantics_matches_expression_via_definition() {
        let actual = ActualIndexFacts {
            name: "idx_search".to_string(),
            columns: vec![],
            unique: false,
            valid: true,
            method: Some("gin".to_string()),
            definition: Some(
                "CREATE INDEX idx_search ON public.t USING gin (to_tsvector('english'::regconfig, title))"
                    .to_string(),
            ),
            predicate: None,
        };
        assert!(match_index_semantics(
            false,
            Some("gin"),
            &[],
            Some("to_tsvector('english'::regconfig, title)"),
            None,
            &actual
        ));
        // 定义变化 → 不匹配
        assert!(!match_index_semantics(
            false,
            Some("gin"),
            &[],
            Some("to_tsvector('simple'::regconfig, title)"),
            None,
            &actual
        ));
    }

    #[test]
    fn index_semantics_rejects_invalid_residue() {
        let mut invalid = ActualIndexFacts::from_plain(
            "idx_broken".to_string(),
            vec![("a".to_string(), false)],
            false,
        );
        invalid.valid = false;
        assert!(!match_index_semantics(
            false,
            None,
            &[("a".to_string(), false)],
            None,
            None,
            &invalid
        ));
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn postgres_truncated_identifier_cuts_at_namedatalen() {
        // 短名原样保留
        assert_eq!(
            postgres_truncated_identifier("idx_t_a"),
            "idx_t_a"
        );
        // 超长名按 63 字节截断（PG NAMEDATALEN-1）：真实事故名
        let long = "idx_collect_collect_report_task_event_exceptions_report_start_time";
        assert_eq!(long.len(), 66);
        assert_eq!(
            postgres_truncated_identifier(long),
            "idx_collect_collect_report_task_event_exceptions_report_start_t"
        );
    }

    #[test]
    fn concurrent_rewrite_covers_plain_and_unique_create_index() {
        assert_eq!(
            to_concurrent_create_index("CREATE INDEX idx_t_a ON t (a)"),
            "CREATE INDEX CONCURRENTLY idx_t_a ON t (a)"
        );
        assert_eq!(
            to_concurrent_create_index("CREATE UNIQUE INDEX uq_t_a ON t (a)"),
            "CREATE UNIQUE INDEX CONCURRENTLY uq_t_a ON t (a)"
        );
        // 非 CREATE INDEX 语句原样返回（不误改）
        assert_eq!(
            to_concurrent_create_index("ALTER TABLE t ADD COLUMN a int"),
            "ALTER TABLE t ADD COLUMN a int"
        );
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn primary_key_change_plan_on_postgresql() {
        let mut actual = BTreeSet::new();
        actual.insert("update_time".to_string());
        actual.insert("project_id".to_string());
        let expected = vec!["agvid", "update_time", "project_id"];
        // PG 原地
        let step = plan_primary_key_change(
            DbType::PostgreSQL,
            "collect.data",
            &actual,
            &expected,
            Some("data_pkey".to_string()),
            false,
            &[],
        )
        .unwrap()
        .unwrap();
        match step {
            MigrationStep::ChangePrimaryKey {
                columns,
                drop_constraint,
                ..
            } => {
                assert_eq!(columns, vec!["agvid", "update_time", "project_id"]);
                assert_eq!(drop_constraint.as_deref(), Some("data_pkey"));
            }
            other => panic!("unexpected step: {other:?}"),
        }
        // 主键一致 → None
        let same = expected.iter().map(|c| c.to_string()).collect();
        assert!(plan_primary_key_change(
            DbType::PostgreSQL,
            "collect.data",
            &same,
            &expected,
            None,
            false,
            &[]
        )
        .unwrap()
        .is_none());
        // 非 PG → rebuild 原因
        #[cfg(feature = "sqlite")]
        assert!(plan_primary_key_change(
            DbType::Sqlite,
            "t",
            &actual,
            &expected,
            None,
            false,
            &[]
        )
        .is_err());
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn primary_key_change_requires_partition_columns_for_hypertable() {
        let mut actual = BTreeSet::new();
        actual.insert("update_time".to_string());
        // 期望主键不含分区键（时间列）→ 不可原地
        let expected = vec!["project_id"];
        assert!(plan_primary_key_change(
            DbType::PostgreSQL,
            "t",
            &actual,
            &expected,
            None,
            true,
            &["update_time"]
        )
        .is_err());
        // 含分区键 → 可原地
        let expected = vec!["project_id", "update_time"];
        assert!(plan_primary_key_change(
            DbType::PostgreSQL,
            "t",
            &actual,
            &expected,
            None,
            true,
            &["update_time"]
        )
        .unwrap()
        .is_some());
    }
}
