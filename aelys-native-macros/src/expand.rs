// codegen for aelys_module attribute

use crate::args::ModuleArgs;
use crate::export::generate_export_wrapper;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Item, ItemFn, ItemMod, spanned::Spanned};

pub fn expand_module(args: ModuleArgs, mut input: ItemMod) -> syn::Result<TokenStream2> {
    let module_name = &args.name;
    let module_version = args.version.as_deref().unwrap_or("0.0.0");

    let (brace, content) = input.content.take().ok_or_else(|| {
        syn::Error::new(
            input.span(),
            "#[aelys_module] requires an inline module (use `mod name { ... }`)",
        )
    })?;

    let mut exports = Vec::new();
    let mut new_content = Vec::new();
    let mut wrapper_functions = Vec::new();

    for item in content {
        if let Item::Fn(func) = &item {
            if has_aelys_export_attr(func) {
                let (wrapper, export_info) = generate_export_wrapper(func)?;
                wrapper_functions.push(wrapper);
                exports.push(export_info);

                let mut clean_func = func.clone();
                clean_func
                    .attrs
                    .retain(|attr| !attr.path().is_ident("aelys_export"));
                new_content.push(Item::Fn(clean_func));
                continue;
            }
        }
        new_content.push(item);
    }

    if exports.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "module has no #[aelys_export] functions",
        ));
    }

    let export_count = exports.len();
    let mut export_statics = Vec::new();
    let mut export_refs = Vec::new();

    for (i, export) in exports.iter().enumerate() {
        let static_name = format_ident!("__AELYS_EXPORT_{}", i);
        let name_static = format_ident!("__AELYS_EXPORT_NAME_{}", i);
        let export_name = &export.name;
        let export_name_bytes = format!("{}\0", export_name);
        let arity = export.arity;
        let wrapper_name = &export.wrapper_name;

        export_statics.push(quote! {
            static #name_static: &[u8] = #export_name_bytes.as_bytes();

            static #static_name: ::aelys_native::AelysExport = ::aelys_native::AelysExport {
                name: #name_static.as_ptr() as *const ::core::ffi::c_char,
                kind: ::aelys_native::AelysExportKind::Function,
                arity: #arity,
                _padding: [0; 3],
                value: #wrapper_name as *const ::core::ffi::c_void,
            };
        });

        export_refs.push(quote! { #static_name });
    }

    let module_name_bytes = format!("{}\0", module_name);
    let module_version_bytes = format!("{}\0", module_version);

    input.content = Some((brace, new_content));

    Ok(quote! {
        #input

        #(#wrapper_functions)*

        static __AELYS_MODULE_NAME: &[u8] = #module_name_bytes.as_bytes();
        static __AELYS_MODULE_VERSION: &[u8] = #module_version_bytes.as_bytes();

        #(#export_statics)*

        static __AELYS_EXPORTS: [::aelys_native::AelysExport; #export_count] = [
            #(#export_refs),*
        ];

        #[unsafe(no_mangle)]
        pub static mut aelys_module_descriptor: ::aelys_native::AelysModuleDescriptor = ::aelys_native::AelysModuleDescriptor {
            abi_version: ::aelys_native::AELYS_ABI_VERSION,
            descriptor_size: ::core::mem::size_of::<::aelys_native::AelysModuleDescriptor>() as u32,
            module_name: __AELYS_MODULE_NAME.as_ptr() as *const ::core::ffi::c_char,
            module_version: __AELYS_MODULE_VERSION.as_ptr() as *const ::core::ffi::c_char,
            vm_version_min: ::core::ptr::null(),
            vm_version_max: ::core::ptr::null(),
            descriptor_hash: 0,
            exports_hash: 0,
            export_count: #export_count as u32,
            exports: __AELYS_EXPORTS.as_ptr(),
            required_module_count: 0,
            required_modules: ::core::ptr::null(),
            init: None,
        };

        ::aelys_native::aelys_init_exports_hash!(aelys_module_descriptor);
    })
}

fn has_aelys_export_attr(func: &ItemFn) -> bool {
    func.attrs
        .iter()
        .any(|attr| attr.path().is_ident("aelys_export"))
}
