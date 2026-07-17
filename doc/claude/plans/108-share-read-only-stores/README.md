<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 108 — Share read-only parent stores across par workers

Tracker: [@PLN108](https://github.com/loft-lang/plans/issues/108) · `subject:loft` · `status:ready`

## Status

**Design ready, step 1 ANSWERED, S0 (win-baseline bench) LANDED — next is S1 (the audit).**
Cherry-picked from the sibling `../loft` checkout into this working tree (the named code is
in sync:
`clone_for_worker` `allocation.rs:1277`, `borrow_locked_for_light_worker` `store.rs:1292`,
`clone_for_light_worker` `allocation.rs:1361`, `run_parallel_discard` `parallel.rs:1184`,
`run_parallel_light` `parallel.rs:1451`). This is the deferred **phase 5** of the legacy
typed-par plan ([`finished/06-typed-par/`](../finished/06-typed-par/README.md)):
`parallel.rs:99` calls it "Phase 5's `Arc<Store>` rewrite makes every path light by
default." The auto-light *heuristic* shipped; the parent-store **sharing** it was meant
to unlock did not. The live par dispatch (Queue family + heavy) still `clone_for_worker()`s
a **byte-copy of every active parent store per worker**. Filed as its own plan because it
is a **memory-model change (loft priority #1)**, not a spot fix; ASan/TSan are load-bearing
gates, not optional.

## Goal

Let a par worker read a provably-unwritten captured parent store **directly**,
eliminating the per-worker byte-copy — a copy-elision with no semantic change.

## Effort + design

- **Effort:** M (Option A) → H if it escalates to Option B.
- **Design:** ✓ (detailed below; A-vs-B decision is the one open question).
- **Last touched:** 2026-07-17 (cherry-picked + gated execution plan added).

## Step 1 — ANSWERED by the routing consumer: NOT a rounding error

The design's step 1 gates the whole plan: *"measure the copy cost — if it's a rounding
error for the consumer's shapes, stop here."* The **routing** consumer (`../routing`)
measured it and reported: **it is not a rounding error — do not stop.**

A `par` over 64 elements whose worker touches **none** of the big structure — so any
time growth is **pure `clone_for_worker`** (a fresh copy of the parent heap per worker):

| unrelated live heap | 0 MB | 15 | 30 | 61 | 122 |
|---|---|---|---|---|---|
| **par_ms** (8 threads, ±2) | 2 | 40 | 98 | 101 | 205 |

| threads @ 61 MB | 1 | 4 | 8 | 16 |
|---|---|---|---|---|
| **par_ms** | 36 | 60 | 98 | 178 |

**`par` gets 5× slower from 1 → 16 threads** on a workload the workers don't even read.
Each worker's marginal cost is a full copy of the parent heap, and it **swamps the
parallel win — `par` is inverted**: adding threads makes it slower. This is the exact
failure the copy-elision removes. Step 1 is therefore **PASS-to-proceed** — reproduce it
as a committed regression bench (S0 below) so the win is measurable and guarded.

## Composition matrix — Stage A

**No new composition surface** — a copy-elision of the memory model, not a new value /
type / operation, so the standard both-backends value-matrix does not apply. The
acceptance instrument is a **data-race + lifetime gate matrix** (§ Gates): ASan for the
borrow-lifetime/UAF axis, TSan for the shared-read race axis (each with a firing positive
control), plus the existing par order/value tests unchanged on both backends — a data race
is the one fault loft cannot null-out, so the gate *is* the spec.

## Why it is safe to share — the proof already exists

A par worker's captured parent state is **read-only** by @PLN102 **C93**: a `ParentWrite`
from inside a worker is a *compile error* (`scopes.rs`; the `Impure(ParentWrite)` purity
is rejected in worker context). So every parent store a worker sees is **provably
unwritten for the lifetime of the par**. The current per-worker byte-copy is therefore
pure conservatism — nothing the worker does could observe a divergence between its copy
and the shared original. Sharing does not *weaken* an invariant; it *rests on* one the
compiler already enforces. Two facts make the lifetime sound:

- **`thread::scope`** — every dispatcher spawns workers inside a scope and **joins them
  before the scope ends** (`parallel_workers`). The parent `Stores` outlives every worker
  by construction; a borrowed pointer cannot dangle.
- **Synchronous par** — the main thread blocks in the join; it does **not** run (so cannot
  mutate a parent store) while workers borrow. (Must VERIFY no dispatcher
  materialises/adopts a parent store mid-par before the join — **S1**.)

## Current state (what exists, what's live)

| Piece | State | Note |
|---|---|---|
| `clone_for_worker()` (`allocation.rs:1277`) | **LIVE default** | byte-copies every active store (`clone_locked_for_worker` `store.rs:1256` → `alloc` + `copy_nonoverlapping`) |
| `borrow_locked_for_light_worker()` (`store.rs:1292`) | exists, **dead** | SHARES `self.ptr`; `read_only:true`, `borrowed:true` (`Drop` does NOT dealloc — `store.rs:243`). Tests: `borrow_locked_reads_original_data`, `borrow_locked_write_panics` (`store.rs:3094`/`3115`) |
| `clone_for_light_worker(pool_slice)` (`allocation.rs:1361`) | exists, **dead** | borrows parents + a pre-allocated pool for worker-owned stores |
| `run_parallel_light` (`parallel.rs:1451`) | exists, **dead** | no live caller; off the `parallel_workers` template pending this rewrite |
| `Store.ptr: *mut u8` | raw owned buffer | `Store: Send`, **NOT `Sync`** |

So the sharing machinery is **built and tested** — parked, not the default, waiting for a
clean lifetime story. This plan provides it.

## Two options

### Option A — revive the existing unsafe borrow (smaller, reuses tested infra)

Route the live dispatch through `clone_for_light_worker` (borrow parents + pool) instead
of `clone_for_worker`. The safety already lives in the flags: `read_only:true` (a worker
write panics, guarded by `borrow_locked_write_panics`) + `borrowed:true` (no double-free).
Needs `unsafe impl Sync for Store` — **justified only because the shared access is
read-only** (workers call `addr()` only); the write-panic guard is the runtime backstop.

- **Cost:** small — wire an existing path; no `Store` representation change.
- **Risk:** safety is *contract-carried* (the `unsafe` borrow trusts C93 + `thread::scope`
  + read-only), not *type-carried*. A future edit that mutates a parent mid-par, or lets a
  worker outlive the scope, reintroduces UAF/race silently. Mitigated by the write-panic
  guard + the ASan/TSan gates — but it is trust, not proof.

### Option B — `Arc<Store>` rewrite (the clean end-state `parallel.rs:99` names)

Make a parent store's buffer **`Arc`-shareable**, so sharing is safe *by construction* —
`Arc` owns the buffer, frees it once when the last worker + parent drop it, and
`Arc<T>: Sync` for read-only access. No `borrowed` bookkeeping, no hand-justified
`unsafe impl Sync`.

- **Cost:** larger — `Store.ptr: *mut u8` becomes an `Arc`-backed buffer for the shared
  read-only range (touches allocation/free/word-addressing), or wrap the parent range as
  `Arc<Store>` in `WorkerStores` (pushes `Arc<Store>` through the `allocations` access
  surface — wide, since `Stores` is shared main+worker).
- **Risk:** wide blast radius on the store representation; but safety is *type-carried*
  (the borrow checker + `Arc` prove no UAF / no double-free) — strictly better than A.

## The invariant (one hypothesis) + its brittleness count

> **A par worker addresses a captured parent store's original buffer directly; the read is
> sound because C93 proves the buffer is unwritten and `thread::scope` proves it outlives
> the worker. Copy-elision, no semantic change.**

**Re-assertion sites (N).** Each `clone_for_worker()` call site that switches to the borrow
path is a place the invariant must hold. loft2 has **6** live sites (`parallel.rs:149, 209,
734, 1092, 1125` + `run_parallel_discard:1184`). Omitting the audit at any one is *silent*
(a race, not a compile error) — so `N × silence` is the brittleness, and the plan drives it
down two ways: **(a)** migrate ONE dispatcher at a time behind a flag (N=1 live at a time),
**(b)** make omission loud where possible — the `read_only` write-panic converts a *worker
write* into a crash, and the TSan gate converts a *race* into a test failure. The `unsafe
impl Sync` is the single type-level assertion; everything else must be *checked*, which is
why the audit (S1) and TSan positive-control (S6) are load-bearing, not ceremony.

## Safe small steps (gated execution plan — A first, behind a flag)

Each step is minimal, independently verifiable, and **reversible** (`LOFT_PAR_SHARE`
default-OFF until proven). Safety is established *before* the optimization is switched on.
The audit is pulled to the FRONT (design-protocol: probe the load-bearing claim before
building on it).

| S | Step | Verify (gate) | Rollback |
|---|---|---|---|
| **S0** ✅ | **Repro bench = committed win baseline.** `bench/par_copy_probe.loft` + `bench/run.sh` (co-located, NOT a CI gate — timing is machine-dependent). Ported from `../routing/tools/par_copy_probe.loft`. | **DONE** — runs both backends; reproduces the inversion (see § S0 baseline). | delete `bench/` |
| **S1** | **Safety audit — NO code change.** For each of the 6 live `clone_for_worker` sites + `run_parallel_discard`, write down: does the dispatcher *materialise / adopt / lazily grow* a parent store between spawn and join? Produce a table (dispatcher × "parent untouched during join? Y/N"). Any `N` is a blocker: move that work BEFORE the spawn, or exclude that dispatcher from sharing. | The audit table is committed to this plan. A confirmed `Y` for `run_parallel_discard` is the green light for S3. | — (doc only) |
| **S2** | **`unsafe impl Sync for Store`** with a comment pinning the justification (read-only shared `addr()` access + `read_only` write-panic backstop) and naming @PLN108. No sharing path goes live yet. | Compiles; `borrow_locked_reads_original_data` + `borrow_locked_write_panics` green; **ASan baseline 47/47 unchanged** (no new live sharing). | revert the `impl` |
| **S3** | **Wire Option A behind `LOFT_PAR_SHARE` (default OFF) on `run_parallel_discard` ONLY** (no return-stitch → smallest surface). Flag ON → route through `clone_for_light_worker` (borrow + pool); flag OFF → `clone_for_worker` (today's copy, byte-identical behaviour). | Flag OFF: **entire threading suite green, unchanged**. Flag ON: `parallel_store_is_read_only_in_workers` + `par_discard_does_not_grow_parent_stores` green. | unset the flag (default) |
| **S4** | **S1 audit follow-through for discard** — if S1 flagged a mid-par parent touch in the discard path, move it before the spawn now; else assert (in a comment) the join-before-touch ordering at the call site. | discard path: no parent-store write/adopt between `scope.spawn` and join (code-read + comment). | — |
| **S5** | **ASan with the flag ON** (`scripts/asan.sh -E 'binary(threading)'`) — the borrow-lifetime / UAF axis, `LOFT_PAR_SHARE=1`. | **47/47** (must match baseline). | flag off |
| **S6** | **TSan with the flag ON + positive control** (@PLN54 S2: `-Zbuild-std` + target-scoped `-Zsanitizer=thread`) over the threading suite — the shared-read race axis. Include a **temporarily-patched positive control** (let a worker write a parent) and confirm **TSan FIRES**, then revert it, so the clean run is non-vacuous. | clean TSan with flag ON **AND** the positive control fires when armed. | flag off |
| **S7** | **Bench the win vs S0** — re-run S0 with `LOFT_PAR_SHARE=1`. | `par_ms` no longer grows with unrelated heap; `par` scales *down* with threads (inversion gone). | flag off |
| **S8** | **Flip `LOFT_PAR_SHARE` default-ON for `discard`** once S5+S6+S7 pass. | full suite **both backends** green with default-on. | flip default back to OFF |
| **S9** | **Extend one dispatcher at a time** — Queue family, then heavy. For EACH: repeat S1(audit)→S3(wire)→S5(ASan)→S6(TSan)→S7(bench) for that dispatcher before enabling it. Unify `run_parallel_light` (`parallel.rs:1451`) into the `parallel_workers` template (`parallel.rs:99`). | per-dispatcher: same gate set green before its default-on. | per-dispatcher flag/default |
| **S10** | **Decide A-vs-B.** If S1's audit or any S6 TSan run showed the contract-carried safety (A) is fragile (a real mid-par mutation, a race the write-panic can't catch), escalate to **Option B** (`Arc<Store>`) as its own follow-on. Otherwise A stands. Record in `DESIGN_DECISIONS.md`. | decision written to `DESIGN_DECISIONS.md` with the evidence. | — |

### S0 baseline (recorded 2026-07-17, `bench/run.sh`)

Reproduced in THIS tree; near-identical to the routing consumer's numbers. `total` is
constant across every cell (identical work) — the moving column is pure copy cost.

| unrelated heap | 0 MB | 15 | 30 | 61 | 122 |
|---|---|---|---|---|---|
| **interpret par_ms** (8 thr) | 5 | 51 | 104 | 107 | 231 |
| **native par_ms** (8 thr) | 1 | — | 89 | 102 | — |
| *routing par_ms* | 2 | 40 | 98 | 101 | 205 |

| threads @ 61 MB | 1 | 4 | 8 | 16 |
|---|---|---|---|---|
| **interpret par_ms** | 54 | 71 | 98 | 184 |
| *routing par_ms* | 36 | 60 | 98 | 178 |

**Two findings the bench surfaced:**

1. **Native par clones too.** `par_ms` grows with the unrelated heap on `--native` as well
   (`n_parallel_queue_native`), so the copy cost is **both-backends** — the interpreter is
   where the sharing infra (S2–S9) lives, but a native follow-on may be needed for parity.
2. **Native rejects a VARIABLE thread count.** `par(expr, var)` emits an `i64` where
   `n_parallel_queue_native` wants `i32` → `rustc E0308`; a literal (`par(expr, 8)`) is fine.
   The bench bakes a literal per thread-sweep value to work around it. This is an
   independent native-codegen bug (cast the thread-count var to `i32`), not part of S0 —
   note it for a separate fix.

**Why this order is the safe one.** S0 makes the win measurable *before* touching the
memory model. S1 (audit) is the falsification probe of the invariant's lifetime claim and
runs *before* any wiring — if a dispatcher mutates a parent mid-par, the invariant is false
and the plan changes shape, so we learn it at zero cost. S2–S4 land the mechanism **dead**
(flag OFF) so nothing can regress. S5–S6 are the data-race spec — the only gates that can
prove a copy-elision of a *shared* buffer is race-free — and the TSan positive control keeps
the clean run honest. Only S8 lets a user reach the new path, and only after both
sanitizers and the win-bench agree. S9 keeps `N`-live at one dispatcher per gate cycle.

## Gates (mandatory — this is the data-race class)

- **ASan** (`scripts/asan.sh -E 'binary(threading)'`) — the borrow-lifetime / UAF check.
  Baseline **47/47 green** (2026-07-17); the change must stay 47/47 (S5).
- **TSan** (`-Zbuild-std` + target-scoped `-Zsanitizer=thread`, @PLN54 S2) — the shared-read
  race check. A data race is the one fault loft cannot null-out (DESIGN_DECISIONS.md), so a
  clean TSan run over the threading suite is the acceptance bar, with a **positive control**
  (temporarily let a worker write a parent → TSan must fire) so the clean run is non-vacuous
  (S6).
- Full suite on **both backends** (interpreter + native).

## Invariants the change must not break (acceptance checklist)

- A worker **never writes** a parent store (C93 compile error + `read_only` runtime panic).
- Parent `Stores` **outlive** every worker (`thread::scope` join before drop) — no dangling borrow.
- No **double-free** of a shared buffer (`borrowed:true` ⇒ `Drop` skips dealloc; Option B: `Arc`).
- No dispatcher **mutates a parent** between spawn and join (S1 audit).
- Result **ordering + values unchanged** (copy-elision, not a semantics change) — existing order/value threading tests stay green on both backends.
- ASan 47/47 + TSan clean (with a firing positive control).

## Open design questions

1. **A vs B** — resolved by S1 + S6: contract-carried (A) stands unless the audit or TSan
   shows it is fragile, then escalate to type-carried (B). Records into `DESIGN_DECISIONS.md`
   when decided (S10).

## Cross-arc dependencies

- **@PLN54** (sanitizer coverage) — supplies the ASan + TSan gates this plan's acceptance rests on.
- **@PLN102 C93** (captured state is read-only) — the compiler-enforced invariant the whole safety argument rests on.

## See also

- [`finished/06-typed-par/`](../finished/06-typed-par/README.md) — the shipped phase-5 auto-light heuristic (the sibling this completes).
- [THREADING.md](../../THREADING.md) § Multi-threading Safety — the intended model (read-only shared, deep-copy conservative).
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C93 — captured state is read-only (the proof).
- Consumer: `../routing` — the par workload whose measurement answered step 1.
- Tracker: [@PLN108](https://github.com/loft-lang/plans/issues/108).
