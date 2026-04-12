// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::match_same_arms,
    clippy::collapsible_if,
    clippy::redundant_closure,
    clippy::used_underscore_binding,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::single_match_else,
    clippy::if_not_else,
    clippy::implicit_hasher,
    clippy::unnecessary_wraps,
    clippy::semicolon_if_nothing_returned,
    clippy::uninlined_format_args,
    clippy::let_underscore_untyped,
    clippy::must_use_candidate,
    clippy::option_if_let_else,
    clippy::manual_let_else,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::map_unwrap_or
)]

// W1.1 HTML export: when loft's own lib is compiled for
// `wasm32-unknown-unknown` without the full `wasm` feature (the target
// used by `loft --html`), the `print` opcode's `#rust` template calls
// `loft_host_print` — a function the browser host is expected to
// provide via the `loft_io` WASM import module.  Declare it here so
// `src/fill.rs` (auto-generated) can reference it unqualified.  This
// cfg is deliberately narrow so native builds, wasm32-wasip2 builds,
// and the full-featured `wasm` feature all see their own branch of
// the template.
#[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
#[link(wasm_import_module = "loft_io")]
unsafe extern "C" {
    pub(crate) safe fn loft_host_print(ptr: *const u8, len: usize);
}

#[macro_use]
pub mod diagnostics;
pub mod base64;
mod calc;
pub mod data;
pub mod database;
pub mod hash;
pub mod keys;
mod lexer;
pub mod native;
pub mod scopes;
pub mod sha256;
mod variables;
pub mod vector;

pub mod codegen_runtime;
pub mod generation;
pub mod ops;
pub mod parser;
#[cfg(feature = "png")]
mod png_store;
mod radix_tree;
mod store;
pub mod tree;
mod typedef;

pub mod const_eval;
pub mod create;
pub mod fill;
pub mod parallel;
pub mod platform;
pub mod state;

pub mod cache;
pub mod compile;
pub mod extensions;
pub mod log_config;
pub mod logger;
pub mod manifest;
pub mod registry;
mod stack;

pub mod documentation;
pub mod formatter;

#[cfg(feature = "wasm")]
pub mod wasm;
pub mod wasm_gl;
