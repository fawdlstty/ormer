use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Meta};

// 列名解析与类型工具与 Model 派生共用同一份实现（crate::shared），
// 保证 #[column(name = ...)] 等属性行为一致
use crate::shared::{
    extract_column_name, is_string_type, normalize_type_string, option_inner_type, to_snake_case,
};

pub fn derive_model_enum(input: DeriveInput) -> TokenStream {
    derive_field_type(input)
}

pub fn derive_field_type(input: DeriveInput) -> TokenStream {
    match derive_field_type_inner(input) {
        Ok(tokens) => tokens,
        // 属性解析/校验失败统一走 compile_error，而不是让派生宏 panic
        Err(error) => error.to_compile_error(),
    }
}

fn derive_field_type_inner(input: DeriveInput) -> syn::Result<TokenStream> {
    match &input.data {
        Data::Enum(data_enum) => derive_enum_field_type(&input, data_enum),
        Data::Struct(data_struct) => derive_tuple_struct_field_type(&input, data_struct),
        _ => Err(syn::Error::new_spanned(
            input.ident,
            "FieldType can only be derived for enums or single-field tuple structs",
        )),
    }
}

fn derive_enum_field_type(
    input: &DeriveInput,
    data_enum: &syn::DataEnum,
) -> syn::Result<TokenStream> {
    if data_enum
        .variants
        .iter()
        .any(|variant| !matches!(variant.fields, Fields::Unit))
    {
        return derive_data_enum_field_type(input, data_enum);
    }

    let name = &input.ident;
    let variants = &data_enum.variants;
    let variant_names: Vec<&Ident> = variants.iter().map(|v| &v.ident).collect();
    let variant_names_str: Vec<String> = variant_names.iter().map(|v| v.to_string()).collect();

    let repr_integer_type = repr_integer_type(input);
    let is_numeric_enum = repr_integer_type.is_some();

    // 数值枚举的宽度校验与转换表达式：
    // - u64/usize 判别值可能超出 i64，`as i64` 会静默回绕写错库值，
    //   在派生期直接拒绝；
    // - 其余 repr 的判别值总是放得下 i64，统一用 `i64::from(v)` 无损转换
    //   （isize 无 From 实现，但指针宽度不超过 64 位，`as i64` 无损）；
    // - `From<Enum> for i32` 只对判别值总能放进 i32 的窄 repr
    //   （i8/i16/i32/u8/u16）生成：宽 repr 上的 `as i32` 会截断回绕，
    //   且注入的 impl 会与用户手写实现冲突。宽 repr 配合 `#[data_type(i32)]`
    //   需要的转换由用户显式提供。
    let (numeric_to_i64, numeric_repr) = match &repr_integer_type {
        Some(repr_type) => {
            let repr_str = quote! { #repr_type }.to_string().replace(' ', "");
            if matches!(repr_str.as_str(), "u64" | "usize") {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    format!(
                        "#[repr({repr_str})] numeric enums are not supported: discriminants can \
                         exceed i64 and would silently wrap when stored; use #[repr(i64)] or a \
                         narrower repr"
                    ),
                ));
            }
            let to_i64 = if repr_str == "isize" {
                quote! { v as i64 }
            } else {
                // 先取判别值（as 转 repr 无损），再经 From 提升到 i64
                quote! { <i64 as ::core::convert::From<#repr_type>>::from(v as #repr_type) }
            };
            (to_i64, repr_str)
        }
        None => (quote! {}, String::new()),
    };

    let from_value_impl = if is_numeric_enum {
        let mut impls = quote! {
            impl From<#name> for ::ormer::model::Value {
                fn from(v: #name) -> Self {
                    ::ormer::model::Value::Integer(#numeric_to_i64)
                }
            }
        };
        if matches!(
            numeric_repr.as_str(),
            "i8" | "i16" | "i32" | "u8" | "u16"
        ) {
            let repr_type = repr_integer_type.as_ref().expect("numeric repr is present");
            impls.extend(quote! {
                impl ::core::convert::From<#name> for i32 {
                    fn from(v: #name) -> Self {
                        <i32 as ::core::convert::From<#repr_type>>::from(v as #repr_type)
                    }
                }
            });
        }
        impls
    } else {
        let match_arms = variant_names.iter().map(|v| {
            quote! {
                #name::#v => ::ormer::model::Value::Text(stringify!(#v).to_string()),
            }
        });

        quote! {
            impl From<#name> for ::ormer::model::Value {
                fn from(v: #name) -> Self {
                    match v {
                        #(#match_arms)*
                    }
                }
            }
        }
    };

    let from_impl = if is_numeric_enum {
        let match_arms = variant_names.iter().map(|v| {
            quote! {
                val if val == #name::#v as i64 => Ok(#name::#v),
            }
        });

        quote! {
            impl ::ormer::model::FromValue for #name {
                fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                    match value {
                        ::ormer::model::Value::Integer(val) => {
                            match *val {
                                #(#match_arms)*
                                _ => Err(::ormer::ormer_error!(
                                    "Unknown numeric value '{}' for {}", val, stringify!(#name)
                                )),
                            }
                        }
                        _ => Err(::ormer::ormer_error!(
                            "Expected Integer value for {}", stringify!(#name)
                        )),
                    }
                }
            }
        }
    } else {
        let match_arms = variant_names.iter().map(|v| {
            let v_str = v.to_string();
            quote! {
                #v_str => Ok(#name::#v),
            }
        });

        quote! {
            impl ::ormer::model::FromValue for #name {
                fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                    match value {
                        ::ormer::model::Value::Text(s) => {
                            match s.as_str() {
                                #(#match_arms)*
                                _ => Err(::ormer::ormer_error!(
                                    "Unknown enum variant '{}' for {}", s, stringify!(#name)
                                )),
                            }
                        }
                        _ => Err(::ormer::ormer_error!(
                            "Expected Text value for {}", stringify!(#name)
                        )),
                    }
                }
            }
        }
    };

    let from_row_values_impl = quote! {
        impl ::ormer::model::FromRowValues for #name {
            fn from_row_values(values: &[::ormer::model::Value]) -> ::ormer::Result<Self> {
                if values.is_empty() {
                    return Err(::ormer::ormer_error!(
                        "Expected at least one value for {}", stringify!(#name)
                    ));
                }
                <#name as ::ormer::model::FromValue>::from_value(&values[0])
            }
        }
    };

    let try_from_i32_impl = if let Some(repr_type) = &repr_integer_type {
        let match_arms = variant_names.iter().map(|v| {
            quote! {
                val if val == #name::#v as #repr_type => Ok(#name::#v),
            }
        });

        quote! {
            impl ::core::convert::TryFrom<i32> for #name {
                type Error = ::ormer::OrmerError;

                fn try_from(value: i32) -> ::ormer::Result<Self> {
                    let repr_value = <#repr_type as ::core::convert::TryFrom<i32>>::try_from(value)
                        .map_err(|err| {
                            ::ormer::ormer_error!(
                                "Failed to convert numeric value '{}' to {}: {}",
                                value,
                                stringify!(#name),
                                err
                            )
                        })?;
                    match repr_value {
                        #(#match_arms)*
                        _ => Err(::ormer::ormer_error!(
                            "Unknown numeric value '{}' for {}", value, stringify!(#name)
                        )),
                    }
                }
            }
        }
    } else {
        quote! {}
    };

    let name_method = {
        // name 的 match 臂只生成一份（FieldType trait 实现）；
        // inherent pub fn name / VARIANTS 委托 trait 实现，不再复制臂与常量
        let name_arms = variant_names.iter().map(|v| {
            let v_str = v.to_string();
            quote! {
                #name::#v => #v_str,
            }
        });

        let from_name_arms = variant_names.iter().map(|v| {
            let v_str = v.to_string();
            quote! {
                #v_str => Ok(#name::#v),
            }
        });

        let from_i64_arms = if is_numeric_enum {
            variant_names
                .iter()
                .map(|v| {
                    quote! {
                        val if val == #name::#v as i64 => Ok(#name::#v),
                    }
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        let numeric_enum_methods = if is_numeric_enum {
            let repr_type = repr_integer_type.as_ref().expect("numeric repr is present");
            let self_to_i64 = if numeric_repr == "isize" {
                quote! { *self as i64 }
            } else {
                // *self 是枚举，先转判别值再提升到 i64
                quote! { <i64 as ::core::convert::From<#repr_type>>::from(*self as #repr_type) }
            };
            quote! {
                fn as_i64(&self) -> i64 {
                    #self_to_i64
                }

                fn from_i64(value: i64) -> ::ormer::Result<Self> {
                    match value {
                        #(#from_i64_arms)*
                        _ => Err(::ormer::ormer_error!(
                            "Unknown numeric value '{}' for {}", value, stringify!(#name)
                        )),
                    }
                }

                fn is_numeric_enum() -> bool {
                    true
                }
            }
        } else {
            quote! {}
        };

        quote! {
            impl #name {
                pub fn name(&self) -> &'static str {
                    <Self as ::ormer::model::FieldType>::name(self)
                }

                pub const VARIANTS: &'static [&'static str] =
                    <#name as ::ormer::model::FieldType>::VARIANTS;
            }

            impl ::ormer::model::FieldType for #name {
                const VARIANTS: &'static [&'static str] = &[#(#variant_names_str),*];

                fn name(&self) -> &'static str {
                    match self {
                        #(#name_arms)*
                    }
                }

                fn from_name(name: &str) -> ::ormer::Result<Self> {
                    match name {
                        #(#from_name_arms)*
                        _ => Err(::ormer::ormer_error!(
                            "Unknown enum variant '{}' for {}", name, stringify!(#name)
                        )),
                    }
                }

                #numeric_enum_methods
            }

            impl ::ormer::model::FieldTypeProvider for #name {
                const ENUM_VARIANTS: Option<&'static [&'static str]> = Some(#name::VARIANTS);
                const DB_VALUE_TYPE: Option<fn(::ormer::DbType) -> &'static str> = None;
            }
        }
    };

    Ok(quote! {
        #try_from_i32_impl
        #from_impl
        #from_value_impl
        #from_row_values_impl
        #name_method
    })
}

enum DbTypeAttr {
    Native,
    String,
    Numeric(syn::Type),
}

struct VariantFieldInfo<'a> {
    variant: &'a Ident,
    field: &'a syn::Field,
    ident: &'a Ident,
    column_name: String,
    rust_type: String,
}

fn derive_data_enum_field_type(
    input: &DeriveInput,
    data_enum: &syn::DataEnum,
) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let db_type = extract_db_type_attr(input)?;
    let variants = &data_enum.variants;
    let variant_names: Vec<&Ident> = variants.iter().map(|v| &v.ident).collect();
    let variant_db_names: Vec<String> = variant_names
        .iter()
        .map(|variant| to_snake_case(&variant.to_string()))
        .collect();

    let mut all_fields = Vec::new();
    let mut seen_columns = std::collections::BTreeSet::new();
    for variant in variants {
        let Fields::Named(fields) = &variant.fields else {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "polymorphic ModelEnum variants must use named fields",
            ));
        };
        for field in &fields.named {
            let ident = field.ident.as_ref().expect("variant field must be named");
            let column_name = extract_column_name(field)?;
            if !seen_columns.insert(column_name.clone()) {
                return Err(syn::Error::new_spanned(
                    field,
                    format!(
                        "polymorphic ModelEnum payload column `{column_name}` is declared more than once"
                    ),
                ));
            }
            all_fields.push(VariantFieldInfo {
                variant: &variant.ident,
                field,
                ident,
                column_name,
                rust_type: field_rust_type(&field.ty),
            });
        }
    }

    let discriminator_rust_type = match &db_type {
        DbTypeAttr::Native => quote! { None },
        DbTypeAttr::String => quote! { Some("String") },
        DbTypeAttr::Numeric(ty) => {
            let ty_str = normalize_type_string(quote! { #ty }.to_string());
            quote! { Some(#ty_str) }
        }
    };
    let enum_variants = match &db_type {
        DbTypeAttr::Native => quote! { Some(#name::VARIANTS) },
        DbTypeAttr::String | DbTypeAttr::Numeric(_) => quote! { None },
    };
    let db_value_type = quote! { None };

    let discriminator_value_arms = variant_names
        .iter()
        .zip(variant_db_names.iter())
        .enumerate()
        .map(|(idx, (variant, db_name))| {
            let pattern = quote! { #name::#variant { .. } };
            discriminator_value_expr(&db_type, idx, db_name, quote! { #pattern })
        });

    let name_arms = variant_names
        .iter()
        .zip(variant_db_names.iter())
        .map(|(variant, db_name)| {
            quote! {
                #name::#variant { .. } => #db_name,
            }
        })
        .collect::<Vec<_>>();
    let field_type_name_arms = name_arms.clone();

    let from_value_match_arms = variant_db_names
        .iter()
        .enumerate()
        .map(|(idx, db_name)| discriminator_known_arm(&db_type, idx, db_name));

    let model_columns = {
        let payload_columns = all_fields.iter().map(|field| {
            let column = field.column_name.as_str();
            quote! { #column }
        });
        quote! {
            fn model_columns(column: &'static str) -> Vec<&'static str> {
                let mut columns = Vec::new();
                columns.push(column);
                columns.extend([#(#payload_columns),*]);
                columns
            }
        }
    };

    let model_column_schema = {
        let payload_schema_entries = all_fields.iter().map(|field| {
            let field_name = field.ident.to_string();
            let column_name = field.column_name.as_str();
            let field_type = &field.field.ty;
            let rust_type = &field.rust_type;
            quote! {
                columns.push(::ormer::model::ColumnSchema {
                    rust_name: ::ormer::model::intern_concat(
                        column_rust_name,
                        concat!(".", #field_name),
                    ),
                    name: #column_name,
                    rust_type: match <#field_type as ::ormer::model::FieldTypeProvider>::RUST_TYPE {
                        Some(rust_type) => rust_type,
                        None => #rust_type,
                    },
                    is_primary: false,
                    is_auto_increment: false,
                    is_nullable: true,
                    unique_group: None,
                    unique_name: None,
                    is_indexed: false,
                    index_group: None,
                    index_name: None,
                    index_order: None,
                    index_where: None,
                    index_method: None,
                    index_expression: None,
                    index_columns: None,
                    foreign_key: None,
                    enum_variants: <#field_type as ::ormer::model::FieldTypeProvider>::ENUM_VARIANTS,
                    data_type: None,
                    db_value_type: <#field_type as ::ormer::model::FieldTypeProvider>::DB_VALUE_TYPE,
                    default: None,
                    check: None,
                    hypertable: None,
                    hypertable_space: None,
                    compress: false,
                    compression: None,
                });
            }
        });
        quote! {
            fn model_column_schema(mut column: ::ormer::model::ColumnSchema) -> Vec<::ormer::model::ColumnSchema> {
                let column_rust_name = column.rust_name;
                column.is_nullable = false;
                let mut columns = Vec::new();
                columns.push(column);
                #(#payload_schema_entries)*
                columns
            }
        }
    };

    let model_has_column = {
        let payload_checks = all_fields.iter().map(|field| {
            let field_name = field.ident.to_string();
            let column_name = field.column_name.as_str();
            quote! {
                || column == #column_name || column == #field_name
            }
        });
        quote! {
            fn model_has_column(
                discriminator_column: &'static str,
                rust_field: &'static str,
                column: &str,
            ) -> bool {
                column == discriminator_column
                    || column == rust_field
                    #(#payload_checks)*
            }
        }
    };

    let model_from_row = {
        let variant_arms = variants
            .iter()
            .zip(variant_db_names.iter())
            .enumerate()
            .map(|(idx, (variant, db_name))| -> syn::Result<TokenStream> {
                let variant_ident = &variant.ident;
                let fields = match &variant.fields {
                    Fields::Named(fields) => &fields.named,
                    _ => unreachable!(),
                };
                let field_values = fields
                    .iter()
                    .map(|field| {
                        let ident = field.ident.as_ref().unwrap();
                        let column_name = extract_column_name(field)?;
                        Ok(quote! {
                            #ident: row.get(#column_name)?
                        })
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                let condition = discriminator_match_condition(&db_type, idx, db_name);
                Ok(quote! {
                    value if #condition => Ok(#name::#variant_ident {
                        #(#field_values),*
                    }),
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        quote! {
            fn model_from_row(
                _rust_field: &'static str,
                discriminator_column: &'static str,
                row: &::ormer::Row,
            ) -> ::ormer::Result<Self> {
                let value = row.get::<::ormer::Value>(discriminator_column)?;
                match &value {
                    #(#variant_arms)*
                    _ => Err(::ormer::ormer_error!(
                        "Unknown discriminator value for {}.{}",
                        stringify!(#name),
                        discriminator_column
                    )),
                }
            }
        }
    };

    let model_from_row_values = {
        let expected_len = 1usize + all_fields.len();
        let variant_arms = variants
            .iter()
            .zip(variant_db_names.iter())
            .enumerate()
            .map(|(idx, (variant, db_name))| -> syn::Result<TokenStream> {
                let variant_ident = &variant.ident;
                let fields = match &variant.fields {
                    Fields::Named(fields) => &fields.named,
                    _ => unreachable!(),
                };
                let field_values = fields
                    .iter()
                    .map(|field| {
                        let ident = field.ident.as_ref().unwrap();
                        let column_name = extract_column_name(field)?;
                        let field_type = &field.ty;
                        // all_fields 收集自同样的 payload 字段，查找必然命中
                        let value_index = all_fields
                            .iter()
                            .position(|candidate| candidate.column_name == column_name)
                            .expect("payload field must be indexed")
                            + 1;
                        let value_index = syn::Index::from(value_index);
                        Ok(quote! {
                            #ident: <#field_type as ::ormer::FromRowValues>::from_row_values(
                                &values[#value_index..#value_index + 1]
                            )?
                        })
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                let condition = discriminator_match_condition(&db_type, idx, db_name);
                Ok(quote! {
                    value if #condition => Ok(#name::#variant_ident {
                        #(#field_values),*
                    }),
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        quote! {
            fn model_from_row_values(
                _rust_field: &'static str,
                _discriminator_column: &'static str,
                values: &[::ormer::Value],
            ) -> ::ormer::Result<Self> {
                if values.len() < #expected_len {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        #expected_len,
                        stringify!(#name)
                    ));
                }
                let value = &values[0];
                match value {
                    #(#variant_arms)*
                    _ => Err(::ormer::ormer_error!(
                        "Unknown discriminator value for {}",
                        stringify!(#name)
                    )),
                }
            }
        }
    };

    let model_field_values = {
        let variant_arms = variants
            .iter()
            .zip(variant_db_names.iter())
            .enumerate()
            .map(|(idx, (variant, db_name))| {
                let variant_ident = &variant.ident;
                let fields = match &variant.fields {
                    Fields::Named(fields) => &fields.named,
                    _ => unreachable!(),
                };
                let pattern_fields = fields.iter().map(|field| field.ident.as_ref().unwrap());
                let field_pushes = all_fields.iter().map(|field| {
                    let ident = field.ident;
                    if field.variant == variant_ident {
                        quote! {
                            values.push(::ormer::Value::from(#ident.clone()));
                        }
                    } else {
                        quote! {
                            values.push(::ormer::Value::Null);
                        }
                    }
                });
                let discriminator = discriminator_value_tokens(&db_type, idx, db_name);
                quote! {
                    #name::#variant_ident { #(#pattern_fields),* } => {
                        let mut values = Vec::new();
                        values.push(#discriminator);
                        #(#field_pushes)*
                        values
                    }
                }
            });
        quote! {
            fn model_field_values(&self) -> Vec<::ormer::Value> {
                match self {
                    #(#variant_arms),*
                }
            }
        }
    };

    let model_column_value = {
        let discriminator_arms = variant_names
            .iter()
            .zip(variant_db_names.iter())
            .enumerate()
            .map(|(idx, (variant, db_name))| {
                let discriminator = discriminator_value_tokens(&db_type, idx, db_name);
                quote! {
                    #name::#variant { .. } => #discriminator,
                }
            });
        let payload_arms = all_fields.iter().map(|field| {
            let field_ident = field.ident;
            let field_name = field_ident.to_string();
            let column_name = field.column_name.as_str();
            let field_variant = field.variant;
            quote! {
                #column_name | #field_name => {
                    Some(match self {
                        #name::#field_variant { #field_ident, .. } => {
                            ::ormer::Value::from(#field_ident.clone())
                        }
                        _ => ::ormer::Value::Null,
                    })
                }
            }
        });
        quote! {
            fn model_column_value(
                &self,
                discriminator_column: &'static str,
                rust_field: &'static str,
                column: &str,
            ) -> Option<::ormer::Value> {
                if column == discriminator_column || column == rust_field {
                    return Some(match self {
                        #(#discriminator_arms)*
                    });
                }
                match column {
                    #(#payload_arms,)*
                    _ => None,
                }
            }
        }
    };

    let model_assign_column_value = {
        // discriminator 列：值与当前变体一致时确认赋值（图写入回填关系键的
        // 场景），不一致时报错——仅凭 discriminator 无法切换变体（payload 未知）。
        let discriminator_arms = variant_names
            .iter()
            .zip(variant_db_names.iter())
            .enumerate()
            .map(|(idx, (variant, db_name))| {
                let condition = discriminator_match_condition(&db_type, idx, db_name);
                quote! {
                    #name::#variant { .. } => #condition,
                }
            });
        // payload 列：当前变体与声明该列的变体一致时整体写入对应字段；
        // 不一致时报错（切换变体需要完整 payload，无法从单列值构造）。
        let payload_arms = all_fields.iter().map(|field| {
            let column_name = field.column_name.as_str();
            let field_name_str = field.ident.to_string();
            let field_ident = field.ident;
            let field_variant = field.variant;
            let field_type = &field.field.ty;
            quote! {
                #column_name | #field_name_str => {
                    match self {
                        #name::#field_variant { #field_ident, .. } => {
                            *#field_ident =
                                <#field_type as ::ormer::model::FromValue>::from_value(&value)?;
                            Ok(true)
                        }
                        _ => Err(::ormer::ormer_error!(
                            "cannot assign payload column {} of {} while a different variant is active",
                            column,
                            stringify!(#name),
                        )),
                    }
                }
            }
        });
        quote! {
            fn model_assign_column_value(
                &mut self,
                discriminator_column: &'static str,
                rust_field: &'static str,
                column: &str,
                value: ::ormer::Value,
            ) -> ::ormer::Result<bool> {
                if column == discriminator_column || column == rust_field {
                    let value = &value;
                    let matches_current = match self {
                        #(#discriminator_arms)*
                    };
                    return if matches_current {
                        Ok(true)
                    } else {
                        Err(::ormer::ormer_error!(
                            "cannot switch {} to the variant for discriminator column {}",
                            stringify!(#name),
                            column,
                        ))
                    };
                }
                match column {
                    #(#payload_arms,)*
                    _ => Ok(false),
                }
            }
        }
    };

    Ok(quote! {
        impl From<#name> for ::ormer::model::Value {
            fn from(value: #name) -> Self {
                match value {
                    #(#discriminator_value_arms)*
                }
            }
        }

        impl ::ormer::model::FromValue for #name {
            fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                match value {
                    #(#from_value_match_arms)*
                    _ => Err(::ormer::ormer_error!(
                        "{} requires flattened row values and cannot be built from a discriminator alone",
                        stringify!(#name)
                    )),
                }
            }
        }

        impl ::ormer::model::FromRowValues for #name {
            fn from_row_values(values: &[::ormer::model::Value]) -> ::ormer::Result<Self> {
                <#name as ::ormer::model::FieldTypeProvider>::model_from_row_values(
                    stringify!(#name),
                    stringify!(#name),
                    values,
                )
            }
        }

        impl #name {
            pub fn name(&self) -> &'static str {
                match self {
                    #(#name_arms)*
                }
            }

            pub const VARIANTS: &'static [&'static str] = &[#(#variant_db_names),*];
        }

        impl ::ormer::model::FieldType for #name {
            const VARIANTS: &'static [&'static str] = &[#(#variant_db_names),*];

            fn name(&self) -> &'static str {
                match self {
                    #(#field_type_name_arms)*
                }
            }

            fn from_name(_name: &str) -> ::ormer::Result<Self> {
                Err(::ormer::ormer_error!(
                    "{} requires flattened row values and cannot be built from a discriminator alone",
                    stringify!(#name)
                ))
            }
        }

        impl ::ormer::model::FieldTypeProvider for #name {
            const ENUM_VARIANTS: Option<&'static [&'static str]> = #enum_variants;
            const DB_VALUE_TYPE: Option<fn(::ormer::DbType) -> &'static str> = #db_value_type;
            const RUST_TYPE: Option<&'static str> = #discriminator_rust_type;

            #model_columns
            #model_column_schema
            #model_has_column
            #model_from_row
            #model_from_row_values
            #model_field_values
            #model_column_value
            #model_assign_column_value
        }
    })
}

fn discriminator_value_expr(
    db_type: &DbTypeAttr,
    idx: usize,
    db_name: &str,
    pattern: TokenStream,
) -> TokenStream {
    let value = discriminator_value_tokens(db_type, idx, db_name);
    quote! {
        #pattern => #value,
    }
}

fn discriminator_value_tokens(db_type: &DbTypeAttr, idx: usize, db_name: &str) -> TokenStream {
    match db_type {
        DbTypeAttr::Numeric(ty) => {
            quote! { ::ormer::Value::from(#idx as #ty) }
        }
        DbTypeAttr::Native | DbTypeAttr::String => {
            quote! { ::ormer::Value::Text(#db_name.to_string()) }
        }
    }
}

fn discriminator_known_arm(db_type: &DbTypeAttr, idx: usize, db_name: &str) -> TokenStream {
    match db_type {
        DbTypeAttr::Numeric(_) => {
            quote! {
                ::ormer::Value::Integer(value) if *value == #idx as i64 => Err(::ormer::ormer_error!(
                    "{} requires flattened row values and cannot be built from a discriminator alone",
                    stringify!(#db_name)
                )),
            }
        }
        DbTypeAttr::Native | DbTypeAttr::String => {
            quote! {
                ::ormer::Value::Text(value) if value == #db_name => Err(::ormer::ormer_error!(
                    "{} requires flattened row values and cannot be built from a discriminator alone",
                    stringify!(#db_name)
                )),
            }
        }
    }
}

fn discriminator_match_condition(db_type: &DbTypeAttr, idx: usize, db_name: &str) -> TokenStream {
    match db_type {
        DbTypeAttr::Numeric(_) => {
            quote! { matches!(value, ::ormer::Value::Integer(raw) if *raw == #idx as i64) }
        }
        DbTypeAttr::Native | DbTypeAttr::String => {
            quote! { matches!(value, ::ormer::Value::Text(raw) if raw == #db_name) }
        }
    }
}

fn extract_db_type_attr(input: &DeriveInput) -> syn::Result<DbTypeAttr> {
    for attr in &input.attrs {
        if !attr.path().is_ident("db_type") {
            continue;
        }
        return match &attr.meta {
            Meta::Path(_) => Ok(DbTypeAttr::Native),
            Meta::List(list) => {
                let ty: syn::Type = syn::parse2(list.tokens.clone())
                    .map_err(|_| syn::Error::new_spanned(attr, "#[db_type] type is invalid"))?;
                if is_string_type(&ty) {
                    Ok(DbTypeAttr::String)
                } else {
                    Ok(DbTypeAttr::Numeric(ty))
                }
            }
            Meta::NameValue(_) => Err(syn::Error::new_spanned(
                attr,
                "#[db_type] must use #[db_type] or #[db_type(Type)]",
            )),
        };
    }
    Ok(DbTypeAttr::Native)
}

// extract_column_name / to_snake_case / normalize_type_string /
// option_inner_type / is_string_type 统一使用 crate::shared 中的实现

fn field_rust_type(ty: &syn::Type) -> String {
    let ty = option_inner_type(ty).unwrap_or(ty);
    normalize_type_string(quote! { #ty }.to_string())
}

fn repr_integer_type(input: &DeriveInput) -> Option<syn::Type> {
    for attr in &input.attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let syn::Meta::List(list) = &attr.meta else {
            continue;
        };
        let tokens = list.tokens.to_string().replace(' ', "");
        for repr in tokens.split(',') {
            if matches!(
                repr,
                "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize"
            ) {
                return syn::parse_str::<syn::Type>(repr).ok();
            }
        }
    }
    None
}

fn derive_tuple_struct_field_type(
    input: &DeriveInput,
    data_struct: &syn::DataStruct,
) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let inner_type = match &data_struct.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0].ty,
        _ => {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "FieldType can only be derived for enums or single-field tuple structs",
            ))
        }
    };
    let inner_type_str = normalize_type_string(quote! { #inner_type }.to_string());

    Ok(quote! {
        impl From<#name> for ::ormer::model::Value {
            fn from(value: #name) -> Self {
                ::ormer::model::Value::from(value.0)
            }
        }

        impl ::ormer::model::FromValue for #name {
            fn from_value(value: &::ormer::model::Value) -> ::ormer::Result<Self> {
                <#inner_type as ::ormer::model::FromValue>::from_value(value).map(#name)
            }
        }

        impl ::ormer::model::FromRowValues for #name {
            fn from_row_values(values: &[::ormer::model::Value]) -> ::ormer::Result<Self> {
                let value = values.first().ok_or_else(|| {
                    ::ormer::ormer_error!("Expected at least one value for {}", stringify!(#name))
                })?;
                <#name as ::ormer::model::FromValue>::from_value(value)
            }
        }

        impl ::ormer::model::FieldTypeProvider for #name {
            const ENUM_VARIANTS: Option<&'static [&'static str]> = None;
            const DB_VALUE_TYPE: Option<fn(::ormer::DbType) -> &'static str> = None;
            const RUST_TYPE: Option<&'static str> = Some(#inner_type_str);
        }

        impl ::ormer::model::FieldType for #name {
            const VARIANTS: &'static [&'static str] = &[];

            fn name(&self) -> &'static str {
                stringify!(#name)
            }

            fn from_name(_name: &str) -> ::ormer::Result<Self> {
                Err(::ormer::ormer_error!(
                    "{} is not an enum field type", stringify!(#name)
                ))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    // 校验失败统一展开为 compile_error!，断言错误文本出现在产物中

    #[test]
    fn rejects_u64_repr_numeric_enum() {
        let input: DeriveInput = parse_quote! {
            #[repr(u64)]
            enum WideFlag {
                Big = 5_000_000_000,
            }
        };
        let tokens = derive_model_enum(input).to_string();
        assert!(tokens.contains("numeric enums are not supported"), "{tokens}");
    }

    #[test]
    fn rejects_usize_repr_numeric_enum() {
        let input: DeriveInput = parse_quote! {
            #[repr(usize)]
            enum WideFlag {
                Big = 1,
            }
        };
        let tokens = derive_model_enum(input).to_string();
        assert!(tokens.contains("numeric enums are not supported"), "{tokens}");
    }

    #[test]
    fn numeric_enum_uses_lossless_i64_conversion_and_skips_i32_for_wide_repr() {
        let input: DeriveInput = parse_quote! {
            #[repr(i64)]
            enum Status {
                Disabled = 0,
                Active = 1,
            }
        };
        let tokens = derive_model_enum(input).to_string();
        assert!(
            tokens.contains("< i64 as :: core :: convert :: From < i64 >> :: from (v as i64)"),
            "expected lossless From conversion, got: {tokens}"
        );
        // 宽 repr 不再注入 From<Enum> for i32（会截断/与用户手写 impl 冲突）
        assert!(
            !tokens.contains("impl :: core :: convert :: From < Status > for i32"),
            "wide repr must not inject From<Enum> for i32: {tokens}"
        );
    }

    #[test]
    fn narrow_repr_numeric_enum_keeps_i32_from_impl() {
        let input: DeriveInput = parse_quote! {
            #[repr(u16)]
            enum Status {
                Disabled = 0,
                Active = 1,
            }
        };
        let tokens = derive_model_enum(input).to_string();
        assert!(
            tokens.contains("impl :: core :: convert :: From < Status > for i32"),
            "narrow repr keeps the lossless From<Enum> for i32 impl: {tokens}"
        );
    }

    #[test]
    fn polymorphic_enum_generates_assign_column_value_impl() {
        let input: DeriveInput = parse_quote! {
            enum Attr {
                Text { text_value: String },
                Num { num_value: i64 },
            }
        };
        let tokens = derive_model_enum(input).to_string();
        assert!(tokens.contains("model_assign_column_value"), "{tokens}");
        // 不再是恒返回 Ok(false) 的空实现
        assert!(!tokens.contains("Ok (false) }, "), "{tokens}");
    }
}
