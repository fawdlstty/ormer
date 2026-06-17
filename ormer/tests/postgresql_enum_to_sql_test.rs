#![cfg(feature = "postgresql")]

use ormer::{Model, ModelEnum};
use ormer::model::DbBackendTypeMapper;
use postgres_types::{Format, ToSql, Type};

#[derive(Debug, Clone, ModelEnum, PartialEq)]
enum TestPgStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Model)]
#[table = "test_pg_enum_to_sql_users"]
struct TestPgEnumUser {
    #[primary(auto)]
    id: i32,
    status: TestPgStatus,
    note: Option<String>,
}

#[test]
fn model_enum_metadata_is_embedded_into_column_schema() {
    let status = TestPgEnumUser::COLUMN_SCHEMA
        .iter()
        .find(|column| column.name == "status")
        .expect("status column should exist");

    assert_eq!(status.enum_variants, Some(TestPgStatus::VARIANTS));
}

#[test]
fn postgresql_create_table_to_sql_uses_enum_type() {
    let sql = ormer::generate_create_table_sql_with_name::<TestPgEnumUser>(
        ormer::DbType::PostgreSQL,
        None,
    )
    .expect("create table sql should succeed");

    assert!(
        sql.contains("status test_pg_status NOT NULL"),
        "expected enum column type in CREATE TABLE statement: {sql}"
    );
    assert!(
        !sql.contains("status TEXT"),
        "status column should not fall back to TEXT: {sql}"
    );
    assert_eq!(
        <ormer::abstract_layer::postgresql_backend::PostgreSQLTypeMapper as DbBackendTypeMapper>::sql_type(
            "TestPgStatus",
            false,
            false,
            false,
            Some(TestPgStatus::VARIANTS),
        ),
        "test_pg_status NOT NULL"
    );
}

#[test]
fn postgres_enum_text_param_accepts_enum_type() {
    #[derive(Debug)]
    struct TestTextParam(String);

    impl ToSql for TestTextParam {
        fn to_sql(
            &self,
            ty: &Type,
            out: &mut bytes::BytesMut,
        ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
            <&str as ToSql>::to_sql(&self.0.as_str(), ty, out)
        }

        fn accepts(ty: &Type) -> bool {
            matches!(ty.kind(), postgres_types::Kind::Enum(_))
        }

        fn encode_format(&self, _ty: &Type) -> Format {
            Format::Text
        }

        postgres_types::to_sql_checked!();
    }

    let ty = Type::new(
        "test_pg_status".to_string(),
        42,
        postgres_types::Kind::Enum(vec!["Active".to_string(), "Disabled".to_string()]),
        "public".to_string(),
    );
    let mut out = bytes::BytesMut::new();
    let param = TestTextParam("Active".to_string());
    let is_null = param.to_sql_checked(&ty, &mut out).expect("enum text param should encode");

    assert!(matches!(is_null, postgres_types::IsNull::No));
    assert!(matches!(param.encode_format(&ty), Format::Text));
    assert_eq!(std::str::from_utf8(&out).unwrap(), "Active");
}
