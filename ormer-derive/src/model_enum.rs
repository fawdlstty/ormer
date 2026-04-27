use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Ident};

pub fn derive_model_enum(input: DeriveInput) -> TokenStream {
    let name = &input.ident;

    // 确保是枚举类型
    let variants = match &input.data {
        Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("ModelEnum can only be derived for enums"),
    };

    // 提取所有变体名称
    let variant_names: Vec<&Ident> = variants.iter().map(|v| &v.ident).collect();

    let variant_names_str: Vec<String> = variant_names.iter().map(|v| v.to_string()).collect();

    // 生成 From<EnumType> for Value 实现 (用于插入)
    let from_value_impl = {
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

    // 生成 FromValue for EnumType 实现 (用于读取)
    let from_impl = {
        let match_arms = variant_names.iter().map(|v| {
            let v_str = v.to_string();
            quote! {
                #v_str => Ok(#name::#v),
            }
        });

        quote! {
            impl ::ormer::model::FromValue for #name {
                fn from_value(value: &::ormer::model::Value) -> Result<Self, ::ormer::Error> {
                    match value {
                        ::ormer::model::Value::Text(s) => {
                            match s.as_str() {
                                #(#match_arms)*
                                _ => Err(::ormer::Error::TypeMismatch(
                                    format!("Unknown enum variant '{}' for {}", s, stringify!(#name))
                                )),
                            }
                        }
                        _ => Err(::ormer::Error::TypeMismatch(
                            format!("Expected Text value for {}", stringify!(#name))
                        )),
                    }
                }
            }
        }
    };

    // 为枚举类型本身实现 FromValue (非 Option)
    // 这已经由上面的 from_impl 实现了

    // 生成 FromRowValues for EnumType 实现
    let from_row_values_impl = quote! {
        impl ::ormer::model::FromRowValues for #name {
            fn from_row_values(values: &[::ormer::model::Value]) -> Result<Self, ::ormer::Error> {
                if values.is_empty() {
                    return Err(::ormer::Error::TypeMismatch(
                        format!("Expected at least one value for {}", stringify!(#name))
                    ));
                }
                <#name as ::ormer::model::FromValue>::from_value(&values[0])
            }
        }
    };

    // 生成 inherent 方法和 trait 实现
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

        quote! {
            impl #name {
                pub fn name(&self) -> &'static str {
                    match self {
                        #(#match_arms_1)*
                    }
                }

                pub const VARIANTS: &'static [&'static str] = &[#(#variant_names_str),*];
            }

            impl ::ormer::model::ModelEnum for #name {
                const VARIANTS: &'static [&'static str] = &[#(#variant_names_str),*];

                fn name(&self) -> &'static str {
                    match self {
                        #(#match_arms_2)*
                    }
                }

                fn from_name(name: &str) -> Result<Self, ::ormer::Error> {
                    match name {
                        #(#from_name_arms)*
                        _ => Err(::ormer::Error::TypeMismatch(
                            format!("Unknown enum variant '{}' for {}", name, stringify!(#name))
                        )),
                    }
                }
            }

            impl ::ormer::model::ModelEnumProvider for #name {
                fn enum_variants() -> Option<&'static [&'static str]> {
                    Some(#name::VARIANTS)
                }
            }
        }
    };

    // 注意: 不能生成 From<Option<EnumType>> for Value,
    // 因为这会违反 orphan rule (Option 和 Value 都不是本地类型)
    // Option<EnumType> 的转换由通用的 Option<T> 实现处理

    quote! {
        #from_impl
        #from_value_impl
        #from_row_values_impl
        #name_method
    }
}
