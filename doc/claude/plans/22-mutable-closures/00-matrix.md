<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

## Goal

Lock the case × destination matrix and wire `tests/mut_closure_matrix.rs`
to the shared `cross_mode!` harness from `tests/common/cross_mode.rs`.
Mirrors plan-15 phase 00 (closure validation) and plan-14 phase 00
(tuple validation) — the donor template is mature; phase 00 is pure
test infrastructure with zero production change.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `REJECT:phase` / `CLOSED:reason`.

| | D1 — local var | D2 — fn parameter | D3 — struct field | D4 — return value |
|---|---|---|---|---|
| **A** read-only | PASS:00 | PASS:00 | PASS:00 | PASS:00 |
| **B** co-scoped mutating (Reference capture) | FIX:02 | FIX:02 | FIX:02 | n/a (escape — would be C) |
| **B** co-scoped mutating (scalar via cell) | FIX:02 | FIX:02 | FIX:02 | n/a (escape — would be C) |
| **C** moved mutating (factory) | n/a (no escape) | n/a (no escape) | FIX:03 | FIX:03 |
| **D** aliased mutating | REJECT:04 | REJECT:04 | REJECT:04 | REJECT:04 |
| **Mutable&lt;T&gt; explicit** | FIX:05 | FIX:05 | FIX:05 | FIX:05 |

Note: D1/D2/D3 are "co-scoped" destinations (closure stays within
the capture's scope); D3 (struct field) becomes "co-scoped" only
when the host struct is allocated in the same scope as the
capture.  D4 (return value) escapes by definition.

## Harness reuse

```rust
// tests/mut_closure_matrix.rs (new binary)

mod common;

cross_mode!(a_d1_read_only_local, r#"
    fn test() {
        n = 5;
        f = fn(x: integer) -> integer { x + n };  // read-only capture
        result = f(10);
        print("{result}\n");
        assert(result == 15, "a_d1");
    }
"#);
```

No new harness code.  Same `#[ignore]` discipline as plan-14/15.

Run with:
```bash
cargo test --release --test mut_closure_matrix -- --ignored
```

## Per-cell test inventory (phase 00 ships only the smokes + Case A baseline)

```
harness_smoke_basic              // print path
harness_smoke_arithmetic         // expression eval

a_d1_read_only_local             // n = 5; f = fn(x) { x + n }; f(10)
a_d2_read_only_arg               // apply(fn(x) { x + n }, 10)
a_d3_read_only_field             // Box{cb: fn(x){x+n}}; b.cb(10)
a_d4_read_only_return            // make() -> fn(integer); reads outer

(B/C/D cells land in their respective phase commits)
```

## Acceptance for phase 00

- `tests/mut_closure_matrix.rs` exists with smokes + 4 Case A cells.
- Matrix table in this file fully populated — no "TBD" cells.
- README phase ladder matches matrix.
- `make ci` green.
- No production change.

## Risks

| Risk | Mitigation |
|---|---|
| Cell name `a_d<N>_*` collides with future Case A sub-shapes | Use `a<suffix>_d<N>_<sub>` if more shapes appear; current single-shape coverage leaves room. |
| Case A baseline accidentally triggers mutation detection (false-positive) once phase 01 lands | Phase 01 cells include a Case A regression check via `--introspect` or test-only helper to confirm A stays classified as A. |

## Cross-references

- [README.md](README.md) — full spec; this phase fixes the test surface.
- [DISCUSSION.md § Analysis sketch](DISCUSSION.md) — algorithm-level walkthrough.
- [plan-15 phase 00](../finished/15-closure-validation/00-matrix.md) — donor template.
- `tests/common/cross_mode.rs` — shared harness.
