use crate::SingleSqlStatement;
use crate::SqlStatement;
use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::Value;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct RawSql {
    sql: String,
    params: Vec<RawSqlParam>,
    parse_placeholders: bool,
}

#[derive(Debug, Clone)]
struct RawSqlParam {
    name: Option<String>,
    value: Value,
}

pub trait IntoRawSql {
    fn into_raw_sql(self) -> RawSql;
}

pub fn sql(sql: impl Into<String>) -> RawSql {
    RawSql::new(sql)
}

impl RawSql {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
            parse_placeholders: true,
        }
    }

    pub fn plain(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
            parse_placeholders: false,
        }
    }

    pub fn bind(mut self, value: impl Into<Value>) -> Self {
        self.params.push(RawSqlParam {
            name: None,
            value: value.into(),
        });
        self
    }

    pub fn bind_named(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.params.push(RawSqlParam {
            name: Some(name.into()),
            value: value.into(),
        });
        self
    }

    pub fn to_statement(&self, db_type: DbType) -> crate::Result<SqlStatement> {
        let (sql, params) = self.render(db_type)?;
        Ok(SqlStatement::batch(
            db_type,
            vec![SingleSqlStatement::new(sql, params)],
        ))
    }

    pub fn render(&self, db_type: DbType) -> crate::Result<(String, Vec<Value>)> {
        if !self.parse_placeholders {
            return Ok((
                self.sql.clone(),
                self.params
                    .iter()
                    .map(|param| param.value.clone())
                    .collect(),
            ));
        }

        let mut out = String::with_capacity(self.sql.len());
        let bytes = self.sql.as_bytes();
        let mut i = 0;
        let mut positional_idx = 0usize;
        let mut used_named = HashSet::new();
        let mut params = Vec::new();

        while i < bytes.len() {
            if starts_with(bytes, i, b"--") {
                i = copy_line_comment(&self.sql, i, &mut out);
            } else if starts_with(bytes, i, b"/*") {
                i = copy_block_comment(&self.sql, i, &mut out);
            } else if bytes[i] == b'\'' {
                i = copy_quoted(&self.sql, i, b'\'', &mut out);
            } else if bytes[i] == b'"' {
                i = copy_quoted(&self.sql, i, b'"', &mut out);
            } else if bytes[i] == b'`' {
                i = copy_quoted(&self.sql, i, b'`', &mut out);
            } else if bytes[i] == b'[' {
                i = copy_bracket_identifier(&self.sql, i, &mut out);
            } else if bytes[i] == b'$' {
                if let Some(next) = copy_dollar_quoted(&self.sql, i, &mut out) {
                    i = next;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            } else if bytes[i] == b'{' {
                if let Some((name, next)) = parse_braced_param(&self.sql, i) {
                    let value = match name {
                        Some(name) => {
                            used_named.insert(name.clone());
                            self.named_value(&name)?.clone()
                        }
                        None => {
                            let value = self.positional_value(positional_idx)?.clone();
                            positional_idx += 1;
                            value
                        }
                    };
                    params.push(value);
                    out.push_str(&placeholder(db_type, params.len()));
                    i = next;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            } else if bytes[i] == b':' {
                if let Some((name, next)) = parse_colon_param(&self.sql, i) {
                    used_named.insert(name.clone());
                    let value = self.named_value(&name)?.clone();
                    params.push(value);
                    out.push_str(&placeholder(db_type, params.len()));
                    i = next;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }

        if positional_idx < self.positional_param_count() {
            return Err(crate::ormer_error!(
                "Unused positional raw SQL parameter at index {}",
                positional_idx
            ));
        }
        for param in self.params.iter().filter_map(|param| param.name.as_ref()) {
            if !used_named.contains(param) {
                return Err(crate::ormer_error!(
                    "Unused named raw SQL parameter: {}",
                    param
                ));
            }
        }

        Ok((out, params))
    }

    fn named_value(&self, name: &str) -> crate::Result<&Value> {
        self.params
            .iter()
            .find(|param| param.name.as_deref() == Some(name))
            .map(|param| &param.value)
            .ok_or_else(|| crate::ormer_error!("Missing raw SQL parameter: {}", name))
    }

    fn positional_value(&self, index: usize) -> crate::Result<&Value> {
        self.params
            .iter()
            .filter(|param| param.name.is_none())
            .nth(index)
            .map(|param| &param.value)
            .ok_or_else(|| {
                crate::ormer_error!("Missing raw SQL positional parameter {}", index + 1)
            })
    }

    fn positional_param_count(&self) -> usize {
        self.params
            .iter()
            .filter(|param| param.name.is_none())
            .count()
    }
}

impl IntoRawSql for RawSql {
    fn into_raw_sql(self) -> RawSql {
        self
    }
}

impl<T: AsRef<str>> IntoRawSql for T {
    fn into_raw_sql(self) -> RawSql {
        RawSql::plain(self.as_ref())
    }
}

fn starts_with(bytes: &[u8], idx: usize, pat: &[u8]) -> bool {
    bytes.get(idx..idx + pat.len()) == Some(pat)
}

fn copy_line_comment(sql: &str, start: usize, out: &mut String) -> usize {
    let bytes = sql.as_bytes();
    let mut end = start + 2;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    if end < bytes.len() {
        end += 1;
    }
    out.push_str(&sql[start..end]);
    end
}

fn copy_block_comment(sql: &str, start: usize, out: &mut String) -> usize {
    let bytes = sql.as_bytes();
    let mut end = start + 2;
    while end + 1 < bytes.len() && !starts_with(bytes, end, b"*/") {
        end += 1;
    }
    end = (end + 2).min(bytes.len());
    out.push_str(&sql[start..end]);
    end
}

fn copy_quoted(sql: &str, start: usize, quote: u8, out: &mut String) -> usize {
    let bytes = sql.as_bytes();
    let mut end = start + 1;
    while end < bytes.len() {
        if bytes[end] == quote {
            if quote != b'`' && end + 1 < bytes.len() && bytes[end + 1] == quote {
                end += 2;
                continue;
            }
            end += 1;
            break;
        }
        end += 1;
    }
    out.push_str(&sql[start..end]);
    end
}

fn copy_bracket_identifier(sql: &str, start: usize, out: &mut String) -> usize {
    let bytes = sql.as_bytes();
    let mut end = start + 1;
    while end < bytes.len() && bytes[end] != b']' {
        end += 1;
    }
    end = (end + 1).min(bytes.len());
    out.push_str(&sql[start..end]);
    end
}

fn copy_dollar_quoted(sql: &str, start: usize, out: &mut String) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut tag_end = start + 1;
    while tag_end < bytes.len() && is_ident_continue(bytes[tag_end]) {
        tag_end += 1;
    }
    if tag_end >= bytes.len() || bytes[tag_end] != b'$' {
        return None;
    }

    let tag = &sql[start..=tag_end];
    let search_start = tag_end + 1;
    let closing = sql[search_start..]
        .find(tag)
        .map(|offset| search_start + offset + tag.len())
        .unwrap_or(bytes.len());
    out.push_str(&sql[start..closing]);
    Some(closing)
}

fn parse_braced_param(sql: &str, start: usize) -> Option<(Option<String>, usize)> {
    let bytes = sql.as_bytes();
    let mut end = start + 1;
    while end < bytes.len() && bytes[end] != b'}' {
        end += 1;
    }
    if end >= bytes.len() {
        return None;
    }
    let name = sql[start + 1..end].trim();
    if name.is_empty() {
        return Some((None, end + 1));
    }
    is_valid_ident(name).then(|| (Some(name.to_string()), end + 1))
}

fn parse_colon_param(sql: &str, start: usize) -> Option<(String, usize)> {
    let bytes = sql.as_bytes();
    if start > 0 && bytes[start - 1] == b':' {
        return None;
    }
    if bytes.get(start + 1) == Some(&b':') {
        return None;
    }
    let first = *bytes.get(start + 1)?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = start + 2;
    while end < bytes.len() && is_ident_continue(bytes[end]) {
        end += 1;
    }
    Some((sql[start + 1..end].to_string(), end))
}

fn is_valid_ident(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.first().is_some_and(|first| is_ident_start(*first))
        && bytes.iter().skip(1).all(|byte| is_ident_continue(*byte))
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

#[macro_export]
macro_rules! sql {
    ($sql:expr $(,)?) => {
        $crate::RawSql::new($sql)
    };
    ($sql:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let raw = $crate::RawSql::new($sql);
        $(
            let raw = raw.bind_named(stringify!($name), $value);
        )+
        raw
    }};
}

#[cfg(all(
    test,
    any(feature = "sqlite", feature = "postgresql", feature = "mssql")
))]
mod tests {
    use super::*;

    #[cfg(feature = "sqlite")]
    #[test]
    fn renders_named_params_and_skips_sql_literals() {
        let raw = RawSql::new(
            "SELECT ':name', col FROM users \
             WHERE name = {name} AND note = :note \
             AND jsonb ? 'key' -- :comment\n\
             AND body = $$ {name} :note $$",
        )
        .bind_named("name", "Alice".to_string())
        .bind_named("note", "hello".to_string());

        let (sql, params) = raw.render(DbType::Sqlite).unwrap();
        assert!(sql.contains("name = ? AND note = ?"));
        assert!(sql.contains("':name'"));
        assert!(sql.contains("-- :comment"));
        assert!(sql.contains("$$ {name} :note $$"));
        assert_eq!(params.len(), 2);
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn renders_postgres_placeholders() {
        let raw = RawSql::new("SELECT * FROM users WHERE id = {} AND name = {name}")
            .bind(1)
            .bind_named("name", "Alice".to_string());

        let (sql, params) = raw.render(DbType::PostgreSQL).unwrap();
        assert_eq!(sql, "SELECT * FROM users WHERE id = $1 AND name = $2");
        assert_eq!(params.len(), 2);
    }

    #[cfg(feature = "mssql")]
    #[test]
    fn renders_mssql_placeholders() {
        let raw = RawSql::new("SELECT * FROM users WHERE id = {} AND name = {name}")
            .bind(1)
            .bind_named("name", "Alice".to_string());

        let (sql, params) = raw.render(DbType::MSSQL).unwrap();
        assert_eq!(sql, "SELECT * FROM users WHERE id = @P1 AND name = @P2");
        assert_eq!(params.len(), 2);
    }

    #[cfg(feature = "postgresql")]
    #[test]
    fn plain_sql_keeps_legacy_colon_text() {
        let raw = RawSql::plain("SELECT 'x'::text, :legacy");
        let (sql, params) = raw.render(DbType::PostgreSQL).unwrap();
        assert_eq!(sql, "SELECT 'x'::text, :legacy");
        assert!(params.is_empty());
    }
}
