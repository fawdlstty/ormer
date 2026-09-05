use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr};

#[derive(Default)]
struct DbTypeAttrs {
    sqlite: Option<String>,
    postgresql: Option<String>,
    questdb: Option<String>,
    mysql: Option<String>,
    mssql: Option<String>,
    duckdb: Option<String>,
    clickhouse: Option<String>,
}

pub fn derive_db_value(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let inner_type = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0].ty,
            _ => panic!("DbValue can only be derived for single-field tuple structs"),
        },
        _ => panic!("DbValue can only be derived for single-field tuple structs"),
    };
    let db_types = extract_db_types(&input);

    let sqlite_arm = db_types.sqlite.map(|ty| {
        quote! {
            #[cfg(feature = "sqlite")]
            ::ormer::DbType::Sqlite => #ty,
        }
    });
    let postgresql_arm = db_types.postgresql.map(|ty| {
        quote! {
            #[cfg(feature = "postgresql")]
            ::ormer::DbType::PostgreSQL => #ty,
        }
    });
    let questdb_arm = db_types.questdb.map(|ty| {
        quote! {
            #[cfg(feature = "questdb")]
            ::ormer::DbType::QuestDB => #ty,
        }
    });
    let mysql_arm = db_types.mysql.map(|ty| {
        quote! {
            #[cfg(feature = "mysql")]
            ::ormer::DbType::MySQL => #ty,
        }
    });
    let mssql_arm = db_types.mssql.map(|ty| {
        quote! {
            #[cfg(feature = "mssql")]
            ::ormer::DbType::MSSQL => #ty,
        }
    });
    let duckdb_arm = db_types.duckdb.map(|ty| {
        quote! {
            #[cfg(feature = "duckdb")]
            ::ormer::DbType::DuckDB => #ty,
        }
    });
    let clickhouse_arm = db_types.clickhouse.map(|ty| {
        quote! {
            #[cfg(feature = "clickhouse")]
            ::ormer::DbType::ClickHouse => #ty,
        }
    });

    quote! {
        impl ::ormer::model::DbValue for #name {
            fn to_value(&self) -> ::ormer::model::Value {
                ::ormer::model::Value::from(self.0.clone())
            }

            fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                <#inner_type as ::ormer::model::FromValue>::from_value(value).map(Self)
            }

            fn db_type(db_type: ::ormer::DbType) -> &'static str {
                match db_type {
                    #sqlite_arm
                    #postgresql_arm
                    #questdb_arm
                    #mysql_arm
                    #mssql_arm
                    #duckdb_arm
                    #clickhouse_arm
                    _ => panic!("db_type mapping for {} is not configured", stringify!(#name)),
                }
            }
        }

        impl ::ormer::model::FieldTypeProvider for #name {
            const ENUM_VARIANTS: Option<&'static [&'static str]> = None;
            const DB_VALUE_TYPE: Option<fn(::ormer::DbType) -> &'static str> =
                Some(<#name as ::ormer::model::DbValue>::db_type);
        }

        impl ::core::convert::From<#name> for ::ormer::model::Value {
            fn from(value: #name) -> Self {
                <#name as ::ormer::model::DbValue>::to_value(&value)
            }
        }

        impl ::ormer::model::FromValue for #name {
            fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                <#name as ::ormer::model::DbValue>::from_value(value)
            }
        }

        impl ::ormer::model::FromRowValues for #name {
            fn from_row_values(values: &[::ormer::model::Value]) -> ::ormer::Result<Self> {
                let value = values.first().ok_or_else(|| {
                    ::ormer::ormer_error!("Type mismatch: expected {}", stringify!(#name))
                })?;
                <#name as ::ormer::model::FromValue>::from_value(value)
            }
        }

        impl ::ormer::query::builder::ColumnValueType for #name {
            fn to_filter_value(value: Self) -> ::ormer::query::filter::Value {
                <#name as ::ormer::model::DbValue>::to_value(&value)
            }

            fn supports_comparison() -> bool {
                false
            }
        }

        impl ::ormer::query::builder::IsInValue<#name> for #name {
            fn to_in_value(self) -> #name {
                self
            }
        }

        impl ::ormer::query::builder::IsInValue<#name> for &#name
        where
            #name: ::core::clone::Clone,
        {
            fn to_in_value(self) -> #name {
                (*self).clone()
            }
        }

        impl ::ormer::query::expr::IntoSqlExpr for #name {
            fn into_sql_expr(self) -> ::ormer::query::expr::SqlExpr {
                ::ormer::query::expr::SqlExpr::Value(
                    <#name as ::ormer::model::DbValue>::to_value(&self),
                )
            }
        }

        impl ::ormer::query::expr::IntoTypedExpr for #name {
            type Output = #name;

            fn into_typed_expr(self) -> ::ormer::query::expr::TypedExpr<Self::Output> {
                ::ormer::query::expr::TypedExpr::new(
                    ::ormer::query::expr::SqlExpr::Value(
                        <#name as ::ormer::model::DbValue>::to_value(&self),
                    ),
                )
            }
        }
    }
}

fn extract_db_types(input: &DeriveInput) -> DbTypeAttrs {
    let mut db_types = DbTypeAttrs::default();
    for attr in &input.attrs {
        if !attr.path().is_ident("db_type") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let value = meta.value()?;
            let lit: LitStr = value.parse()?;
            if meta.path.is_ident("sqlite") {
                db_types.sqlite = Some(lit.value());
            } else if meta.path.is_ident("postgresql") {
                db_types.postgresql = Some(lit.value());
            } else if meta.path.is_ident("questdb") {
                db_types.questdb = Some(lit.value());
            } else if meta.path.is_ident("mysql") {
                db_types.mysql = Some(lit.value());
            } else if meta.path.is_ident("mssql") {
                db_types.mssql = Some(lit.value());
            } else if meta.path.is_ident("duckdb") {
                db_types.duckdb = Some(lit.value());
            } else if meta.path.is_ident("clickhouse") {
                db_types.clickhouse = Some(lit.value());
            } else {
                return Err(meta.error("unsupported #[db_type] argument"));
            }
            Ok(())
        })
        .expect("Failed to parse #[db_type] attribute");
    }
    db_types
}
