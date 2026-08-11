mod model;
mod model_enum;

use proc_macro::TokenStream;

#[proc_macro_derive(
    Model,
    attributes(
        table, column, primary, unique, index, foreign, data_type, default, check, hypertable,
        compress, has_many, belongs_to, has_one, through
    )
)]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_model(input).into()
}

#[proc_macro_derive(ViewModel, attributes(table, column, data_type))]
pub fn derive_view_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_view_model(input).into()
}

#[proc_macro_derive(ModelEnum)]
pub fn derive_model_enum(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model_enum::derive_model_enum(input).into()
}

#[proc_macro_derive(InsertModel, attributes(table, column))]
pub fn derive_insert_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_insert_model(input).into()
}
