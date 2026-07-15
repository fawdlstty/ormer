#[derive(Debug, Clone, PartialEq, ormer::Model)]
#[table = "required_binary_test"]
struct RequiredBinary {
    #[primary]
    id: i32,
    payload: Vec<u8>,
}

#[test]
fn required_binary_from_row_values() -> anyhow::Result<()> {
    let model = <RequiredBinary as ormer::Model>::from_row_values(&[
        ormer::Value::Integer(1),
        ormer::Value::Bytes(vec![1, 2, 3]),
    ])?;

    assert_eq!(model.payload, vec![1, 2, 3]);
    assert_eq!(
        <RequiredBinary as ormer::Model>::COLUMN_SCHEMA[1].rust_type,
        "Vec<u8>"
    );
    assert!(!<RequiredBinary as ormer::Model>::COLUMN_SCHEMA[1].is_nullable);
    Ok(())
}
