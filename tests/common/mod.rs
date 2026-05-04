// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shared helpers for integration test binaries.
//!
//! Each item is `#[allow(dead_code)]` because this module is pulled
//! into multiple test binaries via `mod common;`, and not every
//! binary uses every helper.  Without the allow, binaries that don't
//! consume a given helper produce a warning that turns into a CI
//! failure under `-D warnings`.

#[allow(dead_code)]
pub mod cross_mode;

use loft::data::Data;
use loft::database::Stores;
use loft::parser::Parser;
use std::sync::OnceLock;

#[allow(dead_code)]
static DEFAULT_PARSED: OnceLock<(Data, Stores)> = OnceLock::new();

/// Parse the default library once per test binary and cache the result.
/// Each test clones the schema cheaply instead of re-parsing three files.
#[allow(dead_code)]
pub fn cached_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_PARSED.get_or_init(|| {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}
