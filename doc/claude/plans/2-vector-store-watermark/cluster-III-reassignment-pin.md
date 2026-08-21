# Cluster III — reassignment does not free the overwritten store

## Shape

Reassigning a vector local — `v = [new]` when `v` already holds a vector — does **not**
free `v`'s previous store. Every overwrite pins the now-unreachable old value until the
function scope exits, so a straight-line sequence of reassignments accumulates one pinned
store per overwrite.

## Severity (two fields)

- **Corruption / panic / hang:** none.
- **Leak (escapes scope):** **none** — the overwritten stores free at scope exit; the
  exit gate passes. Watermark concern only — O(reassignments) extra live stores.

## Verified

- ✅ **Overwrite pins the old store.** Probe 14: one local reassigned 10× → 14 allocs,
  all freed at scope-end (`aaa…fff`, zero interleaving). Each `v = [new]` allocates a new
  store and leaves the prior one alive. *(trace: RESULTS.md probe matrix row 14.)*
- ✅ **Loops are exempt.** The same per-iteration `v = [..]` inside a loop reuses a single
  store in place (probe 02) — the overwrite-pin is a *straight-line* phenomenon.

## Hypothesized (Stage B — needs source reading)

- 🤔 **Assignment to an existing store-backed local does not free the prior DbRef before
  rebinding.** The store-free is deferred to the scope sweep instead of firing at the
  overwrite. **Action:** read the assignment codegen for `existing_local = <new vector>`;
  confirm no `OpFreeRef` of the old DbRef is emitted at the overwrite point.
- 🤔 **Shares cluster I's root + fix surface.** Both are "store-free anchored only at scope
  exit, never when the store becomes dead." Cluster III's dead value is *unreachable*
  (stronger than cluster I's dead-but-named), but freeing it early needs the **same
  aliasing guard**: an overwritten store aliased by a still-live binding must not be freed.

## The single-valued-dep root (2026-06) — confirmed mechanism + shared-block variant

Read during the cluster-I if-block work, which surfaced cluster III from a new angle.

- ✅ **The dep field is single-valued (last-write-wins).** A local reassigned a fresh store
  per block carries a dep of only its *last* store. Concrete: shared `z` across three
  `else`-blocks (`z=[..]` in each) ends with `z(1):vector<integer>["__vdb_6"]` — dep
  `[__vdb_6]`, **not** `[__vdb_2, __vdb_4, __vdb_6]`. So the earlier stores `__vdb_2`,
  `__vdb_4` have **no backing local at all** (the `z`→store link was overwritten), and
  `__vdb_6`'s sole backing local `z` looks *single-store* with a function-level LCA. This is
  the same mechanism as probe 14's straight-line overwrite — **the overwrite drops the
  prior store's owning-variable relationship**, which is exactly why the prior store cannot
  be freed at the overwrite point: nothing records that it *was* `z`.
- ✅ **Shared-variable-across-sibling-blocks is a cluster-III variant.** `if {a} else {z=[..]}`
  ×N (shared `z`) and `match { _ => {x=[..]} _ => {y=[..]} }` ×N (shared `x`/`y`) both stay
  at the un-confined watermark (peak 7 / 8) where distinct-variable versions confine to 3.
  Same root: `z` is reassigned per block, so each block's store is an *overwritten* store
  with no live owner — cluster III, not a lexical-scope gap.

**So the fix is a dep-system change, not a scope-analysis tweak.** Two routes (a focused
change of its own, paused pending this cluster's turn):
1. **Dep accumulation** — make a variable's dep hold *every* store it has held, so
   `multi_store` detection works generally. Root-cause fix; bigger; soundness-sensitive.
2. **`OpGetField`-backing recovery** — when a store has no dep-backed local, find the local
   `L` it flows into via the `L = OpGetField(vdb, …)` assignment, then gate on the
   reassignment walk below. ~15 lines; ad-hoc; needs hard testing against `172` + escapes.

**Foundation already in `src/scopes.rs` (inert until the above lands):**
- `confine_reassign_safe(code, local)` — the soundness gate: every READ of `local` is
  dominated by a non-null assignment earlier in the same straight-line block, so the local
  never carries a store across a block boundary unreassigned (conditional `if`/`loop`
  assignments do **not** establish dominance — the walk under-claims, stays sound). This is
  the **same aliasing-safety property** cluster III needs to free an overwritten store: the
  old value must be provably dead (reassigned) before any later read.
- `store_confinement` has a `multi_store` branch (store-span LCA + `confine_reassign_safe`
  gate) wired in — currently never fires because single-valued dep never produces a
  multi-store local. It is the landing site once dep accumulation or `OpGetField`-backing
  supplies the missing link.

## Fix-safety — resolved

The aliasing question for clusters I/III was answered during the @P394/@P395 edge probing:
loft vectors are **copy-semantics** on expression-assignment (`b = a[..]`; then `b[0] = 99`
leaves `a[0] == 1`). Plain locals are therefore **not** aliased to each other — only
explicit `&vector` params alias. So freeing a dead/overwritten local's store is safe except
across an explicit `&` borrow, which is already tracked. This removes the main risk from the
cluster I/III fix.

## Dep cardinality — RESOLVED (2026-06): single-valued, last-write-wins

Dumped (`LOFT_LOG=fn:f`) the canonical straight-line shape `v=[a]; v=[b]; v=[c]`:
three distinct work-refs `__vdb_1/2/3` are created, `v` is repointed each time via
`v = OpGetField(__vdb_N, 0, _)`, but **`v`'s final dep is `["__vdb_3"]` — only the last
store**. The earlier `__vdb_1`/`__vdb_2` are *orphaned* (no dep-backed local), so
`store_confinement`'s `backed` scan skips them and all three free at function scope
(`OpFreeRef(__vdb_1(1)); __vdb_2(1); __vdb_3(1)`). This **refutes** the investigation's
static "dep appends → multi-valued" reading and **confirms** single-valued, so **Route 2
(`OpGetField`-backing recovery) is the route, not Route 1** (which would need a parser
change to accumulate deps).

## III splits by FIX MECHANISM — the convergence (2026-06)

The single-valued-dep root is shared, but the *fix* differs by where the orphaned stores
live:

- **Shared-block variant** (`if {a} else {z=[..]}` ×N — `z` reassigned across *sibling*
  blocks): each orphaned store lives in a **distinct block** → **block-confinement** applies
  (Route 2 backer-recovery + the I-a `relocate_null_init` machinery + `confine_reassign_safe`
  gate). Smaller; builds on the existing foundation.
- **Straight-line variant** (`v=[a]; v=[b]; v=[c]` — probe 14, the *canonical* cluster III):
  all orphaned stores live in **one block** (no sub-scope) → block-confinement gives no
  finer scope (measured: peak 5 ON == OFF). Needs **last-use freeing** — emit `OpFreeRef`
  at each `__vdb`'s live-interval end, not scope-end.

**Straight-line cluster III and cluster I-b converge on last-use freeing.** Both are a
sequence of `__vdb` work-refs in one block, each with a bounded live interval (ending at the
reassignment / last read) but freed at scope-end. `compute_intervals` (`src/scopes.rs`)
already computes those intervals (today only for slot validation); the fix drives
`OpFreeRef` emission from interval-end instead of scope-end, guarded by the copy-semantics /
`&`-borrow / `is_captured` / `guard_escapes` checks already in place. This is the higher-
value mechanism — it closes canonical III *and* I-b at once.

## Route 2 (shared-block variant) — SHIPPED, default ON 2026-08-21 (was gated `LOFT_CONF_RECOVER`)

The straight-line variant + I-b shipped via last-use freeing (reclaim, now default).  The
**shared-block variant** is implemented as a gated experiment in `store_confinement`
(`src/scopes.rs`):

- **`recover_backer`** — for an orphaned store (no dep-backer), find the local `L` it flows
  into via its `L = OpGetField(vdb, …)` repoint.  `L` is treated as a multi-store backer, so
  the I-a block-confinement + `relocate_null_init` machinery applies per block.

- **The soundness gate was the hard part — the doc's original plan was REFUTED.**
  `confine_reassign_safe` (the documented gate) only proves the backer is *defined* at every
  read; the fn-level init `z=[0,0]` satisfies that even when a confined block store is still
  live.  Empirically confirmed UAF: `z=[0]; if c {z=[1,2];} total += z[1];` returns wrong data
  on the branch-not-taken path (the confined block store was freed at block exit but z still
  holds it).  A first "no body-scope read" gate then missed the `for x in v` **loop source**
  read (172 `esc_after` — crashed native, tolerated by interp).  The correct gate is
  **`store_dead_after_block`**: a dominance walk where the init's dominance is **invalidated by
  any conditional reassignment** (inside `If`/`Loop`/`Iter`) — afterwards `local` may hold that
  block's store, so a read with `!dom` is an over-free hazard.  Descends into loop sources.

- **Result:** `shared_z` (5 sibling blocks) peak **8 → 5**; the post-block-read and escape
  shapes correctly do NOT confine (sound).  **Full suite 1923 ✅ with `LOFT_CONF_RECOVER=1`,
  both backends**; cluster-I probes 39/39 identical; `172` green both backends.  Gated off by
  default; the un-gate is a separate decision (the benefit is narrow — shared accumulator
  across sibling conditional blocks — and the gate is soundness-delicate, per the journey above).

> **Discovered sibling bug — FIXED (2026-06):** `fn f() -> vector { z=[a]; z=[b]; z }`
> **returned `[a]` not `[b]`** (a correctness bug, not a watermark one).  Root cause: the
> implicit (last-expression) return NRVO-promoted the returned local to the caller's buffer and
> built into it in place, so a second `z=[..]` appended onto the buffer instead of replacing.
> Fix: `ref_return` (`src/parser/control.rs`) does NOT NRVO-promote a local assigned a vector
> literal ≥2× (detected first-pass via distinct `OpNewRecord(z,…)` element-temps); it stays a
> normal local on the `__vdb` + return-copy path that explicit `return z;` already used.
> Regression `tests/scripts/173-reassign-return.loft` (straight-line / diff-length / nested,
> correct + leak-free, both backends).  **Residual (benign, known):** the *conditional*
> reassign-then-return shape (`z=[a]; if c {z=[b];} z`) now returns correctly but leaks the
> conditionally-allocated returned store at exit — the return-of-conditionally-allocated-vector
> free is missed; exit-safe, no test exercises it, a cluster-III-family follow-up.

## Re-measured 2026-08-21 — the gate still works, on 2.2× the tests

The 2026-06 result above was verified against a 1923-test suite. Re-run today:

| shape | sites | default | `LOFT_CONF_RECOVER=1` |
|---|---|---|---|
| consumed IN the block (confinable) | 2 / 4 / 8 / 16 | peak 6 / 8 / 12 / **20** | peak 5 / 5 / 5 / **5** |
| read AFTER the block (must not confine) | any | unchanged | unchanged |
| straight-line (shipped, last-use freeing) | 4 / 16 / 32 | peak 4 / 4 / 4 | peak 4 / 4 / 4 |

So the residual is **O(reassignment SITES)**, and it is the site count that drives it, not the
executed path: the same program with `k = 0`, `7`, `15` and `99` — one arm taken each time —
all read peak 20 at 16 sites. The gate turns that into a flat 5, reproducing the doc's
`8 → 5` exactly at 4 sites. Values are identical with the gate on and off, on BOTH backends.

**Full suite green with `LOFT_CONF_RECOVER=1`: 4232 passed, 33 skipped** — the evidence the
un-gate decision was parked on, refreshed against 2.2× the tests it originally had.

Two things this re-measurement corrects for whoever picks it up:

- **Measure the site count, not the loop count.** A sweep over loop ITERATIONS shows peak
  flat and reads as "no residual" — each call's stores are freed at return. The pin is
  within one scope, so the axis is how many reassignment sites that scope contains.
- **A post-block read makes the gate look inert.** `z = …; if c { z = … } z[1]` shows
  identical peaks with the gate on and off, which reads as bit-rot and is the soundness
  gate declining to confine — correctly, since that is the shape whose confinement returned
  the wrong element on the branch not taken.

Values under both settings are now pinned by `tests/scripts/reassign-across-sibling-blocks.loft`
(both backends, and written to pass with the gate either way — run the suite with
`LOFT_CONF_RECOVER=1` before changing the default). The watermark itself is not asserted:
loft cannot read its own store peak, so the numbers live in that file's header.

**Un-gated 2026-08-21 on this evidence.** The benefit is real and scales (O(sites) → O(1)),
the analysis still refuses the unsound shapes, and the suite is green with it on. The
narrowness stands — the shape has to be a shared local reassigned across sibling blocks and
never read after them — but a narrow win that costs nothing where it does not apply does not
need to stay behind a flag.

Both branches of the new flag are verified, which is the part worth keeping: flipping a
default makes the OLD path the untravelled one, so it was run too. Full suite **4232 passed
with the new default** and **4232 passed under `LOFT_NO_CONF_RECOVER=1`**.

One caveat for whoever reads a red run here: the first full run with the new default reported
`loft::gl_instancing instancing_bridge_draws_every_instance` TIMED OUT at 300 s. It passes
standalone in 3.6 s and passed on re-run — it launches a real Chrome and shells out six times,
so it starves under full-suite contention. Wall-clock for the same suite ranged 120–431 s
across runs on this machine. That test timing out is a load signal, not a Route 2 signal.

---

## Fix options (Stage C) — **active next focus (2026-06)**

Cluster III is the next piece, chosen because the shared-variable residual of cluster I-a
overlaps it (both are the single-valued-dep / overwritten-store mechanism above) — doing
III first subsumes that residual rather than fixing it twice.  Two landing sizes:
**last-use freeing** (canonical straight-line III + I-b, the bigger/higher-value mechanism)
vs **Route 2 backer-recovery** (the shared-block variant, smaller, builds on I-a).

**Chosen mechanism: last-use freeing via a definition-point liveness guard** — full
implementation plan in
[fix-design-last-use-freeing.md](fix-design-last-use-freeing.md) (diagnostic guard →
spike → freeing → promote to a permanent Goal-E assert → CI lock-in).

1. **Free the old store at the overwrite** (emit `OpFreeRef` of the prior DbRef before the
   new store is bound), guarded by the `&`-borrow check. Folds into cluster I's last-use
   freeing — do them together. Concretely needs the dep-system change (§ The single-valued-
   dep root): without it, the overwritten store has no recorded owner to anchor the free.
2. **Do nothing** (benign) and rely on the heuristic-floor fix to silence the warning.

## Probes

`14` (overwrite pin); contrast `02` (loop reuse).
