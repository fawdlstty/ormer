use ormer::{CompressionAlgorithm, Model};

#[derive(Debug, Model)]
#[table = "compression_users"]
struct CompressionUser {
    #[primary]
    id: i32,
    #[compress(lz4)]
    payload: String,
}

#[derive(Debug, Model)]
#[table = "compression_default_users"]
struct DefaultCompressionUser {
    #[primary]
    id: i32,
    #[compress]
    payload: String,
}

#[test]
fn compression_algorithm_is_part_of_model_schema() {
    let column = CompressionUser::column_schema()
        .into_iter()
        .find(|column| column.name == "payload")
        .expect("payload column");
    assert!(column.compress);
    assert_eq!(column.compression, Some(CompressionAlgorithm::Lz4));
}

#[test]
fn bare_compress_uses_postgresql_default_algorithm() {
    let column = DefaultCompressionUser::column_schema()
        .into_iter()
        .find(|column| column.name == "payload")
        .expect("payload column");
    assert_eq!(column.compression, Some(CompressionAlgorithm::Pglz));
}

#[cfg(feature = "postgresql")]
#[test]
fn compression_is_rendered_for_postgresql() {
    let postgres =
        ormer::generate_create_table_sql::<CompressionUser>(ormer::DbType::PostgreSQL).unwrap();
    assert!(postgres.contains("payload TEXT COMPRESSION lz4 NOT NULL"));
}

#[cfg(feature = "mysql")]
#[test]
fn compression_is_rendered_for_mysql() {
    let mysql = ormer::generate_create_table_sql::<CompressionUser>(ormer::DbType::MySQL).unwrap();
    assert!(mysql.ends_with("COMPRESSION='LZ4'"));
}

#[cfg(feature = "sqlite")]
#[test]
fn compression_is_ignored_for_sqlite() {
    let sqlite =
        ormer::generate_create_table_sql::<CompressionUser>(ormer::DbType::Sqlite).unwrap();
    assert!(!sqlite.contains("COMPRESSION"));
}
