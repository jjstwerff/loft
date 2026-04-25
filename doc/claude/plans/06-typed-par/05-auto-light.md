<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — `is_par_safe` analyser + Arc-wrapping parent stores

**Status: open**

## Goal

Build the scope-analysis pass that **enforces D2.0's read-only-parent
language rule** — a par worker that writes to non-local state is
invalid loft code, not just slow.  The pass populates
`Definition::is_par_safe`; codegen for `par(...)` queries the
flag and the parser emits a compile error with a fix-it for any
worker that fails.

In parallel, phase 5 lands the `Arc<Store>` mechanism that
implements D2 — `Stores.allocations` becomes `Vec<Arc<Store>>`
for parent stores; `WorkerStores.parent_view` is `Vec<Arc<Store>>`;
`WorkerStores.worker_owned` is `Vec<Box<Store>>`.  Together these
two work items make the full-clone path **unreachable**, ready for
phase 6 to delete it.

This is the work that:
- Makes `par_light` redundant (auto-detected by `is_par_safe`).
- Makes the full-clone path semantically dead (D2.0).
- Brings worker-construction cost from ~5 ms to <2 µs at any
  parent size (D14a).

The rules are defined in DESIGN.md D8 (analyser) and D8.1
(`#impure` sub-classes).  This file is the implementation plan.

## What "par-safe" means precisely

A function `f: T → U` is par-safe if every code path through its
body satisfies all four conditions (matches DESIGN.md D8 verbatim):

| Rule | Meaning |
|---|---|
| **R1 — no parent-store writes** | The body does not call any stdlib fn classified `#impure(parent_write)` with a non-local first argument.  Writes to LOCAL variables or to the worker's output slot are fine — those become the worker's output. |
| **R2 — no nested `par(...)`** | Calling `par(...)` / `par_fold(...)` / `parallel_for(...)` from inside a par-safe worker is rejected in 0.9.0.  Nested-pool support is 1.0+; current rejection produces compile error with `note: nested par is supported in 1.0+`. |
| **R3 — only par-allowed stdlib calls** | The body's stdlib calls must be classified `#pure`, `#impure(host_io)`, `#impure(prng)`, or `#impure(io)` (per D8.1).  `#impure(parent_write)` and `#impure(par_call)` are rejected. |
| **R4 — no mutation through captured `Reference<T>`** | A worker holding a `Reference<X>` captured from parent scope cannot write through it (would race).  Writes through reference-typed params are allowed only when the reference points into the worker's output slot or local scratch. |

A fn that satisfies R1–R4 is par-safe; cache the result in
`Definition::is_par_safe: Option<bool>`.

A fn that fails any rule **is a compile error in any par
context** — there is no fallback path.  The diagnostic includes
the offending construct's source location and a fix-it
suggestion (typically: "return the value instead of mutating;
let par collect it").

## How the analyser works

```
fn is_par_safe(d_nr: u32, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
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
            if !is_pure_stdlib(*callee, data) && !is_par_safe(*callee, data, cache) {
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
`Value` variant added in the future defaults to "not par-safe"
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
`#pure`-marked stdlib fns as par-safe leaves without recursing.

Today's stdlib has ~150 fns; ~120 are pure.  Phase 5a annotates
them all in one sweep.  The remaining 30 (vector_add, hash_set,
file ops, log writes, random fns, time fns, par* fns) get explicit
`#impure` annotations or the absence of `#pure` (default = not
known pure).

### How "pure" is decided per stdlib fn

Plan-06 defines purity as **no parent-store mutation and no
observable side effect**.  Below is the operative classifier the
phase-5a sweep applies.  Borderline cases get explicit rationale
in the audit fixture, not silent default.

| Module / category | Default | Examples | Rationale |
|---|---|---|---|
| Arithmetic, logic, bit-ops | `#pure` | `min`, `max`, `abs`, `gcd`, bitops | No state |
| Type conversions | `#pure` | `int_to_text`, `parse_integer` | Return value only |
| Format-string assembly (read-only) | `#pure` | format string `"{x}"` lowering | Allocates into return slot, not parent |
| Pattern-match / destructure | `#pure` | match arms with no side-effect body | Return value only |
| Pure stdlib collection ops (read-only) | `#pure` | `length(v)`, `contains(s)`, `index_of(v, x)` | Read-only access |
| Mutating collection ops | `#impure` | `vector_add`, `vector_insert`, `hash_set`, `vector_remove` | Writes to a parent-side store the caller passed in |
| File / IO | `#impure` | `read_file`, `write_file`, `print_*`, `log_*` | Observable side effect (filesystem, stdout, log sink) |
| Random / time / env | `#impure` | `random_int`, `random_seed`, `now`, `env_get` | Mutates internal PRNG / reads global mutable state |
| Concurrency primitives | `#impure` | `par`, `par_light`, `par_fold`, `parallel_for` | Spawns workers — caller-visible state changes |
| Database / store APIs | `#impure` | `Stores::claim`, `Stores::release`, `database.size` | Mutates store table |

**PRNG-style state.**  `random_int(min, max)` reads and advances
an internal PRNG.  Per the operative definition (no
parent-store mutation, no observable side effect), reading a PRNG
*is* an observable side effect — two consecutive calls return
different values.  Classified `#impure`.  A user-facing `pure_random`
that takes an explicit seed and is referentially transparent could
be `#pure`; not in plan-06 scope.

**Allocation as side effect.**  Allocating into the caller's
return slot does **not** count as a side effect — that's the
worker's own output Store after phase 1.  Allocating into a
caller-passed-in collection (`vector_add(v, x)`) **does** count —
it mutates a parent-side store.

The audit fixture (`tests/issues.rs::par_phase5a_purity_audit`)
walks every fn declared in `default/*.loft` and asserts its purity
classification matches the table above.  A fn missing both `#pure`
and `#impure` annotations is a CI failure — phase 5a closes the
"unknown" set entirely for stdlib so the analyser's default
("Unknown defaults to not par-safe") never fires for stdlib
calls.

## Per-commit landing plan

### 5a — `#pure` annotation infrastructure

- Parser recognises `#pure` and `#impure` annotations on fn
  declarations.  Stores in `Definition::purity: Option<Purity>`
  where `Purity = Pure | Impure | Unknown`.
- All stdlib fns in `default/*.loft` get explicit `#pure` /
  `#impure` annotations.  Unknown defaults to "not par-safe"
  (conservative).
- Smoke test: `tests/issues.rs::par_phase5a_purity_audit` walks
  every stdlib fn and asserts its purity classification matches
  the (hand-written, peer-reviewed) expected list.

### 5b — `is_par_safe` analyser

- Add `src/scopes.rs::analyse_light_safety(data: &mut Data)` that
  runs after pass-2 type checking.  Walks every fn body once,
  populates `Definition::is_par_safe`.
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

### 5b' — caller-graph infrastructure (prerequisite for 5e)

Per DESIGN.md D12: phase 5e's fixed-point iteration needs two
`Data` accessors that do not exist today.  Phase 5b' lands them
**before 5e** so the algorithm has its prerequisites.  5b'
deliberately ships before 5c so the codegen-wiring step has the
final analyser shape available.

**Touch points:**
- `src/data.rs` — add `Data::user_fn_d_nrs() -> &[u32]` and
  `Data::callers_of(d_nr) -> &[u32]`.
- `src/data.rs::Data::build_caller_index()` — internal helper
  that walks every fn body once, collecting `Value::Call(callee, _)`
  and `Value::CallRef(callee, _)` to populate a `HashMap<u32,
  Vec<u32>>`.  Called lazily on first `callers_of` invocation;
  cached.

**Acceptance:**
- `tests/issues.rs::par_phase5b_prime_caller_graph_correctness` —
  fixture with three fns where A calls B, B calls C, C calls A;
  assert `callers_of(A) = [C]`, `callers_of(B) = [A]`,
  `callers_of(C) = [B]`.
- `tests/issues.rs::par_phase5b_prime_caller_graph_cost` —
  synthetic 1000-fn codebase with 5000 random call edges; assert
  `build_caller_index` completes in <100 ms.

**Why this is its own sub-phase, not folded into 5e.**  See
DESIGN.md D12 — 5e is the *user* of the caller graph; 5b' is the
*provider*; future analysis passes (escape, dead-code, more
purity refinements) reuse the same accessor.

### 5c — Arc-wrap parent stores + wire the analyser into codegen

Two coupled changes that land together:

**Arc-wrap parent stores (D2 + D14b).**
- `Stores.allocations` becomes `Vec<Arc<Store>>`.
- `Stores.types` becomes `Arc<Vec<Type>>`.
- `Stores.names` becomes `Arc<HashMap<String, u16>>`.
- New `WorkerStores { parent_view: Vec<Arc<Store>>, worker_owned: Vec<Box<Store>>, ... }`.
- New `Stores::link_for_worker(&self) -> WorkerStores` constructs
  the parent_view via `Arc::clone` per store; appends the
  worker's output slot to `worker_owned`.

Construction cost per worker: <2 µs at any parent size
(Arc bumps + struct copies; no buffer alloc).

**Wire the analyser.**
- Codegen for `par(...)` reads `Definition::is_par_safe` for the
  worker fn.  If `false`, emits compile error per D8 (no
  bytecode generation for that par call).
- If `true`, emits `OpParallel(Stitch::Concat)` with the new
  `WorkerStores` construction path.
- The old `Stitch::ConcatLight` variant from earlier draft is
  deleted — under D2.0 there's only one path; no `Light` vs
  non-light split remains.

Acceptance:
- Existing `par_light(...)` callers (still in the surface until
  phase 7c) compile to identical bytecode as plain `par(...)`.
- Workers that today silently take the full path now produce
  compile errors with the D8 fix-it diagnostic.
- `bench/11_par`'s loft-interp + loft-native columns improve
  measurably (no more 5 ms HashMap clone overhead per call).

### 5d — error diagnostics

D8's diagnostic shape becomes the actual compiler error (not a
warning).  Three forms cover the common rejections:

```
error: par worker `accumulate` writes to non-local `total_v`
  --> src/main.loft:42
   |
42 |     fn accumulate(x: Item) {
43 |         total_v += [score_of(x)]
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^ writes to non-local; results vanish at join
   |
   = note: par workers cannot mutate parent state by language design.
   = help: return the value and let par collect it:
   |       fn accumulate(x: Item) -> Score { score_of(x) }
   |       results = par(items, accumulate, 4)
```

```
error: par worker `process` calls par() recursively
  --> src/lib.loft:88
   |
88 |     fn process(x: Item) -> Result {
89 |         par(x.subitems, sub_process, 4)
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ nested par is not supported in 0.9.0
   |
   = note: nested par-pool support is planned for 1.0+.
   = help: flatten the work or process subitems sequentially:
   |       fn process(x: Item) -> Result {
   |           x.subitems.map(sub_process).collect()
   |       }
```

```
error: par worker `compute` calls #impure(parent_write) fn `vector_add` on non-local
  --> src/lib.loft:120
   |
120|     fn compute(x: Item) -> Result {
121|         vector_add(captured_log, "processed " + str(x))
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ first arg is captured from parent
   |
   = help: use a local + return value:
   |       fn compute(x: Item) -> (Result, text) {
   |           (Result::new(x), "processed " + str(x))
   |       }
   |       (results, logs) = par(items, compute, 4).unzip()
```

The `#impure(host_io)` exception means `log_warn(...)`,
`println(...)`, and `print(...)` calls do **not** trigger the
diagnostic — those are explicitly par-safe per D8.1.

### 5e — cycle-aware purity (fixed-point iteration)

The simple analyser from 5b uses a placeholder trick to break
recursion: insert `false` for the current fn before recursing, so
self-calls and mutual-recursion calls short-circuit.  Correct, but
**pessimistic** — pure mutually-recursive fns get classified `full`
even though both are write-isolated.

```loft
// Both are pure; both end up classified `full` by 5b.
fn is_even(n: integer) -> boolean {
    if n == 0 { true } else { is_odd(n - 1) }
}
fn is_odd(n: integer) -> boolean {
    if n == 0 { false } else { is_even(n - 1) }
}
```

Phase 5e replaces the placeholder trick with a **monotonic
fixed-point iteration** over the call graph.

**Algorithm:**

```rust
fn analyse_purity_fixpoint(data: &Data) -> HashMap<u32, bool> {
    // 1. Build the call graph.
    let callers: HashMap<u32, Vec<u32>> = build_caller_index(data);

    // 2. Initial state: every user fn starts OPTIMISTICALLY light.
    //    Stdlib fns get their explicit annotation immediately
    //    (#pure → true; #impure → false; unknown → false).
    let mut classification: HashMap<u32, bool> = HashMap::new();
    for d_nr in 0..data.definitions() {
        let initial = match data.def(d_nr).purity {
            Purity::Pure => true,
            Purity::Impure | Purity::Unknown => {
                // For user fns, start true; the iteration demotes
                // those that turn out unsafe.
                if is_user_fn(d_nr, data) { true } else { false }
            }
        };
        classification.insert(d_nr, initial);
    }

    // 3. Worklist: any user fn whose body might demote it.
    let mut worklist: VecDeque<u32> = data.user_fn_d_nrs().into_iter().collect();

    while let Some(d_nr) = worklist.pop_front() {
        if !classification[&d_nr] { continue; }       // already false
        let body = &data.def(d_nr).code;
        if !walk_with_current_classification(body, &classification) {
            // This fn is now classified false; its callers may
            // also need to demote.
            classification.insert(d_nr, false);
            for &caller in callers.get(&d_nr).unwrap_or(&vec![]) {
                if classification[&caller] {
                    worklist.push_back(caller);
                }
            }
        }
    }

    classification
}
```

**Why this works:**
- Classifications are monotonic (`true → false`, never reverse),
  so the iteration terminates in at most `N` steps where `N` is
  the number of user fns.
- Pure mutual cycles never produce a demotion event: if every
  fn in the cycle is pure, no one triggers `false`, and the
  cycle stays classified `true`.
- Impure cycles (any fn writes shared state) propagate `false`
  outward through the worklist: the impure fn flips first; its
  callers are added to the worklist; they walk their bodies with
  the updated classification and flip too if they call the
  newly-impure fn.
- Stdlib `#pure` / `#impure` annotations are respected — phase
  5e never overrides them.

**Cost analysis:**
- Worst case: every user fn gets walked twice (once optimistically,
  once after demotion).  For loft's stdlib (~150 fns) plus a
  typical user codebase (a few hundred fns), the analyser runs
  in a few milliseconds.
- Memory: one `HashMap<u32, bool>` for classifications + one
  `HashMap<u32, Vec<u32>>` for the caller index.  Linear in the
  call-graph edge count.

**Replacement for 5b's placeholder mechanism.**  Phase 5e
*replaces* the recursive `analyse_light_safety` from 5b — it's
the same accessor name, same return shape, just a different
implementation underneath.  The cache/placeholder trick goes away.

**Touch points:**
- `src/scopes.rs::analyse_light_safety` (from 5b) — body rewritten
  to use the fixed-point iteration.
- `Data::user_fn_d_nrs` and `Data::callers_of` — already landed in
  5b' (prerequisite sub-phase); 5e uses them directly.

**Tests** (lifted from 5b's documented false negatives):
- `tests/issues.rs::par_phase5e_mutual_recursion_pure` — the
  `is_even` / `is_odd` pair from above; both classified light.
- `tests/issues.rs::par_phase5e_cycle_with_one_impure_fn` — a
  3-cycle where one fn calls `vector_add`; all three classified
  full (impurity propagates correctly).
- `tests/issues.rs::par_phase5e_self_recursion_pure` — `fn fact(n)
  -> integer { if n <= 1 { 1 } else { n * fact(n - 1) } }`
  classified light.
- `tests/issues.rs::par_phase5e_par_light_recursive_pair_now_works`
  — same fixture as 5b's documented `par_phase5_recursive_safe_pair_both_full`,
  inverted: now both fns ARE classified light (the test name
  changes; the previous test gets removed).
- `tests/issues.rs::par_phase5e_termination` — synthetic case
  with 100 fns in a fully-connected pure cycle; analyser
  terminates within 100 ms.

**Acceptance for 5e:**
- All 5b tests still pass with the new analyser.
- The mutual-recursion fixture that was previously a documented
  false negative now classifies both fns as light.
- The `par_phase5_recursive_safe_pair_both_full` fixture from 5b
  is replaced by `par_phase5e_mutual_recursion_pure` with inverted
  expectation.
- DESIGN.md D8's "false negative — cycle pessimism" disclaimer
  is updated to "false negative — only when stdlib fns aren't
  annotated `#pure`; cycles among user fns are handled correctly".
- Bench-1 with a mutual-recursion-heavy worker (synthetic case
  forcing the cycle path) shows the same throughput as a
  non-recursive equivalent.

## Cross-cutting interactions

| DESIGN.md item | Phase 5 contribution |
|---|---|
| D8 auto-light heuristic | This phase implements it |
| D2 worker / parent relationship | Phase 5c lands the Arc-wrap of parent stores (D2.0/D2/D14b).  Workers always use the Arc-borrow path; full-clone path is dead code awaiting phase-6 deletion. |
| D10 migration | Phase 5 makes phase 7c's `par_light` removal safe — auto-light produces equivalent execution profile |

## Test fixtures

| Fixture | Asserts |
|---|---|
| `tests/issues.rs::par_phase5_pure_arithmetic_worker_is_light` | Worker `|x| x * 2 + 1` classified light |
| `tests/issues.rs::par_phase5_vector_allocating_worker_is_full` | Worker that calls `vector_add` is full |
| `tests/issues.rs::par_phase5_text_format_worker_is_light` | Worker `|x| "item-{x}"` classified light (format-string assembly is pure) |
| `tests/issues.rs::par_phase5_struct_returning_worker_is_light` | Worker `|x| Point { x: x, y: x + 1 }` classified light (struct construction is pure if no field mutation outside) |
| `tests/issues.rs::par_phase5_nested_par_is_full` | Worker that itself calls `par(...)` is full (R2) |
| `tests/issues.rs::par_phase5_recursive_pair_resolved_in_5e` | Two mutually-recursive pure workers — 5b conservatively classifies both full; 5e's fixed-point pass classifies both light (this test moves classification expectation between sub-phases) |
| `tests/issues.rs::par_phase5_par_light_alias_works` | Existing `par_light(...)` callers run; the auto-light path is selected; output identical to before |
| `tests/issues.rs::par_phase5_diagnostic_under_warn_flag` | `loft -W par-light-missed program.loft` emits the "almost light" diagnostic for a near-miss worker; not emitted for clean workers |

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- Every previously-passing par fixture still compiles and runs;
  any fixture that today silently took the full-clone path is
  **either** rewritten to be par-safe (most cases — the writes
  weren't doing anything anyway) **or** removed from the test
  suite as invalid loft (with a comment explaining why).  A
  preserved git note records the survey results.
- No false positives: the analyser never marks a fn par-safe
  that is actually unsafe (verified by every par-unsafe fixture
  producing the expected compile error).
- After 5e: false negatives reduce to "workers using stdlib fns
  not yet annotated correctly per D8.1".  Cycles among user fns
  are handled correctly by the fixed-point pass.
- Bench-1 (par-safe primitive worker) ≤ today's hand-annotated
  `par_light` baseline minus 5 ms (HashMap/types clones eliminated
  by D14b's Arc-wrap).
- **Scale fixture `bench/par_huge_parent_small_output`** (added
  in 5a): 200 MB const store + 1 M `i64` input scan + 8-byte
  output per element + par-safe worker.  Asserts:
  - The single par execution path is selected (verified via
    `LOFT_LOG=static` dump showing `Stitch::Concat` and the
    `Arc<Store>` parent_view in WorkerStores construction).
  - Total wall time ≤ `worker_compute_time + 50 µs` (per D14a,
    overhead is sub-µs per worker × few workers; 50 µs is a
    generous gate).
  - Peak resident memory ≤ `parent + 16 MB` (no parent clone).
  - `LOFT_STORES=warn` reports zero leaked stores.

  This fixture is the canonical "huge parent + small output"
  workload — the shape plan-06 is optimised for.  A regression
  here means a stdlib `#pure` annotation was dropped or the
  analyser pessimised an obviously-pure fn; either is a P0 bug.

## Runtime diagnostic — full path on large parent

Independent of `-W par-light-missed` (which is compile-time and
default-off until 1.0.0), phase 5d also emits a **runtime warning**
at every par call site that takes the full-clone path against a
parent with > 50 MB of total store data:

```
warning: par() at file.loft:42 used full-clone path against
         parent state of 300 MB (cloned 4 times for 4 workers,
         ~240 ms overhead).  Worker `compute_score` was rejected
         from the light path because:
           - call to vector_add at file.loft:45 writes to a
             non-local store (rule R1).
         Consider returning the vector instead of mutating.
```

Emitted to `log_warn` (default visible) on the first occurrence
per call site per program run; subsequent occurrences are
counted but not re-logged.  Threshold (50 MB) is conservative —
below it the full clone takes < 10 ms and isn't worth a warning.

This catches the 240 ms-cliff regression even when the user
hasn't enabled the compile-time `-W par-light-missed` warning.
Especially load-bearing for workloads with huge parent state +
tiny output where any false-negative auto-light decision is a
production-visible stall.

## Risks

| Risk | Mitigation |
|---|---|
| Annotating ~150 stdlib fns as `#pure` is mechanical but error-prone | Phase 5a's audit fixture (`par_phase5a_purity_audit`) lists each fn's classification with the operative rule from "How 'pure' is decided" applied; review by reading every annotation against the rule table before commit; CI's purity-audit test catches drift.  Borderline cases (PRNG, time, allocation-into-return) get explicit rationale comments. |
| Cycle pessimism over-rejects | Closed in sub-phase 5e via fixed-point iteration over the call graph; pure mutual cycles correctly classified light |
| New `Value` variants in future code default to "not par-safe" | This is the safe default; future contributors who add new variants explicitly classify them |
| The `-W par-light-missed` diagnostic is noisy | Default off until 1.0.0; when on, can be suppressed per-fn with `#allow(par-light-missed)` |
| Parser changes for `#pure` / `#impure` annotations conflict with existing `#native` / `#rust` | Annotations stack — a fn can be `#pure #native` (a pure native fn).  Parser's annotation list grows; no syntactic conflict |

## Out of scope

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
