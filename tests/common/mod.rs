// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shared helpers for integration test binaries.

use loft::data::Data;
use loft::database::Stores;
use loft::parser::Parser;
use std::sync::OnceLock;

static DEFAULT_PARSED: OnceLock<(Data, Stores)> = OnceLock::new();

/// Parse the default library once per test binary and cache the result.
/// Each test clones the schema cheaply instead of re-parsing three files.
pub fn cached_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_PARSED.get_or_init(|| {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}
