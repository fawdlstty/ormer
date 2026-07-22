use std::collections::HashMap;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum DataTypeStatus {
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
    #[data_type(i32)]
    optional_status: Option<DataTypeStatus>,
}

#[test]
fn data_type_enum_without_model_enum_uses_i32_values() -> anyhow::Result<()> {
    let status_column = <DataTypeEnumModel as ormer::Model>::COLUMN_SCHEMA
        .iter()
        .find(|column| column.name == "status")
        .expect("status column should exist");
    assert_eq!(status_column.data_type, Some("i32"));
    assert_eq!(status_column.enum_variants, None);

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
