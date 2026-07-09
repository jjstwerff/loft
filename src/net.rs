// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Target-uniform HTTP GET behind the `store_load_url*` stdlib functions.
//!
//! The loft-visible API is identical on every target —
//! `store_load_url_trusted(r, url) -> boolean` is a plain synchronous call — but
//! *how* the bytes are fetched differs, and that difference is confined here:
//!
//! - **native** (`registry` feature): `ureq` (via [`crate::registry_index`]),
//!   including the `file://` offline-mirror path.
//! - **browser** (`--html`, `wasm32-unknown-unknown`): the JS `fetch()` is async,
//!   so a *synchronous* call would deadlock the single-threaded event loop.
//!   Instead this bridges to the `loft_io.loft_host_http_get` host import, which
//!   is in the wasm-opt `--asyncify` allowlist: calling it unwinds the whole wasm
//!   stack back to the event loop, the shell's `AsyncifyCtrl` runs
//!   `await fetch(url)`, then rewinds with the bytes — the same async→sync bridge
//!   `loft_web.ws_yield` proves. So the call returns synchronously and the page
//!   never freezes.
//! - **any other build** (native without `registry`): a clear "unavailable" error,
//!   never a silent wrong answer.

/// The browser-wasm target (`--html`): `wasm32-unknown-unknown`, not WASI, and not
/// the in-process `wasm` host feature. HTTP there goes through the asyncify host
/// import; every other target uses a native client or errors.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    // `loft_host_http_get` suspends via asyncify while JS fetches; it returns the
    // response byte length, or `usize::MAX` on a network / non-2xx error.
    let n = crate::loft_host_http_get(url.as_ptr(), url.len());
    if n == usize::MAX {
        return Err(format!("browser fetch failed for {url}"));
    }
    let mut buf = vec![0u8; n];
    crate::loft_host_http_get_copy(buf.as_mut_ptr());
    Ok(buf)
}

/// Native path: delegate to the `ureq`-backed client (and `file://`) that the
/// registry install already uses.  Reached only on a non-browser build with the
/// `registry` feature — the sole other case where the module compiles (below).
#[cfg(all(
    feature = "registry",
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    crate::registry_index::http_get_bytes(url)
}
