use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Lit, Meta};

pub fn derive_model(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let where_name = syn::Ident::new(&format!("{}Where", name), name.span());

    // 提取表名
    let table_name = extract_table_name(&input);

    // 提取字段
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("Model must have named fields"),
        },
        _ => panic!("Model must be a struct"),
    };

    // 提取主键字段
    let primary_key_field = fields
        .iter()
        .find_map(|f| {
            for attr in &f.attrs {
                if attr.path().is_ident("primary") {
                    return Some(f.ident.as_ref().unwrap().clone());
                }
            }
            None
        })
        .expect("Model must have a #[primary] field");

    // 生成字段名列表
    let field_names: Vec<String> = fields
        .iter()
        .map(|f| f.ident.as_ref().unwrap().to_string())
        .collect();

    let field_names_lit = field_names.iter().map(|name| {
        quote! { #name }
    });

    // 生成字段元数据 (COLUMN_SCHEMA)
    let column_schema_entries = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let field_type = &f.ty;
        let type_str = quote! { #field_type }.to_string();

        // 检查是否是主键字段
        let is_primary = f.attrs.iter().any(|attr| attr.path().is_ident("primary"));

        // 检查是否是 Option<T>
        let is_nullable = type_str.starts_with("Option <");

        // 提取基础 Rust 类型
        let rust_type = if is_nullable {
            type_str
                .trim_start_matches("Option <")
                .trim_end_matches(">")
                .trim()
                .to_string()
        } else {
            type_str
        };

        quote! {
            ::ormer::model::ColumnSchema {
                name: stringify!(#field_name),
                rust_type: #rust_type,
                is_primary: #is_primary,
                is_nullable: #is_nullable,
            }
        }
    });

    // 生成 from_row 实现
    let from_row_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        quote! {
            #field_name: row.get(stringify!(#field_name))?
        }
    });

    // 生成 field_values 实现
    let field_names_for_values = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        quote! {
            ::ormer::Value::from(self.#field_name.clone())
        }
    });

    // 生成 Where 结构体的字段
    // 根据字段类型生成不同的列代理
    let where_fields = fields.iter().filter_map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let field_type = &f.ty;
        let type_str = quote! { #field_type }.to_string();

        // 为数值类型生成 NumericColumn
        if is_numeric_type(&type_str) {
            Some(quote! {
                pub #field_name: ::ormer::query::builder::NumericColumn
            })
        } else {
            None
        }
    });

    // 生成 Where 的 Default 实现
    let where_default_fields = fields.iter().filter_map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let field_type = &f.ty;
        let type_str = quote! { #field_type }.to_string();

        if is_numeric_type(&type_str) {
            Some(quote! {
                #field_name: ::ormer::query::builder::NumericColumn::new(stringify!(#field_name))
            })
        } else {
            None
        }
    });

    quote! {
        // 生成 Where 结构体
        pub struct #where_name {
            #(#where_fields),*
        }

        impl Default for #where_name {
            fn default() -> Self {
                Self {
                    #(#where_default_fields),*
                }
            }
        }

        impl ::ormer::Model for #name {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMNS: &'static [&'static str] = &[#(#field_names_lit),*];
            const COLUMN_SCHEMA: &'static [::ormer::model::ColumnSchema] = &[#(#column_schema_entries),*];

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = #where_name;

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> Result<Self, ::ormer::Error> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                vec![
                    #(#field_names_for_values),*
                ]
            }

            fn primary_key_column() -> &'static str {
                stringify!(#primary_key_field)
            }

            fn primary_key_value(&self) -> ::ormer::Value {
                ::ormer::Value::from(self.#primary_key_field.clone())
            }
        }

        // 生成 inherent 方法，使得不需要 import Model trait 也能调用
        impl #name {
            pub fn select() -> ::ormer::Select<Self> {
                ::ormer::Select::new()
            }

            pub fn query() -> ::ormer::Select<Self> {
                ::ormer::Select::new()
            }
        }
    }
}

fn extract_table_name(input: &DeriveInput) -> String {
    // 查找 #[table = "name"] 属性
    for attr in &input.attrs {
        if attr.path().is_ident("table") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr) = &meta.value {
                    if let Lit::Str(lit) = &expr.lit {
                        return lit.value();
                    }
                }
            }
        }
    }

    // 默认使用结构体名的蛇形形式
    to_snake_case(&input.ident.to_string())
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

fn is_numeric_type(type_str: &str) -> bool {
    matches!(
        type_str,
        "i32" | "i64" | "f32" | "f64" | "u32" | "u64" | "i8" | "i16" | "u8" | "u16" | "bool"
    )
}
