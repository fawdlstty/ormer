use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Lit, Meta};

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

    for field in fields {
        validate_data_type(field);
    }

    let normal_fields: Vec<&syn::Field> = fields.iter().filter(|f| !is_relation_field(f)).collect();
    let relation_fields: Vec<RelationField> =
        fields.iter().filter_map(extract_relation_field).collect();

    // 提取主键字段列表（支持复合主键）
    let primary_keys: Vec<_> = normal_fields
        .iter()
        .filter_map(|f| {
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
        .collect();

    // 至少需要一个主键
    if primary_keys.is_empty() {
        panic!("Model must have at least one #[primary] field");
    }

    // 检查是否有多个主键且标记了 auto（只有第一个主键可以是 auto）
    let auto_count = primary_keys.iter().filter(|(_, is_auto)| *is_auto).count();
    if auto_count > 1 {
        panic!("Only one primary key field can have #[primary(auto)]");
    }

    // 获取第一个主键（用于向后兼容）
    let primary_key_field = &primary_keys[0].0;
    let is_auto_increment = primary_keys[0].1;

    // 生成 AutoIncrementKeyType
    // 如果有自增主键，类型为第一个主键的 Rust 类型；否则为 ()
    let auto_increment_key_type = if is_auto_increment {
        let pk_type = &fields
            .iter()
            .find(|f| f.ident.as_ref().unwrap() == primary_key_field)
            .map(|f| &f.ty)
            .expect("Primary key field not found");
        quote! { #pk_type }
    } else {
        quote! { () }
    };

    // 生成主键列名列表（支持复合主键）
    let primary_key_field_names: Vec<_> = primary_keys
        .iter()
        .map(|(field_name, _)| {
            let field = normal_fields
                .iter()
                .find(|f| f.ident.as_ref().unwrap() == field_name)
                .expect("Primary key field not found");
            let column_name = extract_column_name(field);
            quote! { #column_name }
        })
        .collect();
    let primary_key_column_name = {
        let field = normal_fields
            .iter()
            .find(|f| f.ident.as_ref().unwrap() == primary_key_field)
            .expect("Primary key field not found");
        extract_column_name(field)
    };

    // 生成主键值获取（支持复合主键）
    let primary_key_values: Vec<_> = primary_keys
        .iter()
        .map(|(field_name, _)| {
            let field = fields
                .iter()
                .find(|f| f.ident.as_ref().unwrap() == field_name)
                .expect("Primary key field not found");
            field_to_value_expr(field)
        })
        .collect();

    let primary_key_value_expr = {
        let field = fields
            .iter()
            .find(|f| f.ident.as_ref().unwrap() == primary_key_field)
            .expect("Primary key field not found");
        field_to_value_expr(field)
    };

    // 生成字段名列表
    let field_names: Vec<String> = normal_fields
        .iter()
        .map(|f| extract_column_name(f))
        .collect();

    let field_names_lit = field_names.iter().map(|name| {
        quote! { #name }
    });

    // 生成字段元数据 (COLUMN_SCHEMA)
    let column_schema_entries = normal_fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let rust_field_name = field_name.to_string();
        let column_name = extract_column_name(f);
        let field_type = &f.ty;
        let type_str = normalize_type_string(quote! { #field_type }.to_string());

        // 检查是否是主键字段
        let is_primary = f.attrs.iter().any(|attr| attr.path().is_ident("primary"));

        // 检查是否是自增主键（只有主键字段才可能是自增）
        let field_is_auto_increment = if is_primary { is_auto_increment } else { false };

        // 检查是否是 Option<T>
        let is_nullable = type_str.starts_with("Option<");

        // 提取基础 Rust 类型
        let rust_type = if is_nullable {
            type_str
                .strip_prefix("Option<")
                .and_then(|ty| ty.strip_suffix('>'))
                .unwrap_or(&type_str)
                .trim()
                .to_string()
        } else {
            type_str
        };

        // 检查 unique 属性
        let unique_attr = extract_unique_attr(f);
        let unique_group = option_i32_tokens(unique_attr.group);
        let unique_name = option_string_tokens(unique_attr.name.as_deref());

        // 检查 index 属性
        let index_attr = extract_index_attr(f);
        let is_indexed = index_attr.is_some();
        let index_group = option_i32_tokens(index_attr.as_ref().and_then(|attr| attr.group));
        let index_name =
            option_string_tokens(index_attr.as_ref().and_then(|attr| attr.name.as_deref()));
        let index_order =
            option_string_tokens(index_attr.as_ref().and_then(|attr| attr.order.as_deref()));
        let index_where = option_string_tokens(
            index_attr
                .as_ref()
                .and_then(|attr| attr.where_clause.as_deref()),
        );

        // 检查 foreign 属性
        let foreign_key = extract_foreign_key(f);

        // 检查 data_type 属性
        let data_type = extract_data_type(f);
        let has_data_type = has_data_type(f);

        // 检查 default/check 属性
        let default = extract_default(f);
        let check = extract_check(f);

        // 检查 hypertable 属性
        let hypertable = extract_hypertable(f);

        // 检查 compress 属性
        let compress = f.attrs.iter().any(|attr| attr.path().is_ident("compress"));

        let enum_variants = if has_data_type {
            quote! { None }
        } else {
            quote! { <#field_type as ::ormer::model::ModelEnumProvider>::ENUM_VARIANTS }
        };

        quote! {
            ::ormer::model::ColumnSchema {
                rust_name: #rust_field_name,
                name: #column_name,
                rust_type: #rust_type,
                is_primary: #is_primary,
                is_auto_increment: #field_is_auto_increment,
                is_nullable: #is_nullable,
                unique_group: #unique_group,
                unique_name: #unique_name,
                is_indexed: #is_indexed,
                index_group: #index_group,
                index_name: #index_name,
                index_order: #index_order,
                index_where: #index_where,
                foreign_key: #foreign_key,
                enum_variants: #enum_variants,
                data_type: #data_type,
                default: #default,
                check: #check,
                hypertable: #hypertable,
                compress: #compress,
            }
        }
    });

    // 生成 from_row 实现
    let from_row_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        if let Some(default_expr) = relation_default_expr(f) {
            quote! {
                #field_name: #default_expr
            }
        } else if has_i32_data_type(f) {
            let column_name = extract_column_name(f);
            field_from_i32_expr(
                f,
                quote! { row.get::<i32>(#column_name)? },
                quote! { row.get::<Option<i32>>(#column_name)? },
            )
        } else if has_vec_i32_data_type(f) {
            let column_name = extract_column_name(f);
            field_from_vec_i32_expr(f, quote! { row.get::<Vec<i32>>(#column_name)? })
        } else {
            let column_name = extract_column_name(f);
            quote! {
                #field_name: row.get(#column_name)?
            }
        }
    });

    // 生成 from_row_values 实现（按顺序从行值中读取）
    let from_row_values_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        if let Some(default_expr) = relation_default_expr(f) {
            quote! {
                #field_name: #default_expr
            }
        } else {
            let i = normal_fields
                .iter()
                .position(|normal| normal.ident == f.ident)
                .expect("normal field should have an index");
            if has_i32_data_type(f) {
                field_from_i32_expr(
                    f,
                    quote! {
                        <i32 as ::ormer::FromRowValues>::from_row_values(&values[#i..#i+1])?
                    },
                    quote! {
                        <Option<i32> as ::ormer::FromRowValues>::from_row_values(
                            &values[#i..#i+1]
                        )?
                    },
                )
            } else if has_vec_i32_data_type(f) {
                field_from_vec_i32_expr(
                    f,
                    quote! {
                        <Vec<i32> as ::ormer::FromRowValues>::from_row_values(
                            &values[#i..#i+1]
                        )?
                    },
                )
            } else {
                let field_type = &f.ty;
                quote! {
                    #field_name: <#field_type as ::ormer::FromRowValues>::from_row_values(
                        &values[#i..#i+1]
                    )?
                }
            }
        }
    });

    // 生成 field_values 实现
    let field_names_for_values = normal_fields.iter().map(|f| field_to_value_expr(f));

    // 生成 Where 结构体的字段
    // 为所有字段生成类型化列代理
    let where_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        if let Some(relation) = relation_fields
            .iter()
            .find(|relation| relation.field_name == *field_name)
        {
            let target_type = &relation.target_type;
            quote! {
                pub #field_name: ::ormer::model::Relation<#name, #target_type>
            }
        } else {
            let field_type = effective_data_type_type(f).unwrap_or_else(|| f.ty.clone());
            quote! {
                pub #field_name: ::ormer::query::builder::TypedColumn<#field_type>
            }
        }
    });

    // 生成 Where 的 Default 实现
    let where_default_fields = fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        if is_relation_field(f) {
            quote! {
                #field_name: ::ormer::model::Relation::new(stringify!(#field_name))
            }
        } else {
            let column_name = extract_column_name(f);
            quote! {
                #field_name: ::ormer::query::builder::TypedColumn::new(#column_name)
            }
        }
    });

    let relation_schema_entries = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        let local_key = if relation.local_key.is_empty() {
            quote! { stringify!(#primary_key_field) }
        } else {
            let local_key = &relation.local_key;
            quote! { #local_key }
        };
        let target_key = if relation.target_key.is_empty() {
            quote! { "id" }
        } else {
            let target_key = &relation.target_key;
            quote! { #target_key }
        };
        let kind = match relation.kind {
            RelationKindAttr::HasMany => quote! { ::ormer::model::RelationKind::HasMany },
            RelationKindAttr::BelongsTo => quote! { ::ormer::model::RelationKind::BelongsTo },
        };
        quote! {
            ::ormer::model::RelationInfo {
                name: stringify!(#field_name),
                kind: #kind,
                target_table: <#target_type as ::ormer::Model>::TABLE_NAME,
                local_key: #local_key,
                target_key: #target_key,
            }
        }
    });

    let column_value_arms = normal_fields.iter().map(|f| {
        let field_name = f.ident.as_ref().unwrap();
        let rust_field_name = field_name.to_string();
        let column_name = extract_column_name(f);
        let value_expr = field_to_value_expr(f);
        if rust_field_name == column_name {
            quote! {
                #column_name => Some(#value_expr)
            }
        } else {
            quote! {
                #column_name | #rust_field_name => Some(#value_expr)
            }
        }
    });

    let assign_relation_arms = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        match relation.kind {
            RelationKindAttr::HasMany => quote! {
                stringify!(#field_name)
                    if ::std::any::TypeId::of::<Target>() == ::std::any::TypeId::of::<#target_type>() =>
                {
                    let values = ::ormer::model::downcast_relation_vec_as::<#target_type, Target>(values)?;
                    self.#field_name = values;
                    Ok(())
                }
            },
            RelationKindAttr::BelongsTo => quote! {
                stringify!(#field_name)
                    if ::std::any::TypeId::of::<Target>() == ::std::any::TypeId::of::<#target_type>() =>
                {
                    let mut values = ::ormer::model::downcast_relation_vec_as::<#target_type, Target>(values)?;
                    self.#field_name = values.pop();
                    Ok(())
                }
            },
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
            const RELATIONS: &'static [::ormer::model::RelationInfo] = &[#(#relation_schema_entries),*];

            type AutoIncrementKeyType = #auto_increment_key_type;

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = #where_name;

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> anyhow::Result<Self> {
                if values.len() < Self::COLUMNS.len() {
                    return Err(anyhow::anyhow!(
                        "Expected {} values for {}", Self::COLUMNS.len(), stringify!(#name)
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

            fn column_value(&self, column: &str) -> Option<::ormer::Value> {
                match column {
                    #(#column_value_arms,)*
                    _ => None,
                }
            }

            fn assign_relation<Target: ::ormer::Model + 'static>(
                &mut self,
                relation_name: &'static str,
                values: Vec<Target>,
            ) -> anyhow::Result<()> {
                match relation_name {
                    #(#assign_relation_arms,)*
                    _ => Err(anyhow::anyhow!(
                        "Relation {} is not assignable on {}",
                        relation_name,
                        Self::TABLE_NAME
                    )),
                }
            }

            fn primary_key_columns() -> &'static [&'static str] {
                &[#(#primary_key_field_names),*]
            }

            fn primary_key_values(&self) -> Vec<::ormer::Value> {
                vec![#(#primary_key_values),*]
            }

            // 保持向后兼容的旧方法（已废弃）
            fn primary_key_column() -> &'static str {
                #primary_key_column_name
            }

            fn primary_key_value(&self) -> ::ormer::Value {
                #primary_key_value_expr
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

fn normalize_type_string(type_str: String) -> String {
    type_str
        .replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" , ", ",")
}

#[derive(Clone)]
enum RelationKindAttr {
    HasMany,
    BelongsTo,
}

#[derive(Clone)]
struct RelationField {
    field_name: syn::Ident,
    target_type: syn::Type,
    kind: RelationKindAttr,
    local_key: String,
    target_key: String,
}

fn is_relation_field(field: &syn::Field) -> bool {
    field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("has_many") || attr.path().is_ident("belongs_to"))
}

fn relation_default_expr(field: &syn::Field) -> Option<proc_macro2::TokenStream> {
    if field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("has_many"))
    {
        return Some(quote! { Vec::new() });
    }
    if field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("belongs_to"))
    {
        return Some(quote! { None });
    }
    None
}

fn extract_relation_field(field: &syn::Field) -> Option<RelationField> {
    let field_name = field.ident.as_ref()?.clone();
    for attr in &field.attrs {
        if attr.path().is_ident("has_many") {
            let (target_type, target_key) = parse_has_many(attr);
            return Some(RelationField {
                field_name,
                target_type,
                kind: RelationKindAttr::HasMany,
                local_key: String::new(),
                target_key,
            });
        }
        if attr.path().is_ident("belongs_to") {
            let local_key = parse_belongs_to(attr);
            let target_type = option_inner_type(&field.ty)
                .cloned()
                .unwrap_or_else(|| panic!("#[belongs_to] field must be Option<T>"));
            return Some(RelationField {
                field_name,
                target_type,
                kind: RelationKindAttr::BelongsTo,
                local_key,
                target_key: String::new(),
            });
        }
    }
    None
}

fn parse_has_many(attr: &syn::Attribute) -> (syn::Type, String) {
    if let Meta::List(list) = &attr.meta {
        let tokens_str = list.tokens.to_string();
        let parts: Vec<&str> = tokens_str.split('.').collect();
        if parts.len() == 2 {
            let target_type: syn::Type =
                syn::parse_str(parts[0].trim()).expect("#[has_many] target type is invalid");
            return (target_type, parts[1].trim().to_string());
        }
    }
    panic!("#[has_many] must use #[has_many(Target.foreign_key)]");
}

fn parse_belongs_to(attr: &syn::Attribute) -> String {
    if let Meta::List(list) = &attr.meta {
        let key = list.tokens.to_string().trim().to_string();
        if !key.is_empty() {
            return key;
        }
    }
    panic!("#[belongs_to] must use #[belongs_to(local_foreign_key)]");
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

            type AutoIncrementKeyType = <#inner_type as ::ormer::Model>::AutoIncrementKeyType;

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = <#inner_type as ::ormer::Model>::Where;

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> anyhow::Result<Self> {
                let inner = <#inner_type as ::ormer::Model>::from_row(row)?;
                Ok(#name(inner))
            }

            fn from_row_values(values: &[::ormer::Value]) -> anyhow::Result<Self> {
                let inner = <#inner_type as ::ormer::Model>::from_row_values(values)?;
                Ok(#name(inner))
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                self.0.field_values()
            }

            fn primary_key_columns() -> &'static [&'static str] {
                <#inner_type as ::ormer::Model>::primary_key_columns()
            }

            fn primary_key_values(&self) -> Vec<::ormer::Value> {
                self.0.primary_key_values()
            }

            fn primary_key_column() -> &'static str {
                <#inner_type as ::ormer::Model>::primary_key_column()
            }

            fn primary_key_value(&self) -> ::ormer::Value {
                self.0.primary_key_value()
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
    // 查找 #[table = "name"] 或 #[table(schema = "...", name = "...")] 属性
    for attr in &input.attrs {
        if attr.path().is_ident("table") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr) = &meta.value {
                    if let Lit::Str(lit) = &expr.lit {
                        return lit.value();
                    }
                }
            }
            if matches!(&attr.meta, Meta::List(_)) {
                let mut schema = None;
                let mut name = None;
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("schema") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        schema = Some(lit.value());
                        Ok(())
                    } else if meta.path.is_ident("name") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        name = Some(lit.value());
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[table] argument"))
                    }
                })
                .expect("Failed to parse #[table] attribute");

                if let Some(name) = name {
                    return if let Some(schema) = schema {
                        format!("{schema}.{name}")
                    } else {
                        name
                    };
                }
            }
        }
    }

    // 默认使用结构体名的蛇形形式
    to_snake_case(&input.ident.to_string())
}

fn extract_column_name(field: &syn::Field) -> String {
    let default_name = field.ident.as_ref().unwrap().to_string();

    for attr in &field.attrs {
        if attr.path().is_ident("column") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr) = &meta.value
                    && let Lit::Str(lit) = &expr.lit
                {
                    return lit.value();
                }
            }

            if let Meta::List(list) = &attr.meta {
                if let Ok(lit) = syn::parse2::<syn::LitStr>(list.tokens.clone()) {
                    return lit.value();
                }

                let mut name = None;
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("name") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        name = Some(lit.value());
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[column] argument"))
                    }
                })
                .expect("Failed to parse #[column] attribute");

                if let Some(name) = name {
                    return name;
                }
            }
        }
    }

    default_name
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

#[derive(Default)]
struct UniqueAttr {
    group: Option<i32>,
    name: Option<String>,
}

/// 提取 unique 属性。
fn extract_unique_attr(field: &syn::Field) -> UniqueAttr {
    for attr in &field.attrs {
        if attr.path().is_ident("unique") {
            let mut unique = UniqueAttr {
                group: Some(0),
                name: None,
            };
            if let Meta::List(list) = &attr.meta {
                let _ = list;
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("group") {
                        let value = meta.value()?;
                        let lit: syn::LitInt = value.parse()?;
                        unique.group = Some(lit.base10_parse::<i32>()?);
                        Ok(())
                    } else if meta.path.is_ident("name") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        unique.name = Some(lit.value());
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[unique] argument"))
                    }
                })
                .expect("Failed to parse #[unique] attribute");
            }
            return unique;
        }
    }
    UniqueAttr::default()
}

#[derive(Default)]
struct IndexAttr {
    group: Option<i32>,
    name: Option<String>,
    order: Option<String>,
    where_clause: Option<String>,
}

fn extract_index_attr(field: &syn::Field) -> Option<IndexAttr> {
    for attr in &field.attrs {
        if attr.path().is_ident("index") {
            let mut index = IndexAttr::default();
            if let Meta::List(_) = &attr.meta {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("group") {
                        let value = meta.value()?;
                        let lit: syn::LitInt = value.parse()?;
                        index.group = Some(lit.base10_parse::<i32>()?);
                        Ok(())
                    } else if meta.path.is_ident("name") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        index.name = Some(lit.value());
                        Ok(())
                    } else if meta.path.is_ident("order") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        index.order = Some(lit.value());
                        Ok(())
                    } else if meta.path.is_ident("where") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        index.where_clause = Some(lit.value());
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[index] argument"))
                    }
                })
                .expect("Failed to parse #[index] attribute");
            }
            return Some(index);
        }
    }
    None
}

fn option_i32_tokens(value: Option<i32>) -> proc_macro2::TokenStream {
    if let Some(value) = value {
        quote! { Some(#value) }
    } else {
        quote! { None }
    }
}

fn option_string_tokens(value: Option<&str>) -> proc_macro2::TokenStream {
    if let Some(value) = value {
        quote! { Some(#value) }
    } else {
        quote! { None }
    }
}

/// 提取 data_type 属性的类型覆盖信息。
///
/// `Option<T>` 字段允许使用 `#[data_type(Option<U>)]`，但数据库后端只需要
/// 基础类型 `U`，所以这里去掉属性中的 `Option<>` 包装。
fn extract_data_type(field: &syn::Field) -> proc_macro2::TokenStream {
    if let Some(data_type) = effective_data_type_type(field) {
        let type_str = normalize_type_string(quote! { #data_type }.to_string());
        return quote! { Some(#type_str) };
    }

    for attr in &field.attrs {
        if attr.path().is_ident("data_type") {
            if let Meta::List(list) = &attr.meta {
                let tokens_str = list.tokens.to_string().replace('"', "");
                return quote! { Some(#tokens_str) };
            }
        }
    }
    quote! { None }
}

fn has_data_type(field: &syn::Field) -> bool {
    field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("data_type"))
}

fn has_i32_data_type(field: &syn::Field) -> bool {
    effective_data_type_type(field)
        .as_ref()
        .map(is_i32_type)
        .unwrap_or(false)
}

fn has_vec_i32_data_type(field: &syn::Field) -> bool {
    effective_data_type_type(field)
        .as_ref()
        .map(is_vec_i32_type)
        .unwrap_or(false)
}

fn validate_data_type(field: &syn::Field) {
    let Some(data_type) = extract_data_type_type(field) else {
        return;
    };

    let field_is_optional = option_inner_type(&field.ty).is_some();
    let data_type_is_optional = option_inner_type(&data_type).is_some();
    if field_is_optional == data_type_is_optional {
        return;
    }

    let field_name = field
        .ident
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "<unnamed>".to_string());

    if field_is_optional {
        panic!(
            "field `{field_name}` is nullable, so its database type must use \
             #[data_type(Option<...>)]"
        );
    }

    panic!(
        "field `{field_name}` is not nullable, so its database type must not use \
         #[data_type(Option<...>)]"
    );
}

fn effective_data_type_type(field: &syn::Field) -> Option<syn::Type> {
    let data_type = extract_data_type_type(field)?;
    if option_inner_type(&field.ty).is_some() {
        option_inner_type(&data_type).cloned()
    } else {
        Some(data_type)
    }
}

fn extract_data_type_type(field: &syn::Field) -> Option<syn::Type> {
    for attr in &field.attrs {
        if attr.path().is_ident("data_type") {
            if let Meta::List(list) = &attr.meta {
                if let Ok(data_type) = syn::parse2::<syn::Type>(list.tokens.clone()) {
                    return Some(data_type);
                }
            }
        }
    }
    None
}

fn is_i32_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident == "i32")
            .unwrap_or(false),
        _ => false,
    }
}

fn is_vec_i32_type(ty: &syn::Type) -> bool {
    vec_inner_type(ty).map(is_i32_type).unwrap_or(false)
}

fn option_inner_type(ty: &syn::Type) -> Option<&syn::Type> {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            let segment = type_path.path.segments.last()?;
            if segment.ident != "Option" {
                return None;
            }

            match &segment.arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.first().and_then(|arg| {
                    if let syn::GenericArgument::Type(inner) = arg {
                        Some(inner)
                    } else {
                        None
                    }
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn vec_inner_type(ty: &syn::Type) -> Option<&syn::Type> {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            let segment = type_path.path.segments.last()?;
            if segment.ident != "Vec" {
                return None;
            }

            match &segment.arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.first().and_then(|arg| {
                    if let syn::GenericArgument::Type(inner) = arg {
                        Some(inner)
                    } else {
                        None
                    }
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn field_to_value_expr(field: &syn::Field) -> proc_macro2::TokenStream {
    let field_name = field.ident.as_ref().unwrap();
    let field_type = &field.ty;
    let value_type = option_inner_type(field_type).unwrap_or(field_type);

    if has_i32_data_type(field) {
        if option_inner_type(field_type).is_some() {
            quote! {
                match self.#field_name.clone() {
                    Some(value) => {
                        use ::ormer::model::I32DataTypeEncode as _;
                        ::ormer::Value::from(
                            ::ormer::model::I32DataTypeEncoder::<#value_type>::new().encode(
                                value,
                                stringify!(#field_name),
                                stringify!(#value_type),
                            )
                        )
                    },
                    None => ::ormer::Value::Null,
                }
            }
        } else {
            quote! {
                {
                    use ::ormer::model::I32DataTypeEncode as _;
                    ::ormer::Value::from(
                        ::ormer::model::I32DataTypeEncoder::<#value_type>::new().encode(
                            self.#field_name.clone(),
                            stringify!(#field_name),
                            stringify!(#value_type),
                        )
                    )
                }
            }
        }
    } else if has_vec_i32_data_type(field) {
        let Some(_) = vec_inner_type(field_type) else {
            panic!("#[data_type(Vec<i32>)] requires a Vec<T> field");
        };
        quote! {
            ::ormer::Value::from(
                self.#field_name
                    .clone()
                    .into_iter()
                    .map(|value| value as i32)
                    .collect::<Vec<i32>>()
            )
        }
    } else {
        quote! {
            ::ormer::Value::from(self.#field_name.clone())
        }
    }
}

fn field_from_i32_expr(
    field: &syn::Field,
    value_expr: proc_macro2::TokenStream,
    optional_value_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field_name = field.ident.as_ref().unwrap();
    let field_type = &field.ty;

    if let Some(inner_type) = option_inner_type(field_type) {
        let decode_expr = data_type_i32_decode_expr(inner_type, field_name, quote! { value });
        quote! {
            #field_name: {
                let value = #optional_value_expr;
                match value {
                    Some(value) => Some(#decode_expr),
                    None => None,
                }
            }
        }
    } else {
        let decode_expr = data_type_i32_decode_expr(field_type, field_name, value_expr);
        quote! {
            #field_name: #decode_expr
        }
    }
}

fn field_from_vec_i32_expr(
    field: &syn::Field,
    value_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let field_name = field.ident.as_ref().unwrap();
    let field_type = &field.ty;
    let inner_type =
        vec_inner_type(field_type).expect("#[data_type(Vec<i32>)] requires a Vec<T> field");

    quote! {
        #field_name: {
            let values = #value_expr;
            use ::ormer::model::I32DataTypeDecode as _;
            values
                .into_iter()
                .map(|value| {
                    ::ormer::model::I32DataTypeDecoder::<#inner_type>::new()
                        .decode(value, stringify!(#field_name), stringify!(#inner_type))
                })
                .collect::<anyhow::Result<Vec<#inner_type>>>()?
        }
    }
}

fn data_type_i32_decode_expr(
    target_type: &syn::Type,
    field_name: &syn::Ident,
    value_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        {
            let value = #value_expr;
            use ::ormer::model::I32DataTypeDecode as _;
            ::ormer::model::I32DataTypeDecoder::<#target_type>::new()
                .decode(value, stringify!(#field_name), stringify!(#target_type))?
        }
    }
}

/// 提取 hypertable 属性的分片时长信息
/// 支持语法：#[hypertable(Duration::from_hours(1))]
fn extract_hypertable(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if attr.path().is_ident("hypertable") {
            if let Meta::List(list) = &attr.meta {
                let tokens = &list.tokens;
                return quote! { Some(#tokens) };
            }
        }
    }
    quote! { None }
}

/// 提取 foreign 属性的外键信息
/// 支持两种语法：
/// - #[foreign(Type)] - 新语法，自动关联到目标 model 的主键
/// - #[foreign(Type.field)] - 旧语法，显式指定字段
fn extract_default(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if !attr.path().is_ident("default") {
            continue;
        }

        let Meta::List(list) = &attr.meta else {
            panic!("#[default] must use #[default(...)]");
        };

        let mut expression = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("expr") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                expression = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error("unsupported #[default] argument"))
            }
        });

        if let Some(expression) = expression {
            return quote! {
                Some(::ormer::model::ColumnDefault::Expression(#expression))
            };
        }

        match syn::parse2::<Lit>(list.tokens.clone()) {
            Ok(Lit::Str(value)) => {
                let value = value.value();
                return quote! {
                    Some(::ormer::model::ColumnDefault::String(#value))
                };
            }
            Ok(Lit::Int(value)) => {
                let value = value.to_string();
                return quote! {
                    Some(::ormer::model::ColumnDefault::Number(#value))
                };
            }
            Ok(Lit::Float(value)) => {
                let value = value.to_string();
                return quote! {
                    Some(::ormer::model::ColumnDefault::Number(#value))
                };
            }
            Ok(Lit::Bool(value)) => {
                return quote! {
                    Some(::ormer::model::ColumnDefault::Boolean(#value))
                };
            }
            _ => panic!("#[default] supports string, number, bool, or expr = \"...\""),
        }
    }
    quote! { None }
}

fn extract_check(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if !attr.path().is_ident("check") {
            continue;
        }

        if !matches!(&attr.meta, Meta::List(_)) {
            panic!("#[check] must use #[check(expr = \"...\")]");
        }

        let mut expr = None;
        let mut name = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("expr") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                expr = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("name") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                name = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error("unsupported #[check] argument"))
            }
        })
        .expect("Failed to parse #[check] attribute");

        let expr = expr.expect("#[check] requires expr = \"...\"");
        let name = option_string_tokens(name.as_deref());
        return quote! {
            Some(::ormer::model::CheckConstraint {
                name: #name,
                expr: #expr,
            })
        };
    }
    quote! { None }
}

fn normalize_type_path(value: &str) -> String {
    value.split_whitespace().collect::<String>()
}

fn foreign_action_tokens(value: &str) -> proc_macro2::TokenStream {
    let normalized = value
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase()
        .replace('_', "");
    match normalized.as_str() {
        "noaction" => quote! { ::ormer::model::ForeignKeyAction::NoAction },
        "restrict" => quote! { ::ormer::model::ForeignKeyAction::Restrict },
        "cascade" => quote! { ::ormer::model::ForeignKeyAction::Cascade },
        "setnull" => quote! { ::ormer::model::ForeignKeyAction::SetNull },
        "setdefault" => quote! { ::ormer::model::ForeignKeyAction::SetDefault },
        _ => panic!("unsupported foreign-key action: {value}"),
    }
}

fn extract_foreign_key(field: &syn::Field) -> proc_macro2::TokenStream {
    for attr in &field.attrs {
        if attr.path().is_ident("foreign") {
            if let Meta::List(list) = &attr.meta {
                let tokens = list.tokens.to_string();
                let parts: Vec<&str> = tokens.split(',').collect();
                let target = parts
                    .first()
                    .map(|part| part.trim())
                    .filter(|part| !part.is_empty())
                    .expect("#[foreign] requires a target model");
                let target = normalize_type_path(target);
                let (ref_type, ref_field) =
                    if let Some((ref_type, ref_field)) = target.split_once('.') {
                        (ref_type.to_string(), Some(ref_field.to_string()))
                    } else {
                        (target, None)
                    };
                let ref_type: syn::Type =
                    syn::parse_str(&ref_type).expect("#[foreign] target model is invalid");

                let mut constraint_name = None;
                let mut on_delete = None;
                let mut on_update = None;
                for part in parts.into_iter().skip(1) {
                    let Some((key, value)) = part.split_once('=') else {
                        panic!("#[foreign] options must use key = value");
                    };
                    match key.trim() {
                        "name" => {
                            constraint_name = Some(value.trim().trim_matches('"').to_string());
                        }
                        "on_delete" => on_delete = Some(foreign_action_tokens(value)),
                        "on_update" => on_update = Some(foreign_action_tokens(value)),
                        other => panic!("unsupported #[foreign] option: {other}"),
                    }
                }

                let constraint_name = option_string_tokens(constraint_name.as_deref());
                let on_delete = on_delete
                    .map(|action| quote! { Some(#action) })
                    .unwrap_or_else(|| quote! { None });
                let on_update = on_update
                    .map(|action| quote! { Some(#action) })
                    .unwrap_or_else(|| quote! { None });
                let field_name = field.ident.as_ref().unwrap().to_string();
                let ref_type_name = normalize_type_path(&quote! { #ref_type }.to_string())
                    .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
                let ref_fn_name = syn::Ident::new(
                    &format!("__ormer_fk_{field_name}_{ref_type_name}"),
                    proc_macro2::Span::call_site(),
                );

                if let Some(ref_field) = ref_field {
                    return quote! {
                        {
                            fn #ref_fn_name() -> &'static str {
                                <#ref_type as ::ormer::Model>::column_name_for_field(#ref_field)
                                    .unwrap_or(#ref_field)
                            }
                            Some(::ormer::model::ForeignKeyInfo {
                                name: #constraint_name,
                                ref_table: <#ref_type as ::ormer::Model>::TABLE_NAME,
                                ref_column: #ref_field,
                                ref_column_fn: Some(#ref_fn_name),
                                on_delete: #on_delete,
                                on_update: #on_update,
                            })
                        }
                    };
                }

                return quote! {
                    {
                        fn #ref_fn_name() -> &'static str {
                            <#ref_type as ::ormer::Model>::primary_key_columns()[0]
                        }
                        Some(::ormer::model::ForeignKeyInfo {
                            name: #constraint_name,
                            ref_table: <#ref_type as ::ormer::Model>::TABLE_NAME,
                            ref_column: "",
                            ref_column_fn: Some(#ref_fn_name),
                            on_delete: #on_delete,
                            on_update: #on_update,
                        })
                    }
                };
            }
        }
    }
    // 没有 foreign 属性
    quote! { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "field `optional_status` is nullable")]
    fn rejects_non_nullable_data_type_for_option_field() {
        let input: DeriveInput = syn::parse_quote! {
            struct InvalidModel {
                #[primary]
                id: i32,
                #[data_type(i32)]
                optional_status: Option<i32>,
            }
        };
        derive_model(input);
    }

    #[test]
    #[should_panic(expected = "field `status` is not nullable")]
    fn rejects_nullable_data_type_for_non_option_field() {
        let input: DeriveInput = syn::parse_quote! {
            struct InvalidModel {
                #[primary]
                id: i32,
                #[data_type(Option<i32>)]
                status: i32,
            }
        };
        derive_model(input);
    }

    #[test]
    fn unwraps_nullable_data_type_for_backend_mapping() {
        let field: syn::Field =
            syn::parse_quote! { #[data_type(Option<i32>)] optional_status: Option<i32> };
        assert!(has_i32_data_type(&field));

        let effective_type = effective_data_type_type(&field).expect("data type should exist");
        assert_eq!(
            normalize_type_string(quote! { #effective_type }.to_string()),
            "i32"
        );
    }
}
