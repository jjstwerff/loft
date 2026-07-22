<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster — the fold reads a periodic null pattern from `me.vertices`

Stage-A attribution of moros H4. Every mechanism statement is tagged **VERIFIED** (with the
observation that shows it) or **HYPOTHESIZED**.

## Reproducible anchor

- **VERIFIED.** moros `5e677b7` (handoff commit) exported via
  `loft --interpret --path <loft>/ --lib lib/ lib/moros_render/examples/demo_village.loft out.glb`
  → **3/32 accessors nulled** (`[0,8,12]`), each `min=[null,null,null]` with a correct `max`;
  `Warning: … kt=97 Vec3×2` leak alongside.
- **VERIFIED.** moros `7b106ec` (current main) → **0/32 nulled**, but the **same `Vec3×2` leak
  persists**. So the null is *consumer-commit-dependent* (heap-layout-fragile) while the leak is
  the *persistent* store-lifetime fingerprint — they are related but **not one fact** (the leak
  survives without the null).
- Backend: interpreter only so far. `--native` reproduction is **untested** (open Q3).

## Where the null is manufactured (walked in, not inferred)

Instrumenting `glb_pos_min` (`glb-0.1.2/src/glb.loft:51`) on the `5e677b7` repro:

1. **VERIFIED — not the seed.** `mx = verts[0].pos.x` reads a real value (`0`) for the meshes
   that end up nulled.
2. **VERIFIED — not the return store.** Logging `mx/my/mz` immediately before `vec3(mx,my,mz)`
   already shows `null`; the returned `Vec3` faithfully carries the null. So the leaked `Vec3`
   return store is a *separate* symptom, not the null's source.
3. **VERIFIED — the fold reads null from the vector.** A pre-scan `for v2 in verts { … v2.pos.x }`
   over the 84-vertex mesh reads **48 of 84** `pos.x` as `null`, in an **exactly periodic
   pattern, period 7 — 3 real then 4 null, repeated 12× (84 = 12·7)**. The real x-values come in
   groups of three (`0,0,1` · `0.866,0.866,1` · …).
4. **VERIFIED — transient.** `glb_pos_max(me.vertices)`, called immediately after over the *same*
   vector, reads correctly (min is the FIRST fold, max the second). So `me.vertices` is in a bad
   state on first full iteration and a good state on the second.

## What this rules in / out

- **Rules OUT null-flow / a `glb` semantic defect** (again): the null is real data read wrong,
  not an undischarged nullable — discharging `verts[0]?` would only paper over reads 1–4.
- **Rules OUT "a genuine null vertex":** the same vertices read correctly on the second fold and
  in other consumers; the value is real, the *read* is wrong.
- **Rules IN a store layout / materialisation defect on `me.vertices`.** A clean period-7 (3
  valid / 4 null) is a **stride or lazy-copy fingerprint**, not random corruption: the reader
  strides across data that is only partially present/valid on first access.

## Leading hypothesis (HYPOTHESIZED — for arc B/E to confirm or kill)

`me.vertices` is dereferenced on first access before its backing is fully materialised (a
lazy copy/borrow of the mesh's vertex store), OR `Vertex.pos` (a `Vec3`) sub-stores are
prematurely freed/aliased for a subset of records — the period-7 being the allocation/free
stride of those sub-stores, which would also explain the co-occurring `Vec3` leak (the two ends
of one broken lifetime: some `Vec3` freed-and-read-null, others never freed). The first full
iteration appears to *complete* the materialisation, so the second fold is clean.

## Immediate next steps (arc A→B handoff)

1. **Attribute the store.** Turn `LOFT_STORES=timeline` on the repro and correlate the vertex
   store's alloc/free/relocate events with the first-vs-second fold — is `me.vertices` freed,
   relocated, or copied between the mesh-vector build and the first `glb_pos_min`?
2. **Characterise the stride.** Dump the `Vertex` record layout glb reads vs moros builds
   (period 7 ≈ ? fields); confirm stride-mismatch vs sub-store-free.
3. **Native.** Re-run the repro on `--native` (open Q3) — shared model vs interp-only.
4. **Shrink under attribution.** With the store identified, minimise the moros scene toward one
   nulled mesh so the timeline is tractable and graduate a probe.

## Decisive update — persistent store corruption + a comparison artifact (transient hypothesis FALSIFIED)

Two follow-up instruments changed the picture and are the load-bearing result:

1. **VERIFIED — the null is in the SOURCE store, not the pass-by-value copy.** Reading
   `me.vertices[3].pos.x` **directly in `save_scene_glb`** (before any fold call) already gives
   `null` (`v3x=null v4x=null`, `v0x=0`). So `me.vertices` — the `vector<Vertex>` field on the
   scene-mesh loop var — genuinely holds null `pos` for those records; it is not a copy defect.
2. **VERIFIED — `glb_pos_max` reads the SAME nulls.** Instrumenting `glb_pos_max`'s seed shows
   `v3x=null v4x=null` too. So **both folds read the same 48/84 nulls** — the store corruption is
   **persistent, not transient**. The earlier "first-fold-only / lazy-materialisation"
   hypothesis is **FALSIFIED**.

**Why min nulls and max doesn't is a COMPARISON artifact, not two store states.** min does
`if v.pos.x < mx { mx = v.pos.x }`: a null `pos.x` compares **less-than** every real, so min
adopts it and returns null. max does `if v.pos.x > mx`: a null is **not greater-than** any real,
so max ignores it and returns the true maximum. Same data, opposite propagation. (Secondary
concern: a null float comparing as `< real` under `<` is its own semantics question — but it is
the *messenger* here, not the bug.)

### The refined mechanism (HYPOTHESIZED — strongly supported)

The corruption is **persistent null `pos` on 4 of every 7 vertices** (period-7 = one hex's 7
verts; 84 = 12 hexes). The handoff's own reduction — *"delegating the corner table to
`hex_grid::hex_corner_offset` clears it"* — localises it to moros's **local hex-corner-offset
computation** (the `HEX_WIDTH/2.0 = 0.866` path that correlates with every nulled mesh). The
loft-side reading: a **`Vec3` corner-offset temporary is freed before the vertex `pos` is
written from it** (a store-lifetime UAF), so 4/7 stored `pos` read the freed store's null; the
co-occurring `Vec3` leak is the other end of the same broken lifetime (some `Vec3` temporaries
never freed). This is the classic loft store-lifetime edge — a dep dropped on a `Vec3` temporary
that a pending write still needs.

### Where this points arc B / E

- **Arc B target sharpened:** the oracle must catch **a store record written from (or aliasing)
  a `Vec3` that was already freed** — i.e. a write-through / read-through a freed store — *not*
  merely a free-before-dependent-*read* (the existing static overlay). The `LOFT_STORES=timeline`
  tool is too coarse to see it (275k untagged alloc/free lines; no kt tag, no value↔store map) —
  which is itself the tool gap: B needs a **kt-tagged, value-attributed** deref/write check.
- **Next Stage-A step:** instrument moros's `emit_hex_surface` / corner-offset to catch the
  exact `Vec3` temporary whose free precedes the vertex-`pos` write (the producer site), then
  minimise from that one hex.
- **Do NOT** "fix" via the comparison (guarding `null` in min) nor via discharging `glb` — both
  mask a genuinely corrupt store.

## Probes 1–3 — the hypothesis space collapsed

The producer is `emit_hex_surface` (moros_render.loft:90): per hex it builds **1 centre + 6
corner** verts (= the period-7), each corner `pos = vec3(centre.x + off.x, centre.y, centre.z +
off.y)` with `off = mr_corner_offset(ci)` (the local corner fn the handoff flagged).

- **Probe 2 (read-after-write) — VERIFIED.** Instrumenting right after each vertex append:
  `off.x`/`off.y` are **always real** (mr_corner_offset is NOT the culprit); `pos.x` is null for
  **ci ≥ 2**; and the read-back of the just-stored vertex **equals `pos`**. So the vertex `pos` is
  null **at construction**, not overwritten later. This **rules out (c) later-overwrite, (b)
  write-never-happens, (d) stride-write** — it is a value that is already null when written,
  i.e. **(a) a store UAF/reuse**: `vec3(centre.x + off.x, …)` reads a `Vec3` (`centre`, or a slot
  it shares) that has been freed/reused after the loop's first couple of `vec3()`/`vertex()`
  allocations.
- **Probe 1 (temporal) — VERIFIED (producer-side).** The null is present at the emit site, so the
  corruption is a producer event, not a later scene-assembly/relocation one.
- **Probe 3 (native) — VERIFIED, and it's the big one.** `--native` exports **0/32 nulled** — the
  bug **does not reproduce on the native backend**. So it is **interpreter-specific**: the fault
  is in the interpreter's store allocation/reuse path, not the shared IR or the native codegen.
  (Open Q3 answered.) This also hands arc D its oracle for free: **native is the correct
  reference** to run the interpreter differentially against.
- **META — the bug is layout-fragile and loft-level instrumentation PERTURBS it.** Reading
  `centre.x` at the loop *top* shows it correct for all ci, but the after-append probe shows the
  `pos` null for ci ≥ 2 — moving a `println` (which itself allocates format/text stores) shifts
  the heap and moves the fault. This is the plan's thesis made concrete: `println`-probing can't
  attribute this, and `LOFT_STORES=timeline` is too coarse (275k untagged lines) — **arc B needs
  a non-perturbing, kt-tagged, value→store-attributed reuse/UAF tracer.**

### Net mechanism (VERIFIED class; exact slot HYPOTHESIZED)

An **interpreter store-slot reuse** frees a live `Vec3` (`centre` / a corner temp) too early in
`emit_hex_surface`'s corner loop, and a subsequent allocation reuses the slot, so `pos` for the
later corners reads null. Persistent (both folds see it), producer-side, native-clean,
layout-fragile — and the co-occurring `Vec3` leak is the same broken lifetime's other end.

## Arc B — the tool caught it (existing sound detector) + the free-site enhancement

Rather than build a tracer, the repo already had one: **`LOFT_UAF_GEN`** (@PLN54 S3 / cluster-462) —
a per-slot generation bumped on free + an operand-stack shadow stamping each DbRef's gen at push;
a shadow-vs-current mismatch at consume is a freed-then-reused stale read. It is **Rust-level,
non-perturbing** (the null stays 3/32 under it) — exactly what `println`-probing could not be.

- **It catches H4:** `store #50 (Vec3) was gen 5 at push, gen 14 at read — read at line 102`
  (`emit_hex_surface`'s `m.vertices += [vertex(pos, up, …)]`). `LOFT_UAF` (the heuristic,
  variable-based sibling) MISSES `emit_hex_surface` entirely — the stale ref is on the operand
  stack (a shared temp), which only the gen-detector's shadow sees.
- **The gap → the arc-B enhancement (implemented):** the gen-detector reported the *read* site but
  not the *free* site. Added free-site attribution (`src/keys.rs` `FREED_AT_GEN`, `src/state/mod.rs`
  dispatch + report), keyed **per-generation** so a heavily-reused slot names the **causal** free
  (the one at `stamped+1`) not the last occupant's — for #50 that corrected line 476 → **line 102**.
  All behind the existing opt-in env gate; zero cost when off.

### The chokepoint (for arc E)

The causal frees land IN the vertex-append copy:
- `store #52`: **read at line 95** (`vertex(centre, …)` centre-vertex append) → **freed at line 102**
  (the corner append).
- `store #50`: **read at line 102** → **freed at line 102**.

So `m.vertices += [vertex(V, up, vec2(…))]` (emit_hex_surface:95,102) **prematurely frees a `Vec3`
that a still-live operand-stack ref reads** — the shared `up` normal (passed to every one of the 7
`vertex()` calls) and/or the vertex temp: a dropped dep on a value the append copy still needs. The
fix (arc E) is at that append/copy free, in the interpreter's store path (native is clean).

## Artifacts

- Null/real pattern (84 rows): `/tmp/h4pat.txt`; full timeline (275k lines): `/tmp/h4_timeline.txt`.
- Repro worktree: moros `5e677b7`. Native repro attempt: `/tmp/h4_native.glb` (clean, 0/32).
- Detector: `LOFT_UAF_GEN=1` on the repro (non-perturbing) — read + causal-free attribution.
