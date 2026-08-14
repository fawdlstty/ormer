use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, LitStr};

enum Segment {
    Text(String),
    Expr(Expr),
}

pub fn expand(input: TokenStream) -> TokenStream {
    let lit = match syn::parse2::<LitStr>(input) {
        Ok(lit) => lit,
        Err(err) => return err.to_compile_error(),
    };

    let segments = match parse_segments(&lit.value()) {
        Ok(segments) => segments,
        Err(message) => return syn::Error::new(lit.span(), message).to_compile_error(),
    };

    let segments = segments.into_iter().map(|segment| match segment {
        Segment::Text(text) => quote! {
            ::ormer::RawExprSegment::text(#text)
        },
        Segment::Expr(expr) => quote! {
            ::ormer::RawExprSegment::expr(&(#expr))
        },
    });

    quote! {
        ::ormer::RawExpr::<()>::new(::ormer::RawSqlExpr::new(vec![#(#segments),*]))
    }
}

fn parse_segments(input: &str) -> Result<Vec<Segment>, String> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut index = 0;

    while index < input.len() {
        let rest = &input[index..];
        if rest.starts_with("{{") {
            text.push('{');
            index += 2;
        } else if rest.starts_with("}}") {
            text.push('}');
            index += 2;
        } else if rest.starts_with('{') {
            push_text(&mut segments, &mut text);
            let (expr_src, next) = read_braced_expr(input, index + 1)?;
            let expr_src = expr_src.trim();
            if expr_src.is_empty() {
                return Err("raw expression placeholder cannot be empty".to_string());
            }
            let expr = syn::parse_str::<Expr>(expr_src)
                .map_err(|err| format!("invalid raw expression `{expr_src}`: {err}"))?;
            segments.push(Segment::Expr(expr));
            index = next;
        } else if rest.starts_with('}') {
            return Err(
                "unmatched `}` in raw expression; use `}}` for a literal brace".to_string(),
            );
        } else {
            let ch = rest.chars().next().expect("non-empty string slice");
            text.push(ch);
            index += ch.len_utf8();
        }
    }

    push_text(&mut segments, &mut text);
    Ok(segments)
}

fn push_text(segments: &mut Vec<Segment>, text: &mut String) {
    if !text.is_empty() {
        segments.push(Segment::Text(std::mem::take(text)));
    }
}

fn read_braced_expr(input: &str, mut index: usize) -> Result<(&str, usize), String> {
    let start = index;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    while index < input.len() {
        if let Some(next) = skip_raw_string(input, index) {
            index = next;
            continue;
        }

        let rest = &input[index..];
        let ch = rest.chars().next().expect("non-empty string slice");
        match ch {
            '"' | '\'' => {
                index = skip_quoted(input, index, ch);
            }
            '(' => {
                paren_depth += 1;
                index += 1;
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            '[' => {
                bracket_depth += 1;
                index += 1;
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            '{' => {
                brace_depth += 1;
                index += 1;
            }
            '}' => {
                if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                    return Ok((&input[start..index], index + 1));
                }
                brace_depth = brace_depth.saturating_sub(1);
                index += 1;
            }
            _ => {
                index += ch.len_utf8();
            }
        }
    }

    Err("unclosed `{` in raw expression".to_string())
}

fn skip_quoted(input: &str, mut index: usize, quote: char) -> usize {
    index += quote.len_utf8();
    while index < input.len() {
        let rest = &input[index..];
        let ch = rest.chars().next().expect("non-empty string slice");
        index += ch.len_utf8();
        if ch == '\\' {
            if let Some(next) = input[index..].chars().next() {
                index += next.len_utf8();
            }
        } else if ch == quote {
            break;
        }
    }
    index
}

fn skip_raw_string(input: &str, index: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(index) != Some(&b'r') {
        return None;
    }

    let mut cursor = index + 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }

    let hashes = cursor - index - 1;
    cursor += 1;
    while cursor < input.len() {
        if bytes.get(cursor) == Some(&b'"') {
            let mut hash_cursor = cursor + 1;
            let mut matched = 0usize;
            while matched < hashes && bytes.get(hash_cursor) == Some(&b'#') {
                matched += 1;
                hash_cursor += 1;
            }
            if matched == hashes {
                return Some(hash_cursor);
            }
        }
        cursor += 1;
    }

    Some(input.len())
}
