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

### REFUTED (Phase-2 + investigation, 2026-06): free-insertion alone cannot lower the watermark — it is locked at body-0

The original thesis ("emit a real `OpFreeRef` at the def point, the watermark drops") is
**wrong** for the straight-line case, and the spike proved it. The decisive mechanism:

- Every `__vdb` work-ref's **null-init** (`Set(__vdb, Null)`, hoisted to body-0 by
  `parse_code` `src/parser/expressions.rs:354-369`) **allocates a store** at runtime
  (`OpInitRef` → `database.null` → `database_named`). `work_refs` is a `BTreeSet` (ascending
  var_nr) and the hoist `insert(0, …)` **prepends**, so the null-inits run in *reverse*:
  for `v=[a]; v=[b]; v=[c]`, body-0 allocates `__vdb_3→#2, __vdb_2→#3, __vdb_1→#4`.
- So **all three stores coexist the instant body-0 finishes** — `max`/`peak` = 5 *before any
  data is live and before any free can run*. The later `OpDatabase`s only *reuse* those
  stores (`claim` keeps the store_nr; no new alloc). Verified: `+#2 +#3 +#4` all up front,
  then `-#4 -#3 -#2`.
- `Stores::peak` is **monotonic** and `max` only shrinks when the **top** slot frees
  (`src/database/allocation.rs:~289`). An inserted free of a low/dead store reuses its slot
  but never lowers a `peak` already recorded at body-0. So straight-line measures 5 ON==OFF.
- The spike's "frees the wrong store" was a **misread** — the reversed order means
  `__vdb_1`'s slot legitimately holds `ref(4)`, and the free is correct (it frees `#4`, even
  trimming `max` 5→4 momentarily). It just can't move the already-locked `peak`.

**Why `11-vectors` partially drops (26→23) but straight-line doesn't:** 11-vectors' peak is
reached *mid-function* (named-local pins + transient/loop stores at the worst moment), so an
early free *before* a later growth point keeps `max` from climbing there. Straight-line's
peak is entirely body-0.

### The real lever — relocate the **allocation**, not (only) the free

The peak is set by *when the null-store is allocated*. To lower it, move each `__vdb`'s
null-init **out of body-0 to immediately before its own `OpDatabase`** — the I-a
`relocate_null_init` lever (`src/scopes.rs:227`), extended from "into a sub-block" to "to an
index within the body block." Then the stores stop coexisting up front and interleave
(`+alloc, -free, +alloc, -free`) — paired with the early free, `max` stays ~1-2. This is the
I-a mechanism (which works precisely *because* it relocates the null-init) applied to the
no-sub-block case.

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

### Phase 2 — Reclaim spike — **DONE; thesis refuted (2026-06)**

`lastuse_free_spike` (`src/scopes.rs`, gated `LASTUSE_FREE`) inserts `OpFreeRef(dead)` before
the next alloc. Outcome:
- v1 set `skip_free` to avoid double-free → **codegen suppresses every `OpFreeRef` for a
  `skip_free` var** (`src/state/codegen.rs:2202`), killing the inserted free too. (Found via
  the codegen free point — the "gate at that point" instinct.)
- v2 drops `skip_free` (scope-exit free becomes an idempotent double-free). Result:
  `11-vectors` 26→23, no leaks, `172` sound — but **straight-line / seq-stores don't move**,
  because the peak is locked at body-0 (see "REFUTED" above). The spike's free is *correct*,
  just powerless against a monotonic peak set before it runs.
- **Conclusion:** free-insertion is necessary but **not sufficient**. The fix needs the
  allocation-relocation half. Spike kept gated as the record (commit `bb237b7a`).

### Phase 2.5 — Tag/verify gate (store identity — the safety net) — **DONE (interpreter), 2026-06**

A `tag: u32` on each `Store` (`src/store.rs`), stamped by a new `OpStoreTag(vdb, id)` right
after each `OpDatabase` and verified by a new `OpFreeRefTag(vdb, id)` replacing each
`OpFreeRef` (`assert!(store.tag == id)`). Catches **wrong-store / cross-owner free** that
`free_named` otherwise silently no-ops. The two ops are emitted **only** by a gated IR
post-pass (`tag_stores` in `src/scopes.rs`, env `LOFT_STORE_TAG`) — normal builds are
**byte-identical** (the user's "no bytecode bloat" requirement; two new ops over an extra
operand on the existing ones). `id` is a per-function-var allocation-site number, globally
unique. Verified: gate-off runs normally; gate-on shows **0 mismatches** on correct code
(`172`, wrap sample); and it independently **confirmed the Phase-2 spike freed the right
store** (0 mismatches under `LASTUSE_FREE=1 LOFT_STORE_TAG=1`).

- **Interpreter-only for now.** Normal `--native` is unaffected (verified). But under
  `LOFT_STORE_TAG` + `--native`, native codegen emits the tagged ops as Rust calls to
  functions that don't exist → compile error. **Native verification is a deferred
  follow-up** (not unwanted — Goal D parity will want it for the relocation fix): it needs
  native runtime handlers for `OpStoreTag`/`OpFreeRefTag`, the way `pre_eval.rs` special-
  cases `OpFreeRef`. The interpreter gate is the immediate safety net for the interpreter
  relocation fix; native follows when the fix lands on native.

### Phase 3 — Freeing + null-init relocation (the fix) — **reframed**

Two coordinated edits per dead store, in the post-pass:
1. **Relocate the null-init** out of body-0 to immediately before the store's own
   `OpDatabase` (extend `relocate_null_init` to target a body-block *index*, not only a named
   sub-scope) — so allocations stop batching at body-0 and the peak can drop.
2. **Emit the early free** before the next store allocates — so consecutive stores interleave
   (`+alloc, -free`) instead of stacking. Do NOT `skip_free` (codegen suppression); instead
   *remove* the scope-exit free node (or rely on the idempotent double-free, measured safe).

- **Under the Phase-2.5 tag gate** the whole time, so a mis-relocation surfaces loudly.
- **Soundness gates (reuse):** copy-semantics (`&`/`RefVar` `variables/mod.rs:1466`),
  `guard_escapes`, `is_captured`, `is_skip_free`, `confine_reassign_safe` (reassignment-dead).
- **Risk (the I-a lesson):** relocating the null-init changes `first_def` → the interval /
  `assign_slots` graph / `validate_slots` invariants; `compute_intervals` must run *after* the
  relocation. Native required the declaration in-scope.
- **Verify:** peak drops on probe 14 + `11-vectors`/probe-07; `172` + tag-gate + full suite
  green; no leak.

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
