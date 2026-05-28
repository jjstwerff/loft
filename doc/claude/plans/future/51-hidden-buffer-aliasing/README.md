# Plan 51 — Hidden-buffer aliasing under buffer reuse

**Status:** Future · investigation phase
**Trigger:** @P377 closure (S1 + S2 on `p377-fix` branch) revealed the underlying mechanism is broader than the originally-filed bug.
**Related:** `project_drop_store_refcount.md` (the documented long-term direction); @P378(a) `witness_buffer` correction; closed @P377 LEAK (S1) + CORRUPTION (S2).

---

## Context

`ref_return` (`src/parser/control.rs:3059`) promotes a function's returned heap local to a hidden caller-provided buffer parameter.  Inside the callee, the body may reassign that buffer (`cv = expr`), and codegen emits a pre-Set `OpFreeRef(cv)` at `src/state/codegen.rs:1367-1390` before evaluating the RHS.

When `cv` is a ref_return-promoted hidden buffer, this `OpFreeRef` **frees the caller's pre-allocated buffer mid-call**.  The system relies on two invariants to recover:

1. **First-fit slot recycling** (`src/database/allocation.rs:327::find_free_slot`) — the immediately-following allocation reuses the just-freed slot, so the caller's `DbRef(slot_nr)` remains valid (same slot_nr, new contents).
2. **No intervening allocations** between the free and the recycle.

Both invariants break in several composable shapes.  S1 (parse-time NRVO, `nrvo_collapse_tail_set`) and S2 (narrow codegen gate, `s1_substituted`) together close the canonical `cv = inner_call(...); cv` shape — they make the call write directly into `cv`'s slot, no free/realloc needed.  But probe-testing during the diagnostic surfaced additional shapes where the invariant still breaks and corruption / leak occurs.

This plan exists to **systematically catalogue the shapes** in this class, **understand the mechanism** thoroughly enough to choose between fix strategies, and **decide on the principled solution** (refcount, buffer-protocol redesign, extended parse-time substitution, or hybrid).

---

## Investigation documents

| Document | Coverage |
|---|---|
| [`RESULTS.md`](RESULTS.md) | Full 39-probe matrix, cluster definitions, verified-vs-hypothesized findings, fix-arc options |
| [`cluster-II-latent-leak.md`](cluster-II-latent-leak.md) | Per-iter Canvas leak (interpret-only) — pinned via IR diff of probes 01 vs 02; slot trace confirms RUNTIME mechanism |
| [`cluster-III-corruption.md`](cluster-III-corruption.md) | Silent data corruption (interpret-only) — probes 04 + 28 with distinct mechanisms; slot trace confirms RUNTIME mechanism |
| [`cluster-IV-codegen-panic.md`](cluster-IV-codegen-panic.md) | Codegen panic on BOTH backends — **verified mechanism**: `unify_if_branches_work_refs` substitution gap leaves leftover work-refs in main without Set IRs; confirmed across 7/7 probes via `LOFT_LOG=slots:n_main` |
| [`cluster-V-native-only.md`](cluster-V-native-only.md) | Native codegen gaps — tuple-of-heap-structs (probe 29) **verified** via `LOFT_KEEP_NATIVE_RS` (OpFreeRef-after-OpCopyRecord) + lambda-with-heap-return (probe 30, hypothesized) |

---

## Status & next-session roadmap (2026-05-28 — updated post-Cluster-IV fix)

**Stage A (probe catalogue): ✅ COMPLETE.**  39 probes, 5 clusters, both backends covered, A/B/C curation, real-library extractions (38 gridmesh, 39 moros_map), reference↔problem pairings.

**Stage B (mechanism investigation): 🟢 ~80% COMPLETE.**

**Stage D (implementation): 🟡 STARTED — Cluster IV done.**

| Cluster | Mechanism status | Fix status | Action needed next |
|---|---|---|---|
| I (canonical) | ✅ Fully understood | ✅ SHIPPED (S1+S2 on `p377-fix`) | None |
| II (latent leak) | 🟢 Runtime-only confirmed; child-store-orphan hypothesis | ⏸️ Investigated; M effort with over-free risk | Code-only investigation done; needs careful implementation of recursive child-store free in `OpDatabase` + scope-exit cascade. See [`cluster-II-latent-leak.md`](cluster-II-latent-leak.md) |
| III (corruption) | 🟢 Runtime-only confirmed; callee-modifies-local hypothesis | ⏸️ Not started | Same code path as Cluster II (Set-Reference codegen) — fold into Cluster II work |
| IV (codegen panic) | ✅ VERIFIED via slot trace | ✅ **FIXED 2026-05-28 (commit `d630e68b`)** | None — `caller_hidden_buf` flag + null-init relaxation closed the panic class on both backends.  See [`cluster-IV-codegen-panic.md`](cluster-IV-codegen-panic.md) |
| V probe 29 (tuple-return) | ✅ VERIFIED (OpFreeRef-after-OpCopyRecord in generated Rust) | ⏸️ Not started | Investigation agent run; needs implementation.  Likely fix: ownership-transfer pattern instead of copy-then-free |
| V probe 30 (lambda) | 🤔 Hypothesized | ⏸️ Not started | Capture generated Rust via `LOFT_KEEP_NATIVE_RS`; read lambda dispatch |
| Probe 39 (moros_map leak) | 🤔 NEW finding; mechanism unknown | ⏸️ Not started | Different mechanism from Cluster II per Cluster II's investigation; deep-slice borrow on the READ side |

**Total Phase D remaining: ~1-2 weeks** (Cluster II/III combined fix + Cluster V probe 29 + 30 + probe 39).

**Stage C (fix design): ⏸️ Pending Phase B-finish.**

Write `DESIGN.md` comparing:

| Approach | Effort | Subsumes |
|---|---|---|
| (a) Targeted per-cluster fixes | M for IV + M+ for II/III + M each for V | One cluster at a time |
| (b) Path C — store refcount | L (1-2 weeks) | II + III (+ subset of V) |
| (c) Hybrid: Path C for II/III + targeted for IV/V | L + M | Most of the class |

**~1 day to write DESIGN.md and decide.**

**Stage D (implementation): ⏸️ Pending Stage C decision.**

Recommended sequence (whichever design wins):

1. **Cluster IV fix first** — smallest, most contained, eliminates the ONLY hard-panic class affecting both backends.  Estimated ~M (3-5 days).  Ship as `@PLAN51-phase-IV-N` commits.
2. **Cluster V fixes next** — probes 29 + 30, native-specific.  Estimated ~M per shape (~M total).  Ship as `@PLAN51-phase-V-N`.
3. **Cluster II + III** — either Path C arc (1-2 weeks, one big landed change) or shape-by-shape extension (~M+ cumulative).  Decision from Stage C.

**Each implementation phase migrates its probes** from `probes/` to `tests/scripts/NN-<descriptive>.loft` per the plan's promotion rule.

### Aggregate effort estimate

| Phase | Time |
|---|---|
| Phase B-finish (mechanism verification for II/III/V/probe-39) | 1.5-2 days |
| Phase C (design) | 1 day |
| Phase D — Cluster IV implementation | 3-5 days |
| Phase D — Cluster V implementation | 2-3 days |
| Phase D — Cluster II/III implementation (path-dependent) | 1-2 weeks (Path C) OR 1 week (extended S1) |
| **Total to fully close PLAN51** | **2-4 weeks** |

**Quickest user-visible win after this point**: Cluster IV alone (~M), eliminates the only HARD-PANIC class.  Could ship as its own focused PR.

### What's already shipped on `p377-fix`

- Cluster I leak (S1, commit `6909177e`) + corruption (S2, commit `d7d6ebcf`).
- Both regressions auto-running in `loft_suite`.
- All 32 valid Stage-A probes + 5 edge-case + 2 real-lib probes (39 total) committed in plan dir.
- 4 cluster investigation docs + `RESULTS.md` + this README.
- Tool gap closed: `LOFT_KEEP_NATIVE_RS=1` (commit `1f101755`).

13 commits ahead of `origin/main` after rebase.

## Outcome

A choose-one decision artefact:

1. **Comprehensive probe suite** under `probes/` covering all known shapes that trip the aliasing bug, each with deterministic assertion(s) that turn corruption / leak into a loud failure.
2. **Mechanism reference** — formal description of when the pre-Set `OpFreeRef` is sound vs. destructive, with examples from the probe suite.
3. **Fix arc** — chosen approach (Path C / protocol redesign / parse-time extension / hybrid), with phased implementation plan.

When the fix lands, the probes graduate from `probes/` to `tests/scripts/` as permanent regressions, and the plan moves to `finished/`.

---

## Critical files (read-only context)

| File | Role |
|---|---|
| `src/state/codegen.rs:1367-1390` | Pre-Set `OpFreeRef` site; S2's `s1_substituted` gate at `:1378-1383`. |
| `src/parser/control.rs::nrvo_collapse_tail_set` (~line 720) | S1 helper.  Substitutes inner Call's hidden-buffer arg with outer LHS for the immediate `cv = call(...); cv` shape. |
| `src/parser/control.rs:3059::ref_return` | Promotes returned local to hidden buffer attribute (`hidden=true`, `become_argument`). |
| `src/database/allocation.rs:327::find_free_slot` | First-fit slot recycler; load-bearing for the slot-recycle invariant. |
| `src/codegen_runtime.rs:294::OpFreeRef` | Runtime free; trusts the DbRef without ownership validation. |
| `src/scopes.rs::paired_witness` / `witness_buffer` (lines 67-90) | @P378(a) correction for inner-scoped witness aliasing outer buffer — `OpFreeRefIfDistinct` emission.  Similar machinery may extend to this class. |
| `project_drop_store_refcount.md` | Documented long-term direction; eliminates the manual-free model entirely. |

---

## Probe suite (in `probes/`)

The probes are loft-only programs that turn the corruption / leak into deterministic assertion failures.  Each probe:

- Lives under `probes/` until the fix lands.
- Has a comment-header explaining the shape and what it's testing.
- Has an `@EXPECT_FAIL` annotation (or `@EXPECT_ERROR`) where appropriate, so the test suite can pick them up if we choose to wire them in during the fix arc.
- Migrates to `tests/scripts/NN-<descriptive>.loft` as a permanent regression when its shape is fixed.

### Probe catalogue (32 probes after Stage A runs)

After running 32 probes on both backends, the suite is curated into three groups.  See [`RESULTS.md`](RESULTS.md) for full failure-mode analysis and cluster definitions.

**A — Reference (10 probes).**  Pass on both backends.  Used as baselines for "what correct buffer-passing looks like" and to verify future fixes don't break working shapes.

| File | Shape | Why kept |
|---|---|---|
| `01-canonical-immediate.loft` | `cv = call(); cv` | S1+S2 reference; the @P377 leak shape |
| `06-interleaved-two-fns.loft` | 140 oracle (≥ 2 struct-param fns interleaved) | S1+S2 reference; the @P377 corruption shape |
| `12-deep-slice-return.loft` | `return m.items[0]` (deep-slice borrow) | Reference that deep-slice WORKS in isolation |
| `15-mut-ref-arg.loft` | `fn f(&out: T) { out.f = … }` (explicit &) | Reference for what correct buffer-passing looks like (user-explicit form) |
| `16-direct-return-call.loft` | `call()` as tail (no Set) | Reference: no Set = no leak |
| `17-chained-calls.loft` | `a = call1(); b = call2(a); b` (multi-local) | Reference: intermediate locals are safe |
| `18-match-tail.loft` | `match x { … => call_a(), … => call_b() }` | KEY CONTRAST with probe 08 (same shape, match works, if-tail panics) |
| `19-wrap-in-struct.loft` | `Wrapper { canvas: call(), … }` | Reference: struct-construction with heap fields works |
| `23-if-one-branch-call.loft` | if-tail with only ONE branch being heap | KEY CONTRAST with probe 08 (both-branch panics, one-branch fine) |
| `32-vec-of-canvases.loft` | `[call1(), call2(), call3()]` | Reference: collection-literal with heap elements works |

**B — Problem probes (10).**  One per distinct failure mode.  Each defines a bug class for diagnostic + fix work.

| File | Shape | Cluster | Failure |
|---|---|---|---|
| `02-double-set.loft` | `cv = a(); cv = b(); cv` | II | Leak Canvas×6 (per-iter) |
| `04-mixed-lit-call.loft` | `cv = {…}; cv = call(); cv` | III | Corruption iter 2: stale data |
| `07-explicit-return.loft` | `cv = call(); return cv;` | II | Leak ×6; S1 doesn't fire on explicit-return bodies (new finding!) |
| `08-if-tail.loft` | `if c { call_a() } else { call_b() }` (tail) | IV | PANIC both backends (`codegen.rs:2529`) |
| `13-recursive-fn.loft` | `fn f(n) { if n==0 { call() } else { f(n-1) } }` | IV | PANIC both backends (recursive variant) |
| `26-cond-never.loft` | `cv = a(); if false { cv = b(); }; cv` | II | Leak ×6 even though if-false never fires (codegen pattern, not runtime) |
| `28-only-conditional-set.loft` | `cv = init; if c { cv = call(); }; cv` | III | Corruption iter 2: condition true but result not propagated |
| `29-tuple-return.loft` | `fn f() -> (Canvas, Canvas)` | V | Native-only PANIC at generated Rust |
| `30-lambda-return.loft` | lambda body returning heap | III+V | Interp: stack corruption (iter=65535); native: PANIC |
| `33-if-with-explicit-returns.loft` | `if c { return a(); } else { return b(); }` | IV | PANIC both backends (explicit-return variant of 08) |

**C — Attic (12 probes).**  Variants and confirmations that don't add distinct insight.  Kept in the plan dir; not promoted.

| File | Why attic |
|---|---|
| `03-intervening-stmt.loft` | Variant of 02 (Class II); same failure mode, marginal additional insight |
| `05-nested-call.loft` | Passes; redundant with 01 |
| `09-vector-return.loft` | Passes; same mechanism as 01 with vector instead of struct |
| `10-loop-mutate.loft` | Passes; the `alloc_canvas` shape — already exercised by lib code in tree |
| `11-conditional-reassign.loft` | Variant of 02 + cond (Class II); same as 25/26 |
| `14-field-set.loft` | Passes; narrow case |
| `20-method-call.loft` | Passes; same mechanism as 01 via method dispatch |
| `21-many-iters.loft` | Confirmed linear scaling (×100 leaks per 100 iters) — finding captured in RESULTS.md |
| `22-if-simple-tail.loft` | Variant of 08 (Class IV); confirmed shape-not-arg dependence |
| `24-deep-slice-with-call.loft` | Passes; insight (deep-slice + interleaving still works) captured |
| `25-cond-always.loft` | Variant of 11 (Class II) |
| `27-if-as-local.loft` | Variant of 08 (Class IV); confirmed panic isn't tail-position-specific |
| `31-operator-return.loft` | Excluded — operator wasn't registered as an OpAdd; parse-level rejection, separate fix surface |

**The A + B set (20 probes) is the working investigation suite.**  C stays in the directory but informs decisions less directly.

### Real-library extraction probes (38-39)

Added 2026-05-28 to ground theory in production code shapes:

| File | Source | Status |
|---|---|---|
| `38-gridmesh-pattern.loft` | `lib/audience_crystal/audience_crystal.loft::crystal_segments_aged_tuned` + `gridmesh::seg_mesh_*` | ✅ PASS both backends |
| `39-moros-map-pattern.loft` | `lib/moros_map/moros_map.loft::map_get_hex` (deep-slice borrow into nested struct) | ⚠️ Interp: LEAK Hex×12 (1 per successful query × 12 queries); Native: clean |

**Probe 39 is a NEW finding** — distinct from probe 12 (which used a single-level Container and passed clean).  The TWO-LEVEL nesting (`m.m_chunks[k].ck_hexes[idx]`) and iterator-based access pattern (`for gh_c in m.m_chunks`) trigger a leak.  Likely sub-class of Cluster II (latent leak) but mechanism not yet pinned.  Grounds the @P377 historical citation of moros_map as the underlying shape.

### Debug-tool gaps closed during Stage B

Two tools added/verified for this plan:

| Tool | Status | Used for |
|---|---|---|
| `LOFT_KEEP_NATIVE_RS=1` | **NEW** — added in `src/main.rs` (3 cleanup sites gated on the env var) | Capture `/tmp/loft_native_*.rs` for post-mortem; immediately unblocked Cluster V probe 29 mechanism |
| `LOFT_LOG=slots:<fn>` | Already existed (@PLAN22 02d-vii); verified suitable for Stage B | Pinned Cluster IV mechanism across 7/7 panicking probes (`__ref_2 SKIP` + reason) |

Other Stage-B-useful tools are 5-line eprintln patches that can be added during the actual investigation work.

### Reference ↔ problem pairings

Each problem probe (B) has a closest-shape reference (A) that PASSES.  Diffing the pair is the diagnostic shortcut for understanding the failure mode.

| Problem | Reference | What the diff tells us |
|---|---|---|
| 02 (double-set, LEAK) | 01 (canonical) | Adding a second Set to cv defeats S1 → first Set's pre-Set FreeRef leaks |
| 04 (mixed-lit-call, CORRUPT) | 19 (wrap-in-struct) | 19 has struct-literal-with-heap-call as a SINGLE expression (correct); 04 has struct-literal as a separate Set then a Call Set — the sequence is broken |
| 07 (explicit-return, LEAK) | 01 (canonical) | Same body content, `return cv;` vs. bare `cv` tail → ref_return doesn't engage S1 for explicit-return bodies |
| 08 (if-tail, PANIC) | 18 (match-tail) AND 23 (one-branch) | Match works with same logical shape; one-branch call works.  Panic requires BOTH branches heap + if-form lowering |
| 13 (recursive, PANIC) | 17 (chained-calls) | Multiple calls in non-recursive sequence are fine; recursion through hidden buffer panics |
| 26 (cond-never, LEAK) | 01 (canonical) | If we put even a NEVER-EXECUTED `if false { cv = … }` block in a working body, it leaks.  Confirms leak is codegen-pattern-driven, not runtime |
| 28 (only-cond-set, CORRUPT) | 23 (one-branch) | 23 puts the conditional in the if-tail (works); 28 puts the conditional Set as a body statement before the tail (control-flow corruption) |
| 29 (tuple-return, NATIVE-PANIC) | 17 (chained-calls) | Both have multiple heap-returning locals; 17 returns one of them as Canvas, 29 returns both as `(Canvas, Canvas)`.  Tuple-of-heap-structs is the native blind spot |
| 30 (lambda, STACK-CORRUPT) | 16 (direct-return-call) | Same body shape (`call() as tail`); 16 is at file scope (fn), 30 is in a lambda local.  Lambda dispatch destroys the caller's frame |
| 33 (if-explicit-returns, PANIC) | 18 (match-tail) | Match with explicit-equivalent arms works; if with `return` in each branch panics.  The if-codegen path is the issue, not the explicit-return convention |

This pairing structure makes Stage B's mechanism work concrete: "trace probe X vs. probe Y under `LOFT_LOG=fn:F` and identify the divergent opcode emission".

---

## Stages

### Stage A — Probe-suite completion (1-2 sessions)

Goal: a comprehensive catalogue of shapes that trip the aliasing bug.

1. Run each probe under `--interpret` and `--native`.  Record PASS / FAIL / type-of-corruption per probe.  Native should always pass (its codegen uses Rust ownership).
2. Discover additional shapes from variants:
   - Tail wraps: `Return(Var(cv))`, `if cond { cv } else { cv }`, `match x { … => cv, … => cv }`.
   - Multiple hidden buffers: a fn with two ref_return-promoted locals.
   - Loop accumulators: `for x in xs { cv.field += x; }; cv`.
   - Conditional reassignment: `if cond { cv = a(); }; cv`.
   - Closures capturing buffers: `let f = |c| { c = mk(); c }; f(buf)`.
   - Recursion through hidden buffers.
3. For each new shape, add a probe under `probes/` with a one-paragraph header.
4. Update the table in this README with status.

Deliverable: every shape we can think of, with a deterministic probe.

### Stage B — Mechanism reference (1 session)

Goal: precise formal description of the buffer-aliasing invariant.

Write `MECHANISM.md` in this directory.  Cover:

1. Buffer-passing calling convention — how `ref_return` synthesizes hidden buffer attrs; how the caller allocates the buffer; how the callee accesses it.
2. Pre-Set `OpFreeRef` semantics — when it's sound (owned local; first-fit will recycle), when it's destructive (caller-owned buffer; intervening alloc breaks recycle).
3. `find_free_slot` interaction — why the first-fit invariant fails under interleaving.
4. The dangling-DbRef class — main's hidden buffer slots holding DbRefs to stores freed mid-call AND re-allocated to other variables.
5. Why native escapes — Rust ownership / `Drop` semantics.
6. Why S1 + S2 escape — parse-time substitution eliminates the mid-call free entirely for the canonical shape.

Reference each probe to ground the theory in observed behaviour.

### Stage C — Fix arc design (1 session)

Goal: chosen fix approach with phased implementation.

Write `DESIGN.md`.  Compare approaches:

1. **Extended parse-time substitution.**  Broaden S1 to cover more shapes (multi-Set; intervening stmt; if/else tails).  Each addition is a precondition relaxation.  Eliminates the mid-call free for shapes it covers.  Effort: S per shape, may need cumulative trade-offs.
2. **Buffer-protocol redesign.**  Callees never free caller's buffer; new content lands in caller's slot via in-place mutation.  Generalizes S1's principle to all assignment-to-hidden-buffer sites.  Effort: M+.
3. **Path C — full store refcount.**  Per `project_drop_store_refcount.md`, replace the closure-capture refcount with full store-ownership refcount.  Eliminates the manual-free model.  Effort: L (1-2 weeks).  Subsumes this plan and several others.
4. **Hybrid — narrow runtime guard + parse-time substitution.**  Track caller-aliased stores explicitly at the protect-bracket level (re-evaluation of the reverted Path B), combined with extended S1 coverage.  Effort: M.

Recommend a winning approach with rationale, then phase the implementation.

### Stage D — Implementation (multi-session, arc-dependent)

Phased per the chosen design.  Each phase:
1. Lands a focused commit.
2. Picks one probe (or probe family) and migrates it from `probes/` to `tests/scripts/NN-<descriptive>.loft`.
3. Updates the probe-suite table in this README.

### Stage E — Closure

When all probes are green and migrated:
1. Move plan from `future/` to `finished/`.
2. Update `doc/claude/PROBLEMS.md` — close any P-issues spawned during the investigation.
3. Update `doc/claude/QUALITY.md` open-work section if entries were added.
4. Reference the plan from any docs that cited the manual-free model.

---

## Out of scope

- Fixing every existing test that might silently corrupt — only addresses shapes the probe suite covers.
- Native backend changes — native already escapes the class.
- Other runtime-ownership topics outside the buffer-aliasing mechanism (e.g. text refcount, vector escape analysis).

---

## Decisions taken without user input (rationale)

- **Slot 51, in `future/`** — active plans cap (2-3) is exceeded; investigation isn't urgent enough to bump.
- **Probes in plan dir, not `tests/scripts/`** — keeps the suite stable while the shapes are unstable; explicit migration step on plan closure prevents drive-by additions.
- **Probe naming `NN-<short>.loft`** — numeric ordering for stable references; descriptive suffix.
- **No P-issue filing for variants yet** — the plan IS their tracking home.  If/when they need PROBLEMS.md visibility, file under a single umbrella row that points at this plan.
