<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 118 — Store-lifetime UAF: a reference into a mutated vector reads null (glb/moros H4)

Tracks [`loft-lang/plans#118`](https://github.com/loft-lang/plans/issues/118) (`@PLN118`).
**Investigation-style plan** (mechanism understanding before fix design) —
[`_INVESTIGATION_TEMPLATE`](../_INVESTIGATION_TEMPLATE.md).

## Status

**Open — Stage A in progress; the null is ATTRIBUTED.** A loft program silently exports
`"min": [null, null, null]` into a glTF (moros H4). Stage A reproduced it (moros `5e677b7` → 3/32
nulled, interpreter) and walked the null to its source:
[`cluster-fold-reads-null.md`](cluster-fold-reads-null.md). It is **not** the `glb_pos_min` seed,
nor the returned `Vec3` — it is the **fold reading a periodic null pattern out of `me.vertices`**:
iterating that vector reads **48 of 84** `pos.x` as null in an **exactly period-7 pattern (3
real, 4 null)**, and only on the FIRST fold (`glb_pos_max` over the same vector, second, is
correct). Follow-ups then FALSIFIED the
"transient" reading: reading `me.vertices` directly in the consumer (no copy) already gives null,
and `glb_pos_max` reads the SAME nulls — so the corruption is **persistent** (48/84 records hold
a null `pos`, period-7 = one hex's 7 verts), and the min/max asymmetry is a **comparison
artifact** (`null < x` propagates in min; `null > x` is false so max ignores it). The handoff's
own "delegating the corner table clears it" localises the producer to moros's local
hex-corner-offset `Vec3` computation — the loft reading: a `Vec3` corner temporary is freed
before the vertex `pos` is written from it (a store UAF; the `Vec3` leak is the other end).
Probes 1-3 then collapsed the space: the pos is null **at construction** (read-back = pos, ruling
out later-overwrite / write-lost / stride), `off` is always real (mr_corner_offset is fine), and
`--native` is **CLEAN (0/32)** — so this is an **interpreter store-slot-reuse UAF**: a live `Vec3`
(`centre` / a corner temp) freed too early in `emit_hex_surface`'s corner loop, its slot reused by
a later allocation → the pos reads null. It is **layout-fragile** (a `println` moves the fault),
so arc B needs a non-perturbing, kt-tagged tracer; native is the correct differential oracle
(arc D). See [`cluster-fold-reads-null.md`](cluster-fold-reads-null.md). This README is the single source of truth for phase status.

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
| **A** — reproduce + attribute | **DONE** (interp). Mechanism class VERIFIED: an **interpreter store-slot-reuse UAF** in `emit_hex_surface`'s corner loop (a live `Vec3` freed early, slot reused → null pos); native-clean, producer-side, layout-fragile. Remaining: pin the exact freed slot (needs arc-B non-perturbing tracer) + a standalone probe | Mostly done |
| **B** — enhance the lifetime tool | **DONE.** The existing sound `LOFT_UAF_GEN` (per-slot gen + operand-stack shadow) catches H4 non-perturbingly where `LOFT_UAF` (variable-based) misses it. Added **free-site attribution keyed per-generation** (`keys.rs` `FREED_AT_GEN`, `state/mod.rs`) so it names the CAUSAL free, not the last occupant's — read+free both pinned (cluster doc) | **Done** |
| **C** — oracle | make B **non-vacuous**: a known-GOOD probe (must stay silent) + a known-BAD probe (must fire) — a positive control, per "a fooleable oracle is a liability" (plans/README.md). Graduate to a gate | Open |
| **D** — second implementation + switch | a conservative store-handling variant behind an env switch (e.g. **copy-on-held-reference** / no-relocate-under-live-borrow), run **differentially** (@PLN89 pattern) against the current path: identical output ⇒ same; divergence localises the bug. The safe variant is a flip-on **stopgap** while the real fix lands | Open |
| **E** — fix at the chokepoint | **ATTRIBUTED, not fixed.** Bytecode is CORRECT (frees at the right places); native runs it clean. Fault is the INTERPRETER's `OpCopyRecord`/`copy_claims` deep-copy of a `Vertex`'s nested `Vec3` into the vector element not finishing before the source `Vec3` temp is freed (layout-dependent, 4/7 dangle). Fix needs a minimal repro + an `OpCopyRecord`-nested-struct boundary matrix (interp vs native) — a blind edit to a core copy op risks the suite; NOT landed | Attributed, blocked on minimal repro |

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
