<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Case B: co-scoped mutating closures

**Status: open**

## Goal

Implement Case B per [README § Case B](README.md#case-b--co-scoped-mutating)
and [DISCUSSION § Phase 2](DISCUSSION.md#phase-2--case-classification)
classification step 2 ("destination_scope ⊆ each
mutated_capture.defining_scope: case = B").

Case B is the EventLoop / TTT v6 server retrofit case — the main
novice-cliff driver behind plan-22's promotion to current.  When
the closure stays within its captures' defining scope (assigned
to a local, stored in a same-scope struct field, or passed to a
function that doesn't store it), captures are lowered from
inline-by-value to by-reference; mutations made inside the
closure body are visible to the outer scope through the live
record.

## What ships

Two lowerings, both gated on phase 01's `mutated` flag plus a
new destination-scope-inclusion check:

### B/Reference — user-type captures

When `mutated_captures[i].type` is `Type::Reference(d, deps)`
(struct, vector, hash, sorted, index, spacial, struct-enum),
change the closure-record field type from inline-by-value to
`Reference(d, [outer_var])`.  Codegen for the closure body's
field reads and writes already handles `Reference<T>` (it's the
same pattern as a struct method's `self` parameter).

### B/Scalar — primitive captures via hidden cell

When `mutated_captures[i].type` is `Type::Integer(_)` /
`Type::Text(_)` / `Type::Float` / `Type::Single` /
`Type::Boolean` / `Type::Character` / `Type::Enum(_, false, _)`:
allocate a hidden 1-field record (`__cell_<i>`) in the same
store as the closure record.  The captured slot in the closure
struct holds a `Reference(__cell_<i>)`; reads/writes route
through `OpGetX(cell, 0)` / `OpSetX(cell, 0, val)`.

The outer scope's binding is rewritten to also read/write
through the cell (the cell IS the binding's storage from the
closure-construction point onwards).

### Destination-scope-inclusion check

Determine `destination_scope_id` per
[DISCUSSION § Phase 2 step 1](DISCUSSION.md#phase-2--case-classification):

- Local var: scope of the let.
- Struct field assignment: scope of the struct's allocator.
- Function argument: depends on the callee's storage intent.  If
  the callee stores the fn-ref in a struct field of an outer
  store (e.g. `el::on(loop, cb)` where loop lives in the caller
  scope), use the storage scope.  Otherwise treat as immediately-
  consumed (case B always satisfies).
- Returned: caller's scope (always > enclosing fn's scope; case
  B never holds — falls through to phase 03's case C check).

If `destination_scope ⊆ each mutated_capture.defining_scope`:
case B.  Lower as above.

## Test surface

`tests/mut_closure_matrix.rs`:

```
b_d1_int_capture_local_mutated      // n = 0; f = fn() { n = n + 1 }; f(); f(); assert n == 2
b_d1_struct_field_capture_mutated   // s = State{...}; f = fn() { s.x += 1 }; f(); assert s.x updated
b_d1_text_append_local              // s = ""; f = fn() { s += "x" }; f(); f(); assert s == "xx"
b_d1_vec_append_local               // v = []; f = fn() { v += [1] }; f(); assert len(v) == 1
b_d2_state_passed_to_handler        // EventLoop pattern: el::on(loop, fn() { state.score += 1 })
b_d3_closure_in_same_scope_struct   // S{cb: fn() { count = count + 1 }} where S allocated in count's scope
```

EventLoop simulation cell uses a small mock to avoid pulling
the real `lib/server` into the matrix:

```loft
struct Loop { handlers: vector<fn()> }
fn on(self: Loop, h: fn()) { self.handlers += [h]; }
fn fire(self: Loop) { for h in self.handlers { h(); } }

fn test() {
    state = State { score: 0 };
    loop = Loop { handlers: [] };
    on(loop, fn() { state.score += 1 });
    fire(loop);
    fire(loop);
    assert(state.score == 2, "score={state.score}");
}
```

## Critical files

| File | Change |
|---|---|
| `src/parser/closure_analysis.rs` | Add `classify_case` returning A/B/C/D using phase-01 mutation flags + destination-scope check |
| `src/parser/vectors.rs::synthesize_closure_record` | Branch on case: B/Reference → field type = Reference(d, [outer_var]); B/Scalar → allocate hidden `__cell_<i>` and use Reference to it |
| `src/parser/vectors.rs::emit_lambda_code` | When case=B, rewrite the outer binding's slot to point at the hidden cell so post-construction outer reads see the live record |
| `src/data.rs` | Document the destination-scope-id resolution; possibly add a helper `Definition::storage_intent` |
| `src/scopes.rs` | Compute the scope id for each captured binding; pass to classifier |

## Verification

- All 6 b_* cells green under interp + native cross-mode.
- Mutations made inside the closure are visible to outer-scope reads after `fire(loop)` (or analogous trigger).
- Existing closure_matrix.rs cells (22) still green — Case A regression net.
- TTT v5 server (existing) still passes with the new lowering active for its capture-of-Reference patterns.
- CI gate green.

## Risks

| Risk | Mitigation |
|---|---|
| B/Scalar cell allocation leaks | Phase 02 cells include leak guards in `tests/leak.rs` (mirror plan-15 phase 03/04 pattern): 100-iteration tight loops asserting `state.check_store_leaks()` clean. |
| Destination-scope check has false-negative (treats B as C) | Phase 03's liveness check then runs and either also confirms safe (passes as C) or rejects (D).  Either way no incorrect behavior — just suboptimal lowering.  Net: cell still green, classifier could be tightened in a follow-up. |
| Destination-scope check has false-positive (treats C as B) | The closure outlives the cell; reads after escape see freed memory.  CRITICAL.  Mitigation: conservative default — when in doubt, treat as escape (C/D path).  Phase 02 cells include explicit "closure stays in same scope" assertions; any cell where the closure ends up referenced past its scope's exit MUST classify as C or D.  Add a cell that returns the closure (which should classify C) and asserts case is NOT B. |
| Native codegen for B/Scalar via hidden cell unsupported on `--native` | Same `Parts::ChildRec` cascade plumbing P213 used for D3 closure records in plan-15 phase 03/04 — verified clean.  Add a native-specific cell to confirm. |
| Mutating-closure callees with `Unknown` purity over-trigger Case B | Phase 06 audit reduces false-positives at the `default/01_code.loft` source.  Phase 02 ships with the conservative default; over-flagged cells just produce slightly larger closure records but remain correct. |

## Cross-references

- [README § Case B](README.md#case-b--co-scoped-mutating)
- [DISCUSSION § Snippet 2](DISCUSSION.md) — paper-trace of case B classification.
- `src/parser/vectors.rs::synthesize_closure_record` — closure synthesis (the lowering hook).
- `src/database/mod.rs::Parts::ChildRec` — the cascade primitive plan-15 phase 03/04 verified.
- [plan-23 EVENT_LOOP](../23-event-loop/README.md) — downstream consumer waiting on case B.
