// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]
#![allow(
    // Numeric casts: pervasive in the interpreter's hot paths; every
    // stack push/pop goes through an i32/u16/usize conversion and
    // annotating each one kills readability without adding safety.
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    // Style preferences we deliberately keep:
    clippy::match_same_arms,
    clippy::used_underscore_binding,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::implicit_hasher,
    clippy::let_underscore_untyped,
    clippy::must_use_candidate,
    clippy::manual_let_else,
    clippy::too_many_lines,
    clippy::type_complexity,
    // Re-emerges in src/fill.rs every time regen_fill_rs runs; the
    // template format is easier to keep stable than a generator fix.
    clippy::semicolon_if_nothing_returned
)]

// HTML export: when loft's own lib is compiled for
// `wasm32-unknown-unknown` without the full `wasm` feature (the target
// used by `loft --html`), the `print` opcode's `#rust` template calls
// `loft_host_print` — a function the browser host is expected to
// provide via the `loft_io` WASM import module.  Declare it here so
// `src/fill.rs` (auto-generated) can reference it unqualified.
// `not(target_os = "wasi")` keeps this browser-only: wasip2 has working
// std stdout, so its `print` branch uses `print!` (mirrors the @P334 FS
// fix in `src/state/io.rs`); native and the full `wasm` feature each take
// their own branch — so this import is declared only where it is used.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
#[link(wasm_import_module = "loft_io")]
unsafe extern "C" {
    pub(crate) safe fn loft_host_print(ptr: *const u8, len: usize);
    // Input mirror of `loft_host_print`: `host_input()` pulls the bytes the JS
    // host set before `loft_start`.  Two calls because loft must size the buffer
    // before the host fills it — `len` then `copy` (the pattern `web`'s
    // `ws_msg_len`/`ws_msg_copy` bridge already proves).  Used by
    // `Stores::host_input_native`'s browser branch (`src/database/format.rs`).
    pub(crate) safe fn loft_host_input_len() -> usize;
    pub(crate) safe fn loft_host_input_copy(ptr: *mut u8);
    // Output mirror of `host_input` for STRUCTURED host messages (requests to
    // the JS shell — e.g. "fetch this URL"), distinct from user-facing
    // `loft_host_print`.  Used by `Stores::host_output_native`'s browser
    // branch; the page routes it to `globalThis.loftOutput(msg)`.
    pub(crate) safe fn loft_host_output(ptr: *const u8, len: usize);
    // Synchronous-from-loft binary HTTP GET (the browser arm of `net::fetch_bytes`,
    // used by `store_load_url*`).  A browser `fetch()` is async, but this import
    // is in the wasm-opt `--asyncify` allowlist (`asyncify-imports@…loft_io.loft_host_http_get`),
    // so calling it UNWINDS the whole wasm stack back to the JS event loop; the
    // shell's `AsyncifyCtrl` runs `await fetch(url).arrayBuffer()`, then REWINDS
    // with the result — so `store_load_url_trusted(r,url) -> boolean` keeps its
    // synchronous signature and never blocks the page (the same async→sync bridge
    // `loft_web.ws_yield` proves).  Returns the byte length, or `usize::MAX` on a
    // network / non-2xx error.  Then `loft_host_http_get_copy` fills the buffer —
    // the `len`-then-`copy` shape `loft_host_input_len`/`copy` already proves.
    pub(crate) safe fn loft_host_http_get(url_ptr: *const u8, url_len: usize) -> usize;
    pub(crate) safe fn loft_host_http_get_copy(ptr: *mut u8);
    // @PLN105 Phase 2 — `deliver(tag, value)`: hand the JS host a self-describing handle to a
    // live loft value with no serialization.  `store_base` is the value's Store buffer base in
    // wasm linear memory (`Store.ptr`); `(rec, pos)` is its `DbRef` within that store.  The reader
    // addresses ANY node as `store_base + rec*8 + pos` (`Store::checked_offset`), so it can FOLLOW
    // child records (a vector field holds a child record-index; strings are interned records at
    // `id*8+8`) — which a pre-computed root address alone could not do.  `desc_ptr/desc_len` is
    // the layout descriptor as JSON (`LayoutDesc::to_json`, memoized host-side by `type_id`).
    // SYNCHRONOUS — unlike `loft_host_http_get` it is NOT in the asyncify allowlist; the borrow
    // ends when this returns, so JS must finish reading (or copy out) before then (§5 borrow).
    pub(crate) safe fn loft_host_deliver(
        tag: i64,
        store_base: usize,
        rec: u32,
        pos: u32,
        type_id: u32,
        desc_ptr: *const u8,
        desc_len: usize,
    );
    // @PLN105 Phase 3 — `expose`: a LONG-LIVED `deliver` (same handle), stashed by `tag` and
    // re-read across frames; the value's store is pinned loft-side so the addresses stay stable.
    // `loft_host_release(tag)` tells the host to drop the stash when loft unpins.
    pub(crate) safe fn loft_host_expose(
        tag: i64,
        store_base: usize,
        rec: u32,
        pos: u32,
        type_id: u32,
        desc_ptr: *const u8,
        desc_len: usize,
    );
    pub(crate) safe fn loft_host_release(tag: i64);
}

#[macro_use]
pub mod diagnostics;
pub mod api_diff;
pub mod api_surface;
pub mod base64;
pub mod build_phase;
pub mod cache;
mod calc;
pub mod crash_report;
pub mod data;
pub mod data_store;
pub mod database;
pub mod debugger;
pub mod diagnostic_render;
pub mod ffi_deliver;
pub mod hash;
pub mod ir_node;
pub mod ir_read;
pub mod ir_schema;
pub mod ir_schema_gen;
pub mod ir_store;
pub mod json;
pub mod keys;
mod lexer;
pub mod lsp;
pub mod native;
// `net::fetch_bytes` (behind `store_load_url*`) exists exactly where `load_url`
// does: a native `registry` build, or the browser (`--html`) target.
pub mod host;
#[cfg(any(
    feature = "registry",
    all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
))]
pub(crate) mod net;
pub mod ownership_cfg;
#[cfg(feature = "remote-store")]
pub mod paged_reader;
pub mod resolution;
pub mod scopes;
pub mod use_analysis;
mod variables;
pub mod vector;

pub mod trace;

pub mod codegen_runtime;
pub mod generation;
pub mod ops;
pub mod parser;
#[cfg(feature = "png")]
mod png_store;
mod radix_db;
mod radix_tree;
mod spatial;
pub mod store;
pub mod tree;
mod typedef;

pub mod const_eval;
pub mod coroutine_layout;
pub mod create;
pub mod fill;
pub mod parallel;
pub mod platform;
pub mod state;

pub mod compile;
pub mod engine_host;
pub mod extensions;
pub mod live_dispatch;
#[cfg(not(target_arch = "wasm32"))]
pub mod live_reload;
pub mod repl;
pub mod rpc;
pub mod script;
pub mod serve;
pub mod startup_cache;
pub mod wasm_debug;
// @PLAN12 phase 3.5a (2026-05-24) — re-export `extensions::native_call`
// at the crate root so generated native code can write
// `use loft::native_call;` without coupling to the extensions module.
#[cfg(feature = "native-extensions")]
pub use extensions::native_call;
// @PLN53 F1/F2 — raw-source fuzz oracle + keyed-container generator; available
// under cargo-fuzz (the `fuzzing` feature) and under `cargo test`.
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_keyed;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_oracle;
#[cfg(feature = "registry")]
pub mod install;
pub mod introspect;
pub mod libscan;
pub mod lockfile;
pub mod log_config;
pub mod logger;
pub mod manifest;
pub mod native_gate;
pub mod native_lib;
#[cfg(feature = "registry")]
pub mod package;
pub mod registry;
#[cfg(feature = "registry")]
pub mod registry_advisories;
#[cfg(feature = "registry")]
pub mod registry_index;
pub mod registry_keys;
#[cfg(feature = "registry")]
pub mod registry_signing;
pub mod runtime_error;
/// @PLN86 — sandbox policy model (capability-group allow-lists + the loft.toml parser).
pub mod sandbox;
/// @PLN97 Phase D — the schema-description sidecar (a store's self-describing layout identity).
pub mod schema_sidecar;
mod stack;
pub mod timeout;
pub mod triggers;

pub mod documentation;
pub mod migrate_long;
pub mod stdlib_sources;

#[cfg(feature = "wasm")]
pub mod wasm;
pub mod wasm_assets;
pub mod wasm_gl;
