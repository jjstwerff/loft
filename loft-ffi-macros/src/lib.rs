// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `#[loft_native]` — generates the uniform marshal bridge for a loft native
//! extension function (plan-25 FFI generated-dispatch, F2).
//!
//! Applied to a `#[no_mangle] pub extern "C" fn n_foo(...)` impl, it keeps the
//! function unchanged and additionally emits a sibling
//! `n_foo__loft_bridge` of the fixed [`loft_ffi::LoftBridgeFn`] shape
//! (`fn(LoftStore, *const LoftValue, n, *mut LoftValue)`).  The bridge decodes
//! each `LoftValue` arg into the impl's REAL typed parameter (read from this
//! fn's actual Rust signature — not the width-ambiguous loft decl, per
//! @P370), calls the real fn, and tags the return into `*ret`.  The
//! interpreter (plan-25 F4) then calls only this one shape, deleting the
//! ~98-arm `dispatch_call` in `src/extensions.rs`.
//!
//! ## Parameter conventions (mirrors the legacy `dispatch_call`)
//! - first param `LoftStore`     → the bridge's `store` (not a loft arg)
//! - `*const u8` then `usize`    → one loft `text` arg (`LoftStr` → ptr, len)
//! - `LoftRef`                   → one loft ref/vector arg
//! - `bool` / `f64` / `f32`      → one loft arg
//! - any integer (`i32`/`u16`/…) → one loft arg, cast `as <type>`
//!
//! ## Return conventions
//! - `()` → `Void`; `i32` → int via [`loft_ffi::widen_i32`] (null sentinel);
//!   other ints → int; `bool`/`f64`/`f32`/`LoftStr`/`LoftRef` → tagged value.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, ReturnType, Type, parse_macro_input};

/// The last path segment's identifier as a string, e.g. `LoftRef`, `i32`,
/// `loft_ffi::LoftStore` → `LoftStore`.
fn path_ident(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// `*const u8` (any const-ness/elem `u8`) — the first half of a text param.
fn is_ptr_const_u8(ty: &Type) -> bool {
    matches!(ty, Type::Ptr(p) if path_ident(&p.elem).as_deref() == Some("u8"))
}

fn is_int_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "isize" | "usize"
    )
}

fn err(spanned: &dyn quote::ToTokens, msg: &str) -> TokenStream {
    syn::Error::new_spanned(spanned, msg)
        .to_compile_error()
        .into()
}

#[proc_macro_attribute]
pub fn loft_native(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);

    if !func.sig.generics.params.is_empty() {
        return err(
            &func.sig,
            "#[loft_native]: generic native functions are not supported",
        );
    }

    let fn_name = func.sig.ident.clone();
    let bridge_name = format_ident!("{}__loft_bridge", fn_name);

    // Collect typed params (extern "C" fns have no `self`).
    let params: Vec<&syn::PatType> = func
        .sig
        .inputs
        .iter()
        .filter_map(|a| match a {
            FnArg::Typed(pt) => Some(pt),
            FnArg::Receiver(_) => None,
        })
        .collect();

    // Walk params, mapping each to a call-argument expression (and a decode
    // statement for text).  `loft_idx` is the running index into the args[].
    let mut decode = Vec::new();
    let mut call_args = Vec::new();
    let mut loft_idx: usize = 0;
    let mut i = 0;
    while i < params.len() {
        let ty = &*params[i].ty;
        let name = path_ident(ty);

        // LoftStore as the FIRST param is supplied by the bridge, not a loft arg.
        if i == 0 && name.as_deref() == Some("LoftStore") {
            call_args.push(quote!(store));
            i += 1;
            continue;
        }
        // text: `*const u8` immediately followed by `usize` → one loft arg.
        if is_ptr_const_u8(ty)
            && i + 1 < params.len()
            && path_ident(&params[i + 1].ty).as_deref() == Some("usize")
        {
            let li = loft_idx;
            loft_idx += 1;
            let tvar = format_ident!("__t{}", i);
            decode.push(quote!(let #tvar = args[#li].as_text();));
            call_args.push(quote!(#tvar.ptr));
            call_args.push(quote!(#tvar.len));
            i += 2;
            continue;
        }
        match name.as_deref() {
            Some("LoftRef") => {
                let li = loft_idx;
                loft_idx += 1;
                call_args.push(quote!(args[#li].as_ref()));
            }
            Some("bool") => {
                let li = loft_idx;
                loft_idx += 1;
                call_args.push(quote!(args[#li].as_bool()));
            }
            Some("f64") => {
                let li = loft_idx;
                loft_idx += 1;
                call_args.push(quote!(args[#li].as_f64()));
            }
            Some("f32") => {
                let li = loft_idx;
                loft_idx += 1;
                call_args.push(quote!(args[#li].as_f64() as f32));
            }
            Some(n) if is_int_name(n) => {
                let li = loft_idx;
                loft_idx += 1;
                call_args.push(quote!(args[#li].as_i64() as #ty));
            }
            _ => {
                return err(
                    ty,
                    "#[loft_native]: unsupported parameter type (expected LoftStore (first), \
                     text as `*const u8, usize`, LoftRef, bool, f32/f64, or an integer)",
                );
            }
        }
        i += 1;
    }

    let call = quote!(#fn_name(#(#call_args),*));

    // Tag the return into *ret.
    let body_tail = match &func.sig.output {
        ReturnType::Default => quote!(#call; *ret = loft_ffi::LoftValue::VOID;),
        ReturnType::Type(_, ty) => {
            let ret = match path_ident(ty).as_deref() {
                Some("i32") => quote!(loft_ffi::LoftValue::int(loft_ffi::widen_i32(__r))),
                Some("i64") => quote!(loft_ffi::LoftValue::int(__r)),
                Some("bool") => quote!(loft_ffi::LoftValue::boolean(__r)),
                Some("f64") => quote!(loft_ffi::LoftValue::float(__r)),
                Some("f32") => quote!(loft_ffi::LoftValue::float(__r as f64)),
                Some("LoftStr") => quote!(loft_ffi::LoftValue::text(__r)),
                Some("LoftRef") => quote!(loft_ffi::LoftValue::reference(__r)),
                Some(n) if is_int_name(n) => quote!(loft_ffi::LoftValue::int(__r as i64)),
                _ => {
                    return err(
                        ty,
                        "#[loft_native]: unsupported return type (expected (), an integer, \
                         bool, f32/f64, LoftStr, or LoftRef)",
                    );
                }
            };
            quote!(let __r = #call; *ret = #ret;)
        }
    };

    quote! {
        #func

        #[doc(hidden)]
        #[unsafe(no_mangle)]
        #[allow(unused_variables)]
        pub unsafe extern "C" fn #bridge_name(
            store: loft_ffi::LoftStore,
            args: *const loft_ffi::LoftValue,
            n: usize,
            ret: *mut loft_ffi::LoftValue,
        ) {
            // SAFETY (plan-25 contract): the interpreter passes `n` initialised
            // LoftValues at `args` and one writable LoftValue at `ret`, both
            // valid for this call; the real fn may itself be `unsafe`.
            unsafe {
                // `from_raw_parts` forbids a null pointer even at len 0, so
                // guard the zero-arg case (a void/no-arg fn may be passed null).
                let args: &[loft_ffi::LoftValue] = if n == 0 {
                    &[]
                } else {
                    core::slice::from_raw_parts(args, n)
                };
                #(#decode)*
                #body_tail
            }
        }
    }
    .into()
}
