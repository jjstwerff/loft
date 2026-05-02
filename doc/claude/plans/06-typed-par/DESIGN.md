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
by a stitch policy.  The enum has two shapes — a transitional
shape used during phase 3 and a final shape that lands at phase
4c.  Both are documented here so phase-3 and phase-4 designs
agree on what they're building toward.

### D1a — Transitional shape (phase 3a … phase 4b)

```rust
// src/parallel.rs — transitional, lives during phases 3a..4b
pub enum Stitch {
    /// Concatenate per-worker output stores into one result vector.
    /// Carries the legacy element / return sizes that the runtime
    /// still needs while the typed surface (phase 4) has not landed.
    ConcatLegacy { elem_size: u8, ret_size: u8 },

    Discard,
    Reduce { fold_fn: u32 },
    Queue { capacity: u32 },
}
```

`ConcatLegacy` exists only because the worker fn's `Type` is not
yet authoritative for sizes during phases 3a–4b — the parser-side
`parallel_for(input, elem_size, ret_size, threads, fn)` call shape
still feeds them in.  Codegen embeds the sizes at codegen time
(not at runtime), so the opcode payload is a fixed 2 bytes.

### D1b — Final shape (phase 4c onward)

```rust
// src/parallel.rs — after phase 4c
pub enum Stitch {
    /// Concatenate per-worker output stores into one result vector.
    /// Sizes come from `Data::fn_return_type` + `vector<T>`'s element
    /// type; runtime never receives them.
    Concat,

    Discard,
    Reduce { fold_fn: u32 },
    Queue { capacity: u32 },
}
```

Phase 4c renames `ConcatLegacy` → `Concat` and drops the
`elem_size` / `ret_size` payload; codegen reads sizes from the
worker fn's typed signature instead.  Net opcode payload shrinks
by 2 bytes per call.

### Runtime invariants (both shapes)

Run-time enum (not compile-time generics) so codegen emits one
`OpParallel(Stitch)` opcode regardless of policy.  The policy is
selected at parse / scope-analysis time and hardcoded into the
opcode stream; runtime never branches on user data.

**Memory:** the largest variant is `Reduce` at 4 bytes; the opcode
payload is fixed at the variant-max size.

### D1c — Result order is NOT preserved

**Plan-06 par() returns results in completion order, not input
order.**  This is a deliberate design choice: enforcing input
order requires either a per-worker pre-allocated slice (forcing
even chunking; bad for unbalanced workloads) or a serialised
write into a shared result buffer (defeats parallelism).

User contract:

```loft
results = par([1, 2, 3, 4, 5], double, 4)
// results contains {2, 4, 6, 8, 10} as a vector — but order
// is implementation-defined and may vary run-to-run.
```

If a user needs ordered output, they sort post-hoc:

```loft
unordered = par(items, score_of, 4)
ordered = unordered.sort_by(|s| s.input_idx)
```

Or they include the input index in the worker's return value to
re-sort later.  The cost is on the user; the runtime stays fast.

**Implications:**
- Workers write into the result vector in completion order via a
  shared atomic write-cursor (one fetch_add per result).  No
  per-worker slice allocation; no order-preserving stitch pass.
- `Stitch::Reduce` is unaffected — the fold's monoid contract
  already required associativity, which means order doesn't
  matter for the result.
- `Stitch::Queue` (fused for-loop body delivery) **also drops
  order**: the parent body sees `(x, r)` pairs in completion
  order, not input order.  Documented in phase 7's surface doc;
  users who need ordered iteration use a regular `for` loop.

**Why this matters for testing**: with order non-preserving,
**run-to-run output order variation is direct evidence of
parallelism**.  A test that asserts `set(results) == expected_set`
(equal as multiset) AND observes order changes across repeated
runs **proves parallel execution** — see DESIGN.md D8.2 / phase
8f.2 for the test gates.

**Optional ordered policy** (deferred to 1.1+): a
`Stitch::ConcatOrdered` variant could be added if a use case
emerges for ordered output without sort overhead.  Not in
plan-06 scope; the typical workloads (compute → reduce, compute →
hash-by-key, compute → display) don't need it.

**Choice rationale:** trait + vtable dispatch was considered but
rejected — the four policies have very different control-flow
shapes (concat returns a vector store, queue runs a parent body,
discard returns void), so a single trait would need an awkward
"either / or / or" return type.  An enum match in the runtime
dispatcher is simpler and the cost is negligible (one branch per
parallel call, not per worker iteration).

**Binary-format change.**  The `OpParallel` payload format changes
between phases 3a and 4c (2-byte legacy fields drop).  Loft has no
on-disk bytecode cache (`.loftc` was retired in plan-01), so the
change is internal: every `make` rebuilds bytecode from source.
No version pinning needed; the constraint is only "do not commit
phase-3 bytecode dumps as test fixtures and expect phase-4 builds
to read them" — `tests/dumps/*.txt` golden files are regenerated
per build by `LOFT_LOG=static`, so nothing to migrate.

## D2.0 — Language rule: parent stores are read-only to workers

This is the foundational invariant the rest of plan-06 derives
from.  By loft's data-parallel semantics, a worker fn invoked
from `par(input, fn, threads)`:

- **Reads** its input element + any captured non-locals + any
  global / stdlib state.
- **Computes** and returns a value.
- **Cannot affect parent state.**  There is no `par_mut(...)`
  primitive; worker writes never propagate back to the parent.

A worker that *appears* to write to parent state is in one of two
broken modes:

| Path | Behaviour | Semantic verdict |
|---|---|---|
| Full clone (today's default for non-light workers) | Writes hit a worker-private clone of the parent store; clone dropped at join | **Writes silently vanish** — user bug, hidden by the runtime |
| Light borrow (raw pointer) | Writes would race with other workers reading the same store | **Undefined behaviour** |

There is no third path where a worker can legitimately mutate
parent state.  The full-clone path is therefore not an
"alternative execution mode" — it is a compatibility hack that
hides a class of user bugs by silently discarding writes.

**Plan-06 promotes this to language rule, not runtime
convention.**  After phase 5:

- The Rust type system expresses parent stores as `Arc<Store>`
  (read-only by construction); workers cannot acquire a `&mut`
  to a parent store.
- Workers that try to write to non-local state (the analyser's
  `is_par_safe` check) are **compile errors** with a fix-it
  suggestion — not a silent perf-cliff fallback.
- The full-clone path (`clone_for_worker`, `run_parallel_direct`'s
  full variant, etc., ~520 LOC across `parallel.rs` and
  `database/allocation.rs`) is **deleted** in phase 6 because it
  serves no semantic purpose.

**What workers CAN do** (allowed side effects):
- Host I/O (`log_*`, `print_*`) — host bridges serialise.
- PRNG state (`random_int`) — non-deterministic across runs but
  correct.
- Allocate worker-private intermediate stores in their own
  `WorkerStores.worker_owned` (any size; freed at join unless
  adopted as part of the output).

**What workers CANNOT do** (compile errors):
- Mutate non-local variables (`captured_v += [x]`).
- Mutate parent stores via captured `Reference<T>` (writes
  through the reference would race; type system forbids
  `&mut` access to parent stores).
- Call stdlib fns marked `#impure(parent_write)` (e.g.
  `vector_add(non_local_v, x)`).

**Nested `par(...)` IS allowed** (the language rule does not
forbid recursion; only writes to non-local state).  The inner
worker fn is checked by the same `is_par_safe` analyser; the
runtime mechanism is a temporary `Box → Arc` promotion of the
outer worker's `worker_owned` for the inner par's duration —
see D2.1 for the recursion details.  Nested-par cost: <10 µs
per nested call; bounded by `thread::scope`'s stack depth on
the native runtime, removed entirely by phase 1.5's rayon-pool
switch.

See D8 / D8.1 for the analyser; D2.1 for the worker→parent store
relationship that implements this rule.

## D2 — Worker store ↔ parent store relationship

Today (`src/database/allocation.rs:449`): workers get a deep clone
of `Stores` with `locked = true` on every parent-side store.  The
clone is independent but **the entire mechanism is dead weight**
under D2.0's read-only-parent rule — workers can't legitimately
mutate parent state, so cloning the writable bytes serves no
purpose.

After plan-06 (phase 1 onwards): one execution mode, no
fallback.  The relationship has two parts cleanly separated by
ownership:

```
Parent's Stores
  └─ allocations: Vec<Arc<Store>>     ← read-only handles to
                                        every parent store
                                        (stdlib, constants, input,
                                        intermediate parent state)

Worker_N WorkerStores (one per worker thread)
  ├─ parent_view: Vec<Arc<Store>>     ← Arc::clone of parent's
                                        handles; refcount bump per
                                        worker, ZERO buffer alloc,
                                        ZERO memcpy regardless of
                                        parent size
  └─ worker_owned: Vec<Box<Store>>    ← worker-exclusive Stores —
                                        output slot + intermediate
                                        scratch.  Parent never
                                        touches these during par().
```

`WorkerStores` is a single struct with two clearly-typed fields;
no `locked: bool`, no `borrowed: bool`, no per-store flags.  Rust's
type system enforces the invariant — `Arc<Store>` only deref's to
`&Store` (no `&mut` path), `Box<Store>` is exclusively the
worker's.

### Cost per worker (any parent size)

- `Arc::clone` per parent store: ~5 ns × N stores ≈ 100–200 ns total
- `Box::new(Store::new(...))` for output slot: ~1 µs (size-bounded)
- Struct copies for the WorkerStores wrapper: <100 ns

**Total: <2 µs per worker, regardless of parent size** — even at
hundreds of MB.  Compare today's 60 ms full clone of 300 MB
parent state.

### How the output slot works

The output slot is the first entry in `worker_owned`.  Worker
bytecode addresses it via a normal `DbRef { store_nr: N, rec, pos }`
where `N` is the slot's index in the worker's flat allocations
view (parent_view.len() + 0).  Writes via existing
`OpSetInt` / `OpSetText` / `OpSetRef` / `OpVectorAdd` opcodes —
**no opcode surface change**.  The dispatcher hands the worker a
`WorkerOutputSlot { store_nr: N }` marker so the worker's compiled
return path knows which slot to write into.

### Stitch (after join)

Workers can read parent stores via `parent_view`; they cannot
write to them (Arc gives no `&mut` path).  Each worker's
`worker_owned` is exclusively theirs — no contention, no locks.
After all workers join, main thread:

- **Concat policy:** for each worker, `take_slot(0)` → `Store`
  from its `worker_owned`; `Stores::adopt_store(store) -> u16`
  appends to the parent's `allocations`.  Phase 2's rebase walk
  rewrites cross-store DbRefs.
- **Queue policy:** reads worker output stores via the bounded queue.
- **Discard policy:** drops worker `WorkerStores` entirely
  (parent_view's Arc refcounts decrement; worker_owned's Stores
  freed by Drop).

### What's deleted

The full-clone path retires entirely in phase 6:
- `Stores::clone_for_worker` — deleted (was 60 lines).
- `Stores::clone_for_light_worker` — deleted (was 80 lines; the
  raw-pointer borrow + `borrowed: bool` flag is replaced by
  `Arc<Store>`).
- `Store::clone_locked_for_worker` — deleted (was 20 lines).
- `Store::borrow_locked_for_light_worker` — deleted (was 18 lines).
- `Store::locked: bool` and `Store::borrowed: bool` flags — deleted
  (the type system encodes the invariant).
- `run_parallel_direct` and the 5 sibling full-clone variants in
  `src/parallel.rs` — deleted (was ~520 lines).

Net retirement on top of plan-06's already-claimed ~1100 lines:
another ~200 lines of clone/borrow plumbing.  See phase 6 for the
exact accounting.

**What is enforced where.**  The aliasing invariant has three
enforcement layers; D2 is precise about which lands when so phase
2 doesn't over-claim.

| Layer | Mechanism | Lands in |
|---|---|---|
| Rust-runtime | `Arc<Store>` (read-only by construction) for shared / input; the worker's `WorkerStores` exclusively owned by one worker thread, including its output slot which the parent never touches before join | Phase 1 (output slot is exclusively the worker's), phase 2 (parent stops mutating per-worker stores during stitch) |
| Loft-type-checker | Worker fn signature `fn(T) -> U` is `T = input element type`, `U = output value type`; the type checker rejects worker bodies that hold a mutable borrow into the parent's `Stores` table | Phase 4 (typed surface) — pre-phase-4 the type checker has no ParFor-aware rules and falls back to runtime checks |
| Loft-scope-analysis | `is_light_safe` (D8) proves the worker writes nothing outside its own output store | Phase 5 (auto-light analyser) |

Phase 2 closes the **runtime** layer (no more `claims` HashSet);
phase 4 + 5 close the **compile-time** layers.  Until phase 4
lands, the Rust-runtime layer is the only check — D11c's
"compile-time enforceable" claim is conditional on phase 4 having
landed.

### D2.1 — Why the output slot is in the worker's WorkerStores, not a parallel wrapper type

Two designs were considered:

- **(A) Slot-in-WorkerStores** — the output store is allocated as
  a regular slot in the worker's `WorkerStores.allocations`; the
  worker writes to it via ordinary `OpSet*` opcodes addressed by
  a normal `DbRef`; a `WorkerOutputSlot { store_nr: u16 }` marker
  tells the parent which slot to extract after join.
- **(B) Parallel wrapper** — the output store lives in a separate
  `WorkerOutputStore` wrapper that the worker accesses via new
  `OpSet*Output` opcodes (one per write op, ~30 new opcodes).

Plan-06 picks **(A)**.  Reasons:

1. **No opcode surface change.**  Today's `OpSetInt`,
   `OpSetText`, `OpSetRef`, `OpVectorAdd`, etc. — every write
   opcode — already takes a `DbRef`.  If the output store is a
   regular slot in the worker's allocations, those opcodes work
   unchanged.  Design (B) would duplicate every write opcode.
2. **The worker's return value is a DbRef anyway.**  Loft fns
   already return DbRef-shaped values by writing into a slot the
   caller pre-allocated.  Output-slot adoption is the existing
   return convention applied across a thread boundary — not a
   new mechanism.
3. **Drop safety is automatic.**  The worker's `WorkerStores` is
   `Drop`; if the worker panics, the slot's Store is freed along
   with the rest of the worker's allocations.  No Drop
   bookkeeping on a separate wrapper.
4. **Adoption is one Vec entry move.**  `WorkerStores::take_slot(N)`
   yields a `Store`; `Stores::adopt_store(store)` pushes it onto
   the parent's allocations, returns the parent-side `store_nr`.
   No deep-copy, no channel.

The slot-marker is just a `u16` carried in the dispatcher's per-
worker bookkeeping, not a new ownership type.

### D2.1.1 — Recursive Arc promotion for nested par

When an outer worker calls `par(...)` from inside its body, the
mechanism extends recursively.  Outer worker's `worker_owned`
contains its scratch + output Stores as `Box<Store>` — exclusively
the outer worker's.  Inner workers need read access to BOTH
outer's `parent_view` (the grandparent's stores) AND outer's
`worker_owned` (the outer's scratch + accumulated output).

The dispatch step at nested-par entry:

1. **Promote.**  Outer worker walks `worker_owned`, replaces each
   `Box<Store>` with `Arc<Store>` via:
   ```rust
   // Box<Store> → Arc<Store>
   let arc: Arc<Store> = Arc::from(boxed);
   ```
   `Arc::from(Box)` is zero-copy (the Arc takes ownership of the
   Box's allocation; refcount = 1).
2. **Construct inner WorkerStores.**  Each inner worker's
   `parent_view` = `outer.parent_view ++ outer.promoted` (Arc
   clones).  Each inner worker's `worker_owned` is fresh
   `Box<Store>` for its own output + scratch.
3. **Run inner workers.**  Inner workers read outer's promoted
   stores via Arc::deref; they cannot write (no `&mut` path
   from `Arc<Store>` without `get_mut`/`try_unwrap`, both of
   which fail when other clones exist).  Outer worker is
   blocked at the inner par's join — no concurrent access from
   outer to its own (now-promoted) stores.
4. **Demote.**  After inner workers join (`thread::scope` exit
   drops every inner worker's Arc clone), outer worker walks
   the promoted Vec and unwraps:
   ```rust
   // Arc<Store> → Box<Store>
   let boxed: Box<Store> = Arc::try_unwrap(arc)
       .expect("inner workers should have dropped all Arcs at join");
   ```
   `try_unwrap` succeeds because all inner-worker Arc clones were
   dropped at `thread::scope` exit.  Refcount = 1 → demotion
   succeeds.
5. **Adopt inner outputs.**  `take_slot` per inner worker pulls
   the inner output Stores into outer worker's `worker_owned`
   (now back to `Box<Store>`).  Phase-2 rebase rewrites
   cross-store DbRefs within the outer worker's namespace.
6. **Outer worker resumes.**  Continues mutating its
   `worker_owned` normally; the inner par call returns the inner
   result vector.

**Cost per nested call**: `Arc::from + Arc::try_unwrap` per outer
store (~10 ns each) + inner worker construction (<2 µs per inner
worker) + adoption (<1 µs per inner worker).  Total <10 µs
regardless of nesting depth.

**Recursion**: arbitrary nesting depth supported by the same
mechanism applied at each level.  At depth K with M-way fan-out,
data sharing scales linearly (no exponential cloning); thread
spawning scales as M^K (the cost addressed by phase 1.5's rayon-
pool switch).

**Why `Arc::try_unwrap` always succeeds at demotion**: `thread::scope`
guarantees every spawned thread has joined before the scope's
closure returns.  Inner workers' WorkerStores are dropped at
join, decrementing every Arc refcount they held.  By the time
outer reaches the demote step, refcount = 1 (only outer's clone).
`try_unwrap` succeeds with the original Box's allocation
recovered.  `expect` panics only on a runtime invariant violation
that would indicate a deeper bug.

**P1-R5 closure** (THREADING.md's "no Rust-level proof of
non-aliasing"): becomes provable at the **Rust runtime** in phase
2.  Becomes provable at the **loft type system** in phase 4 + 5.
Claims dropped per P1-R3 in phase 2 because the parent-clone's
`claims` HashSet is no longer needed when workers don't share-write.

## D3 — Type-checker access to fn-ref return type

Phase 7's desugar of `par(input, fn, threads)` to `Value::ParFor`
needs `U = fn's return type`.  Phase 4's typed `parallel_for(input:
vector<T>, fn: fn(T) -> U)` needs the same.

**Verified against current source (2026-04-25):** no
`fn_return_type` accessor exists today; `grep -rn fn_return_type
src/` returns nothing.  Phase 4 lands it.  The exact shape depends
on how `Definition` carries return type — the snippet below is
illustrative; the implementer reads `Definition::definition_type`
and the surrounding signature-extraction code in `src/data.rs`
before committing to a shape.

```rust
// src/data.rs — added in phase 4a
impl Data {
    /// Return the result type of a function definition.  None if the
    /// def_nr is not a function (e.g. a struct, a constant).
    pub fn fn_return_type(&self, d_nr: u32) -> Option<&Type> {
        // implementation walks the same fields the parser
        // populates in `parser/definitions.rs::parse_function`.
    }
}
```

**Used by:** phase 7c (desugar's type inference), phase 4 (typed
parallel_for surface).  Other code may use it freely.

**Why not "reuse `map`'s machinery".**  `pub fn map<T, U>(input:
vector<T>, fn: fn(T) -> U)` in `default/01_code.loft` is **a
parser-side compiler special-case**, not generic
monomorphisation.  `src/parser/collections.rs:1490::parse_map`
inlines map calls as `[for elm in v { f(elm) }]`-shaped
comprehensions and infers the return type from the input vector
plus the lambda — no generic substitution machinery executes.
Plan-06 phase 4 cannot "reuse" what isn't there; it must either
- treat `parallel_for` as a parser-side compiler special-case in
  the same way `map` is (the **default** approach: cheaper, no
  new generics infrastructure, same code-gen path), OR
- land bounded-generic substitution as a separate prerequisite
  before phase 4 (estimated ~2 weeks; not in plan-06 scope).

Phase 4's design defaults to option 1 — see 04-typed-input-output.md
§ "Loft-side prerequisites".

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

## D6 — WASM threading: parallel by default, sequential fallback

Plan-06's stance: **`par(...)` must be actually parallel on every
target except no-threads minimal WASM builds.**  WASM is the only
target permitted to run par sequentially — and even then only
when the threading feature isn't compiled in.  Native and
interpreter sequential are bugs (G4 closes the native one);
plan-06 phase 1 makes both real-parallel.

WASM has two compilation modes:

| Mode | Cargo features | Threading | Where used |
|---|---|---|---|
| **wasm + threading** (default for browser deploys) | `wasm`, `wasm-threads` | Real 4-thread Web Worker pool via `wasm-bindgen-rayon` | doc/pkg/ gallery + playground (after phase 8) |
| **wasm minimal** (cdylib, embedded targets) | `wasm` only | Sequential fallback | rare; CI smoke tests only |

The default browser deploy uses **wasm + threading** with a
4-worker pool — phase 8 wires this up and makes it the only real
WASM path users encounter.  The minimal sequential fallback exists
for the narrow case where Web Workers aren't available (no
SharedArrayBuffer / no cross-origin isolation / pre-2022 browsers);
its bench numbers are not load-bearing for plan-06's perf gate.

```rust
#[cfg(all(feature = "wasm", feature = "wasm-threads"))]
fn run_parallel_browser(...) { /* Web Worker pool — phase 8 */ }

#[cfg(all(feature = "threading", not(feature = "wasm")))]
fn run_parallel_native(...) { /* thread::scope — phase 1 */ }

#[cfg(not(any(feature = "threading", feature = "wasm-threads")))]
fn run_parallel_sequential(...) { /* fallback only */ }
```

**Today's gaps fixed by phase 1 + phase 8:**

- **G3** — `--native-wasm` rejects par at codegen (`OpFreeRef not
  found in scope` on the wasm path).  Phase 1's typed pipeline +
  per-worker output stores fix the codegen; phase 8 wires it to
  the Web Worker pool.
- **G4** — `n_parallel_for_native` is sequential by mistake.
  Phase 1 adds the missing `thread::scope` scaffolding.

After both phases, the only acceptable sequential par is **WASM
minimal-feature builds** — every other path is real-parallel.

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

## D8 — `is_par_safe` analyser (defines phase 5)

Per D2.0, a par worker that writes to non-local state is invalid
loft code — its writes vanish (full-clone) or race (light borrow).
Phase 5's analyser is **the language's enforcement of D2.0**, not
a perf heuristic.  The verdict is binary: a worker fn is either
par-safe (compiles) or it is not (compile error with fix-it).

The analyser's name in source: `is_par_safe(d_nr) -> bool`
(replaces the conceptual `is_light_safe` / today's
`check_light_eligible`).

### The rules

| Construct in worker body | Verdict |
|---|---|
| `return expr` where `expr` doesn't reference enclosing mutable state | ✅ par-safe |
| Read from `x` (input parameter) | ✅ par-safe |
| Read from a `pub const`, `pub` global, or stdlib constant | ✅ par-safe |
| Read from a captured non-local (immutable from worker's view by D2.0) | ✅ par-safe |
| Function call to a stdlib fn classified `#pure` or `#impure(host_io)` or `#impure(prng)` | ✅ par-safe (recurse args) |
| Function call to a user fn that is itself `is_par_safe` | ✅ par-safe |
| `vector_add` / `vector_insert` / `hash_set` / `s.field = …` on a **local** variable or on the **worker's output slot** | ✅ par-safe (local IS the worker's output) |
| `vector_add` / `vector_insert` / `hash_set` / `s.field = …` on a **non-local** variable | ❌ compile error |
| Function call to a stdlib fn classified `#impure(parent_write)` | ❌ compile error (transitively writes to parent) |
| Mutation through a captured `Reference<T>` | ❌ compile error (would race) |
| Nested `par(...)` call where the inner worker fn is `is_par_safe` | ✅ par-safe (analyser recurses; runtime promotes outer's `worker_owned` to `Arc<Store>` for inner par's duration; demoted at inner join via `Arc::try_unwrap`) |
| Nested `par(...)` call where the inner worker fn is **not** par-safe | ❌ compile error (same R1 violation as direct call) |
| `LOFT_LOG` / `println` (stderr) | ✅ par-safe (host bridges serialise) |

The pass runs once per fn definition during pass-2 of the
parser, populates `Definition::is_par_safe: Option<bool>`.
Codegen for `par(...)` queries the cached flag; if `false`, the
parser emits the compile error before bytecode generation.

### No fallback path

Plan-06 originally had "if the analyser rejects → take the full-
clone path".  D2.0 removes that escape — the rejected worker is
**invalid code**, not a slow worker.  Diagnostic shape:

```
error: par worker `accumulate` writes to non-local `total`
  --> src/main.loft:42
   |
42 |     fn accumulate(x: Item) {
43 |         total += score_of(x)
   |         ^^^^^^^^^^^^^^^^^^^^ writes to non-local; results vanish at join
   |
   = note: par workers cannot mutate parent state by language design.
           See LOFT.md § Parallel execution for the data-flow model.
   = help: return the value and let par collect it:
   |     fn accumulate(x: Item) -> Score { score_of(x) }
   |     scores = par(items, accumulate, 4)
   |     total = scores.sum()
```

### Recursion handling — fixed-point iteration (phase 5e)

Phase 5b's initial implementation uses a placeholder trick
(insert `false` for the current fn before recursing) which over-
rejects mutually-recursive par-safe fns (`is_even` / `is_odd`-shaped
pairs).  Phase 5e replaces this with monotonic fixed-point
iteration over the call graph (D12 caller-graph infrastructure):
every user fn starts optimistically par-safe; demotions propagate
via a worklist; safe cycles stay safe, unsafe cycles correctly
demote.  See phase 5's detail file for the algorithm.

**5e is correctness, not optimisation** — without fixed-point
iteration, mutually-recursive pure fns fail to compile inside par.

### Test coverage

Phase 5 adds positive (par-safe) and negative (par-unsafe)
fixtures.  Negative fixtures must produce the exact diagnostic
text shown above.  Phase 5e adds mutual-recursion fixtures.

## D8.1 — `#impure` sub-classes

Plan-06's earlier draft had binary `#pure` / `#impure`.  D2.0
makes a finer distinction necessary: stdlib fns with
**observable** side effects (logging, PRNG) are valid in par
workers; only fns that **write to parent stores** are invalid.

Sub-classes added in phase 5a:

| Annotation | Examples | Allowed in par worker? |
|---|---|---|
| `#pure` | `min`, `max`, `format`, `length`, type conversions, pattern destructure | ✅ Always |
| `#impure(host_io)` | `log_warn`, `print`, `println`, file `read_*` | ✅ Host serialises |
| `#impure(prng)` | `random_int`, `random_float`, `random_seed` | ✅ (non-deterministic across runs; documented) |
| `#impure(io)` | `write_file`, `delete_file`, network ops | ⚠️ Allowed; user accepts I/O is parallel |
| `#impure(parent_write)` | `vector_add`, `vector_insert`, `hash_set`, `vector_remove` | ❌ Compile error if first arg is non-local |
| `#impure(par_call)` | `par`, `par_fold`, `parallel_for` | ✅ Allowed — analyser recurses into the inner worker fn and applies the same par-safety check; runtime uses the recursive Arc-promotion mechanism in D2.1 |

**`parent_write` requires a per-call check, not a per-fn check.**
`vector_add(local_v, x)` is fine; `vector_add(captured_v, x)` is
not.  The classifier is "fn taints first arg if it writes through
it"; the per-call check is "is the first arg local-or-output, or
is it captured-from-parent".

The audit fixture (`tests/issues.rs::par_phase5a_purity_audit`)
walks every stdlib fn and asserts its sub-classification matches
the operative table.  Missing annotation = CI failure.

**Backward-compat with the older binary `#pure` / `#impure`.**
Phase 5a introduces the sub-classes as the canonical form; bare
`#pure` becomes an alias for `#pure`; bare `#impure` becomes an
alias for `#impure(parent_write)` (the most conservative
interpretation — anything not classified is assumed to write to
parent state).  Phase 5b replaces every bare annotation with an
explicit sub-class; phase 6 removes the alias.

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
| `vector<(T, U)>` (tuple element) | ❌ rejected today | ✅ | phase 9 — tuple records have layout via `Type::Tuple::element_size`/`element_offsets`; same stride machinery as struct elements |
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
| `(T, U)` tuple return | ❌ rejected today | ✅ | phase 9 — T1.8a function-return convention writes the tuple record into the worker's per-worker output Store; rebase walks tuple `owned_elements` like struct ones |

### D11c — Reference graph rules

A worker's result may carry references that fall into one of three
categories — D11c spells them out so phase 2's rebase walk and
phase 9's tuple-element handling agree.

| Category | Example | Allowed? | Stitch behaviour |
|---|---|---|---|
| **Worker-own** — reference into the worker's own output Store | Worker returns `Point { name: text, … }`; the `name`'s text bytes live in the worker's output Store | ✅ Allowed | Phase 2 rebase translates the `store_nr` from worker-local to parent-side |
| **Parent-shared** — reference into a parent-side Arc-borrowed read-only Store (stdlib constants, input store, parent-allocated read-only data) | Worker returns `(score: integer, ref: Reference<GlobalConfig>)` where `GlobalConfig` lives in a parent stdlib store | ✅ Allowed | Phase 2 rebase **does not translate** — the `store_nr` already names a parent store; the rebase map's lookup misses, and the field passes through unchanged |
| **Cross-worker** — reference into **another** worker's output Store | Worker A returns a `Reference<X>` whose `store_nr` names worker B's output | ❌ Forbidden | Phase 2 rebase has no entry to translate to and treats this as a runtime error in debug builds |

The rebase walk distinguishes worker-own from parent-shared by
**lookup in the worker's own rebase entry**: each worker output
Store gets exactly one entry `(worker_id, worker_local_store_nr)
→ parent_store_nr`.  A DbRef field whose `store_nr` matches that
entry's worker-local key is worker-own (translate); a DbRef field
whose `store_nr` matches **any parent-side store_nr** is
parent-shared (pass through); anything else is cross-worker
(runtime error).

**Cross-worker is forbidden by construction**, not just
discouraged: the worker only has Arc references to parent stores
(read-only) and `&mut` access to its own output Store.  It has no
handle to peer worker stores; the type system gives it no way to
construct such a reference.  The runtime check exists only as a
defence against codegen bugs.

**The restriction is enforceable at compile time** once D2 layer-2
(loft type checker, phase 4) and D2 layer-3 (scope analyser,
phase 5) both land — the worker fn signature constrains its
return type to reference only its own exclusive output store or
read-only parent stores.  Until then, the runtime check is the
only guard.

**Why allow parent-shared at all.**  Many real workers compute
lookups against shared read-only state (e.g. a global config, a
dictionary, a constant table).  Forbidding parent-shared
references would force every worker to copy that state into its
own output, defeating Arc-borrow's whole purpose.  The cost is
one extra branch per DbRef field at stitch time; cheap.

### D11c.1 — Tuples and references

Phase 9's tuple returns inherit D11c verbatim: a tuple element of
type `Reference<T>` follows the same three-category rule.
`(integer, Reference<ParentSharedStruct>)` is **allowed**;
`(integer, Reference<WorkerOwnedStruct>)` is **allowed**;
`(integer, Reference<PeerWorkerOwnedStruct>)` is **forbidden** —
identical semantics to a struct field.  Phase 2's rebase walks
tuples via `data::owned_elements` (returns `(offset, index)` pairs
for elements that need cleanup) and treats each owned element
exactly like a struct field.

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

## D12 — Caller-graph infrastructure (prerequisite for phase 5e)

Phase 5e's fixed-point iteration needs two `Data` accessors that
do not exist today (verified by `grep -rn 'user_fn_d_nrs\|callers_of\|caller_graph' src/`
which returns nothing as of 2026-04-25):

```rust
// src/data.rs — added in phase 5b' (sub-phase before 5e)
impl Data {
    /// Every user-defined function's def_nr (excludes stdlib + native).
    pub fn user_fn_d_nrs(&self) -> &[u32];

    /// Every user fn that calls `d_nr`.  Built lazily on first call,
    /// cached for the program's lifetime.  Linear in call-graph edges.
    pub fn callers_of(&self, d_nr: u32) -> &[u32];
}
```

**How `callers_of` is built.**  Walk every fn body once, collect
every `Value::Call(callee, _)` and `Value::CallRef(callee, _)`,
record `(callee, caller)` pairs, invert the index by callee.

**Cost.**  For loft's stdlib (~150 fns) plus a typical user
codebase (a few hundred fns) the walk runs in <50 ms.  The
inverted index is `HashMap<u32, Vec<u32>>` — linear in the
edge count of the call graph.  Cached on `Data` after first build;
recomputed only when the parser adds new fn definitions (which
doesn't happen post-load).

**Why this is its own design item, not folded into phase 5e.**
Phase 5e is the *user* of the caller graph; phase 5b' is the
*provider*.  Keeping them separate means (a) any future analysis
pass (purity, escape analysis, dead-code) can reuse the same
graph; (b) the build cost is paid once even if multiple analyses
run; (c) the Data accessor's contract is testable independently of
phase 5e's algorithm.

**Phase ordering.**  Phase 5 lands as 5a (purity annotations) →
5b (single-pass analyser using the cache placeholder trick) → 5b'
(caller-graph accessors) → 5c (codegen wiring) → 5d (diagnostic) →
5e (fixed-point iteration that **uses** 5b'). Phase 5b's analyser
is replaced by 5e's; 5b' lives between them as the prerequisite.

## D13 — SAB transfer + DbRef rebase across the worker boundary

Phase 8's Web Worker pool transfers per-worker output Stores via
`postMessage` with `Transferable` SharedArrayBuffer-backed
buffers.  Two facts make the transfer non-trivial:

1. A loft `DbRef` is `(store_nr: u32, rec: u32, pos: u32)`.  The
   `store_nr` is **absolute** in the owning runtime's store table.
2. Each Web Worker has its own runtime instance with its own
   store table.  A worker-local `store_nr` is **not the same
   number** in the parent's store table.

If a worker's output Store contains DbRefs (any field of type
`Reference<T>`, `Vector<T>`, `Hash<T>`, etc.), the raw bytes of
those `store_nr` fields are wrong in the parent's address space
after a `Transferable` postMessage.  Bare SAB transfer is **not
sufficient** — every DbRef field in the transferred buffer must
be rewritten before the parent reads it.

### D13a — The browser stitch path uses phase-2 rebase

After `postMessage`-receive of a worker's output slot's SAB-backed
buffer, the parent reconstitutes the buffer as a `Store`, calls
`Stores::adopt_store(store) -> u16` to install it (per D2.1), and
runs the same rebase walk from phase 2 with the same
`(worker_id, worker_local_store_nr) → parent_store_nr` map.  Bytes
move zero-copy via SAB transfer; DbRef fields get rewritten by
the rebase walk.

Inside the Web Worker, the worker's `WorkerStores` is constructed
with the output slot at a known index `N`; the worker's bytecode
writes into `allocations[N]` via the same `OpSet*` opcodes as any
other slot.  No browser-specific opcode path.

**Cost analysis.**  The rebase walk is one pass per worker output
Store, walking every `owned_elements` field.  For 4 workers each
with a 100 K result vector, that's 400 K field rewrites — same
cost as the native path's rebase walk (~20 ms on the bench host),
plus the postMessage transfer (~1 ms for a 1 MB SAB transfer in
modern browsers).

### D13b — Primitive-only outputs skip the rebase

Workers whose output type contains no `Reference`/`Vector`/`Hash`/
`Sorted`/`Index`/text fields produce a primitive-only output
Store; phase 2's rebase walk is a no-op for primitives (D11b
"primitive scalars" row).  In the browser, this is the optimal
path — pure SAB transfer, no field rewrites, parent reads the
SAB-backed buffer directly as the result vector backing.

### D13c — Cross-origin headers and cache coherence

The Web Worker pool requires SharedArrayBuffer, which requires
COOP/COEP cross-origin isolation (D6).  Phase 8d sets the headers
via `<meta http-equiv>` tags on `doc/gallery.html`,
`doc/playground.html`, `doc/brick-buster.html`.

**Cache-coherence risk.**  GitHub Pages serves HTML and WASM
assets from the same origin but does not version-pin them.  If the
HTML is cached pre-COOP/COEP and the WASM is updated post-phase-8,
the page loads without `crossOriginIsolated === true` and
`SharedArrayBuffer` is undefined — the WASM module errors at pool
init.

**Mitigation: filename-pinned WASM bundles.**  `wasm-pack` already
emits hashed filenames (`loft_wasm_bg.<hash>.wasm`).  Phase 8d
ensures the `<script src=…>` reference in each HTML page is
regenerated alongside the WASM bundle, so any HTML/WASM pair is
mutually consistent.  An older cached HTML references the older
hashed WASM (which still works); a fresh load gets the new pair
together.  Verified by phase-8d's CI step: after `make gallery`,
`grep loft_wasm_bg gallery.html | grep -o 'loft_wasm_bg\.[a-f0-9]*\.wasm'`
must equal the file actually shipped to `doc/pkg/`.

**Browser fallback when SAB unavailable.**  If
`crossOriginIsolated === false` at runtime (mismatch, embedded
webview, older Safari), the JS shim falls back to the sequential
WASM path from D6 with a warning to the JS console — no crash, no
silent wrong-answer; the user sees slower-but-correct execution.

## D14 — Scale considerations (huge parent + small output)

The canonical workload plan-06 targets is **huge parent state +
small per-element output** — e.g., a 200 MB const store of asset
data scanned by 1 M parallel workers each producing a few bytes
of result.  Under D2.0's read-only-parent rule and D2's `Arc<Store>`
mechanism, this workload is **trivially fast**: per-worker
overhead is bounded by metadata (Arc bumps + struct copies),
not by parent-data size.

### D14a — Cost profile (only one path exists)

For 300 MB of parent state, 4 workers, small per-element output:

| Component | Cost | Why |
|---|---|---|
| `Arc::clone` per parent store | ~5 ns × ~30 stores = ~150 ns | Refcount bump only |
| `Box::new(Store::new(...))` for output slot | ~1 µs | Small bounded alloc |
| `WorkerStores` struct copies | <100 ns | Plain Rust struct |
| `Arc::clone(types)` + `Arc::clone(names)` (D14b) | ~10 ns | Refcount bumps |
| **Total per worker, all parent sizes** | **<2 µs** | **Independent of parent data size** |
| **Total for 4 workers** | **<8 µs** | |
| **Peak memory** | parent + ~16 MB output buffers | No cloning of parent data |

The 240 ms cliff from the full-clone path is **structurally
gone** — there is no path to reach it.  Workers that today take
the full path are compile errors under D2.0/D8.

### D14b — `types` / `names` Arc-wrap is in-scope, not deferred

Earlier draft of D14 marked `Stores.types: Vec<Type>` and
`Stores.names: HashMap<String, u16>` as a deferred Arc-wrap
optimisation.  Under D2.0 they're cloned per worker even on the
new path; the per-worker overhead is dominated by their HashMap
clone (~1–5 ms) unless wrapped.  Phase 5 lands the wrap as part
of the WorkerStores construction work — `Arc<Vec<Type>>` and
`Arc<HashMap<String, u16>>` for both fields, refcount bump per
worker, no data copy.

After this:
- Worker construction = `Arc::clone × N + Box::new × 1` = sub-µs.
- Workers can READ both `types` and `names` via `Deref`.
- Workers can never WRITE to either (Arc gives no `&mut`).

Trivial change (~10 LOC); folded into phase 5b.

### D14c — Browser path: SAB allocator is the only structural blocker

Workers in Web Workers run in separate WASM linear memories.
Parent stores allocated via the system allocator are invisible
to workers — the `Arc<Store>` mechanism doesn't help across
linear-memory boundaries.

Plan-06 phase 8 must allocate parent stores from a
SharedArrayBuffer-backed allocator when `wasm-threads` is
enabled.  This is the only design issue at scale that survives
D2.0 — even with the read-only-parent rule, the bytes have to
physically be reachable from the worker thread.

**The decision is sticky**: parent stores allocated pre-SAB
cannot be retrofitted without a 300 MB copy.  The const store
must be SAB-backed from program start under `wasm-threads`.
See phase 8 sub-phase 8a' for the allocator integration plan.

### D14d — Output sizing is a non-issue

Small per-element output means the output slot's initial
allocation is bounded by `output_element_width × per_worker_input_share`.
For 1 M input × 8-byte output = 8 MB total; per worker = 2 MB.
Trivial.  No need for slot pooling, growth strategies, or
adaptive sizing.

### D14e — What this means for plan-06's perf gates

Phase 0's bench harness asserts ±5% on `bench/11_par`.  Under
D2.0, that gate is loose — the new path's cost is independent of
parent size, so the bench should show large improvements at any
non-trivial parent state.  Phase 5 adds the explicit
`bench/par_huge_parent_small_output` fixture (300 MB const store
+ 1 M input scan + 8-byte output) asserting:

- Total wall time = `worker_compute_time + <10 µs overhead`.
- Peak resident memory = `parent + <16 MB`.
- `LOFT_STORES=warn` reports zero leaked stores.

A regression here is a structural correctness issue (the path
fell back to cloning), not a tuning miss.

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
