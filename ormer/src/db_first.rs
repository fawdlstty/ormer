use crate::abstract_layer::DbType;
use crate::model::{ForeignKeyAction, Model};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFirstTable {
    pub schema: Option<String>,
    pub name: String,
    pub columns: Vec<DbFirstColumn>,
    pub indexes: Vec<DbFirstIndex>,
    pub foreign_keys: Vec<DbFirstForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFirstColumn {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub auto_increment: bool,
    pub enum_variants: Vec<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFirstIndex {
    pub name: String,
    pub columns: Vec<DbFirstIndexColumn>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFirstIndexColumn {
    pub name: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFirstForeignKey {
    pub name: Option<String>,
    pub column: String,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_column: String,
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[allow(dead_code)]
pub(crate) fn validate_model_constraints<T: Model>(
    db_type: DbType,
    actual: &DbFirstTable,
) -> crate::Result<()> {
    let mut expected_unique = BTreeMap::<i32, (Option<&str>, Vec<&str>)>::new();
    let mut expected_indexes = BTreeMap::<i32, (Option<&str>, Vec<(&str, bool)>)>::new();
    let mut ungrouped_index = i32::MIN;
    for column in T::COLUMN_SCHEMA {
        if let Some(group) = column.unique_group {
            let entry = expected_unique
                .entry(group)
                .or_insert_with(|| (column.unique_name, Vec::new()));
            entry.1.push(column.name);
        }
        if column.is_indexed {
            let group = column.index_group.unwrap_or_else(|| {
                let value = ungrouped_index;
                ungrouped_index += 1;
                value
            });
            let entry = expected_indexes
                .entry(group)
                .or_insert_with(|| (column.index_name, Vec::new()));
            entry
                .1
                .push((column.name, column.index_order == Some("DESC")));
        }
    }

    for column in T::COLUMN_SCHEMA {
        let expected_default = column
            .default
            .map(|default| normalize_default(&default.to_sql(db_type)));
        let actual_column = actual
            .columns
            .iter()
            .find(|actual| actual.name == column.name);
        let actual_default = actual_column
            .and_then(|actual| actual.default.as_deref())
            .map(normalize_default);
        if actual_column.is_some_and(|actual| actual.auto_increment) && expected_default.is_none() {
            continue;
        }
        if expected_default != actual_default {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Default value mismatch for '{}': expected {:?}, but actual is {:?}",
                T::TABLE_NAME,
                column.name,
                expected_default,
                actual_default
            ));
        }
    }

    let actual_unique = actual
        .indexes
        .iter()
        .filter(|index| index.unique)
        .collect::<Vec<_>>();
    if actual_unique.len() != expected_unique.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Unique constraint count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_unique.len(),
            actual_unique.len()
        ));
    }
    for (name, columns) in expected_unique.values() {
        let found = actual_unique.iter().any(|index| {
            constraint_name_matches(db_type, *name, &index.name)
                && index
                    .columns
                    .iter()
                    .map(|column| column.name.as_str())
                    .eq(columns.iter().copied())
        });
        if !found {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Unique constraint mismatch for columns ({})",
                T::TABLE_NAME,
                columns.join(", ")
            ));
        }
    }

    if supports_enum_variants(db_type) {
        for (name, expected_variants) in T::COLUMN_SCHEMA
            .iter()
            .filter_map(|column| column.enum_variants.map(|variants| (column.name, variants)))
        {
            let Some(actual_column) = actual.columns.iter().find(|column| column.name == name)
            else {
                continue;
            };
            let actual_variants = actual_column
                .enum_variants
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if actual_variants != expected_variants {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Enum variants mismatch for '{}'",
                    T::TABLE_NAME,
                    name
                ));
            }
        }
    }

    let actual_indexes = actual
        .indexes
        .iter()
        .filter(|index| !index.unique)
        .collect::<Vec<_>>();
    if actual_indexes.len() != expected_indexes.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Index count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_indexes.len(),
            actual_indexes.len()
        ));
    }
    for (name, columns) in expected_indexes.values() {
        let found = actual_indexes.iter().any(|index| {
            name.map_or(true, |expected| expected == index.name.as_str())
                && index.columns.len() == columns.len()
                && index
                    .columns
                    .iter()
                    .zip(columns)
                    .all(|(actual, (expected, descending))| {
                        actual.name == *expected && actual.descending == *descending
                    })
        });
        if !found {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Index mismatch for columns ({})",
                T::TABLE_NAME,
                columns
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let expected_foreign_keys = T::COLUMN_SCHEMA
        .iter()
        .filter_map(|column| {
            column
                .foreign_key
                .as_ref()
                .map(|foreign_key| (column.name, foreign_key))
        })
        .collect::<Vec<_>>();
    if actual.foreign_keys.len() != expected_foreign_keys.len() {
        return Err(crate::ormer_error!(
            "Schema mismatch: table {}, reason: Foreign key count mismatch: expected {}, but actual is {}",
            T::TABLE_NAME,
            expected_foreign_keys.len(),
            actual.foreign_keys.len()
        ));
    }
    for (column, expected) in expected_foreign_keys {
        let expected_table = crate::model::normalize_table_name_for_db(db_type, expected.ref_table);
        let expected_ref_column = expected.get_ref_column();
        let found = actual.foreign_keys.iter().any(|foreign_key| {
            let table_matches =
                if let Some((expected_schema, expected_name)) = expected_table.rsplit_once('.') {
                    foreign_key.ref_schema.as_deref() == Some(expected_schema)
                        && foreign_key.ref_table == expected_name
                } else {
                    foreign_key.ref_table == expected_table
                };
            foreign_key.column == column
                && table_matches
                && foreign_key.ref_column == expected_ref_column
                && foreign_key_action_matches(expected.on_delete, foreign_key.on_delete.as_deref())
                && foreign_key_action_matches(expected.on_update, foreign_key.on_update.as_deref())
        });
        if !found {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Foreign key mismatch for '{}'",
                T::TABLE_NAME,
                column
            ));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn foreign_key_action_matches(expected: Option<ForeignKeyAction>, actual: Option<&str>) -> bool {
    expected.map_or(true, |expected| {
        actual.is_some_and(|actual| actual.eq_ignore_ascii_case(expected.as_sql()))
    })
}

#[allow(dead_code)]
#[allow(unused_variables)]
fn constraint_name_matches(db_type: DbType, expected: Option<&str>, actual: &str) -> bool {
    #[cfg(feature = "sqlite")]
    if matches!(db_type, DbType::Sqlite) {
        return true;
    }
    expected.map_or(true, |expected| expected == actual)
}

#[allow(dead_code)]
fn supports_enum_variants(db_type: DbType) -> bool {
    let _ = db_type;
    #[cfg(feature = "postgresql")]
    if matches!(db_type, DbType::PostgreSQL) {
        return true;
    }
    #[cfg(feature = "mysql")]
    if matches!(db_type, DbType::MySQL) {
        return true;
    }
    false
}

fn normalize_default(value: &str) -> String {
    let mut value = value.trim().to_ascii_uppercase();
    while value.starts_with('(') && value.ends_with(')') && value.len() >= 2 {
        value = value[1..value.len() - 1].trim().to_string();
    }
    if let Some((expression, _type_name)) = value.split_once("::") {
        value = expression.trim().to_string();
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value = value[1..value.len() - 1].replace("''", "'");
    }
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone)]
struct EntityTable<'a> {
    table: &'a DbFirstTable,
    struct_name: String,
    fields: Vec<EntityField<'a>>,
}

#[derive(Debug, Clone)]
struct EntityField<'a> {
    column: &'a DbFirstColumn,
    field_name: String,
    rust_type: String,
    enum_name: Option<String>,
}

pub fn generate_entities(db_type: DbType, tables: &[DbFirstTable]) -> String {
    let mut entities = tables
        .iter()
        .map(|table| EntityTable {
            table,
            struct_name: unique_type_name(table, tables),
            fields: Vec::new(),
        })
        .collect::<Vec<_>>();

    for entity in &mut entities {
        let mut used_fields = BTreeSet::new();
        entity.fields = entity
            .table
            .columns
            .iter()
            .map(|column| {
                let field_name = unique_field_name(&column.name, &mut used_fields);
                let enum_name = enum_name_for_column(entity.table, column, tables);
                let base_type = enum_name
                    .clone()
                    .unwrap_or_else(|| rust_type_for_column(db_type, column));
                let rust_type = if column.primary_key {
                    base_type
                } else if column.nullable {
                    format!("Option<{base_type}>")
                } else {
                    base_type
                };
                EntityField {
                    column,
                    field_name,
                    rust_type,
                    enum_name,
                }
            })
            .collect();
    }

    let table_by_key = entities
        .iter()
        .map(|entity| (table_key(entity.table), entity))
        .collect::<BTreeMap<_, _>>();

    let mut code = String::from("use ormer::Model;\n\n");
    for entity in &entities {
        for field in &entity.fields {
            let Some(enum_name) = &field.enum_name else {
                continue;
            };
            code.push_str("#[derive(Debug, Clone, ormer::FieldType, PartialEq)]\n");
            code.push_str(&format!("pub enum {enum_name} {{\n"));
            for variant in &field.column.enum_variants {
                code.push_str("    ");
                code.push_str(variant);
                code.push_str(",\n");
            }
            code.push_str("}\n\n");
        }

        code.push_str("#[derive(Debug, Clone, ormer::Model)]\n");
        code.push_str(&table_attribute(db_type, entity.table));
        code.push_str(&format!("pub struct {} {{\n", entity.struct_name));

        let unique_attrs = unique_attributes(entity.table);
        let index_attrs = index_attributes(entity.table);
        let foreign_attrs = foreign_attributes(entity.table, &table_by_key);

        for field in &entity.fields {
            let column = field.column;
            for attr in
                column_attributes(column, field, &unique_attrs, &index_attrs, &foreign_attrs)
            {
                code.push_str("    ");
                code.push_str(&attr);
                code.push('\n');
            }
            code.push_str(&format!(
                "    pub {}: {},\n",
                field.field_name, field.rust_type
            ));
        }

        for relation in belongs_to_relations(entity, &table_by_key) {
            code.push_str(&relation);
        }
        for relation in has_many_relations(entity, &entities) {
            code.push_str(&relation);
        }

        code.push_str("}\n\n");
    }

    code.trim_end().to_string()
}

fn table_attribute(db_type: DbType, table: &DbFirstTable) -> String {
    match (db_type, table.schema.as_deref()) {
        #[cfg(feature = "postgresql")]
        (DbType::PostgreSQL, Some(schema)) => {
            format!(
                "#[table(schema = \"{}\", name = \"{}\")]\n",
                escape_rust_string(schema),
                escape_rust_string(&table.name)
            )
        }
        #[cfg(feature = "mssql")]
        (DbType::MSSQL, Some(schema)) => {
            format!(
                "#[table(schema = \"{}\", name = \"{}\")]\n",
                escape_rust_string(schema),
                escape_rust_string(&table.name)
            )
        }
        _ => format!("#[table = \"{}\"]\n", escape_rust_string(&table.name)),
    }
}

fn column_attributes(
    column: &DbFirstColumn,
    field: &EntityField<'_>,
    unique_attrs: &BTreeMap<String, Vec<String>>,
    index_attrs: &BTreeMap<String, Vec<String>>,
    foreign_attrs: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut attrs = Vec::new();
    if column.primary_key {
        if column.auto_increment {
            attrs.push("#[primary(auto)]".to_string());
        } else {
            attrs.push("#[primary]".to_string());
        }
    }
    if field.field_name != column.name {
        attrs.push(format!(
            "#[column(name = \"{}\")]",
            escape_rust_string(&column.name)
        ));
    }
    if let Some(values) = unique_attrs.get(&column.name) {
        attrs.extend(values.iter().cloned());
    }
    if let Some(values) = index_attrs.get(&column.name) {
        attrs.extend(values.iter().cloned());
    }
    if let Some(value) = foreign_attrs.get(&column.name) {
        attrs.push(value.clone());
    }
    attrs
}

fn unique_attributes(table: &DbFirstTable) -> BTreeMap<String, Vec<String>> {
    grouped_column_attributes(
        table
            .indexes
            .iter()
            .filter(|index| index.unique)
            .collect::<Vec<_>>(),
        "unique",
    )
}

fn index_attributes(table: &DbFirstTable) -> BTreeMap<String, Vec<String>> {
    grouped_column_attributes(
        table
            .indexes
            .iter()
            .filter(|index| !index.unique)
            .collect::<Vec<_>>(),
        "index",
    )
}

fn grouped_column_attributes(
    indexes: Vec<&DbFirstIndex>,
    attr: &str,
) -> BTreeMap<String, Vec<String>> {
    let mut output = BTreeMap::<String, Vec<String>>::new();
    let mut group = 1;
    for index in indexes {
        if index.columns.is_empty() {
            continue;
        }
        let is_grouped = index.columns.len() > 1;
        for column in &index.columns {
            let mut args = Vec::new();
            if is_grouped {
                args.push(format!("group = {group}"));
            }
            if !index.name.is_empty() {
                args.push(format!("name = \"{}\"", escape_rust_string(&index.name)));
            }
            if attr == "index" && column.descending {
                args.push("order = \"DESC\"".to_string());
            }
            let rendered = if args.is_empty() {
                format!("#[{attr}]")
            } else {
                format!("#[{attr}({})]", args.join(", "))
            };
            output
                .entry(column.name.clone())
                .or_default()
                .push(rendered);
        }
        if is_grouped {
            group += 1;
        }
    }
    output
}

fn foreign_attributes(
    table: &DbFirstTable,
    table_by_key: &BTreeMap<String, &EntityTable<'_>>,
) -> BTreeMap<String, String> {
    table
        .foreign_keys
        .iter()
        .filter_map(|foreign_key| {
            let target = table_by_key.get(&referenced_table_key(foreign_key))?;
            let target_field = target
                .fields
                .iter()
                .find(|field| field.column.name == foreign_key.ref_column)?;
            let mut args = vec![format!(
                "{}.{}",
                target.struct_name, target_field.field_name
            )];
            if let Some(name) = &foreign_key.name {
                args.push(format!("name = \"{}\"", escape_rust_string(name)));
            }
            if let Some(action) = foreign_key_action(&foreign_key.on_delete) {
                args.push(format!("on_delete = {action}"));
            }
            if let Some(action) = foreign_key_action(&foreign_key.on_update) {
                args.push(format!("on_update = {action}"));
            }
            Some((
                foreign_key.column.clone(),
                format!("#[foreign({})]", args.join(", ")),
            ))
        })
        .collect()
}

fn belongs_to_relations(
    entity: &EntityTable<'_>,
    table_by_key: &BTreeMap<String, &EntityTable<'_>>,
) -> Vec<String> {
    let mut used = entity
        .fields
        .iter()
        .map(|field| field.field_name.clone())
        .collect::<BTreeSet<_>>();
    entity
        .table
        .foreign_keys
        .iter()
        .filter_map(|foreign_key| {
            let target = table_by_key.get(&referenced_table_key(foreign_key))?;
            let local = entity
                .fields
                .iter()
                .find(|field| field.column.name == foreign_key.column)?;
            let base_name = relation_name_from_fk(&local.field_name, &target.table.name);
            let field_name = unique_name(base_name, &mut used);
            Some(format!(
                "\n    #[belongs_to({})]\n    pub {}: Option<{}>,\n",
                local.field_name, field_name, target.struct_name
            ))
        })
        .collect()
}

fn has_many_relations(entity: &EntityTable<'_>, entities: &[EntityTable<'_>]) -> Vec<String> {
    let mut used = entity
        .fields
        .iter()
        .map(|field| field.field_name.clone())
        .collect::<BTreeSet<_>>();
    let self_key = table_key(entity.table);
    let mut relations = Vec::new();
    for source in entities {
        for foreign_key in &source.table.foreign_keys {
            if referenced_table_key(foreign_key) != self_key {
                continue;
            }
            let Some(local) = source
                .fields
                .iter()
                .find(|field| field.column.name == foreign_key.column)
            else {
                continue;
            };
            let field_name = unique_name(to_snake_identifier(&source.table.name), &mut used);
            relations.push(format!(
                "\n    #[has_many({}.{})]\n    pub {}: Vec<{}>,\n",
                source.struct_name, local.field_name, field_name, source.struct_name
            ));
        }
    }
    relations
}

fn rust_type_for_column(db_type: DbType, column: &DbFirstColumn) -> String {
    let raw = column.type_name.trim();
    let lower = raw.to_ascii_lowercase();
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => sqlite_rust_type(&lower),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => postgresql_rust_type(&lower),
        #[cfg(feature = "mysql")]
        DbType::MySQL => mysql_rust_type(&lower),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => mssql_rust_type(&lower),
        #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
        _ => "String".to_string(),
    }
}

#[cfg(feature = "sqlite")]
fn sqlite_rust_type(type_name: &str) -> String {
    if type_name.contains("int") {
        "i64".to_string()
    } else if type_name.contains("real") || type_name.contains("floa") || type_name.contains("doub")
    {
        "f64".to_string()
    } else if type_name.contains("blob") {
        "Vec<u8>".to_string()
    } else {
        "String".to_string()
    }
}

#[cfg(feature = "postgresql")]
fn postgresql_rust_type(type_name: &str) -> String {
    match type_name {
        "smallint" | "int2" => "i16",
        "integer" | "int" | "int4" | "serial" => "i32",
        "bigint" | "int8" | "bigserial" => "i64",
        "real" | "double precision" | "float8" | "float4" => "f64",
        "numeric" | "decimal" => "rust_decimal::Decimal",
        "boolean" | "bool" => "bool",
        "bytea" => "Vec<u8>",
        "uuid" => "uuid::Uuid",
        "date" => "chrono::NaiveDate",
        "time" | "time without time zone" => "chrono::NaiveTime",
        "timestamp with time zone" | "timestamptz" => "chrono::DateTime<chrono::Utc>",
        "timestamp without time zone" | "timestamp" => "chrono::NaiveDateTime",
        "json" | "jsonb" => "serde_json::Value",
        "interval" => "std::time::Duration",
        "ARRAY" | "array" | "_text" | "text[]" | "character varying[]" | "varchar[]" => {
            "Vec<String>"
        }
        "_int4" | "integer[]" | "int4[]" => "Vec<i32>",
        "_int8" | "bigint[]" | "int8[]" => "Vec<i64>",
        _ => "String",
    }
    .to_string()
}

#[cfg(feature = "mysql")]
fn mysql_rust_type(type_name: &str) -> String {
    let unsigned = type_name.contains("unsigned");
    let base = type_name.split('(').next().unwrap_or(type_name).trim();
    match base {
        "tinyint" if type_name.starts_with("tinyint(1)") && !unsigned => "bool",
        "tinyint" if unsigned => "u8",
        "tinyint" => "i8",
        "smallint" if unsigned => "u16",
        "smallint" => "i16",
        "mediumint" | "int" | "integer" if unsigned => "u32",
        "mediumint" | "int" | "integer" => "i32",
        "bigint" if unsigned => "u64",
        "bigint" => "i64",
        "float" | "double" => "f64",
        "decimal" | "numeric" => "rust_decimal::Decimal",
        "bit" | "bool" | "boolean" => "bool",
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => "Vec<u8>",
        "date" => "chrono::NaiveDate",
        "time" => "chrono::NaiveTime",
        "datetime" | "timestamp" => "chrono::NaiveDateTime",
        "json" => "serde_json::Value",
        _ => "String",
    }
    .to_string()
}

#[cfg(feature = "mssql")]
fn mssql_rust_type(type_name: &str) -> String {
    let base = type_name.split('(').next().unwrap_or(type_name).trim();
    match base {
        "tinyint" => "u8",
        "smallint" => "i16",
        "int" => "i32",
        "bigint" => "i64",
        "real" | "float" => "f64",
        "decimal" | "numeric" | "money" | "smallmoney" => "rust_decimal::Decimal",
        "bit" => "bool",
        "binary" | "varbinary" | "image" => "Vec<u8>",
        "uniqueidentifier" => "uuid::Uuid",
        "date" => "chrono::NaiveDate",
        "time" => "chrono::NaiveTime",
        "datetime" | "datetime2" | "smalldatetime" => "chrono::NaiveDateTime",
        _ => "String",
    }
    .to_string()
}

fn enum_name_for_column(
    table: &DbFirstTable,
    column: &DbFirstColumn,
    tables: &[DbFirstTable],
) -> Option<String> {
    if column.enum_variants.is_empty() {
        return None;
    }
    if column
        .enum_variants
        .iter()
        .any(|variant| !is_plain_rust_ident(variant))
    {
        return None;
    }
    let base = format!(
        "{}{}",
        unique_type_name(table, tables),
        to_pascal_identifier(&column.name)
    );
    Some(base)
}

fn unique_type_name(table: &DbFirstTable, tables: &[DbFirstTable]) -> String {
    let base = to_pascal_identifier(&singularize(&table.name));
    let duplicate_name = tables
        .iter()
        .filter(|candidate| candidate.name == table.name)
        .count()
        > 1;
    if duplicate_name {
        if let Some(schema) = &table.schema {
            return format!("{}{}", to_pascal_identifier(schema), base);
        }
    }
    base
}

fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with('s') && !name.ends_with("ss") && name.len() > 1 {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

fn unique_field_name(column_name: &str, used: &mut BTreeSet<String>) -> String {
    unique_name(to_snake_identifier(column_name), used)
}

fn unique_name(base: String, used: &mut BTreeSet<String>) -> String {
    let mut name = base;
    let mut counter = 2;
    while used.contains(&name) {
        name = format!("{}_{counter}", name.trim_end_matches('_'));
        counter += 1;
    }
    used.insert(name.clone());
    name
}

fn to_snake_identifier(name: &str) -> String {
    let mut out = String::new();
    let mut previous_was_underscore = false;
    for (idx, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 && !previous_was_underscore {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            previous_was_underscore = false;
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            previous_was_underscore = ch == '_';
        } else if !previous_was_underscore {
            out.push('_');
            previous_was_underscore = true;
        }
    }
    let out = out.trim_matches('_');
    let mut out = if out.is_empty() {
        "field".to_string()
    } else {
        out.to_string()
    };
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if is_rust_keyword(&out) {
        out.push('_');
    }
    out
}

fn to_pascal_identifier(name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize {
                out.push(ch.to_ascii_uppercase());
                capitalize = false;
            } else {
                out.push(ch);
            }
        } else {
            capitalize = true;
        }
    }
    if out.is_empty() {
        out.push_str("Entity");
    }
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert(0, 'T');
    }
    if is_rust_keyword(&out) {
        out.push_str("Entity");
    }
    out
}

fn is_plain_rust_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) && !is_rust_keyword(value)
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}

fn relation_name_from_fk(local_field: &str, target_table: &str) -> String {
    for suffix in ["_id", "_uuid", "_key"] {
        if let Some(stem) = local_field.strip_suffix(suffix) {
            if !stem.is_empty() {
                return to_snake_identifier(stem);
            }
        }
    }
    to_snake_identifier(&singularize(target_table))
}

fn foreign_key_action(value: &Option<String>) -> Option<&'static str> {
    match value.as_deref()?.to_ascii_uppercase().as_str() {
        "CASCADE" => Some("Cascade"),
        "RESTRICT" => Some("Restrict"),
        "NO ACTION" | "NOACTION" => Some("NoAction"),
        "SET NULL" | "SETNULL" => Some("SetNull"),
        "SET DEFAULT" | "SETDEFAULT" => Some("SetDefault"),
        _ => None,
    }
}

fn table_key(table: &DbFirstTable) -> String {
    format!(
        "{}.{}",
        table.schema.as_deref().unwrap_or_default(),
        table.name
    )
}

fn referenced_table_key(foreign_key: &DbFirstForeignKey) -> String {
    format!(
        "{}.{}",
        foreign_key.ref_schema.as_deref().unwrap_or_default(),
        foreign_key.ref_table
    )
}

fn escape_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
