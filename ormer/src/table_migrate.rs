//! 表迁移统一入口：`plan_table` / `apply_table`。
//!
//! 设计原则（规格书 auto.md 第四章）：
//! ① 判断与执行各只有一个入口：`plan_table` 诊断、`apply_table` 执行；
//! ② 内省数据不出 ormer：所有 pg_* 自省由本模块内部消费，不设细粒度公开 API；
//! ③ 项目只声明"模型 + 策略"（[`ApplyOptions`]）；
//! ④ 全部幂等：apply 可被启动重试循环反复调用，CONCURRENTLY 残留在 apply 内自愈；
//! ⑤ plan 与 apply 同源：apply 内部就是重新诊断（`diagnose_table`）后按计划执行。

use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{Database, Transaction};
#[cfg_attr(not(feature = "postgresql"), allow(unused_imports))]
use crate::migration::split_qualified_table_name;
use crate::migration::{
    execute_steps, execute_steps_nontransactional, is_schema_rebuild_error, table_name,
    ExpectedIndexDef, MigrationPlan, MigrationStep, TableMigration,
};
use crate::model::WritableModel;
use std::collections::BTreeSet;

/// [`Database::plan_table`] 产出的执行计划。与版本化迁移的 [`MigrationPlan`]
/// 同一载体（同源设计）：`steps()` / `to_sql()` 可在执行前审查。
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

/// 多余列处理策略：模型已删、库里还在的列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraPolicy {
    /// 保留（默认，最保守）：多余列留在库内，不参与任何 DDL。
    Keep,
    /// 删除（对齐旧 ensure_table_permissive 语义）：生成 DropColumn 步骤。
    Drop,
}

/// 诊断为 [`TableDiagnosis::NeedsRebuild`] 时的处置策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildPolicy {
    /// 报错返回（默认，丢数据不可接受场景）。
    Refuse,
    /// 删表重建——重建由 ormer 全程托管：自动备份表上业务触发器（PG）→
    /// 重建 → 自动原样恢复，项目无感。
    Allow,
}

/// [`Database::apply_table`] 的执行策略。
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// 多余列处理：Keep 保留（默认，最保守）/ Drop 删除（对齐旧
    /// ensure_table_permissive 语义）。
    pub extra_columns: ExtraPolicy,
    /// 推不动时：Refuse 报错返回（默认）/ Allow 删表重建（触发器托管）。
    pub rebuild: RebuildPolicy,
    /// 索引操作走 CREATE INDEX CONCURRENTLY（PG，默认 true，不阻塞业务读写）；
    /// 失败残留的 INVALID 索引由 ormer 自动清理重试。非 PG 后端与 QuestDB
    /// 忽略该选项（事务内普通建索引）。
    pub index_concurrently: bool,
    /// 启动期需确保安装的库级扩展（幂等），如 `&["timescaledb"]`。
    /// 仅 PostgreSQL 支持扩展；其他后端传入非空列表返回 UnsupportedFeature。
    pub extensions: Vec<String>,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            extra_columns: ExtraPolicy::Keep,
            rebuild: RebuildPolicy::Refuse,
            index_concurrently: true,
            extensions: Vec::new(),
        }
    }
}

impl ApplyOptions {
    /// [`crate::Database::ensure_table`] 预设：Keep + Refuse，非并发建索引
    /// （与改造前 ensure_table 的执行形态一致）。
    pub(crate) fn strict() -> Self {
        Self {
            index_concurrently: false,
            ..Self::default()
        }
    }

    /// [`crate::Database::ensure_table_permissive`] 预设：Drop + Allow。
    pub(crate) fn permissive() -> Self {
        Self {
            extra_columns: ExtraPolicy::Drop,
            rebuild: RebuildPolicy::Allow,
            index_concurrently: false,
            extensions: Vec::new(),
        }
    }
}

/// [`Database::apply_table`] 的执行结果。
#[derive(Debug)]
pub struct TableApplyOutcome {
    /// 执行前的诊断结论。
    pub diagnosis: TableDiagnosis,
    /// 本次调用是否执行了基础表的 CREATE TABLE（表原本不存在的新建，或
    /// `rebuild=Allow` 触发的删表重建）。
    pub created_table: bool,
    /// 实际执行的步骤（含建索引/清漂移索引/ChangePrimaryKey）。
    pub executed: Vec<MigrationStep>,
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
// Database API：plan_table / apply_table 与内部编排
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

/// 校验扩展名是合法标识符（CREATE EXTENSION 不支持参数绑定，直接拼接前防御）。
fn is_safe_extension_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
        && !name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
}

impl Database {
    /// 诊断：一次性内省这张表（含 PG 路由子表）的全部现状
    /// （schema/列/主键/索引/约束/触发器），与模型比对。
    ///
    /// 项目侧了解"表现在什么状态"的唯一入口——ormer 不提供任何细粒度
    /// 内省 API。诊断与执行同源：[`Database::apply_table`] 内部就是重新
    /// 调用本方法后按计划执行。
    pub async fn plan_table<T: WritableModel>(&self) -> crate::Result<TableDiagnosis> {
        self.diagnose_family_merged::<T>(&ApplyOptions::default()).await
    }

    /// 执行：把库弄到与模型一致。内部重新诊断后按计划执行（天然幂等，
    /// 适配启动重试循环）。
    ///
    /// 覆盖：扩展确保 → schema 隐式创建 → 建表 → 补列 → 主键原地变更（PG）→
    /// 列类型/默认值对齐 → 索引语义 diff（补缺/清漂移/特殊索引纳入）→
    /// CHECK 约束闭环（PG）→ 外键对齐；诊断为重建且 `rebuild=Allow` 时
    /// 自动备份/恢复业务触发器（PG）后删表重建。PG 路由子表与基础表走
    /// 同一套诊断与执行（子表推不动时按策略重建子表自身）。
    pub async fn apply_table<T: WritableModel>(
        &self,
        opts: &ApplyOptions,
    ) -> crate::Result<TableApplyOutcome> {
        let db_type = self.db_type();
        if !crate::abstract_layer::capabilities::Capabilities::of(db_type).schema_introspection {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "apply_table",
            });
        }
        self.ensure_extensions(&opts.extensions).await?;

        let diagnosis = self.diagnose_family_merged::<T>(opts).await?;
        let family = self.diagnose_family::<T>(opts).await?;
        let table_name = T::table_name_for_db(db_type);

        let mut executed: Vec<MigrationStep> = Vec::new();
        let mut created_table = false;
        let mut base_rebuild: Option<RebuildCause> = None;

        // 基础表
        match family.base {
            TableDiagnosis::Ready => {}
            TableDiagnosis::Migratable(plan) => {
                created_table |= plan.steps().iter().any(|step| {
                    matches!(step, MigrationStep::CreateTable { table, .. } if table == table_name)
                });
                executed.extend(self.execute_plan_steps(&plan, opts).await?);
            }
            TableDiagnosis::NeedsRebuild(cause) => base_rebuild = Some(cause),
        }

        // 路由子表：与基础表同一套 diff 逻辑；推不动时按策略重建子表自身
        let mut child_rebuilt = false;
        if base_rebuild.is_none() {
            for (child, child_diagnosis) in family.children {
                match child_diagnosis {
                    TableDiagnosis::Ready => {}
                    TableDiagnosis::Migratable(plan) => {
                        executed.extend(self.execute_plan_steps(&plan, opts).await?);
                    }
                    TableDiagnosis::NeedsRebuild(cause) => match opts.rebuild {
                        RebuildPolicy::Refuse => {
                            return Err(crate::OrmerError::unmigratable_schema(
                                cause.table,
                                cause.reason,
                            ));
                        }
                        RebuildPolicy::Allow => {
                            #[cfg(feature = "postgresql")]
                            {
                                self.recreate_routed_child_table::<T>(&child).await?;
                                child_rebuilt = true;
                            }
                            #[cfg(not(feature = "postgresql"))]
                            {
                                let _ = &child;
                                let _ = &mut child_rebuilt;
                            }
                        }
                    },
                }
            }
        }

        // 基础表推不动 → 按重建策略收场
        if let Some(cause) = base_rebuild {
            return self.rebuild_or_refuse::<T>(opts, cause).await;
        }

        // 复验（同源：整体重新诊断）。仍有差异说明增量迁移未收敛，
        // 转入重建路径按策略收场。
        match self.diagnose_family_merged::<T>(opts).await? {
            TableDiagnosis::Ready => Ok(TableApplyOutcome {
                diagnosis,
                created_table: created_table || child_rebuilt,
                executed,
            }),
            TableDiagnosis::Migratable(_) => {
                let cause = RebuildCause::new(
                    table_name,
                    "incremental migration did not reconcile the schema; \
                     fixing it requires dropping and recreating the table",
                );
                self.rebuild_or_refuse::<T>(opts, cause).await
            }
            TableDiagnosis::NeedsRebuild(cause) => self.rebuild_or_refuse::<T>(opts, cause).await,
        }
    }

    /// 扩展确保：逐个幂等 `CREATE EXTENSION IF NOT EXISTS`。仅 PostgreSQL
    /// 支持扩展；其他后端传入非空列表返回 UnsupportedFeature。
    pub(crate) async fn ensure_extensions(&self, extensions: &[String]) -> crate::Result<()> {
        if extensions.is_empty() {
            return Ok(());
        }
        let db_type = self.db_type();
        for name in extensions {
            if !is_safe_extension_name(name) {
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
            feature: "ApplyOptions.extensions (CREATE EXTENSION is PostgreSQL-only)",
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

    /// 诊断单张表（基础表或某个路由子表）。`table_name` 为 None 时取模型
    /// 基础表名（含超表元数据校验）；Some 时为子表名（不重复做超表校验）。
    pub(crate) async fn diagnose_single_table<T: WritableModel>(
        &self,
        table_name: Option<&str>,
        opts: &ApplyOptions,
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
            plan.push(MigrationStep::CreateTable {
                table: target.to_string(),
                definition: crate::generate_create_table_sql::<T>(db_type)?,
            });
            return Ok(TableDiagnosis::Migratable(plan));
        }

        let migration =
            TableMigration::<T>::for_diagnosis(self, table_name, opts.extra_columns);
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

    /// 诊断（内部）：基础表 + 路由子表的结构化结论（apply 按表分派执行）。
    async fn diagnose_family<T: WritableModel>(
        &self,
        opts: &ApplyOptions,
    ) -> crate::Result<FamilyDiagnosis> {
        let db_type = self.db_type();
        if !crate::abstract_layer::capabilities::Capabilities::of(db_type).schema_introspection {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "plan_table",
            });
        }
        let base = self.diagnose_single_table::<T>(None, opts).await?;
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
                        .diagnose_single_table::<T>(Some(&child), opts)
                        .await?;
                    children.push((child, child_diagnosis));
                }
            }
        }
        Ok(FamilyDiagnosis { base, children })
    }

    /// 诊断（内部）：基础表 + 路由子表合并为整体结论（plan_table 与复验用）。
    pub(crate) async fn diagnose_family_merged<T: WritableModel>(
        &self,
        opts: &ApplyOptions,
    ) -> crate::Result<TableDiagnosis> {
        let family = self.diagnose_family::<T>(opts).await?;
        let mut merged = family.base;
        for (_, child) in family.children {
            merged = merge_diagnoses(merged, child);
        }
        Ok(merged)
    }

    /// 按诊断结论执行：Migratable → 分段执行计划（PG 并发索引在事务外）；
    /// NeedsRebuild → 按 rebuild 策略收场。
    async fn rebuild_or_refuse<T: WritableModel>(
        &self,
        opts: &ApplyOptions,
        cause: RebuildCause,
    ) -> crate::Result<TableApplyOutcome> {
        let table_name = T::table_name_for_db(self.db_type());
        match opts.rebuild {
            RebuildPolicy::Refuse => Err(crate::OrmerError::unmigratable_schema(
                cause.table,
                cause.reason,
            )),
            RebuildPolicy::Allow => {
                // 重建全程托管：备份业务触发器（PG）→ 删表 → 重建 → 恢复
                let triggers = self.backup_business_triggers(table_name).await?;
                self.drop_table::<T>().execute().await?;
                self.create_table::<T>().execute().await?;
                self.restore_business_triggers(&triggers).await?;

                match self.diagnose_family_merged::<T>(opts).await? {
                    TableDiagnosis::Ready => {
                        let db_type = self.db_type();
                        let mut executed = vec![
                            MigrationStep::Sql {
                                sql: format!(
                                    "DROP TABLE {}",
                                    crate::model::quote_qualified_identifier(db_type, table_name)
                                ),
                            },
                            MigrationStep::CreateTable {
                                table: table_name.to_string(),
                                definition: crate::generate_create_table_sql::<T>(db_type)?,
                            },
                        ];
                        for definition in &triggers {
                            executed.push(MigrationStep::Sql {
                                sql: definition.clone(),
                            });
                        }
                        Ok(TableApplyOutcome {
                            diagnosis: TableDiagnosis::NeedsRebuild(cause),
                            created_table: true,
                            executed,
                        })
                    }
                    other => Err(crate::ormer_error!(
                        "apply_table rebuild of {table_name} did not reconcile the schema: {other:?}"
                    )),
                }
            }
        }
    }

    /// 执行一份迁移计划并返回实际执行的步骤。
    ///
    /// - 非事务后端：逐条执行（现状语义）；
    /// - `index_concurrently=false`：单事务整体执行；
    /// - `index_concurrently=true`（PG）：建索引步骤拆到事务外
    ///   CONCURRENTLY 执行，执行前与失败重试前自动清理同表 INVALID
    ///   残留索引（自愈），其余步骤保持单事务。
    #[cfg_attr(not(feature = "postgresql"), allow(unused_variables))]
    pub(crate) async fn execute_plan_steps(
        &self,
        plan: &MigrationPlan,
        opts: &ApplyOptions,
    ) -> crate::Result<Vec<MigrationStep>> {
        if plan.is_empty() {
            return Ok(Vec::new());
        }
        let db_type = self.db_type();
        #[cfg(any(feature = "sqlite", feature = "postgresql"))]
        let concurrent = {
            #[cfg(feature = "postgresql")]
            {
                opts.index_concurrently
                    && matches!(db_type, DbType::PostgreSQL)
                    && !db_type.is_questdb()
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
                self.create_index_concurrently(step, db_type).await?;
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
    async fn create_index_concurrently(
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

    /// 备份表上的业务触发器定义（PG `pg_get_triggerdef`，排除 internal）。
    /// 其他后端无对应概念，返回空列表（重建路径自然跳过恢复）。
    pub(crate) async fn backup_business_triggers(
        &self,
        qualified_table: &str,
    ) -> crate::Result<Vec<String>> {
        #[cfg(feature = "postgresql")]
        {
            if let Some(db) = self.as_postgresql() {
                let (schema, bare) = split_qualified_table_name(qualified_table);
                let schema = schema.unwrap_or("public");
                return db.business_trigger_definitions(schema, bare).await;
            }
        }
        let _ = qualified_table;
        Ok(Vec::new())
    }

    /// 重建后原样恢复业务触发器。
    pub(crate) async fn restore_business_triggers(
        &self,
        definitions: &[String],
    ) -> crate::Result<()> {
        for definition in definitions {
            self.execute_sql(definition.as_str()).await?;
        }
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

/// 一次诊断的结构化结论：基础表与各路由子表各自独立（apply 按表分派
/// 执行与重建；plan_table 对外合并为单一 [`TableDiagnosis`]）。
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
