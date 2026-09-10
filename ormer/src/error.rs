use crate::abstract_layer::DbType;
use std::error::Error;
use std::fmt;

pub type Result<T> = std::result::Result<T, OrmerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    Unique,
    ForeignKey,
    NotNull,
    Check,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseErrorKind {
    Constraint(ConstraintKind),
    SerializationFailure,
    Deadlock,
    Timeout,
    Connection,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrmerError {
    OptimisticLock {
        table: &'static str,
        column: &'static str,
    },
    Database {
        backend: DbType,
        kind: DatabaseErrorKind,
        code: Option<String>,
        constraint: Option<String>,
        message: String,
    },
    Decode {
        column: Option<String>,
        rust_type: Option<&'static str>,
        message: String,
    },
    Migration {
        message: String,
    },
    UnmigratableSchema {
        table: String,
        message: String,
    },
    Pool {
        backend: DbType,
        message: String,
    },
    Transaction {
        backend: DbType,
        message: String,
    },
    UnsupportedFeature {
        backend: DbType,
        feature: &'static str,
    },
    InvalidOperation {
        message: String,
    },
    Other {
        message: String,
    },
}

impl OrmerError {
    pub fn optimistic_lock(table: &'static str, column: &'static str) -> Self {
        Self::OptimisticLock { table, column }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            message: message.into(),
        }
    }

    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode {
            column: None,
            rust_type: None,
            message: message.into(),
        }
    }

    /// 带定位信息的解码错误（列名 + 目标 Rust 类型）。
    ///
    /// 供行/列解码失败路径迁移使用：相比裸 [`Self::decode`]，调用方应尽量
    /// 传入列名与 `std::any::type_name` 等类型信息，避免解码错误退化为
    /// 无定位的 [`Self::Other`]（`ormer_error!`）。
    pub fn decode_at(
        column: impl Into<String>,
        rust_type: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::Decode {
            column: Some(column.into()),
            rust_type: Some(rust_type),
            message: message.into(),
        }
    }

    pub fn migration(message: impl Into<String>) -> Self {
        Self::Migration {
            message: message.into(),
        }
    }

    /// 自动迁移无法就地演进的 schema（如主键变更、缺省值的非空新列）。
    /// 调用方通常以此决定是否删表重建；与普通迁移失败区分开。
    pub fn unmigratable_schema(table: impl Into<String>, message: impl Into<String>) -> Self {
        Self::UnmigratableSchema {
            table: table.into(),
            message: message.into(),
        }
    }

    pub fn is_unmigratable_schema(&self) -> bool {
        matches!(self, Self::UnmigratableSchema { .. })
    }

    pub fn invalid_operation(message: impl Into<String>) -> Self {
        Self::InvalidOperation {
            message: message.into(),
        }
    }

    pub fn context(self, context: impl fmt::Display) -> Self {
        Self::Other {
            message: format!("{context}: {self}"),
        }
    }

    pub fn is_unique_violation(&self, constraint: &str) -> bool {
        matches!(
            self,
            Self::Database {
                kind: DatabaseErrorKind::Constraint(ConstraintKind::Unique),
                constraint: Some(error_constraint),
                ..
            } if error_constraint == constraint
        )
    }

    pub fn is_unique_violation_any(&self) -> bool {
        matches!(
            self,
            Self::Database {
                kind: DatabaseErrorKind::Constraint(ConstraintKind::Unique),
                ..
            }
        )
    }

    pub fn is_retryable_transaction_error(&self) -> bool {
        matches!(
            self,
            Self::Database {
                kind: DatabaseErrorKind::SerializationFailure | DatabaseErrorKind::Deadlock,
                ..
            }
        )
    }

    /// 后端边界用：把驱动结构化错误码（PostgreSQL SQLSTATE、MySQL
    /// `ServerError::code`、MSSQL `Error::code`）填入 `Database` 变体，并按
    /// 结构化码优先重新分类；文本启发式仅兜底。非 `Database` 变体原样返回。
    ///
    /// 结构化码能纠正文本提取的系统性误判：如 MySQL 错误消息内嵌
    /// `ERROR 1213 (40001)`，文本提取会取到 SQLSTATE `40001` 而误分类为
    /// serialization failure，驱动码 `1213` 才是精确的死锁语义。
    #[cfg_attr(
        not(any(feature = "postgresql", feature = "mysql", feature = "mssql")),
        allow(dead_code)
    )]
    pub(crate) fn with_driver_code(self, driver_code: Option<String>) -> Self {
        match (self, driver_code) {
            (
                Self::Database {
                    backend,
                    constraint,
                    message,
                    ..
                },
                Some(driver_code),
            ) => {
                let lower = message.to_ascii_lowercase();
                let kind = classify_for_code(Some(driver_code.as_str()), &lower);
                Self::Database {
                    backend,
                    kind,
                    code: Some(driver_code),
                    constraint,
                    message,
                }
            }
            (error, _) => error,
        }
    }

    pub(crate) fn from_external<E: Error>(context: &str, error: E) -> Self {
        let message = external_error_message(context, &error);
        let type_name = std::any::type_name::<E>();

        #[cfg(feature = "sqlite")]
        if type_name.contains("turso") {
            return Self::database(DbType::Sqlite, message);
        }
        #[cfg(feature = "postgresql")]
        if type_name.contains("tokio_postgres") || type_name.contains("bb8_postgres") {
            return Self::database(DbType::PostgreSQL, message);
        }
        #[cfg(feature = "mysql")]
        if type_name.contains("mysql_async") {
            return Self::database(DbType::MySQL, message);
        }
        #[cfg(feature = "mssql")]
        if type_name.contains("tiberius") {
            return Self::database(DbType::MSSQL, message);
        }
        #[cfg(feature = "clickhouse")]
        if type_name.contains("clickhouse") {
            return Self::database(DbType::ClickHouse, message);
        }
        #[cfg(feature = "influxdb")]
        if type_name.contains("reqwest") {
            return Self::database(DbType::InfluxDB, message);
        }
        #[cfg(feature = "duckdb")]
        if type_name.contains("duckdb") || type_name.contains("duckcompat") {
            return Self::database(DbType::DuckDB, message);
        }

        Self::other(message)
    }

    fn database(backend: DbType, message: String) -> Self {
        let (kind, code) = classify_database_error(&message);
        Self::Database {
            backend,
            kind,
            code,
            constraint: extract_constraint(&message),
            message,
        }
    }
}

impl fmt::Display for OrmerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OptimisticLock { table, column } => {
                write!(formatter, "optimistic lock conflict on {table}.{column}")
            }
            Self::Database {
                backend,
                kind,
                code,
                constraint,
                message,
            } => {
                write!(
                    formatter,
                    "{} {}",
                    backend_name(*backend),
                    database_error_kind_name(*kind)
                )?;
                if let Some(constraint) = constraint {
                    write!(formatter, " constraint {constraint}")?;
                }
                if let Some(code) = code {
                    write!(formatter, " ({code})")?;
                }
                write!(formatter, ": {message}")
            }
            Self::Decode {
                column,
                rust_type,
                message,
            } => {
                formatter.write_str("decode error")?;
                if let Some(column) = column {
                    write!(formatter, " for column {column}")?;
                }
                if let Some(rust_type) = rust_type {
                    write!(formatter, " as {rust_type}")?;
                }
                write!(formatter, ": {message}")
            }
            Self::Migration { message } => write!(formatter, "migration error: {message}"),
            Self::UnmigratableSchema { table, message } => {
                write!(formatter, "unmigratable schema for table {table}: {message}")
            }
            Self::Pool { backend, message } => {
                write!(
                    formatter,
                    "{} connection pool error: {message}",
                    backend_name(*backend)
                )
            }
            Self::Transaction { backend, message } => {
                write!(
                    formatter,
                    "{} transaction error: {message}",
                    backend_name(*backend)
                )
            }
            Self::UnsupportedFeature { backend, feature } => {
                write!(
                    formatter,
                    "{} does not support {feature}",
                    backend_name(*backend)
                )
            }
            Self::InvalidOperation { message } | Self::Other { message } => {
                formatter.write_str(message)
            }
        }
    }
}

impl Error for OrmerError {}

impl From<chrono::ParseError> for OrmerError {
    fn from(error: chrono::ParseError) -> Self {
        Self::decode(error.to_string())
    }
}

impl From<serde_json::Error> for OrmerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode {
            column: None,
            rust_type: Some("serde_json::Value"),
            message: error.to_string(),
        }
    }
}

impl From<std::io::Error> for OrmerError {
    fn from(error: std::io::Error) -> Self {
        Self::other(error.to_string())
    }
}

impl From<std::num::TryFromIntError> for OrmerError {
    fn from(error: std::num::TryFromIntError) -> Self {
        Self::decode(error.to_string())
    }
}

#[cfg(feature = "sqlite")]
impl From<turso::Error> for OrmerError {
    fn from(error: turso::Error) -> Self {
        Self::from_external("turso::Error", error)
    }
}

#[cfg(feature = "postgresql")]
impl From<tokio_postgres::Error> for OrmerError {
    fn from(error: tokio_postgres::Error) -> Self {
        // 结构化 SQLSTATE 优先（from_external 内的文本提取仅兜底）
        let code = error.code().map(|state| state.code().to_string());
        Self::from_external("tokio_postgres::Error", error).with_driver_code(code)
    }
}

#[cfg(feature = "postgresql")]
impl From<bb8::RunError<tokio_postgres::Error>> for OrmerError {
    fn from(error: bb8::RunError<tokio_postgres::Error>) -> Self {
        Self::Pool {
            backend: DbType::PostgreSQL,
            message: error.to_string(),
        }
    }
}

#[cfg(feature = "mysql")]
impl From<mysql_async::Error> for OrmerError {
    fn from(error: mysql_async::Error) -> Self {
        // MySQL 厂商码（1062 唯一冲突、1213 死锁等）比消息内嵌的 SQLSTATE
        // 更精确，从 ServerError 结构化提取（文本提取仅兜底）
        let code = match &error {
            mysql_async::Error::Server(server) => Some(server.code.to_string()),
            _ => None,
        };
        Self::from_external("mysql_async::Error", error).with_driver_code(code)
    }
}

#[cfg(feature = "mssql")]
impl From<tiberius::error::Error> for OrmerError {
    fn from(error: tiberius::error::Error) -> Self {
        // SQL Server 错误码（2601/2627 唯一冲突、1205 死锁等）结构化提取
        let code = error.code().map(|code| code.to_string());
        Self::from_external("tiberius::error::Error", error).with_driver_code(code)
    }
}

#[cfg(feature = "clickhouse")]
impl From<clickhouse::error::Error> for OrmerError {
    fn from(error: clickhouse::error::Error) -> Self {
        Self::from_external("clickhouse::error::Error", error)
    }
}

#[cfg(feature = "duckdb")]
impl From<crate::abstract_layer::duckdb_backend::duckcompat::Error> for OrmerError {
    fn from(error: crate::abstract_layer::duckdb_backend::duckcompat::Error) -> Self {
        Self::from_external("duckdb::Error", error)
    }
}

fn backend_name(backend: DbType) -> &'static str {
    match backend {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => "SQLite",
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => "PostgreSQL",
        #[cfg(feature = "questdb")]
        DbType::QuestDB => "QuestDB",
        #[cfg(feature = "mysql")]
        DbType::MySQL => "MySQL",
        #[cfg(feature = "mssql")]
        DbType::MSSQL => "MSSQL",
        #[cfg(feature = "duckdb")]
        DbType::DuckDB => "DuckDB",
        #[cfg(feature = "clickhouse")]
        DbType::ClickHouse => "ClickHouse",
        #[cfg(feature = "influxdb")]
        DbType::InfluxDB => "InfluxDB",
    }
}

fn database_error_kind_name(kind: DatabaseErrorKind) -> &'static str {
    match kind {
        DatabaseErrorKind::Constraint(ConstraintKind::Unique) => "unique constraint violation",
        DatabaseErrorKind::Constraint(ConstraintKind::ForeignKey) => "foreign key violation",
        DatabaseErrorKind::Constraint(ConstraintKind::NotNull) => "not-null violation",
        DatabaseErrorKind::Constraint(ConstraintKind::Check) => "check constraint violation",
        DatabaseErrorKind::Constraint(ConstraintKind::Other) => "constraint violation",
        DatabaseErrorKind::SerializationFailure => "serialization failure",
        DatabaseErrorKind::Deadlock => "deadlock",
        DatabaseErrorKind::Timeout => "timeout",
        DatabaseErrorKind::Connection => "connection error",
        DatabaseErrorKind::Other => "database error",
    }
}

fn classify_database_error(message: &str) -> (DatabaseErrorKind, Option<String>) {
    let code = extract_code(message);
    let lower = message.to_ascii_lowercase();
    (classify_for_code(code.as_deref(), &lower), code)
}

/// 按错误码优先分类，消息文本启发式仅兜底。
///
/// `code` 应为驱动结构化错误码（PG SQLSTATE / MySQL `1062` / MSSQL `2627`
/// 等，见 [`OrmerError::with_driver_code`]）；无结构化码时为文本提取值。
/// 文本兜底包含 MySQL 1062 的 "Duplicate entry" 文案。
fn classify_for_code(code: Option<&str>, lower: &str) -> DatabaseErrorKind {
    let kind = if matches!(
        code,
        Some("23505") | Some("1062") | Some("2601") | Some("2627")
    ) || lower.contains("unique constraint failed")
        || lower.contains("duplicate key")
        || lower.contains("duplicate entry")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::Unique)
    } else if matches!(code, Some("23503") | Some("1451") | Some("1452"))
        || lower.contains("foreign key constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::ForeignKey)
    } else if matches!(code, Some("23502") | Some("1048") | Some("515"))
        || lower.contains("not null constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::NotNull)
    } else if matches!(code, Some("23514") | Some("3819") | Some("4025"))
        || lower.contains("check constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::Check)
    } else if matches!(code, Some("40001")) || lower.contains("serialization failure") {
        DatabaseErrorKind::SerializationFailure
    } else if matches!(code, Some("40P01") | Some("1213")) || lower.contains("deadlock") {
        // 40P01（PG SQLSTATE，含字母）依赖 extract_code 的放宽规则按 code 命中；
        // 1213（MySQL 4 位厂商码）不会被提取为 code，由结构化码或消息文本兜底。
        DatabaseErrorKind::Deadlock
    } else if matches!(code, Some("1205") | Some("1222")) || lower.contains("timeout") {
        DatabaseErrorKind::Timeout
    } else if lower.contains("connection") {
        DatabaseErrorKind::Connection
    } else if lower.contains("constraint") {
        DatabaseErrorKind::Constraint(ConstraintKind::Other)
    } else {
        DatabaseErrorKind::Other
    };
    kind
}

/// 从错误消息中提取 SQLSTATE 风格的错误码：5 位、首字符为数字的
/// 字母数字 token（标准 SQLSTATE 为 5 字符且类别位为数字，如 `23505`、
/// PG 死锁 `40P01`；首字符为数字可排除普通英文单词）。
///
/// 注意：MySQL/MSSQL 的 4 位厂商错误码（`1062`、`1213` 等）不符合该形状，
/// 不会被提取，`classify_database_error` 中对它们的 code 匹配仅作兜底记录，
/// 实际依赖消息文本（"duplicate key"/"deadlock" 等）命中分类。
fn extract_code(message: &str) -> Option<String> {
    message
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find(|word| word.len() == 5 && word.as_bytes()[0].is_ascii_digit())
        .map(str::to_string)
}

/// 提取约束名：PostgreSQL `constraint "name"` 与 MySQL
/// `Duplicate entry 'x' for key 'table.constraint'`（MySQL 8 带 `table.`
/// 前缀，取最后的约束/索引名部分，与用户声明的索引名一致）两种格式。
fn extract_constraint(message: &str) -> Option<String> {
    let marker = "constraint \"";
    if let Some(start) = message.find(marker) {
        let start = start + marker.len();
        if let Some(end) = message[start..].find('"') {
            return Some(message[start..start + end].to_string());
        }
    }
    let marker = "for key '";
    if let Some(start) = message.find(marker) {
        let start = start + marker.len();
        if let Some(end) = message[start..].find('\'') {
            let key = &message[start..start + end];
            let name = key.rsplit('.').next().unwrap_or(key);
            return Some(name.to_string());
        }
    }
    None
}

fn external_error_message(context: &str, error: &dyn Error) -> String {
    let error_message = error.to_string();
    if looks_traced(&error_message) {
        return format!("{context}->{error}");
    }

    let mut deepest = error;
    while let Some(source) = deepest.source() {
        deepest = source;
    }

    if std::ptr::eq(deepest, error) {
        format!("{context} failed: {error}")
    } else {
        format!("{context} failed: {deepest}")
    }
}

fn looks_traced(message: &str) -> bool {
    let Some(prefix) = message.split(" failed: ").next() else {
        return false;
    };
    if prefix.is_empty() {
        return false;
    }
    prefix
        .split("->")
        .all(|part| part.split("::").all(is_ident_like))
}

fn is_ident_like(part: &str) -> bool {
    let mut chars = part.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L10：MySQL 1062 唯一冲突经纯消息文本分类为唯一约束违反，并从
    /// `for key '...'` 提取约束名（MySQL 8 带 `table.` 前缀时取末段，
    /// 与用户声明的索引名一致）。
    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_duplicate_entry_classified_as_unique_violation() {
        let error = OrmerError::database(
            DbType::MySQL,
            "ERROR 1062 (23000): Duplicate entry '1' for key 'u.PRIMARY'".to_string(),
        );
        assert!(error.is_unique_violation_any());
        assert!(error.is_unique_violation("PRIMARY"));
        let OrmerError::Database { constraint, .. } = &error else {
            panic!("expected Database error");
        };
        assert_eq!(constraint.as_deref(), Some("PRIMARY"));
    }

    /// L10：驱动结构化错误码路径（`mysql_async::ServerError` 的 1062），
    /// `From` impl 提取厂商码并覆盖消息内嵌的 SQLSTATE。
    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_server_error_code_1062_classified_as_unique_violation() {
        let server_error = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1062,
            state: "23000".to_string(),
            message: "Duplicate entry '1' for key 'u.PRIMARY'".to_string(),
        });
        let error = OrmerError::from(server_error);
        assert!(error.is_unique_violation_any());
        assert!(error.is_unique_violation("PRIMARY"));
        let OrmerError::Database {
            code, constraint, ..
        } = &error else {
            panic!("expected Database error");
        };
        assert_eq!(code.as_deref(), Some("1062"));
        assert_eq!(constraint.as_deref(), Some("PRIMARY"));
    }

    /// L10：PG SQLSTATE 23505——文本路径（"duplicate key" + `constraint "..."`）
    /// 与结构化码路径（消息无唯一冲突特征时仍按 23505 分类）均命中。
    #[cfg(feature = "postgresql")]
    #[test]
    fn postgres_unique_violation_classified_from_text_and_sqlstate() {
        let text_error = OrmerError::database(
            DbType::PostgreSQL,
            r#"duplicate key value violates unique constraint "users_email_key""#.to_string(),
        );
        assert!(text_error.is_unique_violation_any());
        assert!(text_error.is_unique_violation("users_email_key"));
        let OrmerError::Database { constraint, .. } = &text_error else {
            panic!("expected Database error");
        };
        assert_eq!(constraint.as_deref(), Some("users_email_key"));

        let structured = OrmerError::database(DbType::PostgreSQL, "update rejected on row".to_string())
            .with_driver_code(Some("23505".to_string()));
        assert!(structured.is_unique_violation_any());
        let OrmerError::Database { code, .. } = &structured else {
            panic!("expected Database error");
        };
        assert_eq!(code.as_deref(), Some("23505"));
    }

    /// L10：MSSQL 2601（唯一索引）/ 2627（唯一约束）结构化码分类，
    /// 含消息文本无任何唯一冲突特征、纯靠驱动码命中的用例。
    #[cfg(feature = "mssql")]
    #[test]
    fn mssql_unique_violation_codes_classified() {
        for code in ["2601", "2627"] {
            let error = OrmerError::database(
                DbType::MSSQL,
                "Cannot insert duplicate key row in object 'dbo.users' with unique index 'UQ_users_email'.".to_string(),
            )
            .with_driver_code(Some(code.to_string()));
            assert!(error.is_unique_violation_any(), "code {code}");
            let OrmerError::Database {
                code: extracted, ..
            } = &error else {
                panic!("expected Database error");
            };
            assert_eq!(extracted.as_deref(), Some(code));
        }

        let code_only = OrmerError::database(DbType::MSSQL, "insert rejected by server policy".to_string())
            .with_driver_code(Some("2627".to_string()));
        assert!(code_only.is_unique_violation_any());
    }

    /// SQLite 唯一冲突文本分类（默认 feature 组合即可运行）。
    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_unique_constraint_failed_classified_as_unique_violation() {
        let error = OrmerError::database(DbType::Sqlite, "UNIQUE constraint failed: users.email".to_string());
        assert!(error.is_unique_violation_any());
    }

    /// 反例：非 Database 变体不得误报为唯一冲突。
    #[test]
    fn non_database_errors_are_not_flagged() {
        assert!(!OrmerError::other("boom").is_unique_violation_any());
        assert!(!OrmerError::invalid_operation("bad arguments").is_unique_violation_any());
    }

    /// 反例：MySQL 1213 死锁消息内嵌 SQLSTATE 40001，文本路径判为
    /// serialization failure；驱动码 1213 优先时纠正为 Deadlock——
    /// 两种路径都不是唯一冲突。
    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_deadlock_is_not_unique_violation() {
        let text_error = OrmerError::database(
            DbType::MySQL,
            "ERROR 1213 (40001): Deadlock found when trying to get lock; try restarting transaction".to_string(),
        );
        assert!(!text_error.is_unique_violation_any());
        assert!(matches!(
            text_error,
            OrmerError::Database {
                kind: DatabaseErrorKind::SerializationFailure,
                ..
            }
        ));

        let corrected = text_error.with_driver_code(Some("1213".to_string()));
        assert!(!corrected.is_unique_violation_any());
        assert!(matches!(
            corrected,
            OrmerError::Database {
                kind: DatabaseErrorKind::Deadlock,
                ..
            }
        ));
    }

    /// 反例：PG 外键冲突不是唯一冲突。
    #[cfg(feature = "postgresql")]
    #[test]
    fn foreign_key_violation_is_not_unique_violation() {
        let error = OrmerError::database(
            DbType::PostgreSQL,
            r#"update or delete on table "users" violates foreign key constraint "orders_user_fkey""#.to_string(),
        );
        assert!(!error.is_unique_violation_any());
    }
}
