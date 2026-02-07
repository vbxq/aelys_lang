mod args;
mod expand;
mod export;

use proc_macro::TokenStream;
use syn::parse_macro_input;

// marker attr, actual codegen happens in aelys_module
#[proc_macro_attribute]
pub fn aelys_export(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

// generates module descriptor + FFI wrappers
#[proc_macro_attribute]
pub fn aelys_module(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as args::ModuleArgs);
    let input = parse_macro_input!(item as syn::ItemMod);

    match expand::expand_module(args, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
