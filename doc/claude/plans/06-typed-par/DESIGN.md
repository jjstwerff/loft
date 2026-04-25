<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cross-cutting design — plan-06 typed par

Architectural decisions that span multiple phases.  The phase files
(01–07) reference these by name; resolving them once here keeps the
phases consistent.

## D1 — Stitch policy

The unified runtime has one polymorphic entry point parameterised
by a stitch policy.  Concrete shape:

```rust
// src/parallel.rs
pub enum Stitch {
    /// Concatenate per-worker output stores into one result vector.
    /// User-visible behaviour of today's par(input, fn) -> vector<U>.
    Concat,

    /// Run workers, drop their results.  Used by the fused for-loop
    /// when the body never references `r`, and by future par_for_each.
    Discard,

    /// Each worker accumulates over its slice via a user-supplied
    /// monoidal fold; main thread combines per-worker partials with
    /// the same fold fn.  Used by `par_fold(input, init, fold,
    /// threads) -> U` and by the fused for-loop when the body is a
    /// pure single-accumulator update (auto-detected at scope
    /// analysis).  Lands in plan-06 phase 3e (runtime) + 7e (surface).
    Reduce { fold_fn: u32 },

    /// Bounded queue: workers push, parent body pops in input order.
    /// Used by the fused for-loop when the body references `r` or
    /// has side effects.
    Queue { capacity: u32 },
}
```

Run-time enum (not compile-time generics) so codegen emits one
`OpParallel(Stitch)` opcode regardless of policy.  The policy is
selected at parse / scope-analysis time and hardcoded into the
opcode stream; runtime never branches on user data.

**Memory:** the enum's largest variant is `Reduce` at 4 bytes; the
opcode payload is fixed.

**Choice rationale:** trait + vtable dispatch was considered but
rejected — the four policies have very different control-flow
shapes (concat returns a vector store, queue runs a parent body,
discard returns void), so a single trait would need an awkward
"either / or / or" return type.  An enum match in the runtime
dispatcher is simpler and the cost is negligible (one branch per
parallel call, not per worker iteration).

## D2 — Worker store ↔ parent store relationship

Today (`src/database/allocation.rs:449`): workers get a deep clone
of `Stores` with `locked = true` on every parent-side store.  The
clone is independent — workers cannot affect parent state.  Result
collection happens via `copy_block` + `copy_claims` from worker
clones into the parent.

After plan-06 (phase 1 onwards): the relationship has two layers.

```
Parent stores
  ├─ shared (stdlib, constants, parent-allocated read-only data)
  ├─ input_store (the vector<T> being iterated)
  └─ result_store (concat-of-worker-outputs, allocated upfront)

Worker_N stores (one per worker thread)
  ├─ ref → shared (read-only Arc)
  ├─ ref → input_store (read-only, indexed by worker's claimed range)
  └─ output_store (writable, owned exclusively by this worker)
```

Workers can read parent shared / input stores via Arc references;
they cannot write to them.  Each worker's output store is
exclusively theirs — no contention, no locks.  After all workers
join, main thread either:
- **Concat policy:** copies worker output stores into result_store at known offsets (phase 2's "stitch" pass).
- **Queue policy:** reads worker output stores via the bounded queue.
- **Discard policy:** drops worker output stores.

**Why this works.**  The aliasing invariant is enforced by Rust's
borrow checker — `Arc<Store>` is read-only by construction; the
output store is `&mut Store` per worker.  No `locked` boolean to
forget to check.  Phases 4–5 lift this into the loft type system
so the worker fn signature carries the input/output store types
explicitly.

**P1-R5 closure** (THREADING.md's "no Rust-level proof of
non-aliasing"): becomes provable.  Claims dropped per P1-R3 in
phase 2 because the parent-clone's `claims` HashSet is no longer
needed when workers don't share-write.

## D3 — Type-checker access to fn-ref return type

Phase 7's desugar of `par(input, fn, threads)` to `Value::ParFor`
needs `U = fn's return type`.  Phase 4's typed `parallel_for(input:
vector<T>, fn: fn(T) -> U)` needs the same.

Today's `Data` already records every fn's signature.  Add one
public accessor:

```rust
// src/data.rs
impl Data {
    /// Return the result type of a function definition.  None if the
    /// def_nr is not a function (e.g. a struct, a constant).
    pub fn fn_return_type(&self, d_nr: u32) -> Option<&Type> {
        match self.def(d_nr).deftype() {
            DefType::Function | DefType::Routine => {
                self.def(d_nr).attributes
                    .iter()
                    .find(|a| a.name == "return")
                    .map(|a| &a.typedef)
            }
            _ => None,
        }
    }
}
```

The accessor lives in `Data` rather than `Function` because the
existing `Definition::attributes` encoding stores return type as a
synthetic attribute named `"return"`.  This matches how the parser
emits return-type info today; no new schema.

**Used by:** phase 7c (desugar's type inference), phase 4 (typed
parallel_for surface).  Other code may use it freely.

## D4 — Failure handling model

Today (`src/parallel.rs:121`): a worker that panics aborts the join
and propagates the panic to the parent thread, which aborts the
parallel call.  The parent's loft program either catches via
`?? return` or terminates.

Plan-06 preserves this exactly.  In phase 1's transitional state,
the same `std::panic::catch_unwind` wraps each worker; on panic
the queue / result store is left in whatever state it was, and
the join propagates.

**No new error model in plan-06.**  Future L1 (error recovery,
roadmap 0.9.0) may introduce `Result<U, Error>` per worker
result; plan-06 is forward-compatible — the `Stitch` policy can
gain a new `ConcatErr` variant when L1 lands.  Tracked as 1.0+
work, not in scope.

**Tests:** every phase's fixture set includes one panic test:
- worker panics on element 5 of 10,
- assert: parent aborts; if `?? return` used, parent recovers; no
  worker store leaks.

## D5 — Empty / degenerate inputs

Each phase's runtime path explicitly handles three degenerate cases:

| Case | Behaviour |
|---|---|
| `len(input) == 0` | Return empty result store immediately; spawn no workers |
| `len(input) == 1` | Single-worker path: skip stitching, return the worker's output store directly as the result |
| `threads >= len(input)` | Clamp `threads = max(1, len(input))`; over-provisioned workers immediately exit |

Phase 0's characterisation suite covers all three cases under
today's runtime.  Each subsequent phase re-runs that suite plus
adds a one-element fixture for the new code path.

## D6 — WASM single-threaded fallback

Today (`#[cfg(feature = "threading")]` blocks throughout
`src/parallel.rs`): under `--features wasm` without `threading`,
the parallel call becomes a sequential for-loop in the calling
thread.  No actual parallelism.

Plan-06 preserves this fallback unchanged.  Each phase's queue /
output-store / stitch logic is gated:

```rust
#[cfg(feature = "threading")]
fn run_parallel_with_workers(...) { /* real workers */ }

#[cfg(not(feature = "threading"))]
fn run_parallel_sequential(...) {
    // Single-thread: allocate output store directly into the result;
    // no queue, no stitch pass.  Output identical to the worker
    // path for any pure worker fn.
}
```

The user-visible surface is identical: `par(...)` in WASM-single
gives the same result as on threaded targets, just slower.  Phase
0's bench harness records WASM numbers as a separate column;
later phases assert no regression on WASM either.

**Future W1.14** (Web Worker pool, ROADMAP 1.1+): adds real
parallelism on WASM via `wasm-bindgen-rayon`.  Plan-06 is forward-
compatible — same `Stitch` policy enum, same queue store layout,
just a different scheduler.

## D7 — `Value::ParFor` IR shape

Concrete struct fields (referenced from phase 1 onward):

```rust
// src/data.rs
pub enum Value {
    // ... existing variants ...

    ParFor {
        input:    Box<Value>,        // expression of type vector<T>
        x_var:    u16,                // bound to each input element
        r_var:    Option<u16>,        // bound to worker result; None for Discard policy
        worker:   Box<Value>,         // expression in scope of x_var, evaluated by workers
        threads:  Box<Value>,         // expression of type integer
        body:     Box<Value>,         // sequential body in scope of x_var, r_var; Insert or Block
        stitch:   Stitch,             // policy, picked at scope analysis time
        src_span: SourceSpan,         // for diagnostics in desugared paths
    },
}
```

`worker` is a `Value` (not just a `d_nr`) so lambdas, method-bound
calls, and direct fn-ref calls all encode uniformly — the desugar
in phase 7c picks the right shape per syntactic form.

`r_var: Option<u16>` because the Discard policy does not bind a
result variable.

`Stitch` is decided at scope analysis time (`src/scopes.rs`):
- if `body` references `r_var` or has side effects → Queue,
- if `body` is empty → Discard,
- if the call form was used (phase 7c desugar) → Concat (with auto-collect body),
- Reduce is reserved for future `par_fold`.

**Backward compat:** the existing `Value::Call(parallel_for, ...)`
stays alive through phases 1–3 as a transitional encoding; phase
4 retires it in favour of `Value::ParFor`.  Codegen during phases
1–3 lowers either shape to the same opcode.

## D8 — Auto-light heuristic (defines phase 5)

The "light path" today is selected by a parser-side heuristic at
`src/parser/builtins.rs:362::check_light_eligible`.  It checks two
things:
1. Worker fn return type is primitive (not Reference, not Text).
2. Worker fn does not call `vector_add`, `vector_insert`, or any
   stdlib fn marked as allocating.

Plan-06 phase 5 generalises this into a scope-analysis pass over
the worker fn body that proves: **the worker writes nothing outside
its own output store**.  Conservative criterion (rejects unsafe;
may reject some safe cases as false negative):

| Construct in worker body | Verdict |
|---|---|
| `return expr` where `expr` doesn't reference enclosing state | Light |
| Read from `x` (input parameter) | Light |
| Read from a `pub const`, `pub` global, or stdlib constant | Light |
| Function call to a stdlib fn marked `#pure` | Light |
| Function call to any other fn | Full (recurse: if callee is also light-clean, lift to Light, else Full) |
| `vector_add` / `vector_insert` / `hash_set` / `s.field = …` on a non-local | Full (writes shared state) |
| `vector_add` / etc. on a local variable | Light (the local IS the worker's output store after phase 1) |
| `par(...)` nested call | Full (nested workers need full path) |
| `LOFT_LOG` / `println` (stderr) | Light (these go through host bridges, not parent stores) |

The pass runs once per worker fn definition and caches the
result in `Definition::is_light_safe: Option<bool>`.  Phase 5
lands the analyser; subsequent uses (the codegen call site) just
read the cached flag.

**Why this is conservative:** false negatives (rejecting Light for
a safe fn) are a perf regression, not a correctness bug.  False
positives (accepting Light for an unsafe fn) would be a real
regression and must be impossible.  The criterion above accepts
only constructs we can prove are write-isolated.

**Recursion handling — fixed-point iteration (phase 5e).**  Phase
5b's initial implementation uses a placeholder trick (insert
`false` for the current fn before recursing) which over-rejects
mutually-recursive pure fns (`is_even` / `is_odd`-shaped pairs).
Phase 5e replaces this with monotonic fixed-point iteration over
the call graph: every user fn starts optimistically light;
demotions propagate via a worklist; pure cycles stay light, impure
cycles correctly demote.  See phase 5's detail file for the
algorithm.

**Test coverage:** phase 5 adds positive and negative fixtures —
fns provably-light, fns provably-not-light.  Phase 5e adds the
mutual-recursion suite that the simple analyser would
conservatively reject.  No fixture relies on the heuristic
accepting an unsafe fn.

## D9 — Source-span propagation

`Value::ParFor.src_span` carries the original `par(...)` call's
token range from parser → scope-analysis → codegen → bytecode.
At codegen, the span attaches to the emitted opcodes via the
existing per-opcode source-position table (`State::source_table`
in `src/state/mod.rs`).  Runtime errors and stack traces resolve
through that table back to the user-written line.

Format-string desugaring and `?? return` already use this
mechanism; plan-06 reuses it.  No new infrastructure.

## D10 — Migration of existing par call sites

Today's call sites use either:
- `parallel_for(input, elem_size, return_size, threads, fn)` (compiler-checked internal),
- `parallel_for_int(...)` (runtime string-based),
- `parallel_for_light(...)` (ditto).

Phase 4's typed surface change retires the integer-positional
encoding.  Migration:

| Phase | What survives | What rewrites |
|---|---|---|
| 1–3 | All three call shapes | Internals; surface unchanged |
| 4 | Only `parallel_for(input: vector<T>, fn: fn(T) -> U, threads: integer) -> vector<U>` | The `_int` and `_light` shapes become parser-side aliases that emit the same `Value::ParFor` |
| 5 | `parallel_for_light` is unreachable from user surface (plan rule); auto-light picks internally | Existing in-repo callers (`tests/scripts/22-threading.loft`, `lib/`) renamed |
| 7 | `par(input, fn, threads)` as expression-position desugar lands | Surface story: one user-visible name (`par`), one internal IR (`Value::ParFor`) |

The legacy `parallel_for_int(func: text, ...)` runtime-string
dispatch is the only call site that escapes typed-IR analysis; it
gets retired entirely in phase 4 (no replacement — every caller
already has a typed alternative).

## D11 — Type spectrum on input and output

After plan-06, par's accepted type surface equals the language's
ordinary fn-signature surface.  **Anywhere you can write
`fn process(x: T) -> U`, you can also write `par(xs, process, N)`.**
No size carve-outs, no primitive-vs-struct distinction, no "size > 8
not supported".  Today's restrictions are runtime-side limitations
removed by the store-typed pipeline.

### D11a — Input types

`par`'s input is anything that today's `for x in input` accepts:

| Input type | Today | After plan-06 | Closes |
|---|---|---|---|
| `vector<Struct>` | ✅ | ✅ | — |
| `vector<integer>` / `<float>` / `<i32>` / `<u8>` | ❌ garbage (G2) | ✅ | phase 4 (typed input) |
| `vector<text>` | ❌ garbage (G2) | ✅ | phase 4 |
| `vector<Reference<T>>` | ❌ likely garbage | ✅ | phase 1 (type-driven stride) |
| `vector<EnumTag>` (plain enum) | ❓ | ✅ | phase 1 |
| `vector<StructEnum>` | ❌ size > 8 (G1) | ✅ | phase 1 |
| `sorted<T[key]>` / `hash<T[key]>` / `index<T[key]>` | ❌ rejected | ✅ | phase 4 — `for x in sorted/hash/index` semantics already exist; typed surface plumbs them through |
| `vector<vector<T>>` (nested) | ❓ | ✅ | phase 4 — element is a Reference<vector<T>>; workers iterate inner via normal vector ops |
| `vector<fn(...) -> T>` | ❓ | ✅ | phase 1 — fn-refs are 16-byte values, same path as primitives |
| Generic `vector<T>` (bounded) | ❓ depends on monomorphisation | ✅ | bounded generics already monomorphise at call site |

The general rule: **iterable in a regular for-loop ⇒ acceptable as
par input**.  Phase 0a's `tests/threading_chars.rs` carries the
gap canaries (`#[ignore]`d); the closing phase un-ignores each.

### D11b — Output types

`par`'s output is anything an ordinary fn can return:

| Output type | Today | After plan-06 | Closes |
|---|---|---|---|
| primitive scalars (sizes 1–8) | ✅ | ✅ | — |
| text | ✅ | ✅ | (channel removed in phase 1b) |
| Reference<Struct> | ✅ | ✅ | (rebase replaces copy_block in phase 2) |
| Plain enum (1-byte disc) | ✅ | ✅ | — |
| StructEnum (variant w/ fields) | ❌ size > 8 (G1) | ✅ | phase 1 — per-worker output Stores accept any record-shaped output |
| Large value-struct (size > 8) | ❌ | ✅ | phase 1 — same root cause |
| `vector<T>` (worker returns a collection) | ❌ | ✅ | phase 1 — output store can hold a vector record like any other |
| `hash<T>` / `sorted<T>` / `index<T>` | ❌ | ✅ | phase 1 — keyed collections are stores; rebase handles them |
| fn-ref | ❌ likely rejected | ✅ | phase 1 — 16-byte values, primitive-path equivalent |
| null | ✅ degenerate | ✅ | — |
| Optional<T> (nullable) | ✅ for primitives | ✅ for everything | sentinel-based; orthogonal to par |
| Tuples | n/a (1.1+) | ✅ when tuples land | tuples are synthetic structs; ride the same path |

### D11c — Principled exception: cross-worker reference graphs

Workers cannot return references to **another worker's** output
store.  E.g. worker A returning a `Reference<X>` that points into
worker B's output is forbidden; the rebase pass at stitch time
keys translation by `(this_worker_id, local_store_nr)`.

This is a deliberate design choice — relaxing it would require
stitch-time aliasing analysis with significant complexity and
little real-world payoff (workloads that need shared output graphs
are rare; the workaround is a post-stitch transformation pass).

The restriction is enforceable at compile time once D2's
`Arc<read-only>` parent / `&mut` exclusive output relationship is
in place: a worker's output type can only reference its own
exclusive output store or read-only parent stores, never another
worker's exclusive store.

### D11d — Why this is "the full normal spectrum"

Three architectural changes from plan-06 unify the type handling:

1. **D2 relationship** — input is read-only Arc; element stride
   comes from the type system, not a parser-computed integer.
2. **Phase 1 per-worker output Stores** — workers write via
   ordinary `OpSet*` opcodes, the same way every loft fn writes
   its return value.  No size pre-check; no fixed-byte-width
   dispatch.
3. **Phase 4 typed surface** — `parallel_for(input: vector<T>,
   fn: fn(T) -> U, threads) -> vector<U>` gets compile-time T/U
   validation; runtime never has to guess.

After all three land, par is **type-uniform with the rest of the
language**.  The 7-name surface (par, par_light, parallel_for,
parallel_for_int, parallel_for_light, parallel_get_*) and the
size-class restrictions are both artefacts of pre-plan-06
implementation choices, not language design constraints.

## See also

- [README.md](README.md) — plan-06 phase ladder.
- [00-baseline-and-bench.md](00-baseline-and-bench.md) — phase 0
  detail; pins current behaviour before this design takes effect.
- [07-fused-for-par.md](07-fused-for-par.md) — phase 7 detail; the
  user-visible surface that this design targets.
- `src/parallel.rs` — current 683-line runtime that phases 1–3
  collapse.
- `src/database/allocation.rs:449` — current `clone_for_worker` /
  `clone_for_light_worker`; superseded by D2's relationship.
- `src/parser/builtins.rs:362` — current `check_light_eligible`;
  superseded by D8's analyser.
