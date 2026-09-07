extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::{parse_macro_input, parse_quote};

#[proc_macro_attribute]
pub fn on_setup(_args: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
        modifiers: _,
    } = parse_macro_input!(item as ItemFn);

    if sig.asyncness.is_some() {
        return syn::Error::new_spanned(&sig.fn_token, "#[on_setup] cannot be used to `async fn`")
            .to_compile_error()
            .into();
    }

    quote! {
        #(#attrs)*
        #[::linkme::distributed_slice(crate::SETUPS)]
        #vis #sig {
            #block
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn on_update(_args: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        mut sig,
        block,
        modifiers: _,
    } = parse_macro_input!(item as ItemFn);

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(&sig.fn_token, "#[on_update] must be used to `async fn`")
            .to_compile_error()
            .into();
    }

    sig.asyncness = None;
    sig.output = parse_quote!(-> crate::HandlerResult);

    quote! {
        #(#attrs)*
        #[::linkme::distributed_slice(crate::HANDLERS)]
        #vis #sig {
            Box::pin(async move #block)
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn on_new_message(_args: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        mut sig,
        block,
        modifiers: _,
    } = parse_macro_input!(item as ItemFn);

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            &sig.fn_token,
            "#[on_new_message] must be used to `async fn`",
        )
        .to_compile_error()
        .into();
    }

    sig.asyncness = None;
    sig.output = parse_quote!(-> crate::HandlerResult);

    quote! {
        #(#attrs)*
        #[::linkme::distributed_slice(crate::NEW_MESSAGE_HANDLERS)]
        #vis #sig {
            Box::pin(async move #block)
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn on_grouped_messages(_args: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        mut sig,
        block,
        modifiers: _,
    } = parse_macro_input!(item as ItemFn);

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            &sig.fn_token,
            "#[on_grouped_messages] must be used to `async fn`",
        )
        .to_compile_error()
        .into();
    }

    sig.asyncness = None;
    sig.output = parse_quote!(-> crate::HandlerResult);

    quote! {
        #(#attrs)*
        #[::linkme::distributed_slice(crate::GROUPED_MESSAGES_HANDLERS)]
        #vis #sig {
            Box::pin(async move #block)
        }
    }
    .into()
}
