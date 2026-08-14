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

    // 检测是否为数值枚举（检查 #[repr(i*)] 属性）
    let is_numeric_enum = input.attrs.iter().any(|attr| {
        if attr.path().is_ident("repr") {
            if let syn::Meta::List(list) = &attr.meta {
                let tokens_str = list.tokens.to_string();
                tokens_str.starts_with("i") || tokens_str.starts_with("u")
            } else {
                false
            }
        } else {
            false
        }
    });

    let is_i32_repr = input.attrs.iter().any(|attr| {
        if attr.path().is_ident("repr") {
            if let syn::Meta::List(list) = &attr.meta {
                list.tokens.to_string().replace(' ', "") == "i32"
            } else {
                false
            }
        } else {
            false
        }
    });

    // 生成 From<EnumType> for Value 实现 (用于插入)
    let from_value_impl = if is_numeric_enum {
        // 数值枚举：转为 Integer
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
        // 字符串枚举：转为 Text
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
    let from_impl = if is_numeric_enum {
        // 数值枚举：从 Integer 读取
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
        // 字符串枚举：从 Text 读取
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

    // 为枚举类型本身实现 FromValue (非 Option)
    // 这已经由上面的 from_impl 实现了

    // 生成 FromRowValues for EnumType 实现
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

    let try_from_i32_impl = if is_i32_repr {
        let match_arms = variant_names.iter().map(|v| {
            quote! {
                val if val == #name::#v as i32 => Ok(#name::#v),
            }
        });

        quote! {
            impl ::core::convert::TryFrom<i32> for #name {
                type Error = ::ormer::OrmerError;

                fn try_from(value: i32) -> ::ormer::Result<Self> {
                    match value {
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

        // 为数值枚举生成 from_i64_arms
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

        // 生成数值枚举特有方法
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

            impl ::ormer::model::ModelEnum for #name {
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

            impl ::ormer::model::ModelEnumProvider for #name {
                const ENUM_VARIANTS: Option<&'static [&'static str]> = Some(#name::VARIANTS);
                const DB_VALUE_TYPE: Option<fn(::ormer::DbType) -> &'static str> = None;
            }
        }
    };

    // 注意: 不能生成 From<Option<EnumType>> for Value,
    // 因为这会违反 orphan rule (Option 和 Value 都不是本地类型)
    // Option<EnumType> 的转换由通用的 Option<T> 实现处理

    quote! {
        #try_from_i32_impl
        #from_impl
        #from_value_impl
        #from_row_values_impl
        #name_method
    }
}
