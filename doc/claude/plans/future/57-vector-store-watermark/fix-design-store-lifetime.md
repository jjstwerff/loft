<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Fix design — free a vector store's heap allocation at last-use (clusters I + III)

## Why this matters (the real stakes)

This is the implementation of **[GOALS.md Goal E — Predictable memory](../../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)**
(the programmer's model *is* the truth).  Not a watermark cosmetic.  A
block-scoped vector that lives until **function exit**
means a running program holds **more heap than the source implies** — and a memory
model the author can't reason about is a liability in itself, especially for a
data-handling language.  The goal is to make heap lifetime match the obvious mental
model: **a vector's store is freed as soon as its last use is past**, regardless of
how the stack slot is allocated.

This unifies **cluster I** (block-confined local pinned to function exit) and
**cluster III** (reassignment doesn't free the overwritten store) — they are the
**same fix surface**: *free a store at the last use of the data it holds.*

## Root cause (pinned, with sites)

The store backing a vector local is the `__vdb_N` work-ref allocated by
`vector_db` (`src/parser/vectors.rs:2137`).  Two mechanisms lift it **and** the
local to **function scope**:

1. `work_references()` **hoist** — the `__vdb_N` null-init is inserted at
   **function-body position 0** (`src/parser/expressions.rs:354`), so `scan_set`
   registers it at function scope.
2. `scan_if`'s **`small_both` pre-registration** (`src/scopes.rs:827-850`) lifts
   small block locals to the body scope.

Both are **load-bearing for the V1 slot allocator** (`src/scopes.rs:188` is
explicit: zone-1 pre-pass depends on outer-scope registration).  So the heap
store's lifetime is **coupled to the function-scoped stack slot**: the block-exit
free sweep (`free_vars`, `src/scopes.rs:937`) never sees the `__vdb` (it's at
function scope), and the store frees only at function teardown.

Loops escape the watermark not by freeing but by **reuse**: a loop body is scanned
once → one `__vdb` reused in place via `OpDatabase` clear+claim each iteration.

## The design principle

**Decouple HEAP-store lifetime from SLOT-scope lifetime.**

- The stack **slot** stays function-scoped — the allocator needs it; do **not**
  fight the lifting (it's load-bearing).
- The heap **store** is freed at the **last use of the local that points into it**
  (cluster I: the local's last read; cluster III: the point of the overwrite).

## Verification harness (already shipped: `0a45e880`)

`LOFT_STORE_GUARD=1` fires when a vector local's references are confined to one
non-loop nested block yet its `__vdb` store is scoped to an ancestor.  Built on the
`guard_refs` confinement walk (mirrors `compute_intervals`' traversal so no
Value-bearing variant is missed).  Read-only, gated, zero behaviour change.

- Fires on the cluster-I shape: `one_block`, `many_blocks` (5), `168` (7), `03-text` (6).
- **Silent on escape cases** (a local read after its block).
- **Does NOT fire on `11-vectors`** — that watermark is function-level sequential
  locals, a *distinct* mechanism (a local at function scope with no nested block to
  free it earlier; needs last-use freeing of *function-level* locals, a follow-on).

**Use it as the driving regression test:** fix the model until the guard is silent
suite-wide, then promote it to a `debug_assertions` assertion so the model can't
silently regress.

### Confinement = least-common-ancestor, NOT exact scope-match (probes 16-27)

A first cut compared every reference's *innermost* block scope and required them
**equal**.  Probes 16-27 proved that wrong — it UNDER-fires, missing the most
common shapes, which would make the fix skip them:

| Probe | Shape | exact-match | LCA (correct) |
|---|---|---|---|
| 20 | vector created in a block, **iterated** there (`for x in a`) | missed (0) | confined ✅ |
| 25 | vector **read inside a loop body** (declared outside the loop) | missed (0) | confined ✅ |
| 26 | vector created in a block, used in a **nested `if`** | missed (0) | confined ✅ |
| 17 | vector local in a **match arm** | confined | confined ✅ |
| 19 | distinct vectors in then / else branches | both confined | both confined ✅ |
| 18 | set in both branches, **read after** (escape) | silent | silent ✅ |
| 27 | reassigned in a branch, **read after** (escape) | silent | silent ✅ |

Root cause of the miss: a for-loop lowers to a nested `#For block`, a nested `if`
adds a sub-block — so the creating refs sit in block `B` and the using refs in a
**sub-block of `B`**.  Their innermost scopes differ, but the data is still
confined to `B`.

**The fix's confinement (and the guard) MUST track the full block/loop scope-PATH
at each reference and take the least-common-ancestor:** `LCA([B]) ⊔ LCA([B,sub]) =
[B]`.  The store frees at the LCA block's exit.  If the LCA's innermost element is
a **loop** scope, the local lives only inside that loop (per-iteration reuse) →
**not** relocatable.  `guard_refs` now does exactly this; probes 16-27 are its
catalogue (`probes/16_*.loft` … `probes/27_*.loft`).

> Probe hygiene note: avoid stdlib names in probe bodies (`sum`, `first`, …) — a
> name collision is a parse error, not a lifetime finding (probe 21 false alarm).

## THE CRUX — RESOLVED (it was a misdiagnosis: scope registration, not rc)

A read-only **investigation agent** (code + runtime verified) overturned the rc
framing this section used to hold.

**There is no rc hold.** `dec_rc=0` every shape; rc=1 throughout (`allocation.rs:118`
sets `ref_count=1`, `free_named` only decrements when `>1`).  A mid-program
`OpFreeRef(__vdb)` genuinely **does** free the store — proven: a helper fn returning a
vector, called 3×, keeps `active` flat at 3 (each call's function-exit free releases
the store before the next call reuses the slot).

**So why was the relocation a "no-op"?**  The reverted `sink_dead_store_frees` ran as a
post-pass **after `compute_intervals`** — it moved the free *node* but the slot
allocator had already computed the long (to-function-exit) interval, so the move was
cosmetic and the watermark never changed.  Not rc — **timing**.

**The real coupling is the `__vdb`'s scope registration.**  A vector local `v` carries
`dep=["__vdb_N"]` ⇒ `owns=false` (`get_free_vars`, `scopes.rs:1250`) ⇒ `v` is never
freed; the `__vdb` (empty dep) is the sole owner that gets `OpFreeRef`, registered at
**function scope** because its null-init is hoisted to body position 0
(`work_references`, `expressions.rs:354`), so `get_free_vars` only frees it at function
exit.  **The fix is a scope-registration change**: register the confined `__vdb` at its
LCA block scope and the existing block-exit `free_vars` sweep frees it there — **no rc
surgery, no extra decrements** (the old "`OpFreeRef` + `Set(local,Null)`" idea is
unnecessary; rc=1 means a single `OpFreeRef` at the block suffices).

## Failed approaches — do NOT re-walk

| Approach | Result |
|---|---|
| Relocate the `OpFreeRef` node as a **post-pass** (`sink_dead_store_frees`) | **Ineffective — timing, not rc**: ran after `compute_intervals`, so the slot interval was already long; the node move was cosmetic.  Reverted. |
| `OpClearVector` store-reuse (cluster III) | Broke native codegen (`var_v` out of scope — moved the *declaration* into the block) + runtime (`+=`, text iter, keyed concat, a SIGSEGV).  Reverted. |
| `OpFreeRef(old __vdb)` at the overwrite (cluster III, naive) | No-op — the repoint doesn't drop the old store; needs the dead-store free at the overwrite in `scopes.rs`, not `create_vector`. |

## Fix approach — gated two-phase scan (scope re-registration)

**Foundation landed (`00dc10ae`):** `store_confinement(code, vars, free_ref_nr)`
returns `vdb → (local, block_scope)` for every provably-confined store (the
adversarially-hardened analysis; `store_lifetime_guard` is now a thin reporter over
it).

**Cluster I** — the chicken-and-egg (free emitted *during* scan, at the `__vdb`'s
body-position-0 null-init, before scan reaches the confining block; `set_scope`
asserts-once) is resolved by a **gated two-phase scan** (`scan` is fresh per function,
`scopes.rs:99`, so it is re-runnable):
1. scan once → `store_confinement` → if the map is **empty, do nothing** (the common
   case, no second scan);
2. else re-scan the saved original `def.code`/`variables` with the map threaded into
   `Scopes`, so `scan_set`'s `var_scope.insert(__vdb, …)` (`scopes.rs:635`) and
   `scan_if`'s local pre-init (`scopes.rs:846/851`) register the confined `__vdb`(+local)
   at the **block scope**.  `get_free_vars` then frees it at block exit — the *tested*
   free-emission path, no node surgery.

**Keep the `Set(__vdb, Null)` declaration at body position 0** — native emits the
`let mut var___vdb` at function scope, so a free in a nested block is in-scope; moving
the *declaration* is what broke the `OpClearVector` attempt.  Only the registration/
free scope moves.  Verify with `--show-rust` that no free lands where its var is out
of scope.

**Cluster III** (distinct surface, guard-silent — `v` read at fn end ⇒ LCA *is* the
function scope): at each **unconditional** overwrite free the *prior* `__vdb`, guarded
by `!is_captured(v)`, in the `scopes.rs` dead-store path (NOT `create_vector`).

## Soundness constraints (the guards)

- **Escape** — a local read *after* its block must keep its store function-lived
  (the `guard_confine` `Escaped` verdict; danger case `escape_after_block`).
- **Reassignment** (cluster III) — free the *old* store at the overwrite, only when the
  old mapping is dead (conditional reassignment in a branch keeps both stores live —
  see `escape_after_block`'s `v = [..]` in an `if`, read after).
- **Capture / `&vector` ref-params** — never free a store another live binding points
  into (`is_captured` guard; skip work-ref-aliased / ref-param cases).
- **Loops** — excluded; they already reuse in place.
- **LIFO** — `database::free` enforces reverse-alloc free order.

## Verification plan

1. `LOFT_STORE_GUARD` goes **silent** on `one_block` / `many_blocks` / `168` / `03-text`.
2. Watermark drops O(block-locals) → O(1) on `many_blocks` (peak 7 → ~2-3).
3. **Danger cases stay correct on both backends** — `escape_after_block`,
   `escape_skipped`, `dead_at_block_exit`, `loop_reuse_unaffected`
   (`/tmp/cl1/danger.loft`).
4. Concat / self-ref family unbroken — `167`, `168`, `11-vectors`, `135`, `136`, `95`.
5. Full `find_problems` clean; the exit leak gate (`tests/wrap.rs:276`) passes on both
   backends.
6. Promote `LOFT_STORE_GUARD` to a `#[cfg(debug_assertions)]` assertion (regression lock).

## Follow-on (out of scope here, noted)

Function-level **sequential** locals (the `11-vectors` watermark) are *not* cluster I —
no nested block frees them earlier.  Reducing those needs **last-use freeing of
function-scoped locals**, a separate, larger change.  Land the block-confined fix first
(it's the one with the clean scope boundary), then evaluate the function-level case.

## Tail-end experiment — disable store ref-counting once scoping is correct

**Goal (user, 2026-06):** after the scoping work lands and `LOFT_STORE_GUARD` is silent,
**turn the store ref-count OFF and see whether it is still needed.**

The primary objection is **NOT that rc is unsound** — it works.  It is that rc **glosses
over the lifetime details**: it is opaque machinery that decides *behind your back* when a
store really dies, so the actual memory model is hidden and can't be reasoned about
directly.  That is precisely the kind of system the user does not trust, and exactly what
[Goal E](../../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)
rejects (the C-vs-Rust transparency stance: the *programmer's* model is the truth, not a
counter's).  Removing rc is therefore a **transparency** goal first — make the lifetime
explicit (scope-based, source = truth) instead of abstracted away.

Two concrete consequences that follow:

1. **rc HIDES real lifetime bugs** (the diagnostic side of the same coin).  A ref-count
   masks an incorrect lifetime: an over-retained store (an extra rc holder) never crashes,
   it just lives too long, so a *wrong* free site silently looks fine.  Flipping rc off
   turns it into a **detector**: whatever breaks is a place the scoping is still incomplete
   (a real use-after-free / double-free rc was papering over).  *What breaks is the work.*
2. **Better parallel.** The shared rc counter is cross-thread contention; removing it
   helps parallel execution (already largely mitigated, but cleaner).

**Why this is the right tail-end, in this order:** the [rc crux](#the-open-crux--pin-this-first-blocks-the-design)
already showed **no vector store needs rc** (`dec_rc=0` every shape — `inc_rc` fires only
for **closure capture**, `fill.rs:1967` / `allocation.rs:299`).  So once correct scoping
frees stores at scope end, the only remaining rc user is closure capture — and the
hypothesis is that scoping (or an explicit capture-copy) can cover that too, making the
whole `ref_count` field droppable.

**Method:** with the scoping fix in place, make `inc_rc` a no-op and `free_named` free at
rc≤0 always (or compile-out the `ref_count`/`OpIncRc`/`dec_rc` path), then run the full
suite + `LOFT_STORE_GUARD` on **both backends**.  Triage every new failure: each is
either (a) a scoping gap to fix (the bug rc hid), or (b) a genuine closure-capture
dependency to handle explicitly.  Outcome = either rc is droppable (delete the mechanism)
or the exact residual set of stores that still need it.  Own plan/branch — careful
refactor.  See [[project_drop_store_refcount]] + [[project_predictable_memory_model]].
