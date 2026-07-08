// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN101 Slice 5 — the ZERO-COST proof for `value struct`.
//!
//! A value struct is stored INLINE (a struct field sizes to the nested record's full bytes; a
//! `vector<V>` inlines its elements into one backing), and reading a value struct out read-only is
//! left as a zero-cost view by the copy-elision pass (`scopes::value_struct_copy`) — so using a
//! value struct as a record field / vector element adds NO heap allocation over the raw inline
//! layout. These tests pin the true heap metric `Stores::stores_allocated` (alloc/free CYCLES, not
//! logical records): it must stay CONSTANT as the element count N grows (O(1), no per-element cost),
//! and must equal the reference-struct baseline (zero abstraction penalty). A regression that
//! re-introduces a per-element copy (the pre-elision behaviour was ~N+5 allocs) fails here loudly.

use loft::compile::byte_code;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

mod common;
use common::cached_default;

/// Parse + execute `fn test()` and return the true heap metric (`Stores::stores_allocated`) — the
/// count of store slots that went live, i.e. the per-construction abstraction cost.
fn allocs_for(code: &str) -> u64 {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "value_struct_alloc_test", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    state.database.stores_allocated
}

/// A `vector<V>` of value structs, built then summed read-only, at element count `n`.
fn value_vec_prog(n: usize) -> String {
    format!(
        r#"
value struct Pt {{ x: integer, y: integer }}
pub fn test() {{
  ps = [for i in 0..{n} {{ Pt {{ x: i, y: i * 2 }} }}];
  s = 0;
  for p in ps {{ s = s + p.x; }}
  assert(s >= 0, "s={{s}}");
}}
"#
    )
}

/// The same shape with a plain reference `struct` — the zero-cost baseline (loft already inlines
/// reference-struct vector elements and binds the read-only loop var as a view).
fn ref_vec_prog(n: usize) -> String {
    format!(
        r#"
struct Pt {{ x: integer, y: integer }}
pub fn test() {{
  ps = [for i in 0..{n} {{ Pt {{ x: i, y: i * 2 }} }}];
  s = 0;
  for p in ps {{ s = s + p.x; }}
  assert(s >= 0, "s={{s}}");
}}
"#
    )
}

/// A value struct used as a FIELD of a record ("zero cost inside records"), iterated read-only.
fn value_field_prog(n: usize) -> String {
    format!(
        r#"
value struct Pt {{ x: integer, y: integer }}
struct Row {{ p: Pt, tag: integer }}
struct Table {{ rows: vector<Row> }}
pub fn test() {{
  t = Table {{ rows: [for i in 0..{n} {{ Row {{ p: Pt {{ x: i, y: i * 2 }}, tag: i }} }}] }};
  s = 0;
  for r in t.rows {{ s = s + r.p.x; }}
  assert(s >= 0, "s={{s}}");
}}
"#
    )
}

/// The headline zero-cost guarantee: allocations for a `vector<value struct>` are CONSTANT in the
/// element count — no per-element store. (Pre-elision this was N + a small constant.)
#[test]
fn value_struct_vector_allocs_are_flat_in_n() {
    let a100 = allocs_for(&value_vec_prog(100));
    let a1000 = allocs_for(&value_vec_prog(1000));
    assert_eq!(
        a100, a1000,
        "value-struct vector allocs must be O(1): N=100 gave {a100}, N=1000 gave {a1000}"
    );
    // A generous absolute ceiling well below O(N): a per-element copy would be ~1005 at N=1000.
    assert!(
        a1000 <= 16,
        "value-struct vector allocs unexpectedly large ({a1000}) — per-element copy regressed?"
    );
}

/// Zero abstraction PENALTY: grouping scalars into a value struct costs the same as a reference
/// struct (which loft already inlines + views for free).
#[test]
fn value_struct_vector_matches_reference_struct() {
    let value = allocs_for(&value_vec_prog(1000));
    let reference = allocs_for(&ref_vec_prog(1000));
    assert_eq!(
        value, reference,
        "value struct ({value}) must allocate the same as an equivalent reference struct ({reference})"
    );
}

/// A value struct as a record FIELD adds no per-element allocation either.
#[test]
fn value_struct_field_allocs_are_flat_in_n() {
    let a100 = allocs_for(&value_field_prog(100));
    let a1000 = allocs_for(&value_field_prog(1000));
    assert_eq!(
        a100, a1000,
        "value-struct field allocs must be O(1): N=100 gave {a100}, N=1000 gave {a1000}"
    );
    assert!(
        a1000 <= 16,
        "value-struct field allocs unexpectedly large ({a1000})"
    );
}
