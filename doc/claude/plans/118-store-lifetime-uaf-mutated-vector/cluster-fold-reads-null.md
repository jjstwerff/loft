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

## Arc E — attributed to interp `OpCopyRecord` (NOT codegen); fix blocked on a minimal repro

Introspecting `emit_hex_surface` settled where the fault is NOT: **the bytecode is correct.**
`up`/`centre` are freed at the FUNCTION END (line 110); the loop body frees only per-iteration
temps (`pos`, `off`, the `__lift` vertex temp) AFTER `OpCopyRecord(__lift_4, _elm_2)` +
`OpFinishRecord` copy the constructed `Vertex` into the vector element. The dep model emitted the
frees in the right places.

`vertex(vp,vn,vuv) = Vertex { pos: vp, normal: vn, uv: vuv }` and `Vertex { pos: Vec3, normal:
Vec3, uv: Vec2 }` — so the copy is a **record with nested `Vec3`/`Vec2` sub-structs**. **Native
runs this exact IR clean (0/32); the interpreter nulls (3/32).** Therefore the fault is in the
**interpreter's execution** of `OpCopyRecord`/`copy_claims`/`OpFinishRecord`: the deep-copy of the
`Vertex`'s nested `Vec3` into the vector element is not fully materialised before the source `Vec3`
temp (`pos`/`up`) is freed — layout-dependently, so 4/7 records dangle. Loft's #1-weakness class
(nested-record deep-copy vs free ordering), interpreter-side only.

**Why the fix is NOT landed here (rigor / stop-condition).** A safe fix needs a minimal repro to
prove the working bytecode + a boundary matrix on `OpCopyRecord`-of-nested-struct-then-free (interp
vs native) — `OpCopyRecord` is a hot, core op, so a blind change risks the whole suite. Two minimal
attempts (`e_min`, `e_min2` — nested `V3`/`V2` structs, shared centre+up, field-arith `pos`,
per-iter append) did NOT reproduce; like the handoff's six, the trigger needs the full heap
(cross-package `mesh3d` types + path-dep are the untested axis). So arc E is **attributed but not
fixed** — the responsible next step is a focused `OpCopyRecord` boundary-matrix session (or a
cross-package nested-struct minimal repro), not a blind edit to the copy path.

## Arc B refinement — copy-vs-deref verdict (and it CORRECTS arc E)

The arc-B detector said *"prematurely freed"* but arc E argued the free is correct and the copy is
incomplete — a contradiction. To resolve which root it is, the detector now **distinguishes the two
roots that share the one symptom** (`src/keys.rs` `COPY_DEPTH` + `uaf_in_copy`, marked around
`copy_record`/`finish_record` in `src/state/io.rs`, reported in `src/state/mod.rs`):

- a stale read **while a record deep-copy is on the stack** → `INCOMPLETE RECORD-COPY` (the copy
  read a stale sub-ref; the source free is correct — fix the copy);
- a stale read **outside a copy** → `PREMATURE FREE` (a plain deref of a store freed while the ref
  was live — fix the free / the dropped dep).

**Verdict on H4 (VERIFIED, tool-backed):** every one of the 23 stale reads — including the load-
bearing `line 102` `emit_hex_surface` read — classifies **`PREMATURE FREE`, none `INCOMPLETE
RECORD-COPY`**, even though the IR confirms `OpCopyRecord`/`OpFinishRecord` DO run at line 102. So
the sound detector places the caught stale read **outside** `copy_record`/`finish_record` — it is a
plain operand-stack pop (the `vertex(…)` call-arg delivery or an `OpFreeRefIfDistinct`), not the
copy's source-pop.

**What this does to arc E's conclusion (honest correction).** Arc E's *"interp `OpCopyRecord`
deep-copy doesn't finish before the free"* was **inferred from the IR** (frees correctly placed →
must be an execution bug in the copy). The sound tool does **not** corroborate that: it puts the
stale read at a **premature free** at a non-copy pop, i.e. an **interpreter slot-reuse-while-
referenced** root (a live `Vec3` slot freed+reclaimed while an operand-stack ref is still live) —
NOT a copy-materialisation bug. Caveat that keeps this honest: `copy_claims` reads sub-refs via
`get_field` (a record read), which the operand-stack tracer cannot see, so "not during a copy"
cannot *disprove* a sub-ref copy read — but the tracer DID catch a real, separate premature-free
pop, and that is the stronger evidence. **Net: the `OpCopyRecord`-incomplete hypothesis is
downgraded to unconfirmed; the tool-supported root is a premature free / aggressive slot reuse.**
The next arc-E step follows the premature-free lead — identify which op pops the stale `Vec3` and
which free the causal free (`code_pos=74096`) is — rather than the `OpCopyRecord` boundary matrix.

## Arc E — RESOLVED: the interpreter never re-sentinels a freed variable (native does)

The premature-free lead ran to ground. Two cheap, non-perturbing instrument upgrades on the real
repro (synthetic reduction still fails — six+ attempts, same as the corruption) named the exact ops:

1. **Op-naming on `LOFT_UAF_GEN`** (`src/state/mod.rs`, `src/data.rs::operator_name`) — the stale-read
   report now prints the *reading* op (from `crash_report::last_context`) and the *freeing* op (already
   recorded in `FREED_AT_GEN`). Verdict on the load-bearing pair at `emit_hex_surface` line 102
   (`m.vertices += [vertex(pos, up, vec2(u,v))]`): **read by `OpPutRef`, freed by `OpFreeRef`**, the
   free at `code_pos=74096` *preceding* the read at `74134` within the one append statement.
2. **`LOFT_NO_SLOT_REUSE`** (`src/database/allocation.rs::find_free_slot`, arc-D stopgap) — never
   reclaim a freed slot. **The null vanishes: 3/32 → 0/32.** That is the decisive proof the corruption
   is **slot-reuse-while-referenced**, not a copy defect (a `GetFloat` read-trace also showed `centre.x`
   reads REAL at the `pos` computation — the old probe-2 "centre.x null" was a `println` artifact; the
   null is the stored *vertex temp* on a reused slot).

**The chokepoint (the interpreter/native divergence).** The compiler emits, per loop iteration, an
owned-reassign **pre-build `OpFreeRef`** on each `__lift` temp (free the previous value before rebuild)
AND a **block-exit `OpFreeRef`** (`get_free_vars`). Native's `OpFreeRef` of a *variable* additionally
resets it to the null sentinel (`generation/ops/ref_ops.rs`: `OpFreeRef(...); var_x.store_nr =
u16::MAX`), so the next iteration's pre-build free reads `u16::MAX` and no-ops. **The interpreter emitted
`OpVarRef + OpFreeRef` with no `OpInitRefSentinel`** — the variable kept pointing at the freed slot, and
the next iteration's pre-build free re-freed it; if the allocator had reclaimed that slot for a live
value meanwhile, the re-free destroyed it → the stored `pos` reads null on 4-of-7 verts. Native has no
operand stack and re-sentinels, so it is clean.

**The fix (`src/state/codegen.rs::generate_call`, `src/stack.rs`).** Track the vars that take the
unconditional pre-build free (`owned_reassigned`) and emit `OpInitRefSentinel` after their block-exit
`OpFreeRef` — mirroring native, scoped to exactly the vars that need it. **Interpreter-only** (native
already correct); **both backends now export 0/32.** Excludes retbuf/`OpDatabase`-reused locals like
`off` (they keep their DbRef for in-place reuse; sentinelling them destabilises `protect_store_frees`).

## Arc F — UNMASKED leak (the "other end"): the interpret↔shared-library `StaticCall` boundary

Fixing the corruption **unmasked a pre-existing, interpreter-only leak** — `Vec2×8160` on the moros
repro. This is the plan's separately-tracked "leak" severity: the corruption's over-free had been
*accidentally* freeing those stores, so `main` shows wrong data + no leak; the fix shows correct data +
the leak. **VERIFIED pre-existing:** `LOFT_NO_SLOT_REUSE` (an unrelated corruption fix) exposes the same
leak — independent of the codegen fix. **VERIFIED interpreter-only:** `--native` (`LOFT_NATIVE_LEAK_CHECK`)
is clean.

### Localized to a minimal repro (2nd investigation, this session)

The moros leak is `mr_corner_offset`'s returned `Vec2`, and it splits **exactly in half** — the two call
sites `emit_hex_surface` (line 98) and `hex_corner_world` (line 177). Line 98's copies all FREE their
source; `hex_corner_world`'s leak. Walked down to the **minimal repro** (no graphics Scene, so it runs):

```loft
use moros_render;
fn main() {
  n = 0;
  while n < 300 { c = hex_to_world(n, 0, 0); n = n + 1; }   // leaks 299× Vec3, interp only
}
```

`hex_to_world` returns `vec3(x,y,z)`. Two facts settle the root:
- **The callee bytecode is IRRELEVANT.** `n_vec3` (graphics-0.3.0) — like a plain LOCAL `vec3` — allocates
  a **fresh** store and ignores its `__retbuf` param. A purely-local `outer()` returning a local
  `inner()` (byte-for-byte the same shape) does **NOT** leak.
- **The ONLY difference that leaks is `StaticCall` vs `Call`.** The caller bytecode for `c = hex_to_world`
  (shared) and `c = outer` (local) is IDENTICAL — both `OpDatabase + protect + CopyRefOrNull(0x8000) +
  OpFreeRef`. Local resolves the callee to `Call(n_outer)` (runs its bytecode via `fn_call`/`fn_return`);
  shared resolves to `StaticCall(loft_shared_n_hex_to_world)` → `self.library[call]` (the installed
  library's own implementation, `state/mod.rs::static_call`). **Local doesn't leak, shared does.**

So arc F is a store-lifetime bug at the **interpret↔shared-library call boundary**: when interpreted code
calls an installed/shared function that returns a struct via the retbuf ABI, the returned store is
orphaned. A non-perturbing `LOFT_WATCH_PC`/`NTH` slot-lifecycle trace confirmed the leaked store is
**reused-without-`free_named`** mid-loop and leaked at loop end (never routed through a free); the
`LOFT_LEAK_SITES` report (now enriched with `free_protected`) confirmed the leaked stores are NOT stuck-
protected — they are simply never freed.

**Not fixed here:** it is a substrate-level issue in the `StaticCall` / `shared_store_dispatch` retbuf-
return contract, not the @P290 protect it superficially resembles. Per the debugging discipline (finish
the localization, route it — don't blind-patch a fragile ABI boundary), the fix belongs in a focused
session on the interpret↔native-library return path. It is a *resource* bug (correct output; at-exit for
the export consumer, but per-frame for a long-running game), strictly less severe than the *correctness*
corruption it replaces.

## Artifacts

- Null/real pattern (84 rows): `/tmp/h4pat.txt`; full timeline (275k lines): `/tmp/h4_timeline.txt`.
- Repro worktree: moros `5e677b7`. Native repro: clean, 0/32, no leak.
- Detector: `LOFT_UAF_GEN=1` on the repro (non-perturbing) — read + causal-free attribution + reading/
  freeing op names + copy-vs-deref verdict (H4 = all `PREMATURE FREE`).
- Corruption-vs-reuse proof: `LOFT_NO_SLOT_REUSE=1` (3/32 → 0/32). Leak provenance: `LOFT_LEAK_SITES=1`.
