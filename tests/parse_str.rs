// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Regression: `parse_str` sources must survive a `use` clause.  Resolving a
//! `use` HALTS the current file and later RESUMES it by switching back to its
//! NAME — which, for a virtual source (`<probe>`, the REPL's snippet names,
//! live-reload's `"<live-reload>"`), is not an openable path on any platform.
//! The lexer now re-serves registered in-memory sources on `switch`.
//! (Windows-probe-caught as "Fatal: Unknown file:<win-probe>"; the boundary
//! matrix showed it was never Windows-specific — Linux failed identically.)

use loft::compile;
use loft::parser::Parser;
use loft::state::State;

#[test]
fn parse_str_survives_a_use_clause() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.lib_dirs = vec!["lib".to_string()];
    p.parse_str(
        "use engine_host;\nfn main() { println(\"host {engine_host::default_host()}\"); }\n",
        "<parse-str-regression>",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "a virtual source with a use clause must parse: {:?}",
        p.diagnostics.lines()
    );
    // And it runs: the resumed second half of the virtual file (main) exists.
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut data = p.data;
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut data);
    state.execute_argv("main", &data, &[]);
    assert!(
        state.database.runtime_error.is_none(),
        "the resumed main must run: {:?}",
        state.database.runtime_error
    );
}
