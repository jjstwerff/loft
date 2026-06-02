<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Fix design — last-use freeing via a definition-point liveness guard

Closes the two open watermark halves that block-confinement (I-a, shipped) cannot
reach, because they have no narrower lexical scope to relocate a slot into:

- **Cluster I-b** — function-level *sequential distinct* locals (`a=[..]; b=[..]; c=[..]`):
  each dead after its last read but freed at scope exit (probe 07, `11-vectors`).
- **Cluster III straight-line** — *sequential reassignment* of one local
  (`v=[a]; v=[b]; v=[c]`, probe 14): each overwritten store dead at the next assignment,
  freed at scope exit. Measured peak 5 ON==OFF — block-confinement is inert here.

Both are [Goal E](../../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth):
the model says a value dies at its last use; the runtime holds it to scope exit.

## The mechanism — a guard at the definition point, not at scope exit

> At each **store-defining point** `S`, sweep the currently-live store-backed vars; any
> with `last_use(v) < S` "should have been stopped." Report it (diagnostic) or free it
> (fix).

It is the Goal-E model expressed as a per-step check: *is anything alive that the model
says is dead?* The same hook unifies straight-line reassign, sequential distinct locals,
and (subsumes) block-confined — no sub-block required.

### Why this sidesteps the I-a wall

The I-a crux was "the runtime free follows the **slot** (the body-0 null-init / `first_def`);
an IR `OpFreeRef`'s *position* was inert." This design does **not** move a slot or a scope —
it emits a real `OpFreeRef` at the def point and suppresses the scope-exit one. The runtime
backs this: `State::free_ref` (`src/state/io.rs:613`) does `self.database.free(&db)` — frees
the store **immediately**; the next `OpDatabase` reuses the slot via the free-bitmap. So the
reclaim genuinely happens at the def point, and the watermark drops.

### Data — the real liveness needs a flow walk (Phase-1 finding, 2026-06)

The plan first assumed `compute_intervals`' `last_use` (`src/variables/intervals.rs:18`,
fields `mod.rs:133-135`) was the signal. **It is not:** `last_use` counts the scope-exit
`OpFreeRef(__vdb)` as a use, so every function-scoped store's `last_use` is pinned to the
teardown (measured: `__vdb_1`'s `last_use = 158` near the function end though it is the
*first* store). `first_def` is likewise pinned to the body-0 null-init. So
`compute_intervals`' interval is `[body-0 .. scope-exit]` for every store — useless here.

The real signal is **flow-sensitive**: a store lives via the *local that holds it*, ending
at the local's last read or at a **rebind** (`v = OpGetField(other_store, …)`, the
reassignment case). `store_liveness_walk` (`src/scopes.rs`) recovers it — tracing
`holds: local → store`, recording each store's `alloc` (`OpDatabase`), `last_read` (via a
holding local or a direct build op, *excluding* `OpFreeRef`), and `dead` (a rebind-away).
This walk is the shared foundation for the diagnostic AND the Phase-3 fix.

> **Watermark divergence at `S`:** a function-scoped store whose data-death (`dead`, else
> `last_read`) precedes the `alloc` of a *later* store — held dead to scope exit while the
> watermark grows. Block-confined stores (`scope != body`) are excluded (I-a frees them).

## Phases

Order follows the Goal-E pattern (guard reports → fix makes it silent → promote to assert),
which is also the user's "1 and 3": diagnostic first, kept as the permanent guard.

### Phase 1 — Definition-point liveness DIAGNOSTIC (measure) — **DONE (2026-06)**

`last_use_guard` (`src/scopes.rs`), a read-only post-pass after `compute_intervals`
(`src/scopes.rs` call site), gated by `LOFT_LASTUSE_GUARD`. Runs `store_liveness_walk`,
then reports each function-scoped store whose data dies before a later store allocates.

- **Cross-validated** — `peak − divergences = the minimal achievable peak`:

  | case | peak | divergences | predicted min |
  |---|---|---|---|
  | 8 distinct locals (I-b) | 10 | 7 | 3 |
  | 3× reassign (III straight-line) | 5 | 1 | 4 |
  | `11-vectors` | 26 | 18 | 8 |
  | flat-if (I-a, already fixed) | 3 | 0 | 3 |

  (Straight-line yields `N−2`, not `N−1`: a rebind store coexists with the new one — the
  old store dies only *after* the next alloc, so it isn't peak-relevant. I-b yields `N−1`.)
- **Excludes I-a** correctly (flat-if = 0 — block-confined stores leave via block exit).
- **Env-gated, no harm** — normal path untouched; `172` green both backends.
- Strictly more general than the scope-exit `store_lifetime_guard`.

### Phase 2 — Reclaim spike (de-risk the runtime, throwaway)

Before wiring the general fix, hand-emit `OpFreeRef(__vdb_1)` + `skip_free` at the
reassignment point of the straight-line case and confirm peak 5 → 3 and `172` stays green.
This proves the explicit def-point free reclaims (vs the I-a inertness) on the smallest
case. Revert after; gate Phase 3 on it.

### Phase 3 — Freeing (the fix)

Flip Phase 1's report to emission: at each def-point `S`, for each swept dead `v`, insert
`OpFreeRef(v)` into the IR immediately before `S`'s statement (the `prepend`-into-block
shape `relocate_null_init` already uses) and set `v.skip_free = true` so the scope-exit
`get_free_vars` (`src/scopes.rs:1467`) does not double-free.

- **Soundness gates (reuse, do not reinvent):** copy-semantics (plain locals unaliased
  except explicit `&`/`RefVar` — `variables/mod.rs:1466`), `guard_escapes` (return/yield/
  break, block-result, tuple/vector-literal escape), `is_captured` (closure capture),
  `is_skip_free`, and `confine_reassign_safe` for the reassignment-dead proof (the old
  value must be provably reassigned before any later read).
- **Verify:** peak drops on probe 14 (5→3) and `11-vectors`/probe-07 (O(N)→O(1));
  `172` green both backends; full suite green; no double-free under the leak gate.

### Phase 4 — Promote the (now-silent) guard to a permanent Goal-E assert

With Phase 3 making the divergence set empty corpus-wide, promote Phase 1's guard to a
`#[cfg(debug_assertions)]` assertion (per Goal E's "Check"): a store live-but-dead at a
def-point is now a hard failure, so the rule cannot silently re-acquire exceptions. This is
the permanent enforcement deliverable (option 3) — supersedes the scope-exit
`store_lifetime_guard` as THE Goal-E watermark guard.

### Phase 5 — CI lock-in + scaffolding cleanup

- Watermark regression test via the `Stores::peak` field (added this session): assert
  probe-14 / I-b peaks stay at the bound. Both backends.
- Remove debug scaffolding accumulated across the cluster work (`CONF_DBG`, `FREE_DBG`,
  the `CONF_OFF`/`RELOC_OFF` A/B gates) once locked in; keep `RC_OFF` deliberately
  (the rc-removal probe, GOALS.md tail-end experiment).

## Risks / unknowns (ranked)

1. **Liveness precision for indirect uses** — RESOLVED for the direct cases by
   `store_liveness_walk` (Phase 1): it traces the holding local's reads, not the polluted
   `compute_intervals` interval. Remaining edge: a store whose data escapes via a fn arg /
   `&v` borrow / capture / struct field — `last_read` may under-count it. The soundness
   gates (`guard_escapes`, `is_captured`, `&`/`RefVar`) cover the *escape*; Phase 1's
   diagnostic also surfaces any over-flag (a store the corpus re-reads later) before
   Phase 3 frees anything. Branch handling is sequential-approximate (fine for the
   straight-line / sequential shapes this targets; shared-block goes via Route 2 instead).
2. **Double-free vs the I-a relocate** — a block-confined store is freed by I-a at block
   exit AND could be swept by the def-point guard. Sweep must skip vars already block-scoped
   by I-a (or let `skip_free`/`is_skip_free` arbitrate). **Mitigation:** the live-set sweep
   only considers function-scoped store-vars; block-confined ones leave the live set at
   their block exit.
3. **IR insertion position** — inserting `OpFreeRef` before a def statement inside the
   right block (nested blocks, `if`-tails). Reuse `prepend_to_scope`'s tested traversal.
4. **Seq ↔ IR position** — the post-pass must insert at the IR node whose traversal hit
   `seq == S`; replay `compute_intervals`' exact walk order to stay aligned.

## Code sites

| Concern | Site |
|---|---|
| Interval computation (seq, first_def, last_use) | `src/variables/intervals.rs:18`; fields `variables/mod.rs:133-135` |
| Post-pass insertion point | `src/scopes.rs:305-314` (after `compute_intervals`) |
| Scope-exit free emission (the one to suppress) | `get_free_vars` `src/scopes.rs:1343-1513`; gate at `:1467` |
| `skip_free` flag | `src/variables/mod.rs:128` |
| Runtime free (immediate) | `State::free_ref` `src/state/io.rs:613` |
| Soundness helpers | `guard_escapes`/`confine_reassign_safe` `src/scopes.rs`; `is_captured`/`is_skip_free` `variables/mod.rs` |
| Watermark measurement | `Stores::peak` `src/database/mod.rs` |
| IR-insert traversal | `prepend_to_scope` `src/scopes.rs:182` |
