//! `#[main]` makes the firmware's entry point a public function the simulator host calls; `#[handler]` and
//! `#[ram]` leave the function as it is (the simulator calls the ISR through a plain `fn()`).

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn main(_args: TokenStream, item: TokenStream) -> TokenStream {
    let mut f = parse_macro_input!(item as ItemFn);
    f.vis = syn::parse_quote!(pub);
    quote!(#f).into()
}

#[proc_macro_attribute]
pub fn handler(_args: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn ram(_args: TokenStream, item: TokenStream) -> TokenStream {
    item
}
