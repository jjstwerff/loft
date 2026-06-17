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

Both are [Goal E](../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth):
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

### Phase 2.5 — Tag/verify gate (store identity — the safety net) — **DONE (interpreter + native), 2026-06**

A `tag: u32` on each `Store` (`src/store.rs`), stamped by a new `OpStoreTag(vdb, id)` right
after each `OpDatabase` and verified by a new `OpFreeRefTag(vdb, id)` replacing each
`OpFreeRef` (`assert!(store.tag == id)`). Catches **wrong-store / cross-owner free** that
`free_named` otherwise silently no-ops. The two ops are emitted **only** by a gated IR
post-pass (`tag_stores` in `src/scopes.rs`, env `LOFT_STORE_TAG`) — normal builds are
**byte-identical** (the user's "no bytecode bloat" requirement; two new ops over an extra
operand on the existing ones). `id` is a per-function-var allocation-site number, globally
unique.

**Scoped to the reclaim-eligible stores (2026-06).** The first cut tagged *every* `OpDatabase`
and verified *every* `OpFreeRef`, which **false-positives on store-sharing**: when two work-refs
share one store via *adoption* (`{f = file(...)}` across sibling blocks — `f` reuses one slot,
site-1 tags it, site-2 frees it) the per-var tag mismatches even though the free is correct.
This fired on **both backends** (`20-binary` — the interpreter "0 mismatches" was only ever a
curated sample that avoided adoption). Fix: `tag_stores` now takes the reclaim **`owning`** set
(from `reclaim_free_intent`) and tags/verifies *only* those stores — exactly the frees reclaim is
responsible for. Adopted / shared / `file()` stores stay untagged with a plain `OpFreeRef`, so
the gate **cannot false-positive** on them. Verified clean on both backends: `20-binary`,
`issues` (684), `leak`, `native` (6) all 0 mismatches under `LOFT_STORE_TAG=1 LASTUSE_RECLAIM=1`,
and the reclaim win-probes (07/09/11/14) verify clean on interpreter **and** `--native`.

- **Native parity DONE (2026-06).** Native runtime handlers `OpStoreTag` / `OpFreeRefTag`
  (`src/codegen_runtime.rs`) + emitters (`OpStoreTagEmitter` / `OpFreeRefTagEmitter` in
  `src/generation/ops/ref_ops.rs`, registered in `ops/mod.rs`).  `OpFreeRefTag` mirrors
  `OpFreeRefEmitter` byte-for-byte (skip_free → `()`, fn-ref → closure free, plain → free +
  null-reset) but routes through the verifying runtime, so a tagged native build behaves exactly
  like the untagged one plus the tag check.  Native and interpreter now report the **same**
  mismatch on a wrong-store free (faithful parity).
- **Remaining gap: wasm.** `wasm_library_suite` fails under `LOFT_STORE_TAG` because the wasm
  runtime has no `OpStoreTag`/`OpFreeRefTag` handlers (the same gap native had pre-this-session).
  The gate is an interpreter/native diagnostic — don't run it against the wasm suite.  Wiring wasm
  handlers is a small follow-up if Goal-D wasm watermark verification is ever wanted.

### Phase 3 — Freeing + null-init relocation — **DONE: thesis CONFIRMED, gated + HARDENED (2026-06)**

`lastuse_reclaim` (`src/scopes.rs`, gated `LASTUSE_RECLAIM`) does both coordinated edits per
dead store, in the post-pass before `compute_intervals`:
1. **Relocate the null-init** out of body-0 to immediately before the store's own `OpDatabase`
   build (the I-a `relocate_null_init` lever, applied to a body-block *index*).
2. **Early free** before the next store allocates, and **REMOVE the scope-exit `OpFreeRef`** for
   that store (the early free is now its sole free — see the tag-gate finding below).

**Thesis CONFIRMED — the body-0 watermark lock is broken, on BOTH backends** (the pass is a
shared IR phase, so it applies to `--native` — the default — and `--interpret` alike; the
relocation-to-body-index keeps the declaration in body scope, so it is native-safe, unlike I-a's
sub-block move):

| probe | shape | peak base → gate | output |
|---|---|---|---|
| 14-reassignment | 1 local reassigned 11× | **11 → 2** | identical |
| 07-sequential-named-locals | 35 distinct locals | **35 → 2** | identical |
| 09-untyped-named-locals | 35 untyped locals | **35 → 2** | identical |
| 11-comprehension-init | 10 comprehensions | **10 → 2** | identical |
| 01 / 08 / 15 / 17 | already-minimal | 2 → 2 (no regression) | identical |

The probe-14 trace shows the perfect `+alloc, -free, +alloc, -free` interleave: all 11 stores
cycle through 2 physical slots.

**The tag gate (2.5) earned its keep.** The first spike kept the scope-exit free as an
"idempotent double-free" (the Phase-2 assumption). `LOFT_STORE_TAG=1 LASTUSE_RECLAIM=1` flagged
it immediately: `store-tag mismatch on free: store #2 has tag 11 but expected 1`. Mechanism: under
reclaim the freed slot is **reused** by a later store, so the stale scope-exit `OpFreeRef(__vdb_1)`
no longer hits an already-free store — it hits the slot now owned by the live `__vdb_11`. (Masked
functionally because the last read precedes it, but a genuine wrong-owner free.) **Fix:** reclaim
removes the scope-exit free for every early-freed store. Tag gate then clean (0 mismatches).

**Soundness boundary mapped — then HARDENED with the gates (2026-06).** The first cut (no gates)
diverged from baseline on **8 / 208** `tests/scripts` — every one an escape/alias/branch case the
plan predicted:

| script | failure (pre-gate) | escape mechanism the gate must catch |
|---|---|---|
| `98-struct-order-in-use` | `index oob` in `free_named` | stale DbRef over-free (struct-field alias) |
| `131-keyed-nested-struct-uaf` | crash | nested-struct field escape, loop body |
| `repro_p346`, `29-strings` | crash | data live past the early free (refvar / loop) |
| `172-store-confinement-soundness` | hang | corruption → loop |
| `20-binary` | leak (1 store) | scope-exit free removed, early free in untaken branch |

**The gate — `reclaim_safe` + `contains_alloc_unconditional` + `holder_retained`** (`src/scopes.rs`),
mirroring `store_confinement`'s I-a soundness model.  A store is reclaim-eligible only when ALL hold:

- **dominance** — its `OpDatabase` is reached unconditionally (never inside an `If`/`Loop`/`Parallel`
  branch).  Kills the `20-binary` branch-leak and the loop cases (`131`).
- **no escape / capture / alias** — not `skip_free` / `captured` / a `RefVar`, and `!guard_escapes`;
  every holder local is non-arg, non-captured, non-`RefVar`, non-escaping; ≤1 user-var holder; a
  multi-store holder passes `confine_reassign_safe`.
- **no retention** — no holder appears in a value position that keeps the store reachable past the
  free (tuple/vector element, struct-field/keyed value, return, alias-copy, non-receiver arg).
  `_`-prefixed build-internal temps (`_elm`, nested `__vdb`) are skipped — they are part of the store
  being built, not external aliases (matches `store_confinement`'s dep-escape treatment; without it a
  comprehension's per-iteration `_elm` false-positives).

Orphaned stores (no holder — the reassignment case `__vdb_1..10` in probe 14) are safe-because-dead.
**Conservative by design**: an escape/alias case falls back to the (sound) scope-exit free.

**Result — sweep of 208 scripts, baseline vs gate: 0 real divergences, 0 crashes** (3 remaining
diffs are pre-existing nondeterminism — threading/stress/hash-order — that differ baseline-vs-baseline
too).  Win probes keep their watermark and verify tag-clean (0 mismatches):

| probe | peak base → gate | tag gate |
|---|---|---|
| 07 / 09 (distinct locals) | 37 → 3 | clean |
| 11 (comprehension) | 12 → 3 | clean |
| 14 (reassign) | 13 → 4 | clean |

- **Default path byte-identical** (env-gated; `cargo clippy -- -D warnings` + `--all-targets` + `fmt`
  all clean; baselines unchanged).
- **Risk (the I-a lesson) — handled:** relocating the null-init changes `first_def`; the pass runs
  *before* `compute_intervals`, so the interval / `assign_slots` / `validate_slots` graph sees the
  moved def.  Native confirmed (peak drop + correct output on `--native`, the default mode).
- **Remaining for production:** Phase 5 (watermark regression test + un-gate).  The tag gate now
  covers interpreter **and** `--native` (Phase 2.5); only the wasm handlers remain a small follow-up.

### Phase 4 — Permanent Goal-E enforcement assert — **DONE (2026-06)**

**Reframed from the original plan.** Phase 1's `last_use_guard` detects a *static data-flow shape*
("store `st`'s data dies before a later store allocates") — which is present for **every**
sequential-store function and which reclaim does **not** remove (reclaim adds a *free*; the data
still dies before the later alloc).  Promoting it verbatim to a hard assert would panic on nearly
every program.  The coherent Goal-E assert is **runtime, not static**: *did reclaim actually stop the
store before the sibling allocates?*

The implementation (`src/scopes.rs`):

- **`reclaim_free_intent`** — the single source of truth, shared by `lastuse_reclaim` (which acts on
  it) and the guard (which verifies it), so the two cannot drift.  Returns `(owning, intent)` where
  `intent` is the `(store, trigger)` pairs the reclaim must free.
- **`reclaim_unfreed_eligible`** — the guard: after reclaim ran, for each `(store, trigger)` in the
  plan, asserts `store`'s `OpFreeRef` sits at body top-level **before** the op that allocates
  `trigger`.  Returns the count left live-but-dead — must be 0.
- A **hard `assert_eq!`** in `check()`, **gated** by `LASTUSE_RECLAIM` (zero-cost when reclaim is off,
  so the default build is unaffected).  Active in release (unlike `#[cfg(debug_assertions)]`) so it
  guards the release test suite.  When Phase 3 is un-gated (Phase 5) it becomes the default Goal-E
  watermark guard with no rewrite.

**Honest scope:** the assert covers the reclaim-*eligible* stores — the shapes the model says are
dead-and-reclaimable.  The escape/alias/branch cases the soundness gate excludes are **not** in
`intent`; they keep their (sound) scope-exit free and are a documented, non-silent exception (still
visible via the `LOFT_LASTUSE_GUARD` diagnostic), not a watermark win.  Superseding the scope-exit
`store_lifetime_guard` entirely waits on un-gating.

**Verified:** the full suite runs **green with `LASTUSE_RECLAIM=1` and the assert compiled in —
1921 passed, 0 failed, no panics** (both backends; fresh `--lib` rebuild — the first gated run's
native failures were the stale-rlib false-failure, not reclaim).  Default path byte-identical;
`clippy -D warnings` + `--all-targets` + `fmt` clean.  This green full-suite-under-gate run is also
the **un-gate prerequisite** for Phase 5.

### Phase 5 — Un-gate + CI lock-in + scaffolding cleanup — **DONE (2026-06)**

- **Un-gated — reclaim is the DEFAULT.** `lastuse_reclaim` runs for every function in
  `check()`; `LASTUSE_RECLAIM_OFF` disables it for A/B watermark measurement. The Phase-4
  Goal-E assert is now THE watermark guard: on in debug builds, and in release on demand via
  `LOFT_STORE_GUARD` (`reclaim_guard`); zero-cost otherwise. Verified safe in **debug** builds
  too (`validate_slots` I1–I7 + the assert active — the relocation's `first_def` change passes
  the slot graph).
- **Watermark regression guard** — `tests/watermark.rs`: asserts the reassign and
  distinct-locals shapes stay at the small constant peak via `Stores::peak` (reassign ≤ 6,
  was 13; distinct ≤ 6, was 12). The guard bites — `LASTUSE_RECLAIM_OFF` makes both fail.
  (Interpreter, in-process; native watermark is covered by the script suite's output-correctness
  + the per-probe `LOFT_STORES` measurements.)
- **Scaffolding removed:** the `LASTUSE_FREE` Phase-2 spike (`lastuse_free_spike`), `CONF_DBG`,
  `FREE_DBG`, and the `CONF_OFF`/`RELOC_OFF` A/B gates (cluster I-a is now unconditional). Kept
  `RC_OFF` deliberately (the rc-removal probe, GOALS.md tail-end experiment) and `LASTUSE_RECLAIM_OFF`
  (the new A/B switch).
- **Full suite 1923 ✅** with reclaim default + the Goal-E assert active (`LOFT_STORE_GUARD=1`),
  both backends. `clippy -D warnings` + `--all-targets` + `fmt` clean.

This closes the last-use-freeing arc. The remaining plan-57 work is the **cluster III Route 2**
shared-block residual (its own design), plus the deferred follow-ups (rc removal — now unblocked
since reclaim is the default — `parallel {}` feature, nightly parity sweep) and the small items
(wasm tag handlers, `LOFT_STORES=warn` floor).

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
