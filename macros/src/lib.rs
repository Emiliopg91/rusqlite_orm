mod ddls;
mod entity;

use proc_macro::TokenStream;

#[proc_macro]
pub fn dlls(input: TokenStream) -> TokenStream {
    ddls::dlls(input)
}

#[proc_macro_derive(
    Entity,
    attributes(entity, primary_key, trasient, column, indexes, relationship)
)]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    entity::derive_entity(input)
}
