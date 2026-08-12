use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Lit, Meta};

pub fn derive_model(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let where_name = syn::Ident::new(&format!("{name}Where"), name.span());
    let update_name = syn::Ident::new(&format!("{name}Update"), name.span());

    // 提取表名
    let table_name = extract_table_name(&input);
    let filters = extract_model_filters(&input);

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

    let mut field_infos: Vec<_> = fields.iter().map(FieldInfo::new).collect();
    let mut normal_index = 0;
    for info in &mut field_infos {
        if !info.is_relation() && !info.is_ignored {
            info.normal_index = Some(normal_index);
            normal_index += 1;
        }
    }

    let normal_fields: Vec<_> = field_infos
        .iter()
        .filter(|info| !info.is_relation() && !info.is_ignored)
        .collect();
    let value_fields: Vec<_> = field_infos
        .iter()
        .filter(|info| !info.is_relation())
        .collect();
    let relation_fields: Vec<RelationField> = field_infos
        .iter()
        .filter_map(|info| info.relation.clone())
        .collect();
    let has_embed = normal_fields.iter().any(|info| info.embed.is_some());

    // 提取主键字段列表（支持复合主键）
    let primary_keys: Vec<_> = normal_fields
        .iter()
        .copied()
        .filter(|info| info.is_primary)
        .collect();

    // 至少需要一个主键
    if primary_keys.is_empty() {
        panic!("Model must have at least one #[primary] field");
    }

    // 检查是否有多个主键且标记了 auto（只有第一个主键可以是 auto）
    let auto_count = primary_keys.iter().filter(|info| info.primary_auto).count();
    if auto_count > 1 {
        panic!("Only one primary key field can have #[primary(auto)]");
    }

    // 获取第一个主键（用于向后兼容）
    let primary_key_field = primary_keys[0].field_name;
    let is_auto_increment = primary_keys[0].primary_auto;

    // 生成 AutoIncrementKeyType
    // 如果有自增主键，类型为第一个主键的 Rust 类型；否则为 ()
    let auto_increment_key_type = if is_auto_increment {
        let pk_type = primary_keys[0].field_type;
        quote! { #pk_type }
    } else {
        quote! { () }
    };

    // 生成主键列名列表（支持复合主键）
    let primary_key_field_names: Vec<_> = primary_keys
        .iter()
        .map(|info| {
            let column_name = &info.column_name;
            quote! { #column_name }
        })
        .collect();
    let primary_key_column_name = primary_keys[0].column_name.clone();

    // 生成主键值获取（支持复合主键）
    let primary_key_values: Vec<_> = primary_keys
        .iter()
        .map(|info| field_to_value_expr(info))
        .collect();

    let primary_key_value_expr = field_to_value_expr(primary_keys[0]);

    // 生成字段名列表
    let field_names: Vec<String> = normal_fields
        .iter()
        .filter(|info| info.embed.is_none())
        .map(|info| info.column_name.clone())
        .collect();

    let field_names_lit = field_names
        .iter()
        .map(|name| quote! { #name })
        .collect::<Vec<_>>();

    // 生成字段元数据 (COLUMN_SCHEMA)
    let column_schema_entries = normal_fields
        .iter()
        .filter(|info| info.embed.is_none())
        .map(|info| {
            let field_name = info.field_name;
            let rust_field_name = field_name.to_string();
            let column_name = info.column_name.as_str();
            let field_type = info.field_type;

            // 检查是否是自增主键（只有主键字段才可能是自增）
            let is_primary = info.is_primary;
            let field_is_auto_increment = if is_primary { is_auto_increment } else { false };
            let is_nullable = info.is_nullable;
            let rust_type = &info.rust_type;

            // 检查 unique 属性
            let unique_attr = &info.unique_attr;
            let unique_group = option_i32_tokens(unique_attr.group);
            let unique_name = option_string_tokens(unique_attr.name.as_deref());

            // 检查 index 属性
            let index_attr = info.index_attr.as_ref();
            let is_indexed = index_attr.is_some();
            let index_group = option_i32_tokens(index_attr.and_then(|attr| attr.group));
            let index_name = option_string_tokens(index_attr.and_then(|attr| attr.name.as_deref()));
            let index_order =
                option_string_tokens(index_attr.and_then(|attr| attr.order.as_deref()));
            let index_where =
                option_string_tokens(index_attr.and_then(|attr| attr.where_clause.as_deref()));

            // 检查 foreign 属性
            let foreign_key = &info.foreign_key;

            // 检查 data_type 属性
            let data_type = &info.data_type;
            let has_data_type = info.has_data_type;

            // 检查 default/check 属性
            let default = &info.default;
            let check = &info.check;

            // 检查 hypertable 属性
            let hypertable = &info.hypertable;

            // 检查 compress 属性
            let compress = info.compress;

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
        })
        .collect::<Vec<_>>();

    // 生成 from_row 实现
    let from_row_fields = field_infos
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            if info.is_ignored {
                quote! {
                    #field_name: ::std::default::Default::default()
                }
            } else if let Some(default_expr) = &info.relation_default {
                quote! {
                    #field_name: #default_expr
                }
            } else if let Some(embed) = &info.embed {
                let prefix = &embed.prefix;
                let field_type = info.field_type;
                quote! {
                    #field_name: <#field_type as ::ormer::model::Embed>::from_row(row, #prefix)?
                }
            } else if info.has_i32_data_type {
                let column_name = &info.column_name;
                field_from_i32_expr(
                    info.field,
                    quote! { row.get::<i32>(#column_name)? },
                    quote! { row.get::<Option<i32>>(#column_name)? },
                )
            } else if info.has_vec_i32_data_type {
                let column_name = &info.column_name;
                field_from_vec_i32_expr(info.field, quote! { row.get::<Vec<i32>>(#column_name)? })
            } else {
                let column_name = &info.column_name;
                quote! {
                    #field_name: row.get(#column_name)?
                }
            }
        })
        .collect::<Vec<_>>();

    // 生成 from_row_values 实现（按顺序从行值中读取）
    let from_row_values_fields = field_infos
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            if info.is_ignored {
                quote! {
                    #field_name: ::std::default::Default::default()
                }
            } else if let Some(default_expr) = &info.relation_default {
                quote! {
                    #field_name: #default_expr
                }
            } else if info.embed.is_some() {
                let field_type = info.field_type;
                quote! {
                    #field_name: {
                        let start = __ormer_value_index;
                        let end = start + <#field_type as ::ormer::model::Embed>::COLUMNS.len();
                        __ormer_value_index = end;
                        <#field_type as ::ormer::model::Embed>::from_row_values(
                            &values[start..end]
                        )?
                    }
                }
            } else {
                if info.has_i32_data_type {
                    field_from_i32_expr(
                        info.field,
                        quote! {
                            {
                                let i = __ormer_value_index;
                                __ormer_value_index += 1;
                                <i32 as ::ormer::FromRowValues>::from_row_values(&values[i..i+1])?
                            }
                        },
                        quote! {
                            {
                                let i = __ormer_value_index;
                                __ormer_value_index += 1;
                                <Option<i32> as ::ormer::FromRowValues>::from_row_values(
                                    &values[i..i+1]
                                )?
                            }
                        },
                    )
                } else if info.has_vec_i32_data_type {
                    field_from_vec_i32_expr(
                        info.field,
                        quote! {
                            {
                                let i = __ormer_value_index;
                                __ormer_value_index += 1;
                                <Vec<i32> as ::ormer::FromRowValues>::from_row_values(
                                    &values[i..i+1]
                                )?
                            }
                        },
                    )
                } else {
                    let field_type = info.field_type;
                    quote! {
                        #field_name: {
                            let i = __ormer_value_index;
                            __ormer_value_index += 1;
                            <#field_type as ::ormer::FromRowValues>::from_row_values(
                                &values[i..i+1]
                            )?
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    // 生成 field_values 实现
    let field_names_for_values = normal_fields.iter().map(|info| {
        if info.embed.is_some() {
            let field_name = info.field_name;
            quote! {
                values.extend(::ormer::model::Embed::field_values(&self.#field_name));
            }
        } else {
            let value_expr = field_to_value_expr(info);
            quote! {
                values.push(#value_expr);
            }
        }
    });

    // 生成 Where 结构体的字段
    // 为所有字段生成类型化列代理
    let where_infos: Vec<_> = field_infos
        .iter()
        .filter(|info| info.is_relation() || !info.is_ignored)
        .collect();

    let where_fields = where_infos.iter().map(|info| {
        let field_name = info.field_name;
        if let Some(relation) = &info.relation {
            let target_type = &relation.target_type;
            if let RelationKindAttr::Through = relation.kind {
                let via_type = through_via_type(relation, &relation_fields);
                quote! {
                    pub #field_name: ::ormer::model::ThroughRelation<#name, #via_type, #target_type>
                }
            } else {
                quote! {
                    pub #field_name: ::ormer::model::Relation<#name, #target_type>
                }
            }
        } else if info.embed.is_some() {
            let field_type = info.field_type;
            quote! {
                pub #field_name: <#field_type as ::ormer::model::Embed>::Where
            }
        } else {
            let field_type = info
                .effective_data_type_type
                .as_ref()
                .unwrap_or(info.field_type);
            quote! {
                pub #field_name: ::ormer::query::builder::TypedColumn<#field_type, #name>
            }
        }
    });

    // 生成 Where 的 Default 实现
    let where_default_fields = where_infos.iter().map(|info| {
        let field_name = info.field_name;
        if let Some(relation) = &info.relation {
            if let Some(through) = &relation.through {
                let via_relation = &through.via_relation;
                let target_relation = &through.target_relation;
                quote! {
                    #field_name: ::ormer::model::ThroughRelation::new(
                        stringify!(#field_name),
                        #via_relation,
                        #target_relation
                    )
                }
            } else {
                quote! {
                    #field_name: ::ormer::model::Relation::new(stringify!(#field_name))
                }
            }
        } else if info.embed.is_some() {
            let field_type = info.field_type;
            let prefix = info.embed.as_ref().map(|embed| embed.prefix.as_str()).unwrap();
            quote! {
                #field_name: <#field_type as ::ormer::model::Embed>::Where::new_with_prefix(#prefix)
            }
        } else {
            let column_name = &info.column_name;
            quote! {
                #field_name: ::ormer::query::builder::TypedColumn::new(#column_name)
            }
        }
    });

    let update_fields = normal_fields.iter().filter(|info| info.embed.is_none()).map(|info| {
        let field_name = info.field_name;
        let field_type = info
            .effective_data_type_type
            .as_ref()
            .unwrap_or(info.field_type);
        quote! {
            pub #field_name: ::ormer::query::update::UpdateField<#field_type>
        }
    });

    let update_default_fields = normal_fields.iter().filter(|info| info.embed.is_none()).map(|info| {
        let field_name = info.field_name;
        let column_name = &info.column_name;
        quote! {
            #field_name: ::ormer::query::update::UpdateField::new(#column_name)
        }
    });

    let update_assignment_fields = normal_fields.iter().filter(|info| info.embed.is_none()).map(|info| {
        let field_name = info.field_name;
        quote! {
            if let Some(assignment) = self.#field_name.assignment() {
                assignments.push(assignment);
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
            RelationKindAttr::HasOne => quote! { ::ormer::model::RelationKind::HasOne },
            RelationKindAttr::Through => quote! { ::ormer::model::RelationKind::Through },
        };
        let through = if let Some(through) = &relation.through {
            let via_relation = &through.via_relation;
            let target_relation = &through.target_relation;
            quote! {
                Some(::ormer::model::ThroughInfo {
                    via_relation: #via_relation,
                    target_relation: #target_relation,
                })
            }
        } else {
            quote! { None }
        };
        quote! {
            ::ormer::model::RelationInfo {
                name: stringify!(#field_name),
                kind: #kind,
                target_table: <#target_type as ::ormer::Model>::TABLE_NAME,
                local_key: #local_key,
                target_key: #target_key,
                through: #through,
            }
        }
    });

    let column_value_arms = value_fields.iter().map(|info| {
        let field_name = info.field_name;
        let rust_field_name = field_name.to_string();
        let column_name = info.column_name.as_str();
        if let Some(embed) = &info.embed {
            let prefix = &embed.prefix;
            return quote! {
                column if column == #rust_field_name => None,
                column if column.starts_with(#prefix) => {
                    ::ormer::model::Embed::column_value(&self.#field_name, &column[#prefix.len()..])
                }
            };
        }
        let value_expr = field_to_value_expr(info);
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

    let assign_column_value_arms = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let rust_field_name = field_name.to_string();
        if let Some(embed) = &info.embed {
            let prefix = &embed.prefix;
            return quote! {
                column if column.starts_with(#prefix) => {
                    return ::ormer::model::Embed::assign_column_value(
                        &mut self.#field_name,
                        &column[#prefix.len()..],
                        value,
                    );
                }
            };
        }
        let column_name = info.column_name.as_str();
        let assign_expr = field_assign_value_expr(info);
        if rust_field_name == column_name {
            quote! {
                #column_name => {
                    #assign_expr
                    return Ok(());
                }
            }
        } else {
            quote! {
                #column_name | #rust_field_name => {
                    #assign_expr
                    return Ok(());
                }
            }
        }
    });

    let assign_relation_arms = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        match relation.kind {
            RelationKindAttr::HasMany | RelationKindAttr::Through => quote! {
                stringify!(#field_name)
                    if ::std::any::TypeId::of::<Target>() == ::std::any::TypeId::of::<#target_type>() =>
                {
                    let values = ::ormer::model::downcast_relation_vec_as::<#target_type, Target>(values)?;
                    self.#field_name = values;
                    Ok(())
                }
            },
            RelationKindAttr::BelongsTo | RelationKindAttr::HasOne => quote! {
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
    let graph_relation_entries = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        match relation.kind {
            RelationKindAttr::HasMany => quote! {
                if let Some(relation) = <Self as ::ormer::Model>::RELATIONS
                    .iter()
                    .find(|relation| {
                        relation.name == stringify!(#field_name)
                            && relation.target_table == <#target_type as ::ormer::Model>::TABLE_NAME
                    })
                {
                    relations.push(::ormer::model::GraphRelationMut::HasMany {
                        relation,
                        items: &mut self.#field_name,
                    });
                }
            },
            RelationKindAttr::HasOne => quote! {
                if let Some(relation) = <Self as ::ormer::Model>::RELATIONS
                    .iter()
                    .find(|relation| {
                        relation.name == stringify!(#field_name)
                            && relation.target_table == <#target_type as ::ormer::Model>::TABLE_NAME
                    })
                {
                    relations.push(::ormer::model::GraphRelationMut::HasOne {
                        relation,
                        item: self.#field_name
                            .as_mut()
                            .map(|value| value as &mut dyn ::ormer::model::GraphModel),
                    });
                }
            },
            RelationKindAttr::Through => quote! {
                if let Some(relation) = <Self as ::ormer::Model>::RELATIONS
                    .iter()
                    .find(|relation| {
                        relation.name == stringify!(#field_name)
                            && relation.target_table == <#target_type as ::ormer::Model>::TABLE_NAME
                    })
                {
                    relations.push(::ormer::model::GraphRelationMut::Through {
                        relation,
                        items: &mut self.#field_name,
                    });
                }
            },
            RelationKindAttr::BelongsTo => quote! {},
        }
    });
    let graph_insert_entries = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        match relation.kind {
            RelationKindAttr::HasMany => quote! {
                if !self_.#field_name.is_empty() {
                    let relation =
                        ::ormer::model::graph_relation_info::<Self, #target_type>(
                            stringify!(#field_name),
                        )?;
                    let owner_key = <Self as ::ormer::Model>::relation_key_value(self_, relation)?;
                    for item in &mut self_.#field_name {
                        <#target_type as ::ormer::Model>::assign_column_value(
                            item,
                            relation.target_key,
                            owner_key.clone(),
                        )?;
                        let key = tx.insert(&*item).execute().await?;
                        let key_value = ::ormer::model::graph_auto_increment_key_value(key);
                        if !::ormer::model::graph_is_no_auto_increment_key(&key_value) {
                            <#target_type as ::ormer::Model>::assign_column_value(
                                item,
                                <#target_type as ::ormer::Model>::primary_key_columns()[0],
                                key_value,
                            )?;
                        }
                        <#target_type as ::ormer::model::GraphWritable>::insert_graph_relations(
                            tx,
                            item,
                        )
                        .await?;
                    }
                }
            },
            RelationKindAttr::HasOne => quote! {
                if self_.#field_name.is_some() {
                    let relation =
                        ::ormer::model::graph_relation_info::<Self, #target_type>(
                            stringify!(#field_name),
                        )?;
                    let owner_key = <Self as ::ormer::Model>::relation_key_value(self_, relation)?;
                    let item = self_.#field_name.as_mut().expect("checked is_some");
                    <#target_type as ::ormer::Model>::assign_column_value(
                        item,
                        relation.target_key,
                        owner_key,
                    )?;
                    let key = tx.insert(&*item).execute().await?;
                    let key_value = ::ormer::model::graph_auto_increment_key_value(key);
                    if !::ormer::model::graph_is_no_auto_increment_key(&key_value) {
                        <#target_type as ::ormer::Model>::assign_column_value(
                            item,
                            <#target_type as ::ormer::Model>::primary_key_columns()[0],
                            key_value,
                        )?;
                    }
                    <#target_type as ::ormer::model::GraphWritable>::insert_graph_relations(
                        tx,
                        item,
                    )
                    .await?;
                }
            },
            RelationKindAttr::Through => {
                let via_type = through_via_type(relation, &relation_fields);
                quote! {
                    if !self_.#field_name.is_empty() {
                        let (_, via_relation, target_relation) =
                            ::ormer::model::graph_through_infos::<Self, #via_type, #target_type>(
                                stringify!(#field_name),
                            )?;
                        let owner_key =
                            <Self as ::ormer::Model>::relation_key_value(self_, via_relation)?;
                        for item in &mut self_.#field_name {
                            tx.insert_or_update(&*item).execute().await?;
                            <#target_type as ::ormer::model::GraphWritable>::insert_graph_relations(
                                tx,
                                item,
                            )
                            .await?;
                            let target_key =
                                ::ormer::model::graph_target_key_value(item, target_relation)?;
                            ::ormer::model::graph_insert_through_link_values::<#via_type>(
                                tx,
                                via_relation.target_key,
                                owner_key.clone(),
                                target_relation.local_key,
                                target_key,
                            )
                            .await?;
                        }
                    }
                }
            }
            RelationKindAttr::BelongsTo => quote! {},
        }
    });
    let graph_update_entries = relation_fields.iter().map(|relation| {
        let field_name = &relation.field_name;
        let target_type = &relation.target_type;
        match relation.kind {
            RelationKindAttr::HasMany => quote! {
                if !self_.#field_name.is_empty() {
                    let relation =
                        ::ormer::model::graph_relation_info::<Self, #target_type>(
                            stringify!(#field_name),
                        )?;
                    let owner_key = <Self as ::ormer::Model>::relation_key_value(self_, relation)?;
                    for item in &mut self_.#field_name {
                        <#target_type as ::ormer::Model>::assign_column_value(
                            item,
                            relation.target_key,
                            owner_key.clone(),
                        )?;
                        tx.insert_or_update(&*item).execute().await?;
                        <#target_type as ::ormer::model::GraphWritable>::update_graph_relations(
                            tx,
                            item,
                        )
                        .await?;
                    }
                }
            },
            RelationKindAttr::HasOne => quote! {
                if self_.#field_name.is_some() {
                    let relation =
                        ::ormer::model::graph_relation_info::<Self, #target_type>(
                            stringify!(#field_name),
                        )?;
                    let owner_key = <Self as ::ormer::Model>::relation_key_value(self_, relation)?;
                    let item = self_.#field_name.as_mut().expect("checked is_some");
                    <#target_type as ::ormer::Model>::assign_column_value(
                        item,
                        relation.target_key,
                        owner_key,
                    )?;
                    tx.insert_or_update(&*item).execute().await?;
                    <#target_type as ::ormer::model::GraphWritable>::update_graph_relations(
                        tx,
                        item,
                    )
                    .await?;
                }
            },
            RelationKindAttr::Through => {
                let via_type = through_via_type(relation, &relation_fields);
                quote! {
                    if !self_.#field_name.is_empty() {
                        let (_, via_relation, target_relation) =
                            ::ormer::model::graph_through_infos::<Self, #via_type, #target_type>(
                                stringify!(#field_name),
                            )?;
                        let owner_key =
                            <Self as ::ormer::Model>::relation_key_value(self_, via_relation)?;
                        let mut target_keys = Vec::new();
                        for item in &mut self_.#field_name {
                            tx.insert_or_update(&*item).execute().await?;
                            <#target_type as ::ormer::model::GraphWritable>::update_graph_relations(
                                tx,
                                item,
                            )
                            .await?;
                            let target_key =
                                ::ormer::model::graph_target_key_value(item, target_relation)?;
                            ::ormer::model::graph_insert_through_link_values::<#via_type>(
                                tx,
                                via_relation.target_key,
                                owner_key.clone(),
                                target_relation.local_key,
                                target_key.clone(),
                            )
                            .await?;
                            target_keys.push(target_key);
                        }
                        ::ormer::model::graph_sync_through_links::<Self, #via_type>(
                            tx,
                            self_,
                            via_relation,
                            target_relation,
                            &target_keys,
                        )
                        .await?;
                    }
                }
            }
            RelationKindAttr::BelongsTo => quote! {},
        }
    });
    let dynamic_columns_method = if has_embed {
        let embedded_columns = normal_fields.iter().filter_map(|info| {
            info.embed.as_ref().map(|embed| {
                let field_type = info.field_type;
                let prefix = &embed.prefix;
                quote! {
                    for column in <#field_type as ::ormer::model::Embed>::COLUMNS {
                        columns.push(Box::leak(format!("{}{}", #prefix, column).into_boxed_str()));
                    }
                }
            })
        });
        quote! {
            fn columns() -> Vec<&'static str> {
                let mut columns = Vec::new();
                columns.extend([#(#field_names_lit),*]);
                #(#embedded_columns)*
                columns
            }
        }
    } else {
        quote! {}
    };
    let dynamic_column_schema_method = if has_embed {
        let embedded_schemas = normal_fields.iter().filter_map(|info| {
            info.embed.as_ref().map(|embed| {
                let field_type = info.field_type;
                let field_name = info.field_name.to_string();
                let prefix = &embed.prefix;
                quote! {
                    for schema in <#field_type as ::ormer::model::Embed>::COLUMN_SCHEMA {
                        columns.push(::ormer::model::ColumnSchema {
                            rust_name: Box::leak(format!("{}.{}", #field_name, schema.rust_name).into_boxed_str()),
                            name: Box::leak(format!("{}{}", #prefix, schema.name).into_boxed_str()),
                            rust_type: schema.rust_type,
                            is_primary: false,
                            is_auto_increment: false,
                            is_nullable: schema.is_nullable,
                            unique_group: None,
                            unique_name: None,
                            is_indexed: false,
                            index_group: None,
                            index_name: None,
                            index_order: None,
                            index_where: None,
                            foreign_key: None,
                            enum_variants: schema.enum_variants,
                            data_type: schema.data_type,
                            default: None,
                            check: None,
                            hypertable: None,
                            compress: false,
                        });
                    }
                }
            })
        });
        quote! {
            fn column_schema() -> Vec<::ormer::model::ColumnSchema> {
                let mut columns = <[_]>::into_vec(Box::new([#(#column_schema_entries),*]));
                #(#embedded_schemas)*
                columns
            }
        }
    } else {
        quote! {}
    };
    let filter_trait = syn::Ident::new(&format!("{name}FilterExt"), name.span());
    let filter_methods = filters.iter().map(|filter| {
        let method_name = &filter.name;
        let args = &filter.args;
        let body_expr = &filter.body_expr;
        let model_ident = &filter.model_ident;
        quote! {
            fn #method_name(self #(, #args)*) -> Self {
                let __ormer_expr = {
                    let __ormer_where = <#name as ::ormer::Model>::Where::default();
                    let #model_ident = __ormer_where;
                    #body_expr
                };
                ::ormer::FilterQuery::<#name>::append_filter_expr(self, __ormer_expr)
            }
        }
    });
    let filter_trait_tokens = if filters.is_empty() {
        quote! {}
    } else {
        quote! {
            pub trait #filter_trait: ::ormer::FilterQuery<#name> + Sized {
                #(#filter_methods)*
            }

            impl<Q> #filter_trait for Q where Q: ::ormer::FilterQuery<#name> + Sized {}
        }
    };

    quote! {
        #filter_trait_tokens

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

        impl #where_name {
            pub fn field(&self, name: impl Into<String>) -> ::ormer::query::builder::DynamicColumn<#name> {
                ::ormer::query::builder::DynamicColumn::new(name)
            }
        }

        pub struct #update_name {
            #(#update_fields),*
        }

        impl Default for #update_name {
            fn default() -> Self {
                Self {
                    #(#update_default_fields),*
                }
            }
        }

        impl ::ormer::query::update::UpdateFields for #update_name {
            fn assignments(&self) -> Vec<::ormer::query::update::UpdateAssignment> {
                let mut assignments = Vec::new();
                #(#update_assignment_fields)*
                assignments
            }
        }

        impl ::ormer::ViewModel for #name {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMNS: &'static [&'static str] = &[#(#field_names_lit),*];
            const COLUMN_SCHEMA: &'static [::ormer::model::ColumnSchema] = &[#(#column_schema_entries),*];

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = #where_name;

            #dynamic_columns_method
            #dynamic_column_schema_method

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                if values.len() < <Self as ::ormer::ViewModel>::columns().len() {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        <Self as ::ormer::ViewModel>::columns().len(),
                        stringify!(#name)
                    ));
                }
                let mut __ormer_value_index = 0usize;
                Ok(Self {
                    #(#from_row_values_fields),*
                })
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
            type Update = #update_name;

            #dynamic_columns_method
            #dynamic_column_schema_method

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                if values.len() < <Self as ::ormer::Model>::columns().len() {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        <Self as ::ormer::Model>::columns().len(),
                        stringify!(#name)
                    ));
                }
                let mut __ormer_value_index = 0usize;
                Ok(Self {
                    #(#from_row_values_fields),*
                })
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                let mut values = Vec::new();
                #(#field_names_for_values)*
                values
            }

            fn column_value(&self, column: &str) -> Option<::ormer::Value> {
                match column {
                    #(#column_value_arms,)*
                    _ => None,
                }
            }

            fn assign_column_value(
                &mut self,
                column: &str,
                value: ::ormer::Value,
            ) -> ::ormer::Result<()> {
                match column {
                    #(#assign_column_value_arms,)*
                    _ => {}
                }
                Err(::ormer::ormer_error!(
                    "Column {} is not assignable on {}",
                    column,
                    Self::TABLE_NAME
                ))
            }

            fn assign_relation<Target: ::ormer::Model + 'static>(
                &mut self,
                relation_name: &'static str,
                values: Vec<Target>,
            ) -> ::ormer::Result<()> {
                match relation_name {
                    #(#assign_relation_arms,)*
                    _ => Err(::ormer::ormer_error!(
                        "Relation {} is not assignable on {}",
                        relation_name,
                        Self::TABLE_NAME
                    )),
                }
            }

            fn graph_relations_mut(&mut self) -> Vec<::ormer::model::GraphRelationMut<'_>> {
                let mut relations = Vec::new();
                #(#graph_relation_entries)*
                relations
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

        impl ::ormer::WritableModel for #name {}

        impl ::ormer::model::GraphWritable for #name
        where
            #auto_increment_key_type: ::std::convert::Into<::ormer::Value>,
        {
            async fn insert_graph_relations<'tx>(
                tx: &mut ::ormer::Transaction<'tx>,
                self_: &mut Self,
            ) -> ::ormer::Result<()> {
                #(#graph_insert_entries)*
                Ok(())
            }

            async fn update_graph_relations<'tx>(
                tx: &mut ::ormer::Transaction<'tx>,
                self_: &mut Self,
            ) -> ::ormer::Result<()> {
                #(#graph_update_entries)*
                Ok(())
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

pub fn derive_insert_model(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let table_name = extract_table_name(&input);

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("InsertModel must have named fields"),
        },
        _ => panic!("InsertModel must be a struct"),
    };

    let assignment_fields = fields.iter().map(|field| {
        let field_name = field
            .ident
            .as_ref()
            .expect("InsertModel field must be named");
        let column_name = extract_column_name(field);
        if active_value_inner_type(&field.ty).is_some() {
            quote! {
                match &self.#field_name {
                    ::ormer::ActiveValue::NotSet => {}
                    ::ormer::ActiveValue::Set(value)
                    | ::ormer::ActiveValue::Unchanged(value) => {
                        assignments.push(::ormer::query::insert::InsertAssignment::value(
                            #column_name,
                            value.clone(),
                        ));
                    }
                }
            }
        } else {
            quote! {
                assignments.push(::ormer::query::insert::InsertAssignment::value(
                    #column_name,
                    self.#field_name.clone(),
                ));
            }
        }
    });

    quote! {
        impl<T: ::ormer::Model> ::ormer::model::InsertModel<T> for #name {
            fn insert_table_name(&self) -> &'static str {
                #table_name
            }

            fn insert_assignments(&self) -> Vec<::ormer::query::insert::InsertAssignment> {
                let mut assignments = Vec::new();
                #(#assignment_fields)*
                assignments
            }
        }
    }
}

pub fn derive_embed(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let where_name = syn::Ident::new(&format!("{name}Where"), name.span());

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("Embed must have named fields"),
        },
        _ => panic!("Embed must be a struct"),
    };

    let field_infos: Vec<_> = fields.iter().map(FieldInfo::new).collect();
    if field_infos
        .iter()
        .any(|info| info.is_relation() || info.is_primary || info.embed.is_some())
    {
        panic!("Embed cannot use #[primary], relation attributes, or nested #[embed]");
    }
    let normal_fields: Vec<_> = field_infos.iter().filter(|info| !info.is_ignored).collect();

    let field_names: Vec<String> = normal_fields
        .iter()
        .map(|info| info.column_name.clone())
        .collect();
    let field_names_lit = field_names
        .iter()
        .map(|name| quote! { #name })
        .collect::<Vec<_>>();

    let column_schema_entries = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let rust_field_name = field_name.to_string();
        let column_name = info.column_name.as_str();
        let field_type = info.field_type;
        let rust_type = &info.rust_type;
        let is_nullable = info.is_nullable;
        let data_type = &info.data_type;
        let enum_variants = if info.has_data_type {
            quote! { None }
        } else {
            quote! { <#field_type as ::ormer::model::ModelEnumProvider>::ENUM_VARIANTS }
        };

        quote! {
            ::ormer::model::EmbedColumnSchema {
                rust_name: #rust_field_name,
                name: #column_name,
                rust_type: #rust_type,
                is_nullable: #is_nullable,
                enum_variants: #enum_variants,
                data_type: #data_type,
            }
        }
    });

    let where_fields = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let field_type = info
            .effective_data_type_type
            .as_ref()
            .unwrap_or(info.field_type);
        quote! {
            pub #field_name: ::ormer::query::builder::TypedColumn<#field_type, #name>
        }
    });

    let where_default_fields = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let column_name = &info.column_name;
        quote! {
            #field_name: ::ormer::query::builder::TypedColumn::new(#column_name)
        }
    });

    let prefixed_where_default_fields = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let column_name = &info.column_name;
        quote! {
            #field_name: ::ormer::query::builder::TypedColumn::new(
                Box::leak(format!("{}{}", prefix, #column_name).into_boxed_str())
            )
        }
    });

    let from_row_fields = field_infos.iter().map(|info| {
        let field_name = info.field_name;
        if info.is_ignored {
            quote! {
                #field_name: ::std::default::Default::default()
            }
        } else if info.has_i32_data_type {
            let column_name = &info.column_name;
            field_from_i32_expr(
                info.field,
                quote! { row.get::<i32>(&format!("{}{}", prefix, #column_name))? },
                quote! { row.get::<Option<i32>>(&format!("{}{}", prefix, #column_name))? },
            )
        } else if info.has_vec_i32_data_type {
            let column_name = &info.column_name;
            field_from_vec_i32_expr(
                info.field,
                quote! { row.get::<Vec<i32>>(&format!("{}{}", prefix, #column_name))? },
            )
        } else {
            let column_name = &info.column_name;
            quote! {
                #field_name: row.get(&format!("{}{}", prefix, #column_name))?
            }
        }
    });

    let mut value_index = 0usize;
    let from_row_values_fields = field_infos.iter().map(|info| {
        let field_name = info.field_name;
        if info.is_ignored {
            quote! {
                #field_name: ::std::default::Default::default()
            }
        } else if info.has_i32_data_type {
            let i = syn::Index::from(value_index);
            value_index += 1;
            field_from_i32_expr(
                info.field,
                quote! {
                    <i32 as ::ormer::FromRowValues>::from_row_values(&values[#i..#i+1])?
                },
                quote! {
                    <Option<i32> as ::ormer::FromRowValues>::from_row_values(&values[#i..#i+1])?
                },
            )
        } else if info.has_vec_i32_data_type {
            let i = syn::Index::from(value_index);
            value_index += 1;
            field_from_vec_i32_expr(
                info.field,
                quote! {
                    <Vec<i32> as ::ormer::FromRowValues>::from_row_values(&values[#i..#i+1])?
                },
            )
        } else {
            let i = syn::Index::from(value_index);
            value_index += 1;
            let field_type = info.field_type;
            quote! {
                #field_name: <#field_type as ::ormer::FromRowValues>::from_row_values(
                    &values[#i..#i+1]
                )?
            }
        }
    });

    let field_values = normal_fields.iter().map(|info| field_to_value_expr(info));
    let column_value_arms = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let rust_field_name = field_name.to_string();
        let column_name = info.column_name.as_str();
        let value_expr = field_to_value_expr(info);
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
    let assign_column_value_arms = normal_fields.iter().map(|info| {
        let rust_field_name = info.field_name.to_string();
        let column_name = info.column_name.as_str();
        let assign_expr = field_assign_value_expr(info);
        if rust_field_name == column_name {
            quote! {
                #column_name => {
                    #assign_expr
                    return Ok(());
                }
            }
        } else {
            quote! {
                #column_name | #rust_field_name => {
                    #assign_expr
                    return Ok(());
                }
            }
        }
    });

    quote! {
        pub struct #where_name {
            #(#where_fields),*
        }

        impl #where_name {
            pub fn new_with_prefix(prefix: &str) -> Self {
                Self {
                    #(#prefixed_where_default_fields),*
                }
            }
        }

        impl ::ormer::model::EmbedWhere for #where_name {
            fn new_with_prefix(prefix: &str) -> Self {
                #where_name::new_with_prefix(prefix)
            }
        }

        impl Default for #where_name {
            fn default() -> Self {
                Self {
                    #(#where_default_fields),*
                }
            }
        }

        impl ::ormer::model::Embed for #name {
            const COLUMNS: &'static [&'static str] = &[#(#field_names_lit),*];
            const COLUMN_SCHEMA: &'static [::ormer::model::EmbedColumnSchema] =
                &[#(#column_schema_entries),*];

            type Where = #where_name;

            fn from_row(row: &::ormer::Row, prefix: &str) -> ::ormer::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                if values.len() < <Self as ::ormer::model::Embed>::COLUMNS.len() {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        <Self as ::ormer::model::Embed>::COLUMNS.len(),
                        stringify!(#name)
                    ));
                }
                Ok(Self {
                    #(#from_row_values_fields),*
                })
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                vec![#(#field_values),*]
            }

            fn column_value(&self, column: &str) -> Option<::ormer::Value> {
                match column {
                    #(#column_value_arms,)*
                    _ => None,
                }
            }

            fn assign_column_value(
                &mut self,
                column: &str,
                value: ::ormer::Value,
            ) -> ::ormer::Result<()> {
                match column {
                    #(#assign_column_value_arms,)*
                    _ => {}
                }
                Err(::ormer::ormer_error!(
                    "Column {} is not assignable on {}",
                    column,
                    stringify!(#name)
                ))
            }
        }
    }
}

pub fn derive_view_model(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let where_name = syn::Ident::new(&format!("{name}Where"), name.span());
    let table_name = extract_table_name(&input);

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("ViewModel must have named fields"),
        },
        _ => panic!("ViewModel must be a struct"),
    };

    let field_infos: Vec<_> = fields.iter().map(FieldInfo::new).collect();
    if field_infos
        .iter()
        .any(|info| info.is_primary || info.is_relation())
    {
        panic!(
            "ViewModel cannot use #[primary], #[has_many], #[belongs_to], #[has_one], or #[through]"
        );
    }

    let normal_fields: Vec<_> = field_infos.iter().filter(|info| !info.is_ignored).collect();

    let field_names: Vec<String> = normal_fields
        .iter()
        .map(|info| info.column_name.clone())
        .collect();
    let field_names_lit = field_names
        .iter()
        .map(|name| quote! { #name })
        .collect::<Vec<_>>();

    let column_schema_entries = normal_fields
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            let rust_field_name = field_name.to_string();
            let column_name = info.column_name.as_str();
            let field_type = info.field_type;
            let is_nullable = info.is_nullable;
            let rust_type = &info.rust_type;
            let data_type = &info.data_type;
            let enum_variants = if info.has_data_type {
                quote! { None }
            } else {
                quote! { <#field_type as ::ormer::model::ModelEnumProvider>::ENUM_VARIANTS }
            };

            quote! {
                ::ormer::model::ColumnSchema {
                    rust_name: #rust_field_name,
                    name: #column_name,
                    rust_type: #rust_type,
                    is_primary: false,
                    is_auto_increment: false,
                    is_nullable: #is_nullable,
                    unique_group: None,
                    unique_name: None,
                    is_indexed: false,
                    index_group: None,
                    index_name: None,
                    index_order: None,
                    index_where: None,
                    foreign_key: None,
                    enum_variants: #enum_variants,
                    data_type: #data_type,
                    default: None,
                    check: None,
                    hypertable: None,
                    compress: false,
                }
            }
        })
        .collect::<Vec<_>>();

    let from_row_fields = field_infos
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            if info.is_ignored {
                quote! {
                    #field_name: ::std::default::Default::default()
                }
            } else if info.has_i32_data_type {
                let column_name = &info.column_name;
                field_from_i32_expr(
                    info.field,
                    quote! { row.get::<i32>(#column_name)? },
                    quote! { row.get::<Option<i32>>(#column_name)? },
                )
            } else if info.has_vec_i32_data_type {
                let column_name = &info.column_name;
                field_from_vec_i32_expr(info.field, quote! { row.get::<Vec<i32>>(#column_name)? })
            } else {
                let column_name = &info.column_name;
                quote! {
                    #field_name: row.get(#column_name)?
                }
            }
        })
        .collect::<Vec<_>>();

    let mut value_index = 0usize;
    let from_row_values_fields = field_infos
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            if info.is_ignored {
                quote! {
                    #field_name: ::std::default::Default::default()
                }
            } else if info.has_i32_data_type {
                let i = syn::Index::from(value_index);
                value_index += 1;
                field_from_i32_expr(
                    info.field,
                    quote! {
                        <i32 as ::ormer::FromRowValues>::from_row_values(&values[#i..#i+1])?
                    },
                    quote! {
                        <Option<i32> as ::ormer::FromRowValues>::from_row_values(
                            &values[#i..#i+1]
                        )?
                    },
                )
            } else if info.has_vec_i32_data_type {
                let i = syn::Index::from(value_index);
                value_index += 1;
                field_from_vec_i32_expr(
                    info.field,
                    quote! {
                        <Vec<i32> as ::ormer::FromRowValues>::from_row_values(
                            &values[#i..#i+1]
                        )?
                    },
                )
            } else {
                let i = syn::Index::from(value_index);
                value_index += 1;
                let field_type = info.field_type;
                quote! {
                    #field_name: <#field_type as ::ormer::FromRowValues>::from_row_values(
                        &values[#i..#i+1]
                    )?
                }
            }
        })
        .collect::<Vec<_>>();

    let where_fields = normal_fields
        .iter()
        .map(|info| {
            let field_name = info.field_name;
            let field_type = info
                .effective_data_type_type
                .as_ref()
                .unwrap_or(info.field_type);
            quote! {
                pub #field_name: ::ormer::query::builder::TypedColumn<#field_type, #name>
            }
        })
        .collect::<Vec<_>>();

    let where_default_fields = normal_fields.iter().map(|info| {
        let field_name = info.field_name;
        let column_name = &info.column_name;
        quote! {
            #field_name: ::ormer::query::builder::TypedColumn::new(#column_name)
        }
    });

    let field_values = normal_fields.iter().map(|info| field_to_value_expr(info));
    let column_value_arms = field_infos.iter().map(|info| {
        let field_name = info.field_name;
        let rust_field_name = field_name.to_string();
        let column_name = info.column_name.as_str();
        let value_expr = field_to_value_expr(info);
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

    quote! {
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

        impl #where_name {
            pub fn field(&self, name: impl Into<String>) -> ::ormer::query::builder::DynamicColumn<#name> {
                ::ormer::query::builder::DynamicColumn::new(name)
            }
        }

        impl ::ormer::ViewModel for #name {
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

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                if values.len() < <Self as ::ormer::ViewModel>::COLUMNS.len() {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        <Self as ::ormer::ViewModel>::COLUMNS.len(),
                        stringify!(#name)
                    ));
                }
                Ok(Self {
                    #(#from_row_values_fields),*
                })
            }
        }

        impl ::ormer::Model for #name {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMNS: &'static [&'static str] = &[#(#field_names_lit),*];
            const COLUMN_SCHEMA: &'static [::ormer::model::ColumnSchema] = &[#(#column_schema_entries),*];
            const RELATIONS: &'static [::ormer::model::RelationInfo] = &[];

            type AutoIncrementKeyType = ();
            type QueryBuilder = ::ormer::Select<Self>;
            type Where = #where_name;
            type Update = ();

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                if values.len() < <Self as ::ormer::Model>::COLUMNS.len() {
                    return Err(::ormer::ormer_error!(
                        "Expected {} values for {}",
                        <Self as ::ormer::Model>::COLUMNS.len(),
                        stringify!(#name)
                    ));
                }
                Ok(Self {
                    #(#from_row_values_fields),*
                })
            }

            fn field_values(&self) -> Vec<::ormer::Value> {
                vec![
                    #(#field_values),*
                ]
            }

            fn column_value(&self, column: &str) -> Option<::ormer::Value> {
                match column {
                    #(#column_value_arms,)*
                    _ => None,
                }
            }

            fn primary_key_columns() -> &'static [&'static str] {
                &[]
            }

            fn primary_key_values(&self) -> Vec<::ormer::Value> {
                Vec::new()
            }
        }

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
    HasOne,
    Through,
}

#[derive(Clone)]
struct ThroughAttr {
    via_relation: String,
    target_relation: String,
}

#[derive(Clone)]
struct EmbedAttr {
    prefix: String,
}

#[derive(Clone)]
struct RelationField {
    field_name: syn::Ident,
    target_type: syn::Type,
    kind: RelationKindAttr,
    local_key: String,
    target_key: String,
    through: Option<ThroughAttr>,
}

struct FilterArg {
    ident: syn::Ident,
    ty: syn::Type,
}

impl quote::ToTokens for FilterArg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = &self.ident;
        let ty = &self.ty;
        tokens.extend(quote! { #ident: #ty });
    }
}

struct ModelFilter {
    name: syn::Ident,
    args: Vec<FilterArg>,
    body_expr: Box<syn::Expr>,
    model_ident: syn::Pat,
}

struct FieldInfo<'a> {
    field: &'a syn::Field,
    field_name: &'a syn::Ident,
    field_type: &'a syn::Type,
    column_name: String,
    rust_type: String,
    is_nullable: bool,
    is_primary: bool,
    primary_auto: bool,
    relation: Option<RelationField>,
    relation_default: Option<proc_macro2::TokenStream>,
    embed: Option<EmbedAttr>,
    unique_attr: UniqueAttr,
    index_attr: Option<IndexAttr>,
    foreign_key: proc_macro2::TokenStream,
    data_type: proc_macro2::TokenStream,
    effective_data_type_type: Option<syn::Type>,
    has_data_type: bool,
    has_i32_data_type: bool,
    has_vec_i32_data_type: bool,
    default: proc_macro2::TokenStream,
    check: proc_macro2::TokenStream,
    hypertable: proc_macro2::TokenStream,
    compress: bool,
    is_ignored: bool,
    normal_index: Option<usize>,
}

impl<'a> FieldInfo<'a> {
    fn new(field: &'a syn::Field) -> Self {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let column_name = extract_column_name(field);
        let type_str = normalize_type_string(quote! { #field_type }.to_string());
        let is_nullable = type_str.starts_with("Option<");
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

        let data_type_type = extract_data_type_type(field);
        validate_data_type(field, data_type_type.as_ref());
        let effective_data_type_type = data_type_type.as_ref().map(|data_type| {
            if option_inner_type(field_type).is_some() {
                option_inner_type(data_type)
                    .cloned()
                    .unwrap_or_else(|| data_type.clone())
            } else {
                data_type.clone()
            }
        });
        let has_data_type = data_type_type.is_some();
        let has_i32_data_type = effective_data_type_type
            .as_ref()
            .map(is_i32_type)
            .unwrap_or(false);
        let has_vec_i32_data_type = effective_data_type_type
            .as_ref()
            .map(is_vec_i32_type)
            .unwrap_or(false);
        let data_type = data_type_tokens(effective_data_type_type.as_ref());
        let (is_primary, primary_auto) = extract_primary_attr(field);
        let relation = extract_relation_field(field);
        let embed = extract_embed_attr(field);
        if relation.is_some() && embed.is_some() {
            panic!("relation fields cannot use #[embed]");
        }
        if embed.is_some()
            && field
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("primary"))
        {
            panic!("#[embed] fields cannot be primary keys");
        }
        let relation_default = relation_default_expr(relation.as_ref());

        Self {
            field,
            field_name,
            field_type,
            column_name,
            rust_type,
            is_nullable,
            is_primary,
            primary_auto,
            relation,
            relation_default,
            embed,
            unique_attr: extract_unique_attr(field),
            index_attr: extract_index_attr(field),
            foreign_key: extract_foreign_key(field),
            data_type,
            effective_data_type_type,
            has_data_type,
            has_i32_data_type,
            has_vec_i32_data_type,
            default: extract_default(field),
            check: extract_check(field),
            hypertable: extract_hypertable(field),
            compress: field
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("compress")),
            is_ignored: field
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("ormer_ignore")),
            normal_index: None,
        }
    }

    fn is_relation(&self) -> bool {
        self.relation.is_some()
    }
}

fn extract_primary_attr(field: &syn::Field) -> (bool, bool) {
    for attr in &field.attrs {
        if attr.path().is_ident("primary") {
            let is_auto = if let Meta::List(list) = &attr.meta {
                list.tokens.to_string().contains("auto")
            } else {
                false
            };
            return (true, is_auto);
        }
    }
    (false, false)
}

fn relation_default_expr(relation: Option<&RelationField>) -> Option<proc_macro2::TokenStream> {
    relation.map(|relation| match relation.kind {
        RelationKindAttr::HasMany | RelationKindAttr::Through => quote! { Vec::new() },
        RelationKindAttr::BelongsTo | RelationKindAttr::HasOne => quote! { None },
    })
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
                through: None,
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
                through: None,
            });
        }
        if attr.path().is_ident("has_one") {
            let (target_type, target_key) = parse_has_one(attr);
            if option_inner_type(&field.ty).is_none() {
                panic!("#[has_one] field must be Option<T>");
            }
            return Some(RelationField {
                field_name,
                target_type,
                kind: RelationKindAttr::HasOne,
                local_key: String::new(),
                target_key,
                through: None,
            });
        }
        if attr.path().is_ident("through") {
            let through = parse_through(attr);
            let target_type = vec_inner_type(&field.ty)
                .cloned()
                .unwrap_or_else(|| panic!("#[through] field must be Vec<T>"));
            return Some(RelationField {
                field_name,
                target_type,
                kind: RelationKindAttr::Through,
                local_key: String::new(),
                target_key: String::new(),
                through: Some(through),
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

fn parse_has_one(attr: &syn::Attribute) -> (syn::Type, String) {
    if let Meta::List(list) = &attr.meta {
        let tokens_str = list.tokens.to_string();
        let parts: Vec<&str> = tokens_str.split('.').collect();
        if parts.len() == 2 {
            let target_type: syn::Type =
                syn::parse_str(parts[0].trim()).expect("#[has_one] target type is invalid");
            return (target_type, parts[1].trim().to_string());
        }
    }
    panic!("#[has_one] must use #[has_one(Target.foreign_key)]");
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

fn parse_through(attr: &syn::Attribute) -> ThroughAttr {
    if let Meta::List(list) = &attr.meta {
        let tokens_str = list.tokens.to_string();
        let parts: Vec<&str> = tokens_str.split('.').collect();
        if parts.len() == 2 {
            return ThroughAttr {
                via_relation: parts[0].trim().to_string(),
                target_relation: parts[1].trim().to_string(),
            };
        }
    }
    panic!("#[through] must use #[through(via_relation.target_relation)]");
}

fn extract_embed_attr(field: &syn::Field) -> Option<EmbedAttr> {
    for attr in &field.attrs {
        if attr.path().is_ident("embed") {
            let mut prefix = String::new();
            if let Meta::List(_) = &attr.meta {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("prefix") {
                        let value = meta.value()?;
                        let lit: syn::LitStr = value.parse()?;
                        prefix = lit.value();
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[embed] argument"))
                    }
                })
                .expect("Failed to parse #[embed] attribute");
            }
            return Some(EmbedAttr { prefix });
        }
    }
    None
}

fn through_via_type<'a>(
    relation: &RelationField,
    relation_fields: &'a [RelationField],
) -> &'a syn::Type {
    let through = relation
        .through
        .as_ref()
        .expect("#[through] relation metadata is missing");
    relation_fields
        .iter()
        .find(|candidate| candidate.field_name.to_string() == through.via_relation)
        .map(|candidate| &candidate.target_type)
        .unwrap_or_else(|| panic!("#[through] via relation must reference a relation field"))
}

fn extract_model_filters(input: &DeriveInput) -> Vec<ModelFilter> {
    input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("filter"))
        .map(parse_model_filter)
        .collect()
}

fn parse_model_filter(attr: &syn::Attribute) -> ModelFilter {
    let Meta::List(list) = &attr.meta else {
        panic!("#[filter] must use #[filter(filter_name, |m| expr)]");
    };
    let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
    let args = list
        .parse_args_with(parser)
        .expect("Failed to parse #[filter] attribute");
    if args.len() != 2 {
        panic!("#[filter] must use #[filter(filter_name, |m| expr)]");
    }

    let name = match &args[0] {
        syn::Expr::Path(path) if path.path.segments.len() == 1 => {
            path.path.segments[0].ident.clone()
        }
        _ => panic!("#[filter] name must be an identifier"),
    };
    if !name.to_string().starts_with("filter_") {
        panic!("#[filter] name must start with filter_");
    }

    let body = match &args[1] {
        syn::Expr::Closure(closure) => closure.clone(),
        _ => panic!("#[filter] second argument must be a closure"),
    };
    if body.inputs.is_empty() {
        panic!("#[filter] closure must receive the model where object");
    }

    let model_ident = body.inputs[0].clone();
    let mut filter_args = Vec::new();
    for input in body.inputs.iter().skip(1) {
        match input {
            syn::Pat::Type(pat_ty) => {
                let syn::Pat::Ident(pat_ident) = pat_ty.pat.as_ref() else {
                    panic!("#[filter] closure arguments must be simple identifiers");
                };
                filter_args.push(FilterArg {
                    ident: pat_ident.ident.clone(),
                    ty: (*pat_ty.ty).clone(),
                });
            }
            syn::Pat::Ident(_) => panic!("#[filter] closure extra arguments must have types"),
            _ => panic!("#[filter] closure arguments must be simple identifiers"),
        }
    }

    ModelFilter {
        name,
        args: filter_args,
        body_expr: body.body,
        model_ident,
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
        impl ::ormer::ViewModel for #name {
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

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                let inner = <#inner_type as ::ormer::Model>::from_row(row)?;
                Ok(#name(inner))
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
                let inner = <#inner_type as ::ormer::Model>::from_row_values(values)?;
                Ok(#name(inner))
            }
        }

        impl ::ormer::Model for #name {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMNS: &'static [&'static str] = <#inner_type as ::ormer::Model>::COLUMNS;
            const COLUMN_SCHEMA: &'static [::ormer::model::ColumnSchema] = <#inner_type as ::ormer::Model>::COLUMN_SCHEMA;

            type AutoIncrementKeyType = <#inner_type as ::ormer::Model>::AutoIncrementKeyType;

            type QueryBuilder = ::ormer::Select<Self>;
            type Where = <#inner_type as ::ormer::Model>::Where;
            type Update = <#inner_type as ::ormer::Model>::Update;

            fn query() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn select() -> Self::QueryBuilder {
                ::ormer::Select::new()
            }

            fn from_row(row: &::ormer::Row) -> ::ormer::Result<Self> {
                let inner = <#inner_type as ::ormer::Model>::from_row(row)?;
                Ok(#name(inner))
            }

            fn from_row_values(values: &[::ormer::Value]) -> ::ormer::Result<Self> {
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

        impl ::ormer::WritableModel for #name {}

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
fn data_type_tokens(data_type: Option<&syn::Type>) -> proc_macro2::TokenStream {
    if let Some(data_type) = data_type {
        let type_str = normalize_type_string(quote! { #data_type }.to_string());
        return quote! { Some(#type_str) };
    }
    quote! { None }
}

fn validate_data_type(field: &syn::Field, data_type: Option<&syn::Type>) {
    let Some(data_type) = data_type else {
        return;
    };

    let field_is_optional = option_inner_type(&field.ty).is_some();
    let data_type_is_optional = option_inner_type(data_type).is_some();
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

fn active_value_inner_type(ty: &syn::Type) -> Option<&syn::Type> {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            let segment = type_path.path.segments.last()?;
            if segment.ident != "ActiveValue" {
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

fn field_to_value_expr(info: &FieldInfo<'_>) -> proc_macro2::TokenStream {
    let field_name = info.field_name;
    let field_type = info.field_type;
    let value_type = option_inner_type(field_type).unwrap_or(field_type);

    if info.has_i32_data_type {
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
    } else if info.has_vec_i32_data_type {
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

fn field_assign_value_expr(info: &FieldInfo<'_>) -> proc_macro2::TokenStream {
    let field_name = info.field_name;
    let field_type = info.field_type;
    let value_type = option_inner_type(field_type).unwrap_or(field_type);

    if info.has_i32_data_type {
        if option_inner_type(field_type).is_some() {
            quote! {
                self.#field_name = match value {
                    ::ormer::Value::Null => None,
                    value => {
                        let raw = <i32 as ::ormer::FromValue>::from_value(&value)?;
                        use ::ormer::model::I32DataTypeDecode as _;
                        Some(
                            ::ormer::model::I32DataTypeDecoder::<#value_type>::new().decode(
                                raw,
                                stringify!(#field_name),
                                stringify!(#value_type),
                            )?
                        )
                    }
                };
            }
        } else {
            quote! {
                let raw = <i32 as ::ormer::FromValue>::from_value(&value)?;
                use ::ormer::model::I32DataTypeDecode as _;
                self.#field_name =
                    ::ormer::model::I32DataTypeDecoder::<#value_type>::new().decode(
                        raw,
                        stringify!(#field_name),
                        stringify!(#value_type),
                    )?;
            }
        }
    } else if info.has_vec_i32_data_type {
        let inner_type = vec_inner_type(field_type)
            .expect("#[data_type(Vec<i32>)] requires a Vec<T> field");
        quote! {
            let values = <Vec<i32> as ::ormer::FromValue>::from_value(&value)?;
            use ::ormer::model::I32DataTypeDecode as _;
            self.#field_name = values
                .into_iter()
                .map(|value| {
                    ::ormer::model::I32DataTypeDecoder::<#inner_type>::new()
                        .decode(value, stringify!(#field_name), stringify!(#inner_type))
                })
                .collect::<::ormer::Result<Vec<#inner_type>>>()?;
        }
    } else {
        quote! {
            self.#field_name = <#field_type as ::ormer::FromValue>::from_value(&value)?;
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
                .collect::<::ormer::Result<Vec<#inner_type>>>()?
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

#[cfg(test)]
mod view_model_tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn derive_view_model_generates_read_only_tokens() {
        let input: DeriveInput = parse_quote! {
            #[derive(Debug, ViewModel)]
            #[table = "view_users"]
            struct ViewUser {
                id: i32,
                name: String,
            }
        };

        let tokens = derive_view_model(input).to_string();

        assert!(tokens.contains("impl :: ormer :: ViewModel for ViewUser"));
        assert!(tokens.contains("impl :: ormer :: Model for ViewUser"));
        assert!(!tokens.contains("impl :: ormer :: WritableModel for ViewUser"));
        assert!(tokens.contains("type Update = ()"));
        assert!(tokens.contains("primary_key_columns"));
        assert!(tokens.contains("& []"));
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
        let info = FieldInfo::new(&field);
        assert!(info.has_i32_data_type);

        let effective_type = info
            .effective_data_type_type
            .as_ref()
            .expect("data type should exist");
        assert_eq!(
            normalize_type_string(quote! { #effective_type }.to_string()),
            "i32"
        );
    }
}
