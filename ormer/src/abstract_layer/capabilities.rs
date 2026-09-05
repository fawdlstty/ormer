use super::DbType;

/// Public capability boundary for database backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub transactions: bool,
    pub auto_increment: bool,
    pub dml_returning: bool,
    pub insert_conflict: bool,
    pub insert_ignore: bool,
    pub row_delete: bool,
    pub copy: bool,
    pub row_lock: bool,
    pub constraints: bool,
    pub schema_introspection: bool,
    pub advanced_grouping: bool,
}

impl Capabilities {
    pub const fn of(db_type: DbType) -> Self {
        match db_type {
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => Self {
                copy: false,
                row_lock: false,
                advanced_grouping: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => Self::full_oltp(),
            #[cfg(feature = "questdb")]
            DbType::QuestDB => Self {
                transactions: false,
                auto_increment: false,
                dml_returning: false,
                insert_conflict: false,
                insert_ignore: false,
                row_delete: false,
                copy: false,
                row_lock: false,
                constraints: false,
                schema_introspection: false,
                advanced_grouping: false,
            },
            #[cfg(feature = "mysql")]
            DbType::MySQL => Self {
                dml_returning: false,
                copy: false,
                advanced_grouping: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "mssql")]
            DbType::MSSQL => Self {
                dml_returning: false,
                copy: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => Self {
                row_lock: false,
                ..Self::full_oltp()
            },
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => Self {
                transactions: false,
                auto_increment: false,
                dml_returning: false,
                insert_conflict: false,
                insert_ignore: false,
                row_delete: false,
                row_lock: false,
                constraints: false,
                schema_introspection: false,
                ..Self::full_oltp()
            },
        }
    }

    const fn full_oltp() -> Self {
        Self {
            transactions: true,
            auto_increment: true,
            dml_returning: true,
            insert_conflict: true,
            insert_ignore: true,
            row_delete: true,
            copy: true,
            row_lock: true,
            constraints: true,
            schema_introspection: true,
            advanced_grouping: true,
        }
    }
}
