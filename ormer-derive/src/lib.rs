mod model;

use proc_macro::TokenStream;
use syn;

#[proc_macro_derive(Model, attributes(table, primary, unique, index))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    model::derive_model(input).into()
}
