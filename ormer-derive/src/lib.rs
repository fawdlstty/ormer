mod db_value;
mod model;
mod model_enum;
mod raw;

use proc_macro::TokenStream;

#[proc_macro_derive(
    Model,
    attributes(
        table,
        column,
        primary,
        unique,
        index,
        foreign,
        data_type,
        default,
        check,
        hypertable,
        compress,
        has_many,
        belongs_to,
        has_one,
        through,
        embed,
        filter,
        version,
        ormer_ignore
    )
)]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_model(input).into()
}

#[proc_macro_derive(Embed, attributes(column, data_type))]
pub fn derive_embed(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_embed(input).into()
}

#[proc_macro_derive(ViewModel, attributes(table, column, data_type))]
pub fn derive_view_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_view_model(input).into()
}

#[proc_macro_derive(ModelEnum, attributes(db_type, column))]
pub fn derive_model_enum(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model_enum::derive_model_enum(input).into()
}

#[proc_macro_derive(FieldType, attributes(db_type, column))]
pub fn derive_field_type(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model_enum::derive_field_type(input).into()
}

#[proc_macro_derive(DbValue, attributes(db_type))]
pub fn derive_db_value(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    db_value::derive_db_value(input).into()
}

#[proc_macro_derive(InsertModel, attributes(table, column))]
pub fn derive_insert_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_insert_model(input).into()
}

#[proc_macro]
pub fn raw(input: TokenStream) -> TokenStream {
    raw::expand(input.into()).into()
}
