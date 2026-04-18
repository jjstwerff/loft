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
pub mod crash_report;
pub mod data;
pub mod database;
pub mod hash;
pub mod json;
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
pub mod migrate_long;

#[cfg(feature = "wasm")]
pub mod wasm;
pub mod wasm_gl;
