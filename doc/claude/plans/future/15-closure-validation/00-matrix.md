<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

## Goal

Lock the closure-validation matrix and wire `tests/closure_matrix.rs`
to the existing `cross_mode!` harness from
`tests/common/cross_mode.rs`.  No new harness code is needed; the
plan-14 phase-00 infrastructure is reused as-is.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason` /
`PASS-i, FIX-n:phase` (interp passes, native fixes in named phase).

| | D1 — local var | D2 — direct stack | D3 — struct field | D4 — vector element |
|---|---|---|---|---|
| **C0** non-capturing | FIX:01 | FIX:01 | FIX:01 | FIX:01 |
| **C1** basic-type capture | FIX:02 | FIX:02 | FIX:02 | CLOSED:vec-of-capturing-closure |
| **C2** text capture | FIX:03 (decision: leak fix vs document) | FIX:03 | FIX:03 | CLOSED:vec-of-capturing-closure |
| **C3** Reference capture | FIX:04 | FIX:04 | FIX:04 | CLOSED:vec-of-capturing-closure |
| **C5** multi-capture | FIX:02 | FIX:02 | FIX:02 | CLOSED:vec-of-capturing-closure |
| **C6** nested closure | FIX:05 | FIX:05 | FIX:05 (deferred — depends on C3 dep propagation) | CLOSED:vec-of-capturing-closure |
| **C7** vector capture | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal |

D5 (tuple element) is intentionally absent — it's covered by
[plan-14 phase 03](../../finished/14-tuple-validation/03-closures.md).

## Harness reuse

```rust
// tests/closure_matrix.rs (new binary)

mod common;

cross_mode!(my_closure_cell, r#"
    fn add5(x: integer) -> integer { x + 5 }
    fn test() {
        f = fn add5;
        result = f(10);
        print("{result}\n");
        assert(result == 15, "c0_d1 fn-ref");
    }
"#);
```

**No new harness code.**  `tests/common/cross_mode.rs` is shared
between binaries via `mod common;`.  `cross_mode!` already marks
every cell `#[ignore = "tuple_matrix — run with …"]` — the same
`#[ignore]` reason works for `closure_matrix.rs` because the
"heavy by default" rationale is identical.  Run the closure
matrix with:

```bash
cargo test --release --test closure_matrix -- --ignored
# or single cell:
cargo test --release --test closure_matrix -- --ignored c1_d1_int_capture_local
```

## Per-cell test inventory

Cell names use the closure-specific prefix `c<C>_d<D>_<sub>` so
they don't collide with plan-14's `e<E>_d<D>_<sub>` namespace.

```
c0_d1_non_cap_local            // f = |x| { x + 1 }; f(3)
c0_d2_non_cap_arg              // map(nums, |x| { x + 1 })
c0_d2_non_cap_inline           // (|x| { x + 1 })(3)
c0_d3_non_cap_field            // struct S { cb: fn(integer) -> integer }
c0_d4_non_cap_vector           // vector<fn(integer) -> integer> of plain lambdas
c1_d1_int_capture_local        // n = 5; f = |x| { x + n }
c1_d2_int_capture_arg
c1_d3_int_capture_field
c2_d1_text_capture_local       // s = "tag"; f = |x| { "{s}: {x}" }
c2_d2_text_capture_arg
c2_d3_text_capture_field       // active risk: LIFETIME.md leak gap
c3_d1_ref_capture_local        // p = make_point(); f = || { p.x }
c3_d2_ref_capture_arg
c3_d3_ref_capture_field
c5_d1_multi_capture_local      // n + s + p captured together
c5_d2_multi_capture_arg
c5_d3_multi_capture_field
c6_d1_nested_closure_local     // inner captured by outer
c6_d2_nested_closure_arg
c7_d1_vec_capture_rejected     // CLOSED — assert exact parser/scope diagnostic
c1_d4_vec_of_capturing_closed  // CLOSED — vector<fn(...)> of capturing
c2_d4_vec_of_capturing_closed  // CLOSED — same
c3_d4_vec_of_capturing_closed  // CLOSED — same
```

A CLOSED cell uses a manual `#[test]` that runs `code!(...).error(...)`
in `tests/parse_errors.rs` rather than `cross_mode!` — the contract
is "the diagnostic stays exactly as today".  When a CLOSED cell
flips (e.g. the language adds first-class generic fn-refs), it
graduates to a FIX cell in a follow-up commit.

## Acceptance for phase 00

- New file `tests/closure_matrix.rs` exists with one smoke test
  exercising the harness end-to-end (e.g. a `c0_d1_non_cap_local`
  cell that runs green).
- Matrix table in this file is fully populated — no "TBD" cells.
- README phase ladder matches matrix.
- `make ci` green.
- No production code change.

## Risks

| Risk | Mitigation |
|---|---|
| `tests/closure_matrix.rs` adds cargo-test compile time on every binary build | Same mitigation as plan-14: cells `#[ignore]`d by default, default `cargo test` skips them. |
| Cell name namespace `c<C>_d<D>` collides with future axes if more capture shapes appear | Add a `c<C><suffix>_d<D>` rule when adding new shapes; current C0–C7 leaves room. |
| The closure-leak gap (LIFETIME.md) is unpinned until phase 03 — phase 02 cells may produce false-pass results because the leak doesn't surface in those cell shapes | Phase 02 cells run under `tests/leak.rs`-style assertions in addition to `cross_mode!` cross-equivalence.  If the leak surfaces in a C1 capture, it gets filed as an open P-issue on phase 02 instead of waiting for phase 03. |

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/common/cross_mode.rs` — shared harness.
- [plan-14 phase 00](../../finished/14-tuple-validation/00-matrix.md) — donor
  template; same matrix style + cross-mode contract.
- [LIFETIME.md § Function](../../LIFETIME.md) — closure leak gap.
