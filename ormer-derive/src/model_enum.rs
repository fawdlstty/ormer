use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident};

pub fn derive_model_enum(input: DeriveInput) -> TokenStream {
    derive_field_type(input)
}

pub fn derive_field_type(input: DeriveInput) -> TokenStream {
    match &input.data {
        Data::Enum(data_enum) => derive_enum_field_type(&input, data_enum),
        Data::Struct(data_struct) => derive_tuple_struct_field_type(&input, data_struct),
        _ => panic!("FieldType can only be derived for enums or single-field tuple structs"),
    }
}

fn derive_enum_field_type(input: &DeriveInput, data_enum: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;
    let variants = &data_enum.variants;
    let variant_names: Vec<&Ident> = variants.iter().map(|v| &v.ident).collect();
    let variant_names_str: Vec<String> = variant_names.iter().map(|v| v.to_string()).collect();

    let repr_integer_type = repr_integer_type(input);
    let is_numeric_enum = repr_integer_type.is_some();

    let from_value_impl = if is_numeric_enum {
        quote! {
            impl From<#name> for ::ormer::model::Value {
                fn from(v: #name) -> Self {
                    ::ormer::model::Value::Integer(v as i64)
                }
            }

            impl ::core::convert::From<#name> for i32 {
                fn from(v: #name) -> Self {
                    v as i32
                }
            }
        }
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
        let match_arms_1 = variant_names.iter().map(|v| {
            let v_str = v.to_string();
            quote! {
                #name::#v => #v_str,
            }
        });

        let match_arms_2 = variant_names.iter().map(|v| {
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
            quote! {
                fn as_i64(&self) -> i64 {
                    *self as i64
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
                    match self {
                        #(#match_arms_1)*
                    }
                }

                pub const VARIANTS: &'static [&'static str] = &[#(#variant_names_str),*];
            }

            impl ::ormer::model::FieldType for #name {
                const VARIANTS: &'static [&'static str] = &[#(#variant_names_str),*];

                fn name(&self) -> &'static str {
                    match self {
                        #(#match_arms_2)*
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

    quote! {
        #try_from_i32_impl
        #from_impl
        #from_value_impl
        #from_row_values_impl
        #name_method
    }
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
) -> TokenStream {
    let name = &input.ident;
    let inner_type = match &data_struct.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0].ty,
        _ => panic!("FieldType can only be derived for enums or single-field tuple structs"),
    };
    let inner_type_str = normalize_type_string(quote! { #inner_type }.to_string());

    quote! {
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
    }
}

fn normalize_type_string(type_str: String) -> String {
    type_str
        .replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" , ", ",")
}
