//! SQLite/DuckDB 共用的 DDL 内省与约束校验公共层。
//!
//! 两个后端的 `CREATE TABLE` / `CREATE INDEX` 文本解析、约束比对与
//! related/multi count 逻辑逐字相同，仅 `DbType` 方言与查询入口不同；
//! 差异点由调用方透传，后端文件不再各持一份拷贝。

use crate::abstract_layer::DbType;
use crate::db_first::{DbFirstForeignKey, DbFirstIndex, DbFirstIndexColumn, DbFirstTable};
use crate::model::Model;

/// DML 语句追加 `RETURNING 1`（无 RETURNING 时），供受影响行数统计使用。
pub fn sql_with_returning_count(sql: &str) -> Option<String> {
    let sql = sql.trim_start();
    let is_dml = ["INSERT", "UPDATE", "REPLACE"].iter().any(|keyword| {
        sql.get(..keyword.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(keyword))
    });
    if !is_dml || sql.to_ascii_lowercase().contains(" returning ") {
        return None;
    }

    let sql = sql.trim_end().strip_suffix(';').unwrap_or(sql).trim_end();
    Some(format!("{sql} RETURNING 1"))
}

/// 公共 strict helper（`parse_column_value_strict`）覆盖的 rust_type 集合。
/// SQLite 与 MySQL 的模型列解码共用。
pub fn is_strict_rust_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "String"
            | "f32"
            | "f64"
            | "Decimal"
            | "rust_decimal::Decimal"
            | "BigDecimal"
            | "bigdecimal::BigDecimal"
            | "bool"
            | "Uuid"
            | "uuid::Uuid"
            | "Vec<u8>"
            | "&[u8]"
            | "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime"
            | "NaiveDate"
            | "chrono::NaiveDate"
            | "NaiveTime"
            | "chrono::NaiveTime"
    )
}

/// 按 DDL 括号深度与引号上下文拆分 `CREATE TABLE (...)` 的顶层元素
/// （列定义与表约束），忽略注释外的逗号嵌套。
pub fn ddl_table_items(create_sql: &str) -> Vec<String> {
    let Some(open_idx) = create_sql.find('(') else {
        return Vec::new();
    };
    let Some(close_idx) = create_sql.rfind(')') else {
        return Vec::new();
    };
    let body = &create_sql[open_idx + 1..close_idx];
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_bracket = false;
    let mut chars = body.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single {
            current.push(ch);
            if ch == '\'' {
                if matches!(chars.peek(), Some('\'')) {
                    current.push(chars.next().expect("peeked quote"));
                } else {
                    in_single = false;
                }
            }
            continue;
        }
        if in_double {
            current.push(ch);
            if ch == '"' {
                if matches!(chars.peek(), Some('"')) {
                    current.push(chars.next().expect("peeked quote"));
                } else {
                    in_double = false;
                }
            }
            continue;
        }
        if in_backtick {
            current.push(ch);
            if ch == '`' {
                if matches!(chars.peek(), Some('`')) {
                    current.push(chars.next().expect("peeked backtick"));
                } else {
                    in_backtick = false;
                }
            }
            continue;
        }
        if in_bracket {
            current.push(ch);
            if ch == ']' {
                if matches!(chars.peek(), Some(']')) {
                    current.push(chars.next().expect("peeked bracket"));
                } else {
                    in_bracket = false;
                }
            }
            continue;
        }

        match ch {
            '\'' => {
                current.push(ch);
                in_single = true;
            }
            '"' => {
                current.push(ch);
                in_double = true;
            }
            '`' => {
                current.push(ch);
                in_backtick = true;
            }
            '[' => {
                current.push(ch);
                in_bracket = true;
            }
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let item = current.trim();
                if !item.is_empty() {
                    items.push(item.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let item = current.trim();
    if !item.is_empty() {
        items.push(item.to_string());
    }
    items
}

/// 提取首个括号内的逗号分隔列表，逐项去除标识符引号。
pub fn ddl_parenthesized_list(segment: &str) -> Vec<String> {
    let Some(open_idx) = segment.find('(') else {
        return Vec::new();
    };
    let Some(close_idx) = segment[open_idx + 1..].find(')') else {
        return Vec::new();
    };
    segment[open_idx + 1..open_idx + 1 + close_idx]
        .split(',')
        .map(|value| ddl_strip_identifier(value.trim()))
        .filter(|value| !value.is_empty())
        .collect()
}

/// 去除标识符外层引号（`"`、`` ` ``、`[]`）。
pub fn ddl_strip_identifier(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
        })
        .or_else(|| {
            trimmed
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
        })
        .unwrap_or(trimmed)
        .to_string()
}

/// 解析 `CONSTRAINT <name> <tail>` 前缀，返回 (约束名, 其余部分)。
pub fn ddl_parse_constraint_name(item: &str) -> (String, &str) {
    let trimmed = item.trim();
    let upper = trimmed.to_ascii_uppercase();
    if !upper.starts_with("CONSTRAINT ") {
        return (String::new(), trimmed);
    }
    let rest = trimmed["CONSTRAINT ".len()..].trim_start();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim().to_string();
    let tail = parts.next().unwrap_or("").trim_start();
    (ddl_strip_identifier(&name), tail)
}

/// 提取 `ON DELETE` / `ON UPDATE` 动作子句（动作 + 可选关键词，最多两段）。
pub fn ddl_parse_action_clause(item: &str, keyword: &str) -> Option<String> {
    let upper = item.to_ascii_uppercase();
    let start = upper.find(keyword)?;
    let rest = item[start + keyword.len()..].trim_start();
    let end = rest.to_ascii_uppercase().find(" ON ").unwrap_or(rest.len());
    let clause = rest[..end].trim();
    if clause.is_empty() {
        None
    } else {
        Some(
            clause
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

/// 解析 `CREATE [UNIQUE] INDEX` 语句为索引元数据。
pub fn parse_ddl_index_sql(name: &str, sql: &str) -> Option<DbFirstIndex> {
    let upper = sql.to_ascii_uppercase();
    let unique = upper.starts_with("CREATE UNIQUE INDEX");
    let on_idx = upper.find(" ON ")?;
    let before_on = sql[..on_idx].trim();
    let index_name = before_on
        .split_whitespace()
        .last()
        .map(ddl_strip_identifier)
        .unwrap_or_else(|| name.to_string());
    let columns = ddl_parenthesized_list(&sql[on_idx..]);
    if columns.is_empty() {
        return None;
    }
    Some(DbFirstIndex {
        name: index_name,
        columns: columns
            .into_iter()
            .map(|column| DbFirstIndexColumn {
                name: column,
                descending: false,
            })
            .collect(),
        unique,
    })
}

/// 从 `CREATE TABLE` 语句提取表内 UNIQUE 约束（含内联唯一列）。
pub fn parse_ddl_unique_indexes(create_sql: &str) -> Vec<DbFirstIndex> {
    let mut indexes = Vec::new();
    for item in ddl_table_items(create_sql) {
        let upper = item.to_ascii_uppercase();
        if !upper.contains("UNIQUE") || upper.contains("FOREIGN KEY") {
            continue;
        }
        let (name, rest) = ddl_parse_constraint_name(&item);
        let rest_upper = rest.to_ascii_uppercase();
        let columns = if rest_upper.starts_with("UNIQUE") {
            ddl_parenthesized_list(rest)
        } else if let Some(first) = item.split_whitespace().next() {
            vec![ddl_strip_identifier(first)]
        } else {
            Vec::new()
        };
        if columns.is_empty() {
            continue;
        }
        indexes.push(DbFirstIndex {
            name,
            columns: columns
                .into_iter()
                .map(|column| DbFirstIndexColumn {
                    name: column,
                    descending: false,
                })
                .collect(),
            unique: true,
        });
    }
    indexes
}

/// 从 `CREATE TABLE` 语句提取外键约束（复合外键按列展开）。
pub fn parse_ddl_foreign_keys(create_sql: &str) -> Vec<DbFirstForeignKey> {
    let mut foreign_keys = Vec::new();
    for item in ddl_table_items(create_sql) {
        let upper = item.to_ascii_uppercase();
        if !upper.contains("FOREIGN KEY") {
            continue;
        }
        let (name, rest) = ddl_parse_constraint_name(&item);
        let rest_upper = rest.to_ascii_uppercase();
        let Some(foreign_idx) = rest_upper.find("FOREIGN KEY") else {
            continue;
        };
        let local_cols = ddl_parenthesized_list(&rest[foreign_idx + "FOREIGN KEY".len()..]);
        let Some(references_idx) = rest_upper.find("REFERENCES") else {
            continue;
        };
        let after_references = rest[references_idx + "REFERENCES".len()..].trim_start();
        let ref_table = after_references
            .split_once('(')
            .map(|(table, _)| ddl_strip_identifier(table.trim()))
            .unwrap_or_default();
        let ref_cols = ddl_parenthesized_list(after_references);
        let on_delete = ddl_parse_action_clause(&item, "ON DELETE");
        let on_update = ddl_parse_action_clause(&item, "ON UPDATE");
        for (column, ref_column) in local_cols.into_iter().zip(ref_cols.into_iter()) {
            foreign_keys.push(DbFirstForeignKey {
                name: (!name.is_empty()).then_some(name.clone()),
                column,
                ref_schema: None,
                ref_table: ref_table.clone(),
                ref_column,
                on_delete: on_delete.clone(),
                on_update: on_update.clone(),
            });
        }
    }
    foreign_keys
}

/// 比对模型声明与数据库内省结果：唯一约束、普通索引与外键（含动作）。
/// SQLite 与 DuckDB 共用，方言差异（表名规范化）经 `db_type` 透传。
pub fn validate_table_constraints<T: Model>(
    db_type: DbType,
    actual: &DbFirstTable,
) -> crate::Result<()> {
    let mut expected_unique = std::collections::BTreeMap::<i32, Vec<&str>>::new();
    let mut expected_indexes = std::collections::BTreeMap::<i32, Vec<&str>>::new();
    let mut next_index_group = i32::MIN;
    for column in T::COLUMN_SCHEMA {
        if let Some(group) = column.unique_group {
            expected_unique.entry(group).or_default().push(column.name);
        }
        if column.is_indexed {
            let group = column.index_group.unwrap_or_else(|| {
                let group = next_index_group;
                next_index_group += 1;
                group
            });
            expected_indexes.entry(group).or_default().push(column.name);
        }
    }

    let actual_unique = actual
        .indexes
        .iter()
        .filter(|index| index.unique)
        .map(|index| {
            index
                .columns
                .iter()
                .map(|column| {
                    column
                        .name
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches(['"', '`', '[', ']'])
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for columns in expected_unique.values() {
        if !actual_unique.iter().any(|actual| {
            actual.len() == columns.len()
                && actual
                    .iter()
                    .zip(columns)
                    .all(|(actual, expected)| actual == expected)
        }) {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Unique constraint mismatch for columns ({})",
                T::TABLE_NAME,
                columns.join(", ")
            ));
        }
    }
    if actual_unique.len() != expected_unique.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Unique constraint count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_unique.len(),
            actual_unique.len()
        ));
    }

    let actual_indexes = actual
        .indexes
        .iter()
        .filter(|index| !index.unique)
        .map(|index| {
            index
                .columns
                .iter()
                .map(|column| {
                    column
                        .name
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches(['"', '`', '[', ']'])
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for columns in expected_indexes.values() {
        if !actual_indexes.iter().any(|actual| {
            actual.len() == columns.len()
                && actual
                    .iter()
                    .zip(columns)
                    .all(|(actual, expected)| actual == expected)
        }) {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Index mismatch for columns ({})",
                T::TABLE_NAME,
                columns.join(", ")
            ));
        }
    }
    if actual_indexes.len() != expected_indexes.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Index count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_indexes.len(),
            actual_indexes.len()
        ));
    }

    let expected_foreign_keys = T::COLUMN_SCHEMA
        .iter()
        .filter_map(|column| {
            column
                .foreign_key
                .as_ref()
                .map(|foreign_key| (column.name, foreign_key))
        })
        .collect::<Vec<_>>();
    if actual.foreign_keys.len() != expected_foreign_keys.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Foreign key count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_foreign_keys.len(),
            actual.foreign_keys.len()
        ));
    }
    let action_matches = |expected: Option<crate::model::ForeignKeyAction>,
                          actual: Option<&str>| {
        expected.map_or(true, |expected| {
            actual.is_some_and(|actual| actual.eq_ignore_ascii_case(expected.as_sql()))
        })
    };
    for (column, expected) in expected_foreign_keys {
        let ref_column = expected.get_ref_column();
        let ref_table = crate::model::normalize_table_name_for_db(db_type, expected.ref_table);
        let found = actual.foreign_keys.iter().any(|foreign_key| {
            foreign_key.column == column
                && foreign_key.ref_table == ref_table
                && foreign_key.ref_column == ref_column
                && action_matches(expected.on_delete, foreign_key.on_delete.as_deref())
                && action_matches(expected.on_update, foreign_key.on_update.as_deref())
        });
        if !found {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Foreign key mismatch for '{}'",
                T::TABLE_NAME,
                column
            ));
        }
    }
    Ok(())
}

/// 关联/多表查询的同谓词行数统计（SQLite/DuckDB 共用）：
/// `SELECT COUNT(*) FROM (<原子查询>)`，排序与分页已由
/// to_count_sql_with_params 剥离，count 不受其影响。
///
/// 后端差异点经参数透传：`$db_type`（DbType 常量）、`$query`（traced 查询
/// 入口）、`$params_fn`（参数转换）、`$value`（行值枚举别名）、
/// `$trace_label`（trace 标签）。
#[cfg(any(feature = "sqlite", feature = "duckdb"))]
macro_rules! impl_backend_related_count {
    ($executor:ident, [$($g:tt)*], [$($ty:tt)*], $db_type:expr, $query:ident, $params_fn:ident, $value:ident, $trace_label:expr) => {
        impl<$($g)*> $executor<$($ty)*> {
            /// 统计同谓词总行数（列表分页 total_count 用）。
            pub async fn count(self) -> crate::Result<i64> {
                let (sql, params) = self.select.to_count_sql_with_params($db_type);
                let backend_params = $params_fn(&params)?;
                let mut rows = if backend_params.is_empty() {
                    $query(&self.conn, &sql, (), &params).await?
                } else {
                    $query(&self.conn, &sql, backend_params, &params).await?
                };
                if let Some(row) = rows.next().trace().await? {
                    match row.get_value(0).trace_for($trace_label)? {
                        $value::Integer(i) => Ok(i),
                        $value::Real(r) => Ok(r as i64),
                        $value::Text(t) => t.parse::<i64>().map_err(|e| {
                            crate::ormer_error!("Invalid COUNT value: {t} ({e})")
                        }),
                        _ => Err(crate::ormer_error!("COUNT returned a non-numeric value")),
                    }
                } else {
                    Ok(0)
                }
            }
        }
    };
}
#[cfg(any(feature = "sqlite", feature = "duckdb"))]
pub(crate) use impl_backend_related_count;
