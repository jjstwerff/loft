<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Fix design — free a vector store's heap allocation at last-use (clusters I + III)

## Why this matters (the real stakes)

Not a watermark cosmetic.  A block-scoped vector that lives until **function exit**
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

## THE OPEN CRUX — pin this FIRST (blocks the design)

Relocating `OpFreeRef(__vdb_N)` into the confined block is **sound** (escape +
concat/self-ref family stay correct on both backends) but a runtime **no-op**: the
store is not released there; `active` keeps climbing.  So **something holds the
store's rc past block exit** — yet there is **no obvious `inc_rc`** in the vector
fill path (`inc_rc` is only the explicit closure-capture op at `fill.rs:1967`).

At function exit the *same* `OpFreeRef(__vdb)` *does* free it (rc must reach 0
there).  So between block-exit and function-exit the rc drops.  **What decrements
it?**  The candidates to trace, on a single block-local store, rc step by step:

- `OpDatabase` — initial rc (1? more?) — `src/database/allocation.rs` alloc path.
- `OpGetField` (`a = field of __vdb`) — does the local's DbRef carry an rc? — `fill.rs:1374`.
- `OpPreAllocVector` / `OpNewRecord` / `OpFinishRecord` — does filling bump the store rc?
- The **frame discard** at `fn_return` (`State::fn_return`, `discard=N`) — does popping
  the frame drop the local's reference, and is *that* what lets the function-exit free
  reach 0?
- `free_named` (`src/database/allocation.rs:166`) — rc>1 path vs the `store.free` guard.

**Until this is answered the free site cannot be chosen correctly.**  Method: a
minimal one-block-local repro (`/tmp/cl1/one.loft` shape) + `LOFT_STORES=log` with rc
events (add a `dec_rc`/`inc_rc` trace if the log lacks it), watch store #N's rc from
`OpDatabase` to teardown.

## Failed approaches — do NOT re-walk

| Approach | Result |
|---|---|
| Relocate `OpFreeRef(__vdb)` into the confined block (post-pass `sink_dead_store_frees`, confinement + LIFO + loop-exclusion) | **Sound but ineffective** — runtime no-op (the rc crux above).  Reverted. |
| Re-scope the local/`__vdb` to the block during `scan` | **Chicken-and-egg** — free emission runs during `scan`; inner-block sweeps fire before the full reference picture is known.  Would need a pre-pass replicating scan's scope-numbering, or un-hoisting with parser-side escape analysis. |
| `OpFreeRef(old __vdb)` at the overwrite (cluster III) | Runtime **no-op** (rc — same crux). |
| `OpClearVector` store-reuse (cluster III) | Broke native codegen (`var_v` out of scope) + runtime (`+=`, text iter, keyed concat, a SIGSEGV). |

## Fix approach (design — finalise after the rc crux is pinned)

Once the rc-hold is known, the free site emits, at the **block-confined last use**
(cluster I) or the **overwrite point** (cluster III), whatever set of decrements is
needed to bring the store's rc to 0 — likely `OpFreeRef(__vdb)` **plus** releasing
the local's reference (a `Set(local, Null)` / dec, mirroring what frame-discard does
at function exit).  Reuse the **`guard_confine` analysis** to locate the block; emit
the free at the end of that block (LIFO-correct: relocate in reverse-alloc order,
loops excluded — `Value::Loop`, not `Value::Block`).

**Native:** the `__vdb` declaration stays at function scope (slot not re-scoped), so
a free in a nested Rust block reading a function-scoped var is valid (the existing
`__lift_N` pattern already does declaration-at-function-scope + conditional-free).
Verify with `--show-rust` that no relocated free lands where its var is out of scope.

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
