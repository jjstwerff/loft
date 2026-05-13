<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Basic + text scalars across D1 / D2

**Status: open**

## Goal

Close every E1 / E1n / E2 cell in the D1 (local var) and D2 (direct
stack) columns.  Every cell test runs under the
[`cross_mode!`](00-matrix.md#cross-mode-harness) harness from phase
00, so a green cell means **interp and native produce byte-identical
stdout** on the same source.

After phase 01: scalar and text tuples are independently verified
correct in both modes for variable storage, function-argument passing,
inline-expression construction, `match` subject construction, and
`if`/`else` arm construction.  Tuple-typed function returns (`-> (A,
B)`) remain ignored pending T1.8a.

## Cells closed in this phase

From the [matrix](00-matrix.md#the-frozen-matrix):

| Cell | Test name | Notes |
|---|---|---|
| E1×D1 (i,i) | `e1_d1_int_int_local` | Sanity baseline |
| E1×D1 (f,b) | `e1_d1_float_bool_local` | Mixed scalar |
| E1×D1 (c,i) | `e1_d1_char_int_local` | Character element |
| E1×D2 arg | `e1_d2_arg_int_int` | `f((3, 7))` — call site builds tuple |
| E1×D2 inline | `e1_d2_inline_get` | `(3, 7).0` — element access on literal |
| E1×D2 match | `e1_d2_match_subj` | `match (3, 7) { ... }` — built inline |
| E1×D2 if-arm | `e1_d2_if_arm` | `x = if cond { (1, 2) } else { (3, 4) }` |
| E1×D2 return | `e1_d2_return_int_int` | `#[ignore = "T1.8a"]` |
| E1n×D1 | `e1n_d1_local` | `(integer not null, integer)` (T1.7) |
| E1n×D2 arg | `e1n_d2_arg` | Pass `not null` tuple as arg |
| E2×D1 (t,t) | `e2_d1_text_text_local` | Both text — T1.8b lifetime |
| E2×D1 (t,i) | `e2_d1_text_int_local` | Mixed text+scalar |
| E2×D2 arg | `e2_d2_arg_text_text` | Pass tuple-with-text as arg |
| E2×D2 inline | `e2_d2_inline_text` | `("a", "b").0` |
| E2×D2 return | `e2_d2_return_text_text` | `#[ignore = "T1.8a"]` |

15 cells total.  4 marked `#[ignore = "T1.8a"]` until @PLAN06 phase 9a
ships; 11 must run green at phase close.

## Per-cell snippets (illustrative)

```loft
// e1_d2_inline_get
print("{(3, 7).0},{(3, 7).1}\n");
assert((3, 7).0 == 3 && (3, 7).1 == 7, "inline literal access");
```

```loft
// e1_d2_match_subj
result = match (3, 7) {
    (0, _) -> "zero"
    (n, m) -> "{n},{m}"
};
print("{result}\n");
assert(result == "3,7", "match subj inline tuple");
```

```loft
// e1_d2_if_arm
cond = true;
x = if cond { (1, 2) } else { (3, 4) };
print("{x.0},{x.1}\n");
assert(x.0 == 1 && x.1 == 2, "if-arm tuple result");
```

```loft
// e2_d2_arg_text_text
fn show(p: (text, text)) {
    print("{p.0}|{p.1}\n");
    assert(p.0 == "alpha" && p.1 == "beta", "arg text tuple");
}
show(("alpha", "beta"));
```

Each snippet is wrapped by the `cross_mode!` macro, which adds the
`fn test() { … }` boilerplate and the cross-mode comparison.

## Pre-flight: which D2 cells already work?

Before writing fix code, run each cell against the current tree to
discover whether the cell is already passing or genuinely broken.
The matrix in 00-matrix.md marks each E1/E2 D2 cell as `PASS-i + FIX`
because interp coverage exists in `tests/expressions.rs` but no
cross-mode test exists.  A cell may turn out to be green on both modes
already — in that case the "fix" is just the cross-mode test itself
and the matrix promotes the cell from FIX to PASS without any
production change.

```bash
# Per-cell pre-flight: write the snippet to a tmp file, run interp + native.
cargo test --release --test tuple_matrix e1_d2_arg_int_int 2>&1 | tail -20
```

If the test fails, the failure mode (compile error / interp panic /
native panic / divergence) tells us which subsystem to touch.  This
is the same actual-error-survey discipline the user has flagged as
mandatory before writing fix code.

## Likely fix subjects (predicted, verified per cell)

The phase opens with a pre-flight loop; the predictions below are
informed guesses, **not** the implementation list.  Each gets
confirmed or revised per cell before any fix lands.

- **D2 inline literal in `match` subject** — `parse_match` may not
  accept a `Type::Tuple` literal as subject without a temp; check
  `parse_tuple_match`'s preamble.
- **D2 if-arm result** — type unification of two tuple branches in
  `parse_if`; verify `merge_types` handles `Type::Tuple` symmetrically
  with struct-ref branches.
- **E2 D2 inline `("a", "b").0`** — text element from a literal tuple
  in expression position; lifetime tracker may emit a free for the
  whole tuple before the element read.

## Acceptance for phase 01

- 11 unignored cells from the table above run green under
  `cargo test --release --test tuple_matrix` on Linux, macOS, Windows.
- 4 ignored cells have the `#[ignore = "T1.8a — plan-06 phase 9a"]`
  attribute and are listed in @PLAN06 phase 9a's reverse-link table
  (the cell appears in 9a's "un-ignore on close" list).
- The matrix in [README.md](README.md) and [00-matrix.md](00-matrix.md)
  shows each cell as `PASS:test_name`.
- No regression in `tests/expressions.rs` T1.x sections.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| A cell that's expected to pass surfaces a real compiler bug under cross-mode comparison (e.g. native silently truncates a float) | File a P-issue, mark the cell `#[ignore = "P-XXX"]`, remove from acceptance list, add to phase risks.  Don't ship the phase with a known divergence as a pass. |
| Inline tuple literal in `match` subject requires parser work | Scope the parser change to its own commit between cells; smaller blast radius if the change breaks unrelated match dispatch. |
| Cross-mode harness flakes on the first cell because of stdout buffering | Phase 00's harness normalises trailing whitespace; if cells still flake, add explicit `flush()` after each `print` in snippets. |

## Out of scope

- D3 (struct field) cells — covered by phase 05.
- E3, E4, E5 cells — covered by phases 02, 03, 04.
- T1.8a itself — landed by @PLAN06 phase 9a, not here.

## Cross-references

- [README.md](README.md) § matrix
- [00-matrix.md](00-matrix.md) — harness + naming convention
- `tests/expressions.rs` — existing tuple test surface (T1.10
  section)
- `tests/parse_errors.rs` — T1.11 negative tests (do not regress)
- [PLANNING.md § T1.8a](../../PLANNING.md) — return-convention
  prerequisite
