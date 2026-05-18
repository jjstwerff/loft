<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — Case C: moved mutating closures (factory pattern)

**Status: fully shipped 2026-05-13.**  Single-factory works
via 02d-iii.e (cell + auto-Reference machinery);
multi-instance interleave fixed by the @P259 four-commit
chain (OpIncRc emission on capture + cascade-free in
`Stores::free_named` gated on `__closure_` type prefix).

## Major finding (2026-05-13)

Case C ALREADY WORKS for single-factory patterns under
phase 02d-iii.e's cell + auto-Reference machinery.  The
explicit liveness-check + closure-bound cell deps described
below ARE NOT NEEDED for the single-factory case — the
existing dep chain (closure-record auto-Reference attribute
holds a DbRef into the cell store; the closure DbRef in the
returned 16B fn-ref slot keeps the closure record alive past
the parent fn's scope exit; the cell stays alive transitively)
already produces correct behaviour.

Verified via:
- `c_d4_make_counter_factory` cell in `tests/mut_closure_matrix.rs`
- `c_d3_factory_into_struct_field` cell (struct embedding)
- `p22_phase03_factory_no_leak` leak guard (100-iter loop)

All three pass on both interp + native.

**Multi-instance interleave (`f1 = make(); f2 = make();
f1(); f2(); f1();`) shipped 2026-05-13 via @P259** — the
closure record now actually OWNS the cells it captures
(`inc_rc` on capture + cascade-free on close-record free
gated on `__closure_` type prefix).  See
[PROBLEMS.md row 259](../../../PROBLEMS.md#open-issues--quick-reference)
for the full closing story (commits, files touched,
regression pins).

The original phase-03 design (below) anticipated needing
explicit liveness checks + cell-dep rewriting; the actual
implementation reused the existing dep chain for
single-factory and added `inc_rc + cascade-free` for
multi-factory.

## Goal

Implement Case C per [README § Case C](README.md#case-c--moved-mutating)
and [DISCUSSION § Phase 2 step 3](DISCUSSION.md#phase-2--case-classification)
("liveness check on captures past the closure's construction
site … if NO for all mutated captures: case = C").

Case C is the factory pattern: a closure escapes the scope where
its captures were defined, AND the outer scope makes no use of
the captures after the closure's construction.  The closure
takes ownership of the captured cells; their lifetime is bound
to the closure's lifetime, not the construction-site scope's.

## What ships

Add a liveness check that runs after phase 02's destination-scope
check fails (i.e., destination > some capture's scope).  For each
mutated capture, ask: "is this name read or written in the outer
scope after the closure construction site, within the defining-
scope's live range?"

If NO for all: classify as C.  Lower like B (Reference for user
types, hidden cell for scalars) BUT bind the cell's lifetime to
the closure's lifetime — the closure carries the cell.

If YES for any capture: classify as D (phase 04 handles).

Implementation: extends loft's existing per-variable live-interval
tracking (visible via `LOFT_LOG=variables`) with a check at each
closure-construction site.

```rust
fn captures_read_after(
    captured_name: &str,
    construction_pos: u32,
    scope: &Scope,
    function: &Function,
) -> bool {
    let var_nr = function.var_by_name(captured_name);
    if var_nr.is_none() { return false; }
    let interval = function.live_interval(var_nr.unwrap());
    // Live-interval is (first_use, last_use).  If last_use is
    // strictly greater than construction_pos AND the position
    // is within the scope's range, the binding is read after.
    interval.last_use > construction_pos
        && scope.contains(interval.last_use)
}
```

The lifetime-binding lowering: the cell's free is emitted at the
closure's drop site (when the fn-ref variable goes out of scope
in the caller) instead of at the construction-site scope's exit.
This reuses @PLAN15 phase 03's finding that closure records
free correctly via `Parts::ChildRec` at scope exit — case C just
moves the freeing scope from "construction-site" to "closure-
escape-site."

## Test surface

`tests/mut_closure_matrix.rs`:

```
c_d4_make_counter_factory      // make_counter() -> fn(integer); count owned by closure
c_d4_make_text_appender        // make_appender() -> fn(text); buf owned by closure
c_d4_factory_called_many_times // 100x calls to factory; each closure has independent count
c_d3_factory_into_struct_field // s = S{cb: make_counter()}; s.cb invokes the captured count
```

`make_counter` cell uses the canonical pattern from
[DISCUSSION § Snippet 3](DISCUSSION.md):

```loft
fn make_counter() -> fn(integer) -> integer {
    count = 0;
    fn(delta: integer) -> integer {
        count += delta;
        count
    }
}

fn test() {
    add = make_counter();
    a = add(5);
    b = add(3);
    c = add(10);
    print("{a},{b},{c}\n");
    assert(a == 5 && b == 8 && c == 18, "factory state");
}
```

Plus a leak guard in `tests/leak.rs`:

```rust
#[test]
fn p22_phase03_factory_no_leak() {
    // 100 closures, each with its own captured cell; verify
    // all cells freed when closures go out of scope.
    run_leak_check_str(r#"
        fn make() -> fn() -> integer {
            n = 0;
            fn() -> integer { n = n + 1; n }
        }
        fn test() {
            i = 0;
            while i < 100 {
                f = make();
                _ = f();
                _ = f();
                _ = f();
                i = i + 1;
            }
        }
    "#);
}
```

## Critical files

| File | Change |
|---|---|
| `src/parser/closure_analysis.rs` | Add `liveness_check_captures_read_after` using existing var-live-interval API |
| `src/parser/vectors.rs::synthesize_closure_record` | Branch on case=C: lower like B but mark closure-record cell-deps with `dep=[w]` (closure work var) instead of `dep=[outer_var]` so the cell frees at closure scope, not construction-site scope |
| `src/scopes.rs` | Expose live-interval read API used by the analysis |
| `src/variables/mod.rs` | Add `var_by_name(name)` lookup helper if not already present |

## Verification

- All 4 c_* cells green under interp + native cross-mode.
- Each `make_counter()` instance has independent state (no
  cross-instance leakage).
- Returning the closure from a non-tail position (e.g.
  `let f = make(); f` vs `make()` directly) both work.
- `p22_phase03_factory_no_leak` clean over 100 iterations.
- Existing closure_matrix.rs cells (22) still green.
- CI gate green.

## Risks

| Risk | Mitigation |
|---|---|
| Liveness check has false-negative (treats C as B) | Cell is freed too early (at construction-site scope exit while closure still holds it).  Reads after escape see freed memory.  Mitigation: conservative default — when liveness is uncertain, treat as D (rejection).  Add a cell that constructs closure inside an `if` branch and returns it to verify the branch-collapse doesn't lose the escape signal. |
| Liveness check has false-positive (treats B as C) | Cell is freed late (closure-scope exit instead of construction-scope exit).  Suboptimal but correct: cell stays live longer than necessary.  No correctness bug.  Acceptable. |
| Recursive factories (factory returns a closure that itself constructs a closure) | The inner closure's captures need to flow through the outer's lifetime.  Mitigation: phase 03 cells include a recursive-factory cell to verify; if it surfaces a bug, file as a P-issue rather than blocking phase 03. |
| Captures used "after" via a method call where the call's purity is unknown | Conservative: treat as Case D rejection.  Phase 06 audit may relax via tighter purity annotations. |
| `c_d3_factory_into_struct_field` interacts with phase 02's struct-field destination scope check | The factory call returns first (closure already case-C-classified), THEN the result is stored in the struct field.  The struct-field assignment is case A from the destination's perspective (just assigns a fn-ref).  Should work; cell verifies. |

## Cross-references

- [README § Case C](README.md#case-c--moved-mutating)
- [DISCUSSION § Snippet 3](DISCUSSION.md) — paper-trace of case C classification.
- `src/scopes.rs` — live-interval tracking.
- `src/variables/mod.rs` — variable lookup.
- [@PLAN15 phase 03 leak findings](../15-closure-validation/00-matrix.md) — `Parts::ChildRec` cascade verified.
