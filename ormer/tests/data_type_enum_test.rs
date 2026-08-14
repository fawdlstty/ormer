use std::collections::HashMap;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum DataTypeStatus {
    Disabled = 0,
    Active = 1,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ormer::ModelEnum)]
enum SmallReprStatus {
    Disabled = 0,
    Active = 1,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_data_type_enum"]
struct DataTypeEnumModel {
    #[primary]
    id: i32,
    #[data_type(i32)]
    status: DataTypeStatus,
    #[data_type(Option<i32>)]
    optional_status: Option<DataTypeStatus>,
}

mod external {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(transparent)]
    pub struct WrappedStatus(pub u16);
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_data_type_model_enum_repr"]
struct DataTypeModelEnumReprModel {
    #[primary]
    id: i32,
    #[data_type(i32)]
    status: SmallReprStatus,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_data_type_optional_wrapped"]
struct DataTypeOptionalWrappedModel {
    #[primary]
    id: i32,
    #[data_type(Option<i32>)]
    status: Option<external::WrappedStatus>,
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_data_type_wrapped_i64"]
struct DataTypeWrappedI64Model {
    #[primary]
    id: i32,
    #[data_type(i64)]
    status: external::WrappedStatus,
}

#[test]
fn data_type_model_enum_repr_u16_uses_i32_values() -> ormer::Result<()> {
    let model = DataTypeModelEnumReprModel {
        id: 1,
        status: SmallReprStatus::Active,
    };
    let values = <DataTypeModelEnumReprModel as ormer::Model>::field_values(&model);
    match &values[1] {
        ormer::Value::Integer(value) => assert_eq!(*value, 1),
        other => panic!("expected integer status value, got {other:?}"),
    }

    let from_values = <DataTypeModelEnumReprModel as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(2),
        ormer::Value::Integer(0),
    ])?;
    assert_eq!(from_values.status, SmallReprStatus::Disabled);

    Ok(())
}

#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "test_data_type_wrapped"]
struct DataTypeWrappedModel {
    #[primary]
    id: i32,
    #[data_type(i32)]
    status: external::WrappedStatus,
}

#[test]
fn data_type_enum_without_field_type_uses_i32_values() -> ormer::Result<()> {
    let status_column = <DataTypeEnumModel as ormer::Model>::COLUMN_SCHEMA
        .iter()
        .find(|column| column.name == "status")
        .expect("status column should exist");
    assert_eq!(status_column.data_type, Some("i32"));
    assert_eq!(status_column.enum_variants, None);
    let optional_status_column = <DataTypeEnumModel as ormer::Model>::COLUMN_SCHEMA
        .iter()
        .find(|column| column.name == "optional_status")
        .expect("optional_status column should exist");
    assert_eq!(optional_status_column.data_type, Some("i32"));
    assert!(optional_status_column.is_nullable);

    let model = DataTypeEnumModel {
        id: 7,
        status: DataTypeStatus::Active,
        optional_status: Some(DataTypeStatus::Disabled),
    };
    let values = <DataTypeEnumModel as ormer::Model>::field_values(&model);
    assert_eq!(values.len(), 3);
    match &values[1] {
        ormer::Value::Integer(value) => assert_eq!(*value, 1),
        other => panic!("expected integer status value, got {other:?}"),
    }
    match &values[2] {
        ormer::Value::Integer(value) => assert_eq!(*value, 0),
        other => panic!("expected integer optional status value, got {other:?}"),
    }

    let from_values = <DataTypeEnumModel as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(8),
        ormer::Value::Integer(0),
        ormer::Value::Null,
    ])?;
    assert_eq!(from_values.id, 8);
    assert_eq!(from_values.status, DataTypeStatus::Disabled);
    assert_eq!(from_values.optional_status, None);

    let row = ormer::Row::new(HashMap::from([
        ("id".to_string(), ormer::Value::Integer(9)),
        ("status".to_string(), ormer::Value::Integer(1)),
        ("optional_status".to_string(), ormer::Value::Integer(0)),
    ]));
    let from_row = <DataTypeEnumModel as ormer::Model>::from_row(&row)?;
    assert_eq!(from_row.id, 9);
    assert_eq!(from_row.status, DataTypeStatus::Active);
    assert_eq!(from_row.optional_status, Some(DataTypeStatus::Disabled));

    Ok(())
}

#[test]
fn data_type_i32_where_uses_database_type_for_in_filter() -> ormer::Result<()> {
    let sql = ormer::Select::<DataTypeWrappedModel>::new()
        .filter(|w| w.status.is_in(&[1, 2, 3]))
        .to_sql();
    assert!(sql.contains("status IN"), "SQL: {sql}");
    assert!(
        sql.contains("$1") || sql.contains("?") || sql.contains("@P1"),
        "SQL: {sql}"
    );

    let model = DataTypeWrappedModel {
        id: 1,
        status: external::WrappedStatus(2),
    };
    let values = <DataTypeWrappedModel as ormer::Model>::field_values(&model);
    assert_eq!(values.len(), 2);
    match &values[1] {
        ormer::Value::Integer(value) => assert_eq!(*value, 2),
        other => panic!("expected integer wrapped status value, got {other:?}"),
    }

    let from_values = <DataTypeWrappedModel as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(1),
        ormer::Value::Integer(3),
    ])?;
    assert_eq!(from_values.status, external::WrappedStatus(3));

    let optional = DataTypeOptionalWrappedModel {
        id: 2,
        status: Some(external::WrappedStatus(4)),
    };
    let optional_values = <DataTypeOptionalWrappedModel as ormer::Model>::field_values(&optional);
    match &optional_values[1] {
        ormer::Value::Integer(value) => assert_eq!(*value, 4),
        other => panic!("expected integer optional wrapped status value, got {other:?}"),
    }
    let optional_from_values = <DataTypeOptionalWrappedModel as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(2),
        ormer::Value::Integer(5),
    ])?;
    assert_eq!(
        optional_from_values.status,
        Some(external::WrappedStatus(5))
    );

    let wrapped_i64 = DataTypeWrappedI64Model {
        id: 3,
        status: external::WrappedStatus(6),
    };
    let wrapped_i64_values = <DataTypeWrappedI64Model as ormer::Model>::field_values(&wrapped_i64);
    match &wrapped_i64_values[1] {
        ormer::Value::Integer(value) => assert_eq!(*value, 6),
        other => panic!("expected integer wrapped i64 status value, got {other:?}"),
    }
    let wrapped_i64_from_values = <DataTypeWrappedI64Model as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(3),
        ormer::Value::Integer(7),
    ])?;
    assert_eq!(wrapped_i64_from_values.status, external::WrappedStatus(7));

    Ok(())
}
