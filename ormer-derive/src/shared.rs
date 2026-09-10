//! `Model` 与 `ModelEnum`/`FieldType` 派生共享的属性解析与类型工具。
//! 两个派生入口必须使用同一份实现，避免列名/类型推断行为分叉。

use syn::{Expr, Lit, Meta};

/// 提取字段/变体的 SQL 列名。
///
/// 支持三种写法：`#[column = "..."]`、`#[column("...")]`、`#[column(name = "...")]`。
/// 未指定列名时使用字段名，raw identifier（如 `r#type`）剥离 `r#` 前缀。
pub(crate) fn extract_column_name(field: &syn::Field) -> syn::Result<String> {
    let default_name = unraw_ident(field.ident.as_ref().expect("field must be named"));

    for attr in &field.attrs {
        if attr.path().is_ident("column") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let Expr::Lit(expr) = &meta.value
                    && let Lit::Str(lit) = &expr.lit
                {
                    return Ok(lit.value());
                }
            }

            if let Meta::List(list) = &attr.meta {
                if let Ok(lit) = syn::parse2::<syn::LitStr>(list.tokens.clone()) {
                    return Ok(lit.value());
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
                })?;

                if let Some(name) = name {
                    return Ok(name);
                }
            }
        }
    }

    Ok(default_name)
}

/// raw identifier（`r#type` 等）的裸名字；普通标识符原样返回。
pub(crate) fn unraw_ident(ident: &syn::Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").unwrap_or(&name).to_string()
}

/// 按词边界转蛇形命名：连续大写序列视为一段缩略词，
/// 在缩略词末尾（后跟小写）或小写/数字之后断词。
/// `HTTPRequest` → `http_request`、`UserID` → `user_id`、`ModelName` → `model_name`。
pub(crate) fn to_snake_case(s: &str) -> String {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let starts_new_word = i > 0
                && (chars[i - 1].is_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || (chars[i - 1].is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase())));
            if starts_new_word {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

/// 规范化 token 展开的类型字符串（去掉多余空格）。
pub(crate) fn normalize_type_string(type_str: String) -> String {
    type_str
        .replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" , ", ",")
}

/// `Option<T>` 的内层类型；非 Option 类型返回 `None`。
pub(crate) fn option_inner_type(ty: &syn::Type) -> Option<&syn::Type> {
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

/// 是否为裸 `String` 类型（不含 `Option` 包装与限定路径）。
pub(crate) fn is_string_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident == "String")
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::to_snake_case;

    #[test]
    fn snake_case_splits_acronyms_at_word_boundaries() {
        assert_eq!(to_snake_case("HTTPRequest"), "http_request");
        assert_eq!(to_snake_case("UserID"), "user_id");
        assert_eq!(to_snake_case("ModelName"), "model_name");
        assert_eq!(to_snake_case("simple"), "simple");
        assert_eq!(to_snake_case("AB"), "ab");
        assert_eq!(to_snake_case("OAuth2Token"), "o_auth2_token");
    }
}
