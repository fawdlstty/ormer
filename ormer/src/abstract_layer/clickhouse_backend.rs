use crate::model::DbBackendTypeMapper;

/// ClickHouse SQL type mapping for schema generation and SQL rendering.
pub struct ClickHouseTypeMapper;

impl DbBackendTypeMapper for ClickHouseTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        _is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        if enum_variants.is_some() {
            return nullable_type("String", is_nullable || is_primary);
        }
        let base = match rust_type {
            "i8" => "Int8",
            "i16" => "Int16",
            "i32" => "Int32",
            "i64" => "Int64",
            "i128" => "Int128",
            "u8" => "UInt8",
            "u16" => "UInt16",
            "u32" => "UInt32",
            "u64" => "UInt64",
            "f32" => "Float32",
            "f64" => "Float64",
            "bool" => "UInt8",
            "Vec<u8>" | "&[u8]" => "String",
            "DateTime" | "chrono::DateTime" | "chrono::DateTime<chrono::Utc>" => "DateTime64(3)",
            "NaiveDate" | "chrono::NaiveDate" => "Date32",
            "NaiveTime" | "chrono::NaiveTime" => "String",
            "Uuid" | "uuid::Uuid" => "UUID",
            "JsonValue" | "serde_json::Value" => "String",
            "Decimal" | "rust_decimal::Decimal" => "Decimal128(38)",
            "BigDecimal" | "bigdecimal::BigDecimal" => "String",
            "Duration" | "std::time::Duration" => "Int64",
            _ => "String",
        };
        let mut ty = if is_nullable {
            nullable_type(base, true)
        } else {
            base.to_string()
        };
        if is_primary {
            ty.push_str(" PRIMARY KEY");
        }
        ty
    }
}

fn nullable_type(base: &str, nullable: bool) -> String {
    if nullable {
        format!("Nullable({base})")
    } else {
        base.to_string()
    }
}
