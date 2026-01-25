// generates extern "C" wrappers for #[aelys_export] fns

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{FnArg, Ident, ItemFn, Pat, PatType, ReturnType, Type};

pub struct ExportInfo {
    pub name: String,
    pub arity: u8,
    pub wrapper_name: Ident,
}

pub fn generate_export_wrapper(func: &ItemFn) -> syn::Result<(TokenStream2, ExportInfo)> {
    let fn_name = &func.sig.ident;
    let wrapper_name = format_ident!("__aelys_wrapper_{}", fn_name);

    if func.sig.abi.is_some() {
        let arity = if func.sig.inputs.len() >= 4 {
            0
        } else {
            func.sig.inputs.len() as u8
        };

        let wrapper = quote! {
            #[doc(hidden)]
            #[unsafe(no_mangle)]
            pub use exports::#fn_name as #wrapper_name;
        };

        return Ok((
            wrapper,
            ExportInfo {
                name: fn_name.to_string(),
                arity,
                wrapper_name: fn_name.clone(),
            },
        ));
    }

    let mut param_extractions = Vec::new();
    let mut call_args = Vec::new();
    let mut arity: u8 = 0;

    for (i, arg) in func.sig.inputs.iter().enumerate() {
        if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
            let param_name = match &**pat {
                Pat::Ident(ident) => &ident.ident,
                _ => return Err(syn::Error::new(pat.span(), "expected identifier pattern")),
            };

            let extraction = generate_extraction(param_name, ty, i)?;
            param_extractions.push(extraction);
            call_args.push(quote! { #param_name });
            arity += 1;
        }
    }

    let return_conversion = match &func.sig.output {
        ReturnType::Default => quote! { ::aelys_native::value_null() },
        ReturnType::Type(_, ty) => generate_return_conversion(ty)?,
    };

    let mod_name = format_ident!("exports");
    let wrapper = quote! {
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        pub extern "C" fn #wrapper_name(
            _vm: *mut ::core::ffi::c_void,
            args: *const ::aelys_native::AelysValue,
            _arg_count: usize,
            out: *mut ::aelys_native::AelysValue,
        ) -> i32 {
            let result = (|| {
                #(#param_extractions)*
                let ret = #mod_name::#fn_name(#(#call_args),*);
                #return_conversion
            })();

            unsafe {
                *out = result;
            }
            0
        }
    };

    Ok((
        wrapper,
        ExportInfo {
            name: fn_name.to_string(),
            arity,
            wrapper_name,
        },
    ))
}

fn generate_extraction(name: &Ident, ty: &Type, index: usize) -> syn::Result<TokenStream2> {
    let ty_str = quote!(#ty).to_string().replace(' ', "");
    let idx = index;

    let extraction = match ty_str.as_str() {
        "i64" => quote! {
            let #name: i64 = unsafe { ::aelys_native::value_as_int(*args.add(#idx)) };
        },
        "f64" => quote! {
            let #name: f64 = unsafe { ::aelys_native::value_as_float(*args.add(#idx)) };
        },
        "bool" => quote! {
            let #name: bool = unsafe { ::aelys_native::value_as_bool(*args.add(#idx)) };
        },
        _ => {
            return Err(syn::Error::new(
                ty.span(),
                format!(
                    "unsupported parameter type: {}. Supported: i64, f64, bool",
                    ty_str
                ),
            ));
        }
    };

    Ok(extraction)
}

fn generate_return_conversion(ty: &Type) -> syn::Result<TokenStream2> {
    let ty_str = quote!(#ty).to_string().replace(' ', "");

    let conversion = match ty_str.as_str() {
        "i64" => quote! { ::aelys_native::value_int(ret) },
        "f64" => quote! { ::aelys_native::value_float(ret) },
        "bool" => quote! { ::aelys_native::value_bool(ret) },
        "()" => quote! { { ret; ::aelys_native::value_null() } },
        _ => {
            return Err(syn::Error::new(
                ty.span(),
                format!(
                    "unsupported return type: {}. Supported: i64, f64, bool, ()",
                    ty_str
                ),
            ));
        }
    };

    Ok(conversion)
}
