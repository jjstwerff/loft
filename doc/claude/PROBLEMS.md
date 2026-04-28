
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

**Before opening a new issue here, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** — the closed-by-decision
register holds items explicitly evaluated and declined (C3 / C38 /
C54.D / …).  If your symptom maps onto one of those, the fix is to
produce new evidence (reproducer, incident, measurement) on the
existing entry, not re-open it as a bug.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 194 | Tuple-field reassignment `p.v = (1, 2)` parses as tuple destructuring (LHS `p.v` not recognised as a tuple-typed field expression).  Initial construction `Pair { v: (1, 2) }` works; only `p.v = (...)` is rejected. | Medium | Reassign element-by-element via existing field updates, or rebuild the host struct (`p = Pair { v: (1, 2) }`). |
| 195 | Chained literal field indexing `n.v.0.0` mis-parses — the lexer reads `0.0` as a single float literal.  Affects any nested-tuple access where two consecutive integer indices appear without an intervening identifier. | Low | Stash the inner element first: `inner = n.v.0; inner.0`. |
| 196 | Native codegen for `(fn(int) -> int, int)` (or any tuple containing a fn-ref) fails with `(u32, DbRef).0 as i32` — the fn-ref tuple element's runtime shape doesn't fit the OpSet/OpGet narrowing path used for primitive ints.  Interpreter mode works; only native compilation breaks. | Medium | Use a struct field for fn-ref instead of tucking it in a tuple: `struct H { f: fn(...) -> ..., n: int }`. |

## Interpreter Robustness

### 194. Tuple-field reassignment parses as destructuring

**Symptom:** updating a tuple struct field after construction fails
to parse:

```loft
struct Pair { v: (integer, integer) }
fn test() {
  p = Pair { v: (3, 4) };        // OK
  p.v = (100, 200);              // ← "Tuple destructuring requires
                                 //   plain variable names"
}
```

**Where:** the parser's tuple-LHS handler (in `parser/expressions.rs`,
the `(a, b) = expr` destructuring path) fires whenever the left-hand
side is a parenthesised list, regardless of whether the LHS is
`(name, name)` (destructuring) or `(field_path, field_path)`
(impossible) or — the case here — a single field whose RHS is a
tuple literal.  The disambiguation key (LHS *value* is a `Value::Var`
of tuple type vs. a parenthesised name list) isn't checked.

**Fix path:** when the LHS parses as a single field-access expression
and the RHS parses as a tuple literal, route through `set_field_check`
with the `Type::Tuple` arm (which already exists and handles writes).
Concretely: in the assignment parser, check if `lhs` is a
`Value::Call(OpGetField, …)` or `Value::TupleGet(...)` of tuple type
before falling into the destructuring branch.

**Test:** add a test once fixed:

```loft
struct Pair { v: (integer, integer) }
fn test() {
  p = Pair { v: (3, 4) };
  p.v = (100, 200);
  assert(p.v.0 == 100);
}
```

### 195. Chained literal indexing — lexer reads `0.0` as float

**Symptom:** `n.v.0.0` (read element 0 of inner tuple of element 0)
fails to parse:

```loft
struct Nested { v: ((integer, integer), (integer, integer)) }
fn test() {
  n = Nested { v: ((1, 2), (3, 4)) };
  a = n.v.0.0;                    // ← parse error: float `0.0`
                                  //   followed by stray `.` ?
}
```

**Where:** `src/lexer.rs` greedy-reads `<digit>.<digit>` as a single
floating-point number token.  The post-field path doesn't unread the
fractional part when the previous token was a `.` separator.

**Fix path:** in the lexer's number-reading routine, when the previous
non-trivia token was `.` (or, more conservatively, when a tuple-index
context is pending), treat `<digit>` followed by `.` as a single-digit
integer rather than the start of a float.  The cleanest mechanism is
an "integer-only" lexing mode the parser opts into when it knows it's
expecting a tuple index.

**Workaround:** stash the inner element first:

```loft
inner = n.v.0;
a = inner.0;     // OK
```

**Test:** add a test once fixed.

### 196. Tuple-of-fn-ref native codegen — `(u32, DbRef).0 as i32`

**Symptom:** native compilation fails with `non-primitive cast` when
a tuple struct field contains a fn-ref element:

```loft
struct C { pair: (fn(integer) -> integer, integer) }
fn dbl(x: integer) -> integer { x + x }
fn test() {
  c = C { pair: (dbl, 21) };       // interpreter: OK
                                   // native: rustc rejects
                                   //   (var_tmp.0) as i32
}
```

**Where:** my `set_field_check::Type::Tuple` arm dispatches the
fn-ref element to `OpSetInt4(ref, pos, TupleGet(tmp, i))`.  At native
emit, `TupleGet(tmp, i)` for a fn-ref element resolves to
`var_tmp.0` whose Rust type is `(u32, DbRef)` (the native fn-ref
representation).  `OpSetInt4`'s `#rust` body wraps the value with
`as i32`, which rustc rejects on a tuple type.

**Fix path:** in `set_field_check::Type::Tuple`, when the element is
`Type::Function`, extract the fn-ref's `d_nr` (`var_tmp.i.0 as u32`,
then `as i32` is fine on `u32`) before passing to `OpSetInt4`.  The
top-level `Type::Function` set arm already handles `Value::FnRef` ↔
`Value::Int(d_nr)` reduction; extend that to also unwrap a Var-of-
fn-ref when the LHS is a fn-ref tuple element.  Alternatively, emit
a small helper like `OpGetFnRefDnr(var, idx)` returning `i32`.

**Workaround:** lift the fn-ref out of the tuple into its own
struct field:

```loft
struct C { f: fn(integer) -> integer, n: integer }
```

**Test:** `tests/issues.rs::p4d_fn_ref_as_struct_field` covers the
top-level case.  Add `p4d_tuple_field_with_fn_ref` once fixed.

## Web Services

*(none)*

## Graphics / WebGL

*(none)*

## Package / Multi-file

*(none)*

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
