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

    pub fn migration(message: impl Into<String>) -> Self {
        Self::Migration {
            message: message.into(),
        }
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
        Self::from_external("tokio_postgres::Error", error)
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
        Self::from_external("mysql_async::Error", error)
    }
}

#[cfg(feature = "mssql")]
impl From<tiberius::error::Error> for OrmerError {
    fn from(error: tiberius::error::Error) -> Self {
        Self::from_external("tiberius::error::Error", error)
    }
}

fn backend_name(backend: DbType) -> &'static str {
    match backend {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => "SQLite",
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => "PostgreSQL",
        #[cfg(feature = "mysql")]
        DbType::MySQL => "MySQL",
        #[cfg(feature = "mssql")]
        DbType::MSSQL => "MSSQL",
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
    let lower = message.to_ascii_lowercase();
    let code = extract_code(message);
    let kind = if matches!(
        code.as_deref(),
        Some("23505") | Some("1062") | Some("2601") | Some("2627")
    ) || lower.contains("unique constraint failed")
        || lower.contains("duplicate key")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::Unique)
    } else if matches!(code.as_deref(), Some("23503") | Some("1451") | Some("1452"))
        || lower.contains("foreign key constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::ForeignKey)
    } else if matches!(code.as_deref(), Some("23502") | Some("1048") | Some("515"))
        || lower.contains("not null constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::NotNull)
    } else if matches!(code.as_deref(), Some("23514") | Some("3819") | Some("4025"))
        || lower.contains("check constraint failed")
    {
        DatabaseErrorKind::Constraint(ConstraintKind::Check)
    } else if matches!(code.as_deref(), Some("40001")) || lower.contains("serialization failure") {
        DatabaseErrorKind::SerializationFailure
    } else if matches!(code.as_deref(), Some("40P01") | Some("1213")) || lower.contains("deadlock")
    {
        DatabaseErrorKind::Deadlock
    } else if matches!(code.as_deref(), Some("1205") | Some("1222")) || lower.contains("timeout") {
        DatabaseErrorKind::Timeout
    } else if lower.contains("connection") {
        DatabaseErrorKind::Connection
    } else if lower.contains("constraint") {
        DatabaseErrorKind::Constraint(ConstraintKind::Other)
    } else {
        DatabaseErrorKind::Other
    };
    (kind, code)
}

fn extract_code(message: &str) -> Option<String> {
    message
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find(|word| word.len() == 5 && word.chars().all(|character| character.is_ascii_digit()))
        .map(str::to_string)
}

fn extract_constraint(message: &str) -> Option<String> {
    let marker = "constraint \"";
    let start = message.find(marker)? + marker.len();
    let end = message[start..].find('"')? + start;
    Some(message[start..end].to_string())
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
