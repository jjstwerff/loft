// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal reproduction of store leak in block-copy optimisation (O-B2).

extern crate loft;

mod testing;

use loft::compile::byte_code;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
#[path = "common/mod.rs"]
mod common;
use common::cached_default;

/// Nested struct return leaks inner stores: `make_pt` allocates a store
/// for the returned Pt, but when the call is used as a field initializer
/// in a Box constructor, the inner store is never freed.
#[test]
fn nested_struct_return_leak() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(
        r#"
struct Pt { x: float not null, y: float not null }
struct Rect { pos: Pt, size: Pt }

fn make_pt(a: float, b: float) -> Pt {
  Pt { x: a, y: b }
}

fn make_rect(rx: float, ry: float, rw: float, rh: float) -> Rect {
  Rect { pos: make_pt(rx, ry), size: make_pt(rw, rh) }
}

pub fn test() {
  r = make_rect(1.0, 2.0, 10.0, 20.0);
  assert(r.pos.x == 1.0, "x");
}
"#,
        "leak_test",
        false,
    );
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    state.execute("test", &p.data);
    state.check_store_leaks();
}
