<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

## Goal

Lock the coroutine-validation matrix and wire
`tests/coroutine_matrix.rs` to the existing `cross_mode!` harness
from `tests/common/cross_mode.rs`.  No new harness code is needed;
the @PLAN14 phase-00 infrastructure is reused as-is.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason` /
`PASS-i, FIX-n:phase` (interp passes, native fixes in named phase).

| | X1 — for-loop | X2 — manual `next()` | X3 — higher-order (map/filter/reduce) | X4 — comprehension | X5 — yield from |
|---|---|---|---|---|---|
| **Y1** scalar | FIX:01 | FIX:01 | FIX:01 | FIX:01 | CLOSED:CO1.4-deferred |
| **Y2** text | FIX:02 (active risk: lifetime) | FIX:02 | FIX:02 | FIX:02 | CLOSED:CO1.4-deferred |
| **Y3** Reference | FIX:03 | FIX:03 | FIX:03 | FIX:03 | CLOSED:CO1.4-deferred |
| **Y4** tuple | FIX:04 | FIX:04 (gated on T1.8a) | FIX:04 | FIX:04 | CLOSED:CO1.4-deferred |
| **Y5** closure | FIX:05 (depends on @PLAN15) | FIX:05 | FIX:05 | FIX:05 (depends on @PLAN15) | CLOSED:CO1.4-deferred |
| **Y6** vector | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:CO1.4-deferred |

## Harness reuse

```rust
// tests/coroutine_matrix.rs (new binary)

mod common;

cross_mode!(my_coroutine_cell, r#"
    fn count_up() -> iterator<integer> {
        i = 0;
        while i < 3 {
            yield i;
            i += 1;
        }
    }
    fn test() {
        sum = 0;
        for v in count_up() {
            sum += v;
        }
        print("{sum}\n");
        assert(sum == 3, "y1_x1 for-loop sum");
    }
"#);
```

**No new harness code.**  Same cross-mode contract as @PLAN14 and
@PLAN15.  Run with:

```bash
cargo test --release --test coroutine_matrix -- --ignored
# or single cell:
cargo test --release --test coroutine_matrix -- --ignored y2_x1_text_for_loop
```

## Per-cell test inventory

Cell names use the coroutine-specific prefix `y<Y>_x<X>_<sub>` so
they don't collide with @PLAN14's `e<E>_d<D>` or @PLAN15's
`c<C>_d<D>` namespaces.

```
y1_x1_int_for_loop                   // for v in gen() { sum += v }
y1_x1_int_for_loop_helper_yield      // generator calls helper that yields
y1_x2_int_manual_next                // it = gen(); v = next(it)
y1_x3_int_map                        // map(gen(), |v| { v * 2 })
y1_x3_int_filter                     // filter(gen(), |v| { v > 0 })
y1_x3_int_reduce                     // reduce(gen(), 0, |a, b| { a + b })
y1_x4_int_comprehension              // [v for v in gen()]

y2_x1_text_for_loop                  // active risk — yielded text lifetime
y2_x2_text_manual_next
y2_x3_text_map
y2_x4_text_comprehension

y3_x1_ref_for_loop                   // yielded DbRef; record owned by gen
y3_x1_ref_for_loop_parent_owned      // record owned by caller (no rebase)
y3_x2_ref_manual_next
y3_x3_ref_map
y3_x4_ref_comprehension

y4_x1_tuple_for_loop                 // for (a, b) in gen() — destructuring loop
y4_x2_tuple_manual_next              // requires T1.8a — #[ignore]
y4_x3_tuple_map
y4_x4_tuple_comprehension

y5_x1_closure_for_loop               // depends on plan-15
y5_x2_closure_manual_next
y5_x3_closure_invoked_through_map
y5_x4_closure_comprehension

y6_*_vector_yield_rejected           // CLOSED — assert exact diagnostic
y*_x5_yield_from_rejected            // CLOSED — CO1.4 not yet implemented
```

A CLOSED cell uses a manual `#[test]` running `code!(...).error(...)`
in `tests/parse_errors.rs` rather than `cross_mode!`.

## Acceptance for phase 00

- New file `tests/coroutine_matrix.rs` exists with one smoke test
  exercising the harness end-to-end (e.g. `y1_x1_int_for_loop`).
- Matrix table in this file is fully populated — no "TBD" cells.
- README phase ladder matches matrix.
- `make ci` green.
- No production code change.

## Risks

| Risk | Mitigation |
|---|---|
| Yielded text (Y2) surfaces a state-machine lowering bug | Phase 02 starts with X1 alone (smallest cell); if the bug surfaces, the plan pauses, the issue is filed, and a fix lands before extending to X2–X4. |
| Y4 manual-next requires T1.8a | The cell carries `#[ignore = "T1.8a — plan-06 phase 9a"]` until T1.8a lands; un-ignore in a one-line follow-up.  Other Y4 cells (X1/X3/X4) don't need T1.8a because the for-loop / map / comprehension paths receive yielded values via the iterator protocol, not return-by-value. |
| Y5 closure cells exercise both coroutine state-machine AND closure dep tracking; failures may mis-attribute | Phase 05 only opens after @PLAN15 phases 01-04 are green.  If a Y5 cell fails when @PLAN15 is green, the bug is in coroutine × closure interaction, not in either subsystem alone. |
| Test runtime balloons (each cell shells out to interp + native) | Same mitigation as @PLAN14 / @PLAN15: cells `#[ignore]`d by default; on-demand run with `-- --ignored`. |

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/common/cross_mode.rs` — shared harness.
- [@PLAN14 phase 00](../../finished/14-tuple-validation/00-matrix.md) — donor
  template.
- [@PLAN15 README](../../finished/15-closure-validation/README.md) — phase 05
  prerequisite (now SHIPPED 2026-05-12).
- [COROUTINE.md](../../COROUTINE.md) — coroutine design.
