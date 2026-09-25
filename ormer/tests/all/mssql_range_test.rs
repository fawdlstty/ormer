#![cfg(feature = "mssql")]

use ormer::{DbType, Model, Select};

#[derive(Debug, Clone, Model)]
#[table = "mssql_range_users"]
struct MssqlRangeUser {
    #[primary]
    id: i32,
}

#[test]
fn open_mssql_range_keeps_offset_clause() {
    let (sql, _) = Select::<MssqlRangeUser>::new()
        .range(10..)
        .to_sql_with_params(DbType::MSSQL);

    assert!(sql.contains("OFFSET 10 ROWS"), "SQL: {sql}");
    assert!(!sql.contains("FETCH NEXT"), "SQL: {sql}");
}
