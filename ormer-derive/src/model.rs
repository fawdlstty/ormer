use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr, ExprLit, Lit, Meta};

pub fn derive_model(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let where_name = syn::Ident::new(&format!("{name}Where"), name.span());

    // 提取表名
    let table_name = extract_table_name(&input);

    // 检查是否为元组结构体（用于包装现有模型）
    let is_tuple_struct = matches!(&input.data, syn::Data::Struct(data) if matches!(&data.fields, syn::Fields::Unnamed(_)));

    if is_tuple_struct {
        return derive_model_tuple_wrapper(&input, name, &where_name, table_name);
    }

    // 提取字段（普通命名字段结构体）
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("Model must have named fields or be a tuple struct wrapper"),
        },
        _ => panic!("Model must be a struct"),
    };

    // 提取主键字段及自增信息
    let primary_key_info = fields
        .iter()
        .find_map(|f| {
            for attr in &f.attrs {
                if attr.path().is_ident("primary") {
                    let field_name = f.ident.as_ref().unwrap().clone();
                    // 检查是否有 (auto) 参数
                    let is_auto = if let Meta::List(list) = &attr.meta {
                        list.tokens.to_string().contains("auto")
                    } else {
                        false
                    };
                    return Some((field_name, is_auto));
                }
            }
            None
        })
        .expect("Model must have a #[primary] field");

    let primary_key_field = primary_key_info.0;
    let is_auto_increment = primary_key_info.1;

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

        // 检查是否是自增主键（只有主键字段才可能是自增）
        let field_is_auto_increment = if is_primary { is_auto_increment } else { false };

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

        // 检查 unique 属性
        let unique_group = extract_unique_group(f);

        // 检查 index 属性
        let is_indexed = f.attrs.iter().any(|attr| attr.path().is_ident("index"));

        // 检查 foreign 属性
        let foreign_key = extract_foreign_key(f);

        quote! {
            ::ormer::model::ColumnSchema {
                name: stringify!(#field_name),
                rust_type: #rust_type,
                is_primary: #is_primary,
                is_auto_increment: #field_is_auto_increment,
                is_nullable: #is_nullable,
                unique_group: #unique_group,
                is_indexed: #is_indexed,
                foreign_key: #foreign_key,
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

    // 生成 from_row_values 实现（按顺序从行值中读取）
    let from_row_values_fields = fields.iter().enumerate().map(|(i, f)| {
        let field_name = f.ident.as_ref().unwrap();
        let field_type = &f.ty;
        quote! {
            #field_name: <#field_type as ::ormer::FromRowValues>::from_row_values(
                &values[#i..#i+1]
            )?
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
    // 为所有字段生成类型化列代理
    let where_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let field_type = &f.ty;
        quote! {
            pub #field_name: ::ormer::query::builder::TypedColumn<#field_type>
        }
    });

    // 生成 Where 的 Default 实现
    let where_default_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        quote! {
            #field_name: ::ormer::query::builder::TypedColumn::new(stringify!(#field_name))
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

            fn from_row_values(values: &[::ormer::Value]) -> Result<Self, ::ormer::Error> {
                if values.len() < Self::COLUMNS.len() {
                    return Err(::ormer::Error::TypeMismatch(
                        format!("Expected {} values for {}", Self::COLUMNS.len(), stringify!(#name))
                    ));
                }
                Ok(Self {
                    #(#from_row_values_fields),*
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

            fn is_primary_auto_increment() -> bool {
                #is_auto_increment
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

/// 为元组结构体包装模型生成实现（例如：struct NewUser(User);）
fn derive_model_tuple_wrapper(
    input: &DeriveInput,
    name: &syn::Ident,
    _where_name: &syn::Ident,
    table_name: String,
) -> TokenStream {
    // 提取元组结构体中的内部类型
    let inner_type = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Unnamed(fields) => {
                if fields.unnamed.len() != 1 {
                    panic!("Tuple struct wrapper must have exactly one field");
                }
                &fields.unnamed[0].ty
            }
            _ => panic!("Expected unnamed fields"),
        },
        _ => panic!("Expected struct"),
    };

    // 生成代码：元组结构体包装器将委托给内部类型的所有 Model 功能，但使用自定义表名
    quote! {
        impl ::ormer::Model for #name {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMNS: &'static [&'static str] = <#inner_type as ::ormer::Model>::COLUMNS;
            const COLUMN_SCHEMA: &'static [::ormer::model::ColumnSchema] = <#inner_type as ::ormer::Model>::COLUMN_SCHEMA;

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = <#inner_type as ::ormer::Model>::Where;

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> Result<Self, ::ormer::Error> {
                let inner = <#inner_type as ::ormer::Model>::from_row(row)?;
                Ok(#name(inner))
            }

            fn from_row_values(values: &[::ormer::Value]) -> Result<Self, ::ormer::Error> {
                let inner = <#inner_type as ::ormer::Model>::from_row_values(values)?;
                Ok(#name(inner))
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                self.0.field_values()
            }

            fn primary_key_column() -> &'static str {
                <#inner_type as ::ormer::Model>::primary_key_column()
            }

            fn primary_key_value(&self) -> ::ormer::Value {
                self.0.primary_key_value()
            }

            fn is_primary_auto_increment() -> bool {
                <#inner_type as ::ormer::Model>::is_primary_auto_increment()
            }
        }

        // 生成 inherent 方法
        impl #name {
            pub fn select() -> ::ormer::Select<Self> {
                ::ormer::Select::new()
            }

            pub fn query() -> ::ormer::Select<Self> {
                ::ormer::Select::new()
            }
        }

        // 为包装器类型实现 Into<InnerType> 和 From<InnerType>
        impl From<#inner_type> for #name {
            fn from(inner: #inner_type) -> Self {
                #name(inner)
            }
        }

        impl #name {
            pub fn into_inner(self) -> #inner_type {
                self.0
            }

            pub fn inner(&self) -> &#inner_type {
                &self.0
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

/// 提取 unique 属性的 group 值
fn extract_unique_group(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if attr.path().is_ident("unique") {
            // 检查是否有 group 参数
            if let Meta::List(list) = &attr.meta {
                // 解析 tokens 查找 group = N
                let tokens_str = list.tokens.to_string();
                if tokens_str.contains("group") {
                    // 尝试提取 group 值
                    if let Ok(Meta::NameValue(meta)) = syn::parse2(list.tokens.clone()) {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Int(lit_int),
                            ..
                        }) = &meta.value
                        {
                            let group_value: i32 = lit_int.base10_parse().unwrap_or(0);
                            return quote! { Some(#group_value) };
                        }
                    }
                }
            }
            // 没有 group 参数，使用 0 作为默认组
            return quote! { Some(0) };
        }
    }
    // 没有 unique 属性
    quote! { None }
}

/// 提取 foreign 属性的外键信息
/// 支持两种语法：
/// - #[foreign(Type)] - 新语法，自动关联到目标 model 的主键
/// - #[foreign(Type.field)] - 旧语法，显式指定字段
fn extract_foreign_key(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if attr.path().is_ident("foreign") {
            if let Meta::List(list) = &attr.meta {
                let tokens_str = list.tokens.to_string();

                // 尝试解析为 Type.field 格式（旧语法）
                let parts: Vec<&str> = tokens_str.split('.').collect();
                if parts.len() == 2 {
                    let ref_type = parts[0].trim();
                    let ref_field = parts[1].trim();
                    let ref_type_ident = syn::Ident::new(ref_type, proc_macro2::Span::call_site());

                    // 使用目标模型的实际表名，而不是简单转换
                    return quote! {
                        Some(::ormer::model::ForeignKeyInfo {
                            ref_table: <#ref_type_ident as ::ormer::Model>::TABLE_NAME,
                            ref_column: #ref_field,
                            ref_column_fn: None,
                        })
                    };
                } else if parts.len() == 1 {
                    // 新语法：只传递类型，自动关联到目标 model 的主键
                    let ref_type = parts[0].trim();
                    let ref_type_ident = syn::Ident::new(ref_type, proc_macro2::Span::call_site());

                    // 使用函数指针在运行时获取目标模型的主键字段名（避免在常量上下文中调用非 const 函数）
                    return quote! {
                        Some(::ormer::model::ForeignKeyInfo {
                            ref_table: <#ref_type_ident as ::ormer::Model>::TABLE_NAME,
                            ref_column: "",
                            ref_column_fn: Some(<#ref_type_ident as ::ormer::Model>::primary_key_column),
                        })
                    };
                }
            }
        }
    }
    // 没有 foreign 属性
    quote! { None }
}
