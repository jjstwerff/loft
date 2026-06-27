<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Copy-vs-borrow elision — derive the materialization mode from USE, not RHS shape

> ## OPEN TICKET — Tier-1.5: elide a vector whose ELEMENT is borrowed (`e = v[i]`)
> **Why:** Tier-0 *refuses* to elide a `v` that another var borrows (`borrowed_by_other`
> in `scopes::elide_borrows`) — the conservative fix for the dangling-dep codegen panic.
> But that fallback leaves the **most common accessor shape copying**: validated on
> crawler, **~20 functions** (`sim_enemy_hp`/`_alive`/`_pos`/…, `sim_dead_indices`,
> `front_enemy`, `aim_target`, `enemy_blocks`) all do `enemies = s.enemies; e = enemies[i];
> … e.field` and so copy the source vector per call — exactly the residual slow paths.
> The @PLN85 regression corpus (28/28) passes and elides cleanly; this gap is consumer-shaped.
>
> **Fix:** when eliding `v = s.f` (inlining `v`→`S`, `S = OpGetField(s,…)`, base var `s`),
> instead of refusing on a borrower `e` (dep ∋ `v`), **re-point `e`'s dep `v → base_var(S)`**
> (`make_independent(e, v)` + `depend(e, s)`). After inlining, `e = v[i]` becomes
> `e = (s.f)[i]` — `e` borrows the live source element, owned by the param `s`; the
> borrowed-view codegen (`stack(dep[0])`) then reads `s`'s valid slot. No panic, and the
> accessor elides.
>
> **Safety gate (load-bearing):** re-point + elide ONLY if every borrower `e` is itself
> **read-only**. Under elision `e` aliases the LIVE source, so a write through `e` would
> hit `s` (D1 on the borrower) — whereas copy-mode `e` wrote the discarded copy. If any
> borrower is mutated, keep the copy (value-semantics fallback). Reuse the use-analysis to
> classify each borrower (copy-idiom-aware, since `e`'s own def is a `Set`).
>
> **Validation:** matrix {read-only borrower, mutated borrower} × {index-read `v[i]`,
> field-read `v.x`} × both backends + leak; assert the ~20 crawler accessors elide and the
> @PLN85 corpus stays green; confirm the 8× extends to the accessor-heavy paths. Diagnostic:
> `LOFT_ELIDE_REFUSED` lists every fallback so the gap is measurable. **Goal: elision is the
> only materialization for every borrowable case; copy remains ONLY where a mutated/escaping
> local genuinely needs value semantics.**

> **Status:** **Tier-0 LANDED, default-on** (opt-out `LOFT_NO_BORROW_ELIDE`). Decision
> layer `src/use_analysis.rs`; inline rewrite `scopes::elide_borrows`. The default-on
> cutover was reverted once — the crawler dogfood surfaced a codegen panic the suite
> missed: eliding `v` left any var that BORROWS `v` (`e = v[i]`) with a stale dep, so the
> borrowed-view codegen dereferenced the dead slot (`codegen.rs` `stack(dep[0])`). Fixed
> at the chokepoint: `elide_borrows` refuses to elide a `v` that another var borrows.
> Re-landed default-on after: full default-config suite green (2558), full elide-mode
> suite, the copy-vs-borrow differential, crawler equiptest+selftest clean, and ~8× on
> crawler's sim-tick (the v-borrowed accessors stay copy; `tile_at`/`edge_wall_raw` —
> the map copies — still elide). The copy mechanism is the substrate (conservative
> fallback + opt-out A-B lever). Tiers 1–3 remain design-only below.
> Tiers 1–3 (local-source dominance, ¬D2 mutable source, return-delivery #465
> unification) remain design-only below. Harvested from the
> @PLN25 dense dogfood: verifying crawler on the dense branch surfaced a ~5× per-tick
> slowdown whose root cause is a copy that *should* be a borrow. This is the
> **performance face of Cluster C / OWNERSHIP_MODEL** ("ownership read once from the
> carried fact, not re-derived from shape"). Cross-refs:
> [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md), [STABILITY_ROADMAP.md § Cluster C](../../STABILITY_ROADMAP.md),
> the p379 + #465 fixes (the inverse-direction siblings of this same decision).
>
> **Depends on the pre-work:** the materialization predicate here is a *query* of the
> reusable USE-analysis defined in
> [use-analysis-prework-design.md](use-analysis-prework-design.md) — build that phase
> first (it computes the read-only / escape / source facts once, over the full IR, where
> every decision site can consult them). This doc is the *first optimizing consumer* of
> that phase.

---

## 1. Evidence — the bottleneck, measured

`sim_wait` (one crawler simulation tick over a populated 101×101 world) is ~5× slower
on dense than on the pre-dense baseline. Per-opcode timing (throwaway instrument, since
reverted) attributed **27.9s of the ~30s in 3 ticks to a single op: `OpAppendVector`**
(vector deep-copy), and the append-by-calling-function tally named two crawler
accessors:

```loft
pub fn tile_at(s: Sim, q, r) -> integer {
  ...
  tiles = s.tiles;            // copies the WHOLE tiles vector (10 201 ints) per call
  ti = ...; if ti < len(tiles) { tiles[ti] } else { 1 }
}
fn edge_wall_raw(s: Sim, q, r, edge) -> integer {
  ...
  ws = s.walls;               // copies the WHOLE walls vector (30 603 ints) per call
  ei = ...; if ei < len(ws) { ws[ei] } else { 0 }
}
```

Measured tick copies (3 ticks): `tile_at` 159 690 + `edge_wall_raw` 156 070 ≈ **316k
full-map copies**, ~6.4 B element-copies ≈ the entire tick wall-time. These are
one-tile read accessors; the copy is **pure waste** — the local is only read, never
mutated, never escapes, and the source (`s`) lives for the whole call.

The trigger is the `local = byvalue_param.vectorfield` binding hitting the **#415
deep-copy** path in `parse_assign_op` (`src/parser/expressions.rs`): the rule copies
based on the **shape of the RHS** (base is a `Type::Reference` with empty `deps`),
regardless of how the local is *used*. The contrasting `enemies = s.enemies` (where `s`
is `&Sim`, a `Type::RefVar`) already **borrows** — proving the aliasing machinery
exists; the copy path is reached only because a by-value param's field read looks like
an owning copy to the shape rule.

> **Dense regression vs latent:** the copy fires on BOTH backends at equal per-call
> cost (isolated probe: 1.4s either way). Dense does ~1.6× more of these calls and each
> is ~3× costlier under the big-`Sim` allocator path → the ~5×. So this is a **latent
> inefficiency the dense value-model amplifies**, not a pure dense bug. The fix below
> removes it on **both** backends.

---

## 2. The semantic envelope — what is *possible* (precise)

The question is exact, not heuristic: **when may `v = copy(s.f)` be replaced by
`v = borrow(s.f)` (alias) with identical observable behavior for every execution?**

A copy makes `v` an independent owner; a borrow makes `v` an alias of `s.f`'s storage.
The two diverge **only** through one of three observable events:

| | Divergence event | copy semantics | borrow semantics |
|---|---|---|---|
| **D1** | a write *through* `v` (`v[i]=…`, `v += …`, `v` passed to a `&`/mutating param) | source unchanged | source mutated |
| **D2** | a write to `s.f` (or to `s`) **while `v` is live**, then `v` read | `v` sees the pre-copy snapshot | `v` sees the new value |
| **D3** | `v` outlives `s.f`'s storage (source freed / reassigned / `v` escapes the scope) | `v` valid | `v` dangles (UAF) |

**Theorem (the borrow-elision condition).** `borrow ≡ copy` for a given `v = s.f`
**iff none of D1, D2, D3 can occur on any path.** Equivalently, all three must hold:

- **¬D1 — read-only:** `v` is never written through (no element/whole-vector mutation,
  never handed to a mutating callee).
- **¬D2 — no aliasing mutation in the live range:** no write to `s.f` or `s` between
  `v`'s definition and `v`'s last read.
- **¬D3 — source dominates:** `s.f`'s storage strictly outlives `v`'s last use, which
  subsumes *escape* (if `v` is returned / stored into longer-lived state / captured,
  its lifetime is unbounded and ¬D3 cannot be guaranteed).

This is the **maximal sound optimization**: borrow exactly when ¬D1 ∧ ¬D2 ∧ ¬D3
provably hold; **copy otherwise** (the safe fallback). It is decidable conservatively —
"cannot prove ⇒ copy" is always correct, only less fast.

**Why this is the right frame (the invariant, §3).** The current rule keys on RHS
*shape*; the envelope keys on `v`'s *use*. Shape is a lossy proxy for use — it
mis-classifies the read-only accessor (a borrow shape, copied) exactly as it
mis-classified p379 (a borrow wrongly copied) and #465 (a borrow wrongly aliased). All
three are the same defect: **the copy/borrow/move decision read off shape instead of
the carried ownership fact.**

---

## 3. The one invariant

> **A binding's materialization is its USE, carried in `deps`, not its RHS shape:**
> `v = <heap RHS>` is a **BORROW** (alias the source, `skip_free`) when `v` is a
> *reader whose source dominates its lifetime* (¬D1 ∧ ¬D2 ∧ ¬D3); a **COPY** (own a
> fresh store) when `v` is an *owner* (it is mutated, escapes, or outlives its source);
> a **MOVE** when the source is a dying temporary `v` adopts. One predicate decides;
> every assignment- and return-delivery site consults it.

A case never tested behaves correctly for the same reason the tested ones do: the
predicate is evaluated from `v`'s actual uses, so an unseen use-pattern is classified by
the same three events, not by a shape enumeration that may not list it.

---

## 4. Re-assertion sites — the brittleness, counted now (step 2)

The copy/borrow/move decision is **today spread across N > 1 sites, and omission is
silent** (a wrong result or a UAF, never a compile error):

| Site | File | Decides |
|---|---|---|
| `b = a` (var RHS) | `expressions.rs` @P292/@P394 | copy (own store) |
| `af = bx.v` (field-read RHS) | `expressions.rs` #415 `struct_vec_field` | copy vs borrow ← **the bug** |
| keyed local assign | `expressions.rs` @P295 | deep-copy via `OpReplaceKeyed` |
| return tail | `control.rs` `classify_vector_delivery` | borrow-copy / move / alias (already a chokepoint) |
| match/if arm tail | `control.rs` `materialize_vector_arms_into` | per-arm delivery |

`N = 5` with silent omission ⇒ **brittleness = 5, known before any code.** The cure is
the protocol's: **collapse N toward 1** — one `materialization_mode(v, rhs) -> {Borrow,
Copy, Move}` predicate that all sites call — and where collapse is staged, **make
omission loud** (a debug assert that a heap binding without a recorded mode is a bug).
`classify_vector_delivery` is the proof a chokepoint is reachable on the return side;
this design extends the same shape to the **assignment** side, which is still a spray of
branches.

**Do not over-reach here (step 6, over-broad face).** The universal predicate is
correct, but the *defect* lives in one branch (the field-read copy). The mitigation
path (§6) therefore **narrows to the broken path first** and only generalizes outward
as each tier is proven — universal stays the goal, the rewrite scope tracks the proven
domain.

---

## 5. Detecting the bottleneck (standing instrument)

The root cause was invisible to op-*counting* (the hot op was low-count, high-per-call)
and only a per-op *timing* profiler caught it. Bake that capability in so the next
copy-bottleneck is found in minutes, not a multi-hour investigation:

- **`LOFT_PROF` per-opcode timing + append-by-calling-fn tally** (the throwaway
  instrument, productized): cumulative ns and call-count per opcode, plus a
  `OpAppendVector`-by-`call_stack` histogram, dumped on a `LOFT_PROF` run. A single op
  with µs-scale `ns/call` is the signature of an O(n) copy in a hot loop.
- **A static lint (`api-lint` / a `cargo`-time pass):** flag `local = <param>.<vectorfield>`
  where the local is read-only and non-escaping — these are *guaranteed* elidable copies
  today. This turns the perf cliff into a compile-time warning at the source, where the
  author can also choose the consumer-side fix.
- **No silent caps:** when the optimizer falls back to copy (cannot prove the envelope),
  emit a `LOFT_PROF`-gated note naming the binding — so "still copying" is visible, not
  assumed elided.

---

## 6. Mitigation path — conservative tiers, each its own gate

Each tier widens the proven domain; every tier **copies when it cannot prove the
envelope** (sound fallback) and lands with its own boundary matrix on **both backends**.

### Tier 0 — read-only param-field accessor (captures `tile_at`/`edge_wall_raw`)
The cheapest, highest-value, safest slice. Borrow `v = s.f` when:
- `s` is a **parameter** (by-value or `&`) — so ¬D3 is trivial (a param's source lives
  to function end, dominating any local) and ¬D2 for a by-value/read-only param is
  trivial (the param is not mutated through), **and**
- `v` is **read-only** (¬D1, via the existing `written` set computed for the "param has
  & but never modified" check in `parser/mod.rs`), **and**
- `v` does **not escape** (¬D3: not returned, not stored into a field/vector, not
  captured — a scan of `v`'s uses, the same `written`/deps pass).

Mechanism already exists: route through the **alias path `enemies = s.enemies` uses**
(dep = source var, `skip_free`), instead of the `OpDatabase + OpAppendVector` copy.
Concretely: in `parse_assign_op`'s `struct_vec_field` branch, gate the deep-copy on
`is_owner(v)` (mutated ∨ escapes) and otherwise fall through to the borrow path.

**Expected impact:** `tile_at`/`edge_wall_raw` drop from O(map) to O(1) per call →
removes the 27.9s; crawler `surfacetest` returns to baseline-class time on both
backends. (Independently, the consumer can index `s.tiles[ti]` directly — see §8 — but
the language fix removes the cliff for *every* consumer of this ubiquitous pattern.)

### Tier 1 — local-struct source with scope dominance
Extend ¬D3 to `v = x.f` where `x` is a **local** (not a param): borrow iff
scope-analysis proves `x` (hence `x.f`'s store) outlives `v`'s last use — the `deps` /
`scopes.rs` dominance fact. Copy when the relation is unproven.

### Tier 2 — mutable source, no intervening aliasing mutation (¬D2)
Extend to a source that *could* be mutated: borrow iff no write to `s.f`/`s` lies
between `v`'s definition and last read (a live-range check over `written`). This is the
hardest analysis and the smallest marginal win — gate behind a measured need.

### Tier 3 — unify assignment + return delivery onto one predicate
Fold the assignment lowering and `classify_vector_delivery`/`materialize_vector_arms_into`
onto the single `materialization_mode` predicate (drives N → 1, §4). This is where this
design **meets #465 and Cluster C** — the return-delivery over-free and the
assignment over-copy become one decision. Largest scope; do last, after Tiers 0–2 prove
the predicate.

---

## 7. Falsification probes — the build gate (steps 3–4)

A design is a hypothesis; these are the cheapest tests that could prove it **false**.
Build each tier only after its probes are written, with hand-computed expectations, run
on **interpret AND `--native`**, asserting **value + length + leak** (a copy-elision
that doubles or dangles a vector is leak-clean but wrong).

**Probe the envelope itself — each row MUST stay a COPY (borrow would corrupt):**

| Claim under test | Falsifying probe | Required outcome |
|---|---|---|
| ¬D1 load-bearing | `v = s.f; v[0] = 9; …` then read `s.f[0]` | borrow would mutate source → **must copy** (source unchanged) |
| ¬D1 via callee | `v = s.f; mutate(&v)` | **must copy** |
| ¬D3 escape | `fn g(s:S)->vector{ v = s.f; v }` (return the borrow) | **must copy** (else dangles past `s`) |
| ¬D3 store | `v = local.f; free(local); read v` | **must copy** |
| ¬D2 aliasing | `v = s.f; s.f[0] = 9; read v[0]` | snapshot semantics → **must copy** (v sees old) |

**Probe the elision actually fires (the win):** the read-only accessor cases
(`tile_at`/`edge_wall_raw` shape) — assert correct value, **no leak**, and that the IR
is the BORROW form (`OpGetField` + dep on source, no `OpDatabase`/`OpAppendVector`).

**Attack the cleanest claim (step 4 — over-unification guard):** the seductive claim is
*"a read-only non-escaping local is always borrow-safe."* Its hidden hole is **¬D2** —
a read-only local whose *source* is mutated mid-live-range. Tier 0 sidesteps this by
restricting to read-only/by-value param sources (¬D2 trivial); Tier 2 is where the
claim is actually exercised and must be probed hardest. **Do not let Tier 0's success
license Tier 2 without its own ¬D2 matrix.**

---

## 8. Risks, interactions, non-goals

- **#426B store-reuse substrate (the documented hazard).** The opposite-direction
  generalization (#426: make more reads *copy*) hit a store-reuse-after-free because the
  copy *freed the source*. **This design moves the other way (copy → borrow); a borrow
  never frees the source**, so it does not re-open #426B — provided the borrowed local
  is `skip_free` (the existing alias mechanism guarantees this). Index-read views stay
  as they are; this design changes only the **field-read binding**, not index reads.
- **Interaction with p379 / #465.** Both are the same predicate read off shape; this
  design is their generalization. Tier 3 should *subsume* the p379 `struct_vec_field`
  dep check and the #465 `classify_vector_delivery` into the one predicate — but only
  after Tiers 0–2; until then they coexist unchanged.
- **Conservative by construction.** Every tier copies on "cannot prove." A wrong borrow
  is a UAF; the asymmetry is deliberate — soundness over completeness, widen only with a
  passing matrix.
- **Consumer-side immediate relief (orthogonal, not a substitute).** crawler can drop
  the `tiles = s.tiles` binding (index `s.tiles[ti]` directly) and/or pass `s: &Sim`,
  removing the copy now without waiting for the language change. The language fix is what
  makes the pattern safe-and-fast for *every* consumer; ship both.
- **Non-goal:** changing value semantics. The envelope (§2) is *behavior-preserving by
  definition* — the optimization is invisible except in time and allocation.

---

## 8b. Pre-build probe results (design-protocol — claims tested, not asserted)

Cheap falsification run before any code (the prose is where the error hides):

- **¬D1 detection is reusable — CONFIRMED.** `fn f(p: &vector<integer>) { p[0]=9 }`
  and `{ p += [9] }` both compile and mutate (element-write + append ARE tracked as
  "written"); a read-only `&` param correctly ERRORs "never modified". So Tier 0 can
  reuse the existing `written` set to prove read-only-ness — element mutation does not
  slip through as a false read-only (which would have been a silent mis-borrow / UAF).
- **¬D3 trivial for a by-value param source — CONFIRMED.** Passing a `Sim` with a
  10 201-element vector by value, 5 000×, ran in 0.028s (no per-call copy) → a read-only
  by-value struct param is internally an alias of the caller's value, which outlives the
  call. Borrowing its field is therefore lifetime-safe.
- **Borrow mechanism already exists — CONFIRMED.** `enemies = s.enemies` (RefVar base)
  emits the borrow form (dep on source, `skip_free`, pure `OpGetField`); Tier 0 routes
  the field-read copy through this same path. No new runtime op needed.

These validate Tier 0's feasibility. Tiers 2 (¬D2 mutable-source) and 3 (predicate
unification) remain hypotheses to probe at their own build time.

## 9. Validation gates (definition of done per tier)

1. Envelope probes (§7) all RED→GREEN correctly (copies stay copies; borrows borrow),
   interpret + `--native`, value + length + leak.
2. `tile_at`/`edge_wall_raw`-shape benchmark: per-call O(map) → O(1); crawler
   `surfacetest` back to baseline-class wall-time on both backends.
3. Full suite green (`find_problems.sh --bg`), no leak regressions
   (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`).
4. The `git diff main` IR for the **non-elided** paths is byte-identical (behaviour-
   preserving refactor gate, per `loft-codegen`): only the proven-borrow bindings change.
