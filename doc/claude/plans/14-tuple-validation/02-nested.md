<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Nested tuples (E3 × D1, D2)

**Status: open**

## Goal

Verify that tuples whose elements are themselves tuples — `((A, B),
C)`, `(A, (B, C))`, `((A, B), (C, D))` — round-trip correctly through
local-variable storage (D1) and direct-stack destinations (D2), with
**interp/native byte-identical stdout** asserted by the
[phase-00 harness](00-matrix.md#cross-mode-harness).

After phase 02: `TupleGet(TupleGet(v, i), j)` chains and the
corresponding `TuplePut` write paths are verified at all reachable
nesting depths.  T1.9 already produced nested tuple lowering for
`match`; this phase makes sure the same nesting works outside `match`
and that codegen lays out the nested record correctly under both
backends.

## Cells closed

| Cell | Test name | Source shape |
|---|---|---|
| E3×D1 two-deep | `e3_d1_nested_local` | `t = ((1, 2), 3)` |
| E3×D1 deep | `e3_d1_nested_deep` | `t = ((1, 2), (3, 4))` |
| E3×D2 arg | `e3_d2_nested_arg` | `f(((1, 2), 3))` |
| E3×D2 return | `e3_d2_nested_return` | `#[ignore = "T1.8a"]` |
| E3 mixed text inside | `e3_d1_text_inside` | `((1, "a"), (2, "b"))` |
| E3 element-of-element assign | `e3_d1_elem_elem_assign` | `t.0.1 = 99` |

## Snippets (illustrative)

```loft
// e3_d1_nested_local
t = ((1, 2), 3);
print("{t.0.0},{t.0.1},{t.1}\n");
assert(t.0.0 == 1 && t.0.1 == 2 && t.1 == 3, "two-deep");
```

```loft
// e3_d1_elem_elem_assign
t = ((1, 2), (3, 4));
t.0.1 = 99;
t.1.0 = 77;
print("{t.0.0},{t.0.1},{t.1.0},{t.1.1}\n");
assert(t.0.1 == 99 && t.1.0 == 77, "nested element write");
```

```loft
// e3_d2_nested_arg
fn show(p: ((integer, integer), integer)) {
    print("{p.0.0},{p.0.1},{p.1}\n");
    assert(p.0.0 == 1 && p.0.1 == 2 && p.1 == 3, "nested arg");
}
show(((1, 2), 3));
```

## Pre-flight

```bash
# Confirm `t.0.0` parses today (it must — T1.9 emits it).  Confirm
# native codegen handles nested TupleGet without an extra temp:
LOFT_LOG=static cargo test --release --test tuple_matrix \
    e3_d1_nested_local -- --nocapture 2>&1 | head -30
```

If native diverges (silent zero, wrong offset), the fix lives in
codegen for `Value::TupleGet` chains — verify `element_offsets` adds
correctly through the inner-tuple stride.

## Acceptance

- 5 unignored cells run green; 1 ignored cell (`e3_d2_nested_return`)
  carries the T1.8a tag.
- Matrix updated.
- `make ci` green.

## Cross-references

- [00-matrix.md](00-matrix.md) — harness
- T1.9 nested lowering in `src/parser/control.rs` (already proven for
  `match`)
- `src/data.rs::Type::Tuple::element_offsets`
