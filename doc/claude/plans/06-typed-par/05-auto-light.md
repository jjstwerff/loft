<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Auto-light heuristic

**Status: open**

## Goal

Build a scope-analysis pass that proves whether a worker fn is
**light-safe** — meaning the worker writes nothing outside its own
output store.  When the proof succeeds, the runtime picks the light
execution path (read-only borrow of parent stores, no claim
HashSet, no per-worker store clone) automatically.  Users never
opt in.

This is the work that makes `par_light` redundant as a user-facing
distinction.  Phase 7c removes the user surface; phase 5 lands the
analyser that makes the removal safe.

The heuristic itself is defined in DESIGN.md D8.  This file is
the implementation plan.

## What "light-safe" means precisely

A function `f: T → U` is light-safe if every code path through its
body satisfies all four conditions:

| Rule | Meaning |
|---|---|
| **R1 — no parent-store writes** | The body does not call any stdlib fn that writes to a non-local store (`vector_add`, `vector_insert`, `hash_set`, `s.field = …` on a non-local target).  Writes to LOCAL variables or to the implicit return-store are fine — those become the worker's output. |
| **R2 — no nested `par(...)`** | Calling `par(...)` from inside a light-safe worker is allowed but forces full path (the nested call needs its own scratch).  Conservatively, R2 rejects light for any worker that calls `par`. |
| **R3 — only pure stdlib calls** | The body's calls are restricted to fns marked `#pure` (a new attribute introduced in phase 5).  All math fns, format-string assembly, type conversions, and pattern-match destructuring are pure; vector/hash ops that grow the data structure are not. |
| **R4 — no `LOFT_STORES`-style mutation** | Calls that mutate runtime state (e.g. random_init, time_set) are not pure even if their return type looks innocuous.  Their declarations get the `#impure` annotation explicitly. |

A fn that satisfies R1–R4 is light-safe; cache the result in
`Definition::is_light_safe: Option<bool>`.

A fn that fails any rule is "full" — picks the today-shape
clone-everything path.

## How the analyser works

```
fn is_light_safe(d_nr: u32, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    // Check cache.
    if let Some(&cached) = cache.get(&d_nr) { return cached; }

    // Insert a "false" placeholder to break recursion cycles.
    cache.insert(d_nr, false);

    let body = &data.def(d_nr).code;
    let result = walk(body, data, cache);

    cache.insert(d_nr, result);
    result
}

fn walk(value: &Value, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    match value {
        // R1 — direct writes to non-local
        Value::Call(callee, _) if is_writing_stdlib(*callee, data) => false,
        Value::Call(callee, _) if data.def(*callee).name == "n_par" => false,  // R2

        // R3 — recurse into user fns
        Value::Call(callee, args) => {
            if !is_pure_stdlib(*callee, data) && !is_light_safe(*callee, data, cache) {
                return false;
            }
            args.iter().all(|a| walk(a, data, cache))
        }

        // Compound expressions
        Value::Insert(ops) | Value::Block(ops) => ops.iter().all(|v| walk(v, data, cache)),
        Value::If(c, t, e) => walk(c, data, cache) && walk(t, data, cache) && walk(e, data, cache),
        Value::Loop(body) => walk(body, data, cache),
        Value::Match(subject, arms) => {
            walk(subject, data, cache) && arms.iter().all(|a| walk(a, data, cache))
        }

        // Trivially safe
        Value::Var(_) | Value::Int(_) | Value::Float(_) | Value::Text(_)
        | Value::Bool(_) | Value::Null => true,

        Value::Set(_, rhs) => walk(rhs, data, cache),

        // Conservative for unknown shapes
        _ => false,
    }
}
```

The full `walk` covers every `Value` variant (~30 today); each
case either short-circuits to `false` (unsafe), recurses (compound),
or returns `true` (leaf).  Conservative on unknowns: any new
`Value` variant added in the future defaults to "not light-safe"
until explicitly classified.

## The `#pure` attribute

Phase 5 introduces a new fn-declaration annotation:

```loft
#pure
fn min(a: integer, b: integer) -> integer;
#rust"if a < b { a } else { b }"
```

`#pure` declares: "this fn does not write to any parent store and
does not have observable side effects".  The analyser treats
`#pure`-marked stdlib fns as light-safe leaves without recursing.

Today's stdlib has ~150 fns; ~120 are pure.  Phase 5a annotates
them all in one sweep.  The remaining 30 (vector_add, hash_set,
file ops, log writes, random fns, time fns, par* fns) get explicit
`#impure` annotations or the absence of `#pure` (default = not
known pure).

## Per-commit landing plan

### 5a — `#pure` annotation infrastructure

- Parser recognises `#pure` and `#impure` annotations on fn
  declarations.  Stores in `Definition::purity: Option<Purity>`
  where `Purity = Pure | Impure | Unknown`.
- All stdlib fns in `default/*.loft` get explicit `#pure` /
  `#impure` annotations.  Unknown defaults to "not light-safe"
  (conservative).
- Smoke test: `tests/issues.rs::par_phase5a_purity_audit` walks
  every stdlib fn and asserts its purity classification matches
  the (hand-written, peer-reviewed) expected list.

### 5b — `is_light_safe` analyser

- Add `src/scopes.rs::analyse_light_safety(data: &mut Data)` that
  runs after pass-2 type checking.  Walks every fn body once,
  populates `Definition::is_light_safe`.
- Recursive cycle handling via the cache placeholder (mark `false`
  before recursing; if the recurse returns true, keep the recursion
  pessimistic — cycles are not provably safe).
- Smoke test: `tests/issues.rs::par_phase5b_classifications` runs
  the analyser on a fixture set with known classifications:
  - pure-arithmetic worker → light
  - vector-allocating worker → full
  - text-format-only worker → light
  - struct-returning worker that doesn't mutate enclosing → light
  - nested-par worker → full
  - mutually-recursive worker pair, one safe one unsafe → both full

### 5c — wire the analyser into codegen

- Codegen reads `Definition::is_light_safe` when emitting the
  `Stitch` payload.  Light-safe → `Stitch::ConcatLight` (a new
  internal variant; not user-visible).  Full → `Stitch::Concat`.
- Runtime branches once per parallel call; light-safe path skips
  the parent-store clone (just borrows read-only).
- `Stores::clone_for_light_worker` (today's manual API) becomes the
  default for `Stitch::ConcatLight` calls.

Acceptance: existing `par_light(...)` callers (still in the surface
until phase 7c) continue to work but now produce identical bytecode
to plain `par(...)` against a light-safe worker.  Bench shows the
auto-light path is selected for the same workers users would have
manually annotated.

### 5d — diagnostic for "almost light"

When a worker fn fails the light heuristic by a small margin (one
shared-state write, one nested par call), emit a `loft --warn`
diagnostic:

```
warning: par() worker `compute_score` is not light-safe; full clone path used
  --> src/lib.loft:42
   |
42 |     fn compute_score(x: Item) -> float {
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |     the call to `vector_add` on line 45 prevents light-path optimisation
   |     consider returning the vector instead of mutating a captured one
```

This is the user-visible artefact of phase 5 — they don't see
"par_light" any more, but they get a hint when their worker could
be faster if they restructure.  Diagnostic is emitted under
`-W par-light-missed` (default off in 0.9.0; default on in 1.0.0
once W-warn lands).

## Cross-cutting interactions

| DESIGN.md item | Phase 5 contribution |
|---|---|
| D8 auto-light heuristic | This phase implements it |
| D2 worker / parent relationship | Light-safe path uses Arc-borrow of parent (D2's proper relationship); full path uses today's locked clone |
| D10 migration | Phase 5 makes phase 7c's `par_light` removal safe — auto-light produces equivalent execution profile |

## Test fixtures

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase5_pure_arithmetic_worker_is_light` | Worker `|x| x * 2 + 1` classified light |
| `tests/issues.rs::par_phase5_vector_allocating_worker_is_full` | Worker that calls `vector_add` is full |
| `tests/issues.rs::par_phase5_text_format_worker_is_light` | Worker `|x| "item-{x}"` classified light (format-string assembly is pure) |
| `tests/issues.rs::par_phase5_struct_returning_worker_is_light` | Worker `|x| Point { x: x, y: x + 1 }` classified light (struct construction is pure if no field mutation outside) |
| `tests/issues.rs::par_phase5_nested_par_is_full` | Worker that itself calls `par(...)` is full (R2) |
| `tests/issues.rs::par_phase5_recursive_safe_pair_both_full` | Two mutually-recursive workers, both pure, classified full (R3 cycle pessimism) — known false negative; documented |
| `tests/issues.rs::par_phase5_par_light_alias_works` | Existing `par_light(...)` callers run; the auto-light path is selected; output identical to before |
| `tests/issues.rs::par_phase5_diagnostic_under_warn_flag` | `loft -W par-light-missed program.loft` emits the "almost light" diagnostic for a near-miss worker; not emitted for clean workers |

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- Auto-light selects the light path on every test fixture that
  previously used `par_light(...)` explicitly.
- No false positives: the analyser never marks a fn light-safe
  that is actually unsafe (verified by every fixture in the
  full-path category).
- False negatives are documented: cycle-recursive workers, workers
  using opaque stdlib fns not yet annotated `#pure`.
- Bench-1 (light-eligible primitive worker) within ±5 % of
  hand-annotated `par_light` baseline.

## Risks

| Risk | Mitigation |
|---|---|
| Annotating ~150 stdlib fns as `#pure` is mechanical but error-prone | Phase 5a's audit fixture lists each fn's classification with rationale; review by reading every annotation before commit; CI's purity-audit test catches drift |
| Cycle pessimism over-rejects | Documented as a known false negative.  Future improvement: fixed-point analysis (compute purity in waves) — out of scope for plan-06 |
| New `Value` variants in future code default to "not light-safe" | This is the safe default; future contributors who add new variants explicitly classify them |
| The `-W par-light-missed` diagnostic is noisy | Default off until 1.0.0; when on, can be suppressed per-fn with `#allow(par-light-missed)` |
| Parser changes for `#pure` / `#impure` annotations conflict with existing `#native` / `#rust` | Annotations stack — a fn can be `#pure #native` (a pure native fn).  Parser's annotation list grows; no syntactic conflict |

## Out of scope

- Cycle-aware fixed-point purity analysis.
- User-facing `#pure` annotations (only stdlib gets them in
  phase 5; user fns get inferred-only purity).
- Effects / capability tracking beyond the binary pure / impure
  split.
- The `par_light` user-surface removal (phase 7c).
- Cleanup / doc (phase 6).

## Hand-off to phase 6

After phase 5:
- Auto-light heuristic correctly classifies stdlib + user fns.
- Light-safe path selected automatically by codegen.
- `par_light(...)` user-facing call still works (no behaviour
  change visible to users) but produces identical bytecode to
  plain `par(...)`.

Phase 6 sweeps the cleanup: deletes the now-unused runtime
variants, retires the `Stores::clone_for_light_worker` distinction
(it's just `clone_for_worker` with the right flag), updates docs.
