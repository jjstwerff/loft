// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1109 — a tuple RETURN copies its heap member ONCE.
//!
//! A tuple literal copies a heap local it is given, so the element cannot become a second
//! name for the local's store (loft#1102, `@FR-T-Cons`). A tuple written as a function's
//! RETURN tail is then rewritten to a synthetic `__tuple<…>` record, and filling that
//! record's vector field COPIES into the record's own storage — so the return path had two
//! copies where one is enough.
//!
//! This is a shape assertion rather than a timing: the cost is one `OpAppendVector` per
//! call, and a wall clock cannot say which of the two copies it measured. The cases are
//! paired so the elision cannot be widened by accident — the local-bound tuple keeps the
//! copy that closed the aliasing, and an ARGUMENT member (never wrapped, because a
//! parameter reaches the caller's store) is unchanged.

extern crate loft;

use loft::data::{Data, Value};
use loft::parser::Parser;

fn parse(script: &str) -> Data {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "issue_1109", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    p.data
}

/// How many times `fn_name`'s body copies a vector's ELEMENTS into another vector.
fn append_ops(data: &Data, fn_name: &str) -> usize {
    let d_nr = data.def_nr(fn_name);
    assert!(d_nr != u32::MAX, "`{fn_name}` is not defined");
    let append = data.def_nr("OpAppendVector");
    let mut n = 0;
    data.def(d_nr).code.walk(&mut |v: &Value| {
        if let Value::Call(d, _) = v.unspan()
            && *d == append
        {
            n += 1;
        }
    });
    n
}

/// The reported shape: the member is copied into the record and nowhere else.
#[test]
fn a_tuple_return_copies_its_heap_member_once() {
    let data = parse(
        "fn mk1109() -> (vector<integer>, integer) { vl: vector<integer> = [10, 20]; (vl, 9) }
         fn main() { t = mk1109(); print(\"{len(t.0)}\"); }",
    );
    assert_eq!(
        append_ops(&data, "n_mk1109"),
        1,
        "a tuple RETURN must copy its heap member exactly once — the synthetic \
         `__tuple` record's own copy; the literal's frame-local backing is redundant \
         with it and is unwrapped"
    );
}

/// The half that must NOT move: a tuple bound to a LOCAL has no second copy to fall back
/// on, so dropping the literal's copy there is exactly the aliasing loft#1102 closed.
#[test]
fn a_tuple_bound_to_a_local_still_copies_its_heap_member() {
    let data = parse(
        "fn use1109() -> integer { vl: vector<integer> = [10, 20]; t = (vl, 9); len(t.0) }
         fn main() { print(\"{use1109()}\"); }",
    );
    assert!(
        append_ops(&data, "n_use1109") >= 1,
        "a tuple LITERAL bound to a local must still copy the heap local it is given, \
         or the element and the local become two names for one store (loft#1102)"
    );
}

/// A member that is a PARAMETER is never wrapped in an owned backing — a parameter reaches
/// the caller's store and copying it would change what the caller sees (`@FR-B-Ref-Alias`).
/// The record's own copy is the only one, before and after.
#[test]
fn a_tuple_return_whose_member_is_an_argument_is_unchanged() {
    let data = parse(
        "fn arg1109(v: vector<integer>) -> (vector<integer>, integer) { (v, 9) }
         fn main() { w: vector<integer> = [1, 2]; t = arg1109(w); print(\"{len(t.0)}\"); }",
    );
    assert_eq!(
        append_ops(&data, "n_arg1109"),
        1,
        "an argument member is copied once, by the record"
    );
}
