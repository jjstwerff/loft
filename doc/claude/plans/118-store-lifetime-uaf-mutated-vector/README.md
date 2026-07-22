<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 118 — Store-lifetime UAF: a reference into a mutated vector reads null (glb/moros H4)

Tracks [`loft-lang/plans#118`](https://github.com/loft-lang/plans/issues/118) (`@PLN118`).
**Investigation-style plan** (mechanism understanding before fix design) —
[`_INVESTIGATION_TEMPLATE`](../_INVESTIGATION_TEMPLATE.md).

## Status

**Open — Stage A not started.** A loft program silently exports `"min": [null, null, null]`
into a glTF (moros H4): a real, non-null value becomes null inside `glb_pos_min`, with **no
diagnostic anywhere** (producing package 161 tests / 0 warnings; `loft --check` clean). The
loft-internals diagnosis is a **store-lifetime / reference-invalidation bug** (not null-flow),
in loft's #1 weak area. This plan characterises the mechanism with matrix-first probes, upgrades
the @PLN103 lifetime inspector to *see* this class (its documented blind spot), uses that as an
oracle, and validates the fix differentially against a switchable conservative implementation.
This README is the single source of truth for phase status.

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
| root = a reference into a **mutated `vector<Mesh>`** (by-name-lookup-then-append) relocating the element store → stale `verts[0]` read → null sentinel | **HYPOTHESIZED** — the leading mechanism; Stage A must confirm/falsify |
| "first call nulls, second correct" ⇒ a transient bad store state on first access | HYPOTHESIZED (consistent with the min-before-max order) |

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
| **A** — reproduce + boundary | matrix probes → minimal repro that nulls on BOTH backends; map pass/fail to the real store edge (the filed "by-name append" scope is a hypothesis) | Open |
| **B** — enhance the lifetime tool | close the @PLN103 inspector's **missing-dep blind spot** (LIFETIME.md:677): a runtime check that flags a **deref of a store whose backing record was relocated/freed** and ATTRIBUTES the null to that store edge (extend `LOFT_STORES=timeline`/the store `read_only`/relocation path; and a static `free-before-dependent-read` sibling for the *dropped-dep* case) | Open |
| **C** — oracle | make B **non-vacuous**: a known-GOOD probe (must stay silent) + a known-BAD probe (must fire) — a positive control, per "a fooleable oracle is a liability" (plans/README.md). Graduate to a gate | Open |
| **D** — second implementation + switch | a conservative store-handling variant behind an env switch (e.g. **copy-on-held-reference** / no-relocate-under-live-borrow), run **differentially** (@PLN89 pattern) against the current path: identical output ⇒ same; divergence localises the bug. The safe variant is a flip-on **stopgap** while the real fix lands | Open |
| **E** — fix at the chokepoint | enforce exactly the violated invariant (the missing borrow-dep / the relocation-under-live-reference), no narrower/wider; verify the full matrix both backends; graduate probes to `tests/scripts/` | Open |

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
3. **native vs interpret.** Does it reproduce identically on both, or is it interp-only (a
   different store path)? Determines whether the fix is in the shared dep model or a backend.

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
