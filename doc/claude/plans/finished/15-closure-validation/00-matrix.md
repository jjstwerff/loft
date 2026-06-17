<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: SHIPPED 2026-05-12** — `tests/closure_matrix.rs` created;
3 cells green under `cargo test --release --test closure_matrix --
--ignored` (harness_smoke_basic, harness_smoke_arithmetic,
c0_d1_non_cap_local).  No production change.

**Phase 01 SHIPPED 2026-05-12** — C0 row complete: 5 cells
(c0_d1_non_cap_local + c0_d2_non_cap_arg + c0_d2_non_cap_inline
+ c0_d3_non_cap_field + c0_d4_non_cap_vector).  All non-capturing
shapes pass interp + native cross-mode.  No production change —
the underlying support shipped earlier (@P214 closed the
vector-of-non-capturing path 2026-05-05).

**Phase 02 SHIPPED 2026-05-12** — C1 + C5 (basic-type captures)
across D1/D2/D3: 6 cells (c1_d1_int_capture_local +
c1_d2_int_capture_arg + c1_d3_int_capture_field +
c5_d1_multi_capture_local + c5_d2_multi_capture_arg +
c5_d3_multi_capture_field).  Single integer capture and
multi-basic capture (int + bool + int) both pass.  D4 stays
CLOSED for capturing closures.  No production change —
@P213's `Parts::ChildRec` layout-widening (closed 2026-05-04)
already supports the struct-field capture surface; phase 02
pins it as a regression guard.

**Phase 03 SHIPPED 2026-05-12** — C2 (text captures) across
D1/D2/D3: 3 cells (c2_d1_text_capture_local +
c2_d2_text_capture_arg + c2_d3_text_capture_field).  Plus 2
leak guards in `tests/leak.rs`:
`p15_phase03_closure_text_capture_field_no_leak` and
`p15_phase03_closure_text_capture_local_no_leak` — both run
100-iteration tight loops calling capturing closures and
assert `state.check_store_leaks()` passes after.

**Phase 03 decision** (the active LIFETIME risk slice):
the closure-DbRef leak feared in LIFETIME.md does NOT manifest
in any C2/D1/D2/D3 shape.  D1 frees the closure record at
stack-frame exit; D3 frees via @P213's `Parts::ChildRec`
cascade when the host struct goes out of scope.  No P-issue
filed — the LIFETIME.md "NOT YET HANDLED" annotation is
overstated documentation drift, not a runtime bug.  Phase 06
should update LIFETIME.md to reflect the actual freed-at-
scope-exit behaviour.

No production change — @P227 closed text-returning fn-ref
calls (interp + native) 2026-05-05; phase 03 pins it as a
regression guard plus adds the leak surface coverage.

**Phase 04 SHIPPED 2026-05-12** — C3 (Reference captures)
across D1/D2/D3: 3 cells (c3_d1_ref_capture_local +
c3_d2_ref_capture_arg + c3_d3_ref_capture_field).  Each
captures a struct (DbRef-allocated) into a closure body
that reads field(s) from it.  Both backends green.

Plus 2 leak guards in `tests/leak.rs`:
`p15_phase04_closure_ref_capture_field_no_leak` and
`p15_phase04_closure_ref_capture_local_no_leak` — 100-iteration
tight loops, both clean.

**Phase 04 finding**: no DbRef-in-closure-record leak or
read-after-free.  The "move-vs-copy semantics gap analogous
to @PLAN14 T1.8c" feared in the plan does NOT manifest for
closures.  The dep mechanism in `vectors.rs:666-669`
(Type::Function carries closure-record dep `[w]`) plus
`Parts::ChildRec` cascade for D3 ensures the captured Point's
store record stays live until the closure record is freed.
No P-issue filed.

No production change — the underlying support shipped earlier
(@P213 closed struct-field captures 2026-05-04 with
`Parts::ChildRec`; the dep-tracking for closure records was
in place from the original closure surface).  Phase 04 pins
it as a regression guard.

**Phase 05 SHIPPED 2026-05-12** — C6 (nested closures: closure
captures another closure) across D1/D2/D3: 3 cells
(c6_d1_nested_closure_local + c6_d2_nested_closure_arg +
c6_d3_nested_closure_field).  Inner lambda is non-capturing;
outer lambda captures the inner fn-ref into its closure record.
Both backends green.

Plus 1 leak guard in `tests/leak.rs`:
`p15_phase05_nested_closure_no_leak` — 100-iteration tight loop
exercising the 3-link dep chain (outer fn-ref ← outer closure
record ← inner fn-ref).  Clean — no leak accumulation.

D3 was matrix-flagged as deferred ("depends on C3 dep
propagation") but phase 04 confirmed C3 dep propagation works,
so D3 is included.  Constraint: inner lambda must be
non-capturing (@P215's supported case); capturing-source-into-
closure remains deferred (would need `synthesize_closure_record`
to register the 8B split layout when the captured lambda itself
captures).

No production change — @P215 closed nested-closure name
resolution 2026-05-05; phase 05 pins it as a regression guard.

**Phase 06 SHIPPED 2026-05-12** — closeout.  LIFETIME.md
"Implementation path" trimmed (legacy 6-step contemplated work
that never landed because `Parts::ChildRec` + standard local-
cleanup already covers the surface); ROADMAP.md / USER_FACING.md
/ plans/6-audience-generative-art cross-refs updated;
plan moved to `plans/finished/15-closure-validation/`.

**Phase 06 finding — 1 new bug filed**: probing during closeout
surfaced **@P257** — capturing a `vector<T>` into a closure body
crashes both backends with no clean parse-time rejection (interp
panics with `Write to locked store`, native rejects with rustc
E0308 + E0605).  The matrix's C7 row was CLOSED:non-goal but the
failure mode is unstable.  Filed as Low severity — no user code
in lib/* depends on capturing vectors into closures.

**Plan-15 final bug yield**: 1 new P-issue (@P257) across 22 matrix
cells + 5 leak guards.  Below the 2-3 yield predicted in the
README ("@PLAN14 phase 01 found 2 P-issues in 15 cells; @PLAN15
likely surfaces a comparable rate").  The lower yield reflects
that @P213/P214/P215/P216/P227 cleared the closure surface in the
May 4-5 sprint BEFORE @PLAN15 ran, so the matrix mostly pinned
regression guards rather than finding new bugs.  @P257 was found
by deliberately probing CLOSED cells during phase 06 closeout —
the matrix's PASS/FIX cells all worked because the support
landed earlier.  Lesson for @PLAN16+: aggressive probing of the
CLOSED-cell boundary during closeout is the highest-yield part
of the validation arc when the underlying surface is already
mostly clean.

## Goal

Lock the closure-validation matrix and wire `tests/closure_matrix.rs`
to the existing `cross_mode!` harness from
`tests/common/cross_mode.rs`.  No new harness code is needed; the
@PLAN14 phase-00 infrastructure is reused as-is.

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
[@PLAN14 phase 03](../14-tuple-validation/03-closures.md).

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
they don't collide with @PLAN14's `e<E>_d<D>_<sub>` namespace.

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
| `tests/closure_matrix.rs` adds cargo-test compile time on every binary build | Same mitigation as @PLAN14: cells `#[ignore]`d by default, default `cargo test` skips them. |
| Cell name namespace `c<C>_d<D>` collides with future axes if more capture shapes appear | Add a `c<C><suffix>_d<D>` rule when adding new shapes; current C0–C7 leaves room. |
| The closure-leak gap (LIFETIME.md) is unpinned until phase 03 — phase 02 cells may produce false-pass results because the leak doesn't surface in those cell shapes | Phase 02 cells run under `tests/leak.rs`-style assertions in addition to `cross_mode!` cross-equivalence.  If the leak surfaces in a C1 capture, it gets filed as an open P-issue on phase 02 instead of waiting for phase 03. |

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/common/cross_mode.rs` — shared harness.
- [@PLAN14 phase 00](../14-tuple-validation/00-matrix.md) — donor
  template; same matrix style + cross-mode contract.
- [LIFETIME.md § Function](../../../LIFETIME.md) — closure leak gap.
