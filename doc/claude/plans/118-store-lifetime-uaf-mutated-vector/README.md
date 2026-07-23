<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 118 — Store-lifetime UAF: a reference into a mutated vector reads null (glb/moros H4)

Tracks [`loft-lang/plans#118`](https://github.com/loft-lang/plans/issues/118) (`@PLN118`).
**Investigation-style plan** (mechanism understanding before fix design) —
[`_INVESTIGATION_TEMPLATE`](../_INVESTIGATION_TEMPLATE.md).

## Status

> **CLOSED 2026-07-22 — BOTH ENDS FIXED (corruption arc E + leak arc F), both with graduated
> regressions; full suite 3385/3385 green on both backends.**
>
> **Arc E (corruption).** The interpreter's `OpFreeRef` of a variable did **not** reset it to the null
> sentinel (native's does), so a loop temp freed at the block-exit sweep was **re-freed** by the next
> iteration's pre-build free — and if the allocator had reclaimed that slot, the re-free destroyed a
> live value → stored `pos` reads null on 4-of-7 verts. Fix: `src/state/codegen.rs::generate_call`
> emits `OpInitRefSentinel` after the block-exit free of the owned-reassigned vars
> (`src/stack.rs::owned_reassigned`), mirroring native, interpreter-only. Proven by `LOFT_NO_SLOT_REUSE`
> (3/32→0/32) + op-named `LOFT_UAF_GEN`. **Regression:**
> `tests/codegen_emitter.rs::pln118_arce_owned_reassign_emits_sentinel` — an emission-structure guard
> (the symptom is layout-fragile and unrepro'able in-tree; see *Regression migration* below).
>
> **Arc F (the unmasked leak) — FIXED (oracle-first).** Fixing E unmasked a pre-existing interp-only
> leak. Following the retrospective's rule this time, arc C (the differential oracle,
> [`oracle/`](oracle/)) + a self-contained synthetic probe matrix ([`probes/`](probes/)) were built
> FIRST, and the root fell straight out — correcting the prior localization on two points (both
> stale-cdylib / attribution artifacts):
> - It is **not "cross-lib `StaticCall`"** (a stale-cdylib confound); the real axis is a **nested-call
>   return vs a direct struct-literal return** — same-lib nested leaks identically to cross-lib.
> - The leaked store is **the bridge's own FALLBACK destination**, not the caller's retbuf. When the
>   interpreted caller forwards a null hidden-dest retbuf (the nested-return codegen re-sentinels it
>   each iteration), `native_lib.rs::shared_bridge_wrapper` allocates a fallback record; the inner
>   struct-literal callee ignores its retbuf and returns a fresh store, **orphaning the fallback —
>   one leaked store per call**. Interp-only: whole-`--native` has no bridge.
>
> **Fix:** the bridge frees the fallback dest after the call when the callee returned a different store
> (`(*ret).dbref` differs). Gated by `LOFT_NO_BRIDGE_ORPHAN_FREE` (the **arc-D switch + the oracle's
> positive control**). Verified on the whole matrix + the real moros F1, both backends; regression
> `tests/n3_parity.rs::shared_bridge_nested_return_no_orphan_leak`. Full trail:
> [`cluster-fold-reads-null.md`](cluster-fold-reads-null.md) § Arc F — RESOLVED.

### Regression migration (both clusters guarded)

| Cluster | Guard | Kind |
|---|---|---|
| **Arc F** (leak) | `tests/n3_parity.rs::shared_bridge_nested_return_no_orphan_leak` | runtime leak diff; `LOFT_NO_BRIDGE_ORPHAN_FREE` is the non-vacuous positive control |
| **Arc E** (corruption) | `tests/codegen_emitter.rs::pln118_arce_owned_reassign_emits_sentinel` | emission-structure guard |

Arc E's corruption is **layout-fragile** — it reproduces only on the external moros `5e677b7` scene.
The in-tree `demo_village` fixture exports **0/32 even with the fix reverted** (verified during
close-out), so **no in-suite SYMPTOM repro is possible** — a driver replaying the real
`emit_hex_surface` across 40 meshes stays clean both with and without the fix. The fix's emitted
signature IS deterministic, though: the minimal owned-reassign-in-loop program emits exactly **one**
`InitRefSentinel` with the fix and **zero** without it (verified by neutralizing
`owned_reassigned.insert` in `generate_call` and re-introspecting). The arc-E guard asserts that
signature — non-vacuous by construction, guarding the FIX rather than the unreproducible symptom.

This README is the single source of truth for phase status; the full mechanism trail is in
[`cluster-fold-reads-null.md`](cluster-fold-reads-null.md).

## Goal

Turn a silent, non-deterministic store-lifetime corruption into a **named invariant enforced at
one chokepoint**, with the tooling to catch the whole class — not just this instance. Deliverable
(investigation): the verified mechanism + the fix-design decision + a permanent oracle.

## The bug — what we are characterising

- **Symptom.** `glb_pos_min(verts)` seeds `mx = verts[0].pos.x` (index 0, guarded by
  `verts.len()==0`) and returns `vec3(mx,my,mz)`; for 3 of 32 meshes all three components come
  back null. `glb_pos_max` — **byte-identical with `>` for `<`**, same data, same run — is always
  correct.
- **The null is manufactured, not carried.** All 7796 vertices scan non-null; the consumer's own
  identical fold is correct on the same meshes. So the value is real everywhere observable and
  becomes null *inside* the call.
- **Two severity fields, tracked separately** (a false-fix trap otherwise): **corruption** (the
  null min) + **leak** (`Warning: … kt=97 Vec3×2` — `Vec3` is `glb_pos_min`'s return type; the
  leak and the null "have never appeared apart").

### Verified vs hypothesized (accountability — do not let hypotheses drift into fact)

| Claim | Status |
|---|---|
| `glb_pos_min`/`glb_pos_max` are structurally identical bar `<`/`>` | **VERIFIED** — `glb-0.1.2/src/glb.loft:51,68` read directly |
| min nulls, max correct, same run/data | VERIFIED (handoff, re-tested 3 builds incl. 16:34) |
| all vertices non-null; consumer's fold correct | VERIFIED (handoff scan) |
| `Vec3` leak co-occurs with the null | VERIFIED (handoff; survives 16:34 build) |
| the null is in the **fold reading `me.vertices`** (period-7 3-real/4-null, first fold only), NOT the seed or the return `Vec3` | **VERIFIED** — instrumented `glb_pos_min` on the `5e677b7` repro (cluster doc) |
| root = `me.vertices` store layout / lazy-materialisation defect (stride mismatch or premature `Vec3` sub-store free), linked to the `Vec3` leak | **HYPOTHESIZED** — the refined mechanism; arc B must confirm/kill |
| "transient bad store state on first access" | **FALSIFIED** — `glb_pos_max` reads the SAME nulls; the source store (read directly, no copy) is null before either fold |
| the corruption is **persistent** (48/84 records null `pos`); min/max asymmetry is a **comparison artifact** (`null < x` true, `null > x` false) | **VERIFIED** — direct `me.vertices[3]` read + `glb_pos_max` seed instrument |
| the pos is null **at construction** (read-back = pos); rules out later-overwrite / write-lost / stride | **VERIFIED** — probe 2 (read-after-write in emit_hex_surface) |
| `off` (mr_corner_offset) is always real — not the culprit | **VERIFIED** — probe 2 |
| the bug is **interpreter-specific** (native exports 0/32) | **VERIFIED** — probe 3 (`--native`) |
| the bug is **layout-fragile**; loft `println` instrumentation perturbs it | **VERIFIED** — centre.x reads real at loop-top but the after-append pos is null |
| root class = an **interpreter store-slot-reuse UAF** on a live `Vec3` (`centre`/corner temp) in the corner loop; leak = its other end | **VERIFIED class** (exact slot HYPOTHESIZED) |

## Stage A — the probe matrix (write probes BEFORE reading more source)

Probes live in [`probes/`](probes/); each varies **one** composition axis, uses distinctive
values, asserts **value AND length AND leak** on **both backends** (`--interpret` + `--native`),
and hand-computes the expected cell. At least one probe is **extracted from the real moros path**
(not only synthetic). The bug is *understood* when a probe-pair diff pins the boundary; a
no-output cell is vacuous — reject it. Six prior synthetic reductions (in the handoff) stayed
clean, so the trigger is a specific composition — the axes below are chosen to cross it.

| Axis | Cells |
|---|---|
| held reference vs owned copy | `f(m.vertices)` where `m` is a **borrow into** `vector<Mesh>` · vs a `let owned = m.vertices` copy first |
| mutation during hold | append to the `vector<Mesh>` **after** taking `m` (grow/relocate) · vs no append |
| lookup shape | **by-name scan then append** (`emit_to_material`) · vs index append |
| call order | first-fold-over-store (min) · second-fold (max) — does *only the first* null? |
| provenance | element-store built via `emit_*` by-value `Mesh` param mutation (handoff row 6) · direct |
| package boundary | path-dep (`{ path = ... }`) beside registry pkgs, `--lib` · vs single program |
| backend | `--interpret` · `--native` (values identical) |

The load-bearing cell: **reference-into-relocated-vector reads the null sentinel** — the moment a
`verts[0]` deref returns null where the vertex is provably non-null.

## Arcs

| Item | Concern | Status |
|---|---|---|
| **A** — reproduce + attribute | **DONE** (interp). Mechanism class VERIFIED: an **interpreter store-slot-reuse UAF** in `emit_hex_surface`'s corner loop (a live `Vec3` freed early, slot reused → null pos); native-clean, producer-side, layout-fragile. Exact slot pinned in arc E; a standalone symptom probe is provably impossible (the in-tree scene exports 0/32 with the fix off) so arc E's guard is emission-structure, not symptom | **Done** |
| **B** — enhance the lifetime tool | **DONE + refined.** The existing sound `LOFT_UAF_GEN` (per-slot gen + operand-stack shadow) catches H4 non-perturbingly where `LOFT_UAF` (variable-based) misses it. Added **free-site attribution keyed per-generation** (`keys.rs` `FREED_AT_GEN`, `state/mod.rs`) so it names the CAUSAL free. **Refinement:** a **copy-vs-deref verdict** (`keys.rs` `COPY_DEPTH`/`uaf_in_copy`, marked around `copy_record`/`finish_record` in `state/io.rs`) — a stale read *during* a record deep-copy is `INCOMPLETE RECORD-COPY` (fix the copy), one *outside* is `PREMATURE FREE` (fix the free). This turns "prematurely freed" (which pointed at a correctly-placed temp free) into a decisive root. **H4 verdict: all 23 reads = `PREMATURE FREE`, none copy** — which CORRECTS arc E (below) | **Done** |
| **C** — oracle | **DONE (this session, arc-F fix built on it).** [`oracle/leak-oracle.sh`](oracle/) — a **differential interp-vs-native leak oracle** (@PLN89 pattern): native is the clean reference (no bridge), so the interp-minus-native leaked-store set IS the bug. [`oracle/run-matrix.sh`](oracle/run-matrix.sh) drives the matrix; the FLIP (`--flip` = `LOFT_NO_BRIDGE_ORPHAN_FREE=1`) is the positive control that MUST fire, the direct-return probe is the negative control that MUST stay clean. One run gave the boundary a dozen ad-hoc traces did not | **Done** |
| **D** — second implementation + switch | **Two, both kept.** Arc E: `LOFT_NO_SLOT_REUSE` (`allocation.rs::find_free_slot`, 3/32→0/32). Arc F: **`LOFT_NO_BRIDGE_ORPHAN_FREE`** (`keys.rs`) — disables the bridge orphan-free; run differentially it resurrects the leak, the decisive proof + the oracle's positive control | **Done** |
| **E** — fix at the chokepoint | **DONE — both backends 0/32, suite 3385/3385 green.** The interpreter's `OpFreeRef` of a variable did **not** reset it to the sentinel (native does), so a loop temp freed at the block-exit sweep was **re-freed** by the next iteration's pre-build free of a possibly-reclaimed slot. Fix: `generate_call` emits `OpInitRefSentinel` after the block-exit free of `owned_reassigned` vars (`src/stack.rs`), mirroring native, interpreter-only. Regression `tests/codegen_emitter.rs::pln118_arce_owned_reassign_emits_sentinel` (emission-structure guard, non-vacuous 1-vs-0 differential) | **Done** |
| **F** — the unmasked leak (other end) | **DONE — fixed at the bridge chokepoint.** Oracle-first this time. Root (corrected): a **nested-call return** (not "cross-lib") drives the interpreted caller to forward a null hidden-dest retbuf, so `native_lib.rs::shared_bridge_wrapper` allocates a **fallback dest** that the inner struct-literal callee ignores and orphans — one leaked store per call, interp-only (whole-`--native` has no bridge). Fix: the bridge frees the fallback dest when the callee returned a different store; gated by `LOFT_NO_BRIDGE_ORPHAN_FREE`. Regression `tests/n3_parity.rs::shared_bridge_nested_return_no_orphan_leak` | **Done** |

## Phase ordering

1. **A** first — no fix on the first clean read; the boundary (which composition triggers it) is
   the spec. Extract the real-path probe early; synthetic-only missed it six times.
2. **B alongside A** — if repeated probes won't converge (non-monotonic "sometimes nulls"), that
   is the signal the *tool* is blind, not that the bug is subtle: upgrade the inspector to
   ATTRIBUTE the null to its store cause before theorising further (the blind-instrument rule).
3. **C** — lock B behind a positive control so it can't silently stop catching the class.
4. **D** — the differential switch both *localises* (divergence = the edge) and *de-risks* (the
   conservative variant unblocks moros while E is designed). Decide keep-as-safety vs retire.
5. **E** — fix where the fact is PRODUCED (the dropped dep / the relocation), not where the null
   is consumed in `glb`. **Do NOT** discharge `glb`'s `verts[0]?` seed as the resolution — that
   converts the corruption to a plausible `0.0` and masks the loft bug.

## Open questions

1. **Relocation vs free.** Is `verts[0]` reading a *relocated* store (vector grew, element moved)
   or a *freed* one (dropped borrow-dep, over-eager `OpFreeRef`)? B's attribution decides; the fix
   differs (pin the dep vs suppress the free).
2. **Why first-only.** Why does min (first) null and max (second) not — does the first fold's own
   allocation (the leaked `Vec3`) perturb the heap, or is the store lazily materialised on first
   deref? The leak↔null link (are they one fact?) is the tell.
3. ~~native vs interpret~~ **ANSWERED: interpreter-only** (native exports 0/32). The fix is in
   the interpreter's store allocation/reuse path; native is the correct differential oracle (arc D).

## Cross-arc dependencies / see also

- **[LIFETIME.md](../../LIFETIME.md)** — the deps/free model + the @PLN103 inspector (the tool B
  extends) and its stated blind spot; **[OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md)** — the deps
  borrow-system north-star the violated invariant lives in.
- **[@PLN85 store-lifetime retirement](../85-store-lifetime-retirement/README.md)** — the
  bytecode-comparison method + prior UAF chokepoints; **[@PLN89 differential oracle](../89-differential-oracle.md)**
  — the switchable-second-implementation pattern arc D reuses.
- **moros H4 handoff** — `moros/doc/claude/LOFT_HANDOFF.md`; the consuming glb
  (`glb-0.1.2/src/glb.loft:51`) and the moros `emit_to_material` by-name-append site.
- Bug-fix discipline: the `engineering-rigor` + `loft-debug` + `loft-codegen` skills; matrix-first
  (CLAUDE.md § Debugging policy).
- [`loft-lang/plans#118`](https://github.com/loft-lang/plans/issues/118) — the tracking issue.
