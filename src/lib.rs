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
}

#[macro_use]
pub mod diagnostics;
pub mod base64;
pub mod cache;
mod calc;
pub mod crash_report;
pub mod data;
pub mod data_store;
pub mod database;
pub mod debugger;
pub mod hash;
pub mod ir_node;
pub mod ir_read;
pub mod ir_schema;
pub mod ir_schema_gen;
pub mod ir_store;
pub mod json;
pub mod keys;
mod lexer;
pub mod native;
pub mod scopes;
mod variables;
pub mod vector;

pub mod trace;

pub mod codegen_runtime;
pub mod generation;
pub mod ops;
pub mod parser;
#[cfg(feature = "png")]
mod png_store;
mod radix_tree;
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
pub mod serve;
pub mod startup_cache;
// @PLAN12 phase 3.5a (2026-05-24) — re-export `extensions::native_call`
// at the crate root so generated native code can write
// `use loft::native_call;` without coupling to the extensions module.
#[cfg(feature = "native-extensions")]
pub use extensions::native_call;
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
/// @PLN86 — sandbox policy model (capability-group allow-lists + the loft.toml parser).
pub mod sandbox;
#[cfg(feature = "registry")]
pub mod registry_index;
pub mod registry_keys;
#[cfg(feature = "registry")]
pub mod registry_signing;
pub mod runtime_error;
mod stack;
pub mod timeout;
pub mod triggers;

pub mod documentation;
pub mod formatter;
pub mod migrate_long;
pub mod stdlib_sources;

#[cfg(feature = "wasm")]
pub mod wasm;
pub mod wasm_assets;
pub mod wasm_gl;
