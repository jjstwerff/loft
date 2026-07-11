<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — The LOFT_NULLFLOW default-flip: staged conversion plan

> **Status: measured 2026-07-11, not yet flipped.** All five null-flow phases are landed +
> gated (`LOFT_NULLFLOW`, opt-in). The default-flip (make it default-on) is the one-way cutover.
> Flipping it against the full suite gave **24/2826 failures**; this doc enumerates them by
> category with the fix steps and the EXACT sites (measured by running the corpus under
> `LOFT_NULLFLOW=1` — reproduce with `LOFT_NULLFLOW=1 loft <file>`).
>
> **D1 LANDED (2026-07-11, commit 21ec7837).** Category D is closed in-tree by two source-level
> language fixes (no consumer `?? d` churn): a computed integer-arithmetic index over constants +
> active loop vars (`m[k*4+row]`) now types non-null (`index_provably_fit`/`index_arith_trusted`),
> and a custom iterator's `next(self) -> Item?` binds the loop variable non-null (`for_type` peels
> the terminator `?`). This also removes the mat4-multiply friction from E's numeric libs at the
> source. Remaining: A, B, C, E, F, then the flip.

## The categories at a glance

| | Category | Fix | In-tree sites | Scope |
|---|---|---|---|---|
| **A** | bare `text as numeric` → error (N-Cast, Phase 4) | **DONE — `as τ?`** | ~~44~~ **43 done**, F5 at flip | this repo |
| **B** | nullable returned/stored into a non-null slot → warning | `?? d`, or declare `τ?` | 18 (5 files) | this repo |
| **C** | `possibly-null as non-null` (e.g. `float? as integer`) → error | `as τ?` or `?? d` | 18 (5 files) | this repo |
| **D** | matrix / vector arithmetic `v[i]*v[j]` → `τ?` (DN3-index × N-Prop) | **DONE — range-track (D1)** | ~~3 (2 files)~~ **0 in-tree**; helps EXTERNAL | this repo + range-tracking |
| **E** | external registry libs break | republish | ~9 libs | loft-libs-* (out of repo) |
| **F** | goldens shift (new warnings) | regenerate | 2 test files | this repo |

The one-way blocker is **E** — the suite cannot go fully green in this repo until the registry
libs are republished. A/B/C/F are tractable in-repo; **D** is where a *language* fix (better
vector-index range-tracking) beats a mechanical `?? d` sweep.

---

## Category A — text-parse assertion (`text as numeric` → error) — DONE (commit ccbac29f)

**Cause.** Phase 4: a cast `as τ` is an assertion, and a text parse can't be proven, so bare
`s as integer` / `s as float` is a compile error. **Fix used: uniformly the checked `as τ?`** — it
is faithful because the pre-flip DN3 N-Parse ALREADY auto-wraps `text as numeric` to `τ?`, so the
gate-off type is unchanged AND the null-on-bad-parse test cases (`!("abc" as integer?)`) keep their
meaning. (`?? d` would force non-null and destroy those cases; only reach for it where a *non-null*
value is genuinely needed.) Verified each file clean on both gates.

**Sites (44) — 43 converted:**
- `tests/scripts/01-integers.loft` (7) · `03-text.loft` (6) · `25-tuple-nstore.loft` (4) ·
  `52-single.loft` (2) · `02-floats.loft` (1 A + its 2 C `float?`-source casts, see below)
- `tests/docs/03-integer.loft` (2) · `05-float.loft` (1). **`16-parser.loft`/`15-lexer.loft`/
  `lib/parser.loft`/`lib/docs.loft` needed NO direct edit** — all four number-parse casts live in
  the shared **`lib/lexer.loft`** (`int`/`long_int`/`get_float`/`get_single`, each returns `τ?` and
  null-checks `result`), so fixing it once cleared every consumer.
- **`features/F5.loft` (1) — DEFERRED to the flip landing.** It is GENERATED from the canonical
  `loft-lang/features#5`; the drift guard forbids editing the generated file, so the fix is an
  ISSUE edit (`"42" as integer` → `as integer?`) + `make features-fetch && make features-gen`.
  Bundled with the flip's other outward steps (E republish, contract bump).

**Bonus (commit 60e2937c) — the checked cast now accepts a nullable SCALAR source.** `float? as
integer?` used to report "Unknown cast" (the per-type `OpCast` is base-keyed); it now peels the
`Optional`, letting the runtime propagate the in-band null. This also pre-clears the category-C
`float?`/`single? as integer` sites — a `?? d` sweep is NOT needed for them, just `as τ?`.

---

## Category B — nullable stored into a non-null slot (warning)

**Cause.** A nullable value (div / `sqrt` / `abs` / … result, or a `τ?` field) flows into a
non-null return/field/local. Full-width types (`integer`/`float`/…) WARN (compile + run, the slot
holds null); this is the nudge, not a break. **Fix per site:** discharge with `?? <default>` at
the return, or declare the return / field `τ?` if null is a real outcome.

**Sites (18):**
- `lib/audience_crystal/src/audience_crystal.loft` (4) + its tests
  `tests/01-editor-helpers.loft` / `02-gridmesh-equiv.loft` / `03-crystal-incr.loft` (4 each)
- `tests/docs/17-libraries.loft` (1) · `lib/testlib.loft` (1)

**Step B.** audience_crystal is the concentration — a numeric consumer lib; convert its returns
(`?? d` / `τ?`) and its 3 test files move with it. The 2 stragglers are one-liners.

---

## Category C — cast of a nullable to a non-null scalar (`float? as integer`)

**Cause.** DN5: a `possibly-null` value cast to a non-null scalar. **Fix per site:** `as τ?`
(checked) or `?? d` before the cast.

**Sites (18):**
- `lib/audience_crystal/src/audience_crystal.loft` (4) + its 3 test files (4 each)
- `tests/scripts/02-floats.loft` (2)

**Step C.** Same files as B (audience_crystal) — do B and C together per file.

---

## Category D — matrix / vector-arithmetic friction (`v[i]*v[j]` → `τ?`)

**Cause.** A possibly-OOB vector index is `float?` (DN3), and N-Prop propagates it through the
multiply, so `sum += v[i]*v[j]` fails (`cannot change type … to …?`). **This is the one category
where a `?? d` sweep is the WRONG fix** — the honest fix is better range-tracking so a
provably-in-bounds index (`m[k*4+row]` with bounded `k`,`row`) stays non-null.

**In-tree sites — DONE (2026-07-11, commit 21ec7837).** Both cleared by the D1 language fix, no
`?? d` churn, verified on both backends + both gates (`tests/nullflow_index_dn.rs` guards them):
- `tests/scripts/85-yield-resume.loft` — the mat4-multiply `ma.m[mul_k*4+mul_row] * mb.m[…]`: the
  computed index is now trusted non-null. Fix: `index_arith_trusted` in `src/parser/fields.rs`
  recurses through `+ - * / %` when every leaf is a constant or an **active loop var** (the
  matrix-indexing contract — trusted like a bare loop var; a real OOB still faults → null, C80). It
  deliberately does **not** thread the `i < len(v)` guard through arithmetic (that proof is specific
  to `v[i]`, not `v[i*2]`).
- `tests/scripts/81-iterator-protocol.loft` — a DIFFERENT root cause the original triage lumped in
  here: a custom iterator's `next(self) -> integer?` returns null as the loop **terminator**, so
  `for x in c` never binds null in the body, yet `x` typed `integer?` and N-Prop nullified
  `total + x`. Fix: `for_type` (`src/parser/control.rs`) peels the `Optional` off the custom-iterator
  element type; termination is structural (`parse_for_iter_setup`), so peeling is safe.

**External (the bulk):** `graphics-0.3.0/src/math.loft` (mat4 multiply) and any registry lib doing
matrix/vector math now benefit from the SAME fix at the source — the mat4 multiply no longer needs a
`?? d` sweep once republished (see E). Verify per-lib under `LOFT_NULLFLOW=1`.

---

## Category E — external registry libs (the one-way blocker)

**Cause.** Libs under `~/.loft/registry` were compiled against pre-flip behaviour. They break on
A/B/C/D and CANNOT be fixed in this repo. **Fix:** republish each from its loft-libs-* source
(the consumer agent's half of the dogfood split).

**Candidate libs to audit + republish** (installed set — verify each with `LOFT_NULLFLOW=1`):
`graphics-0.3.0` (mat4 — D), `gridmesh-0.1.2`, `hex_grid-0.1.0`, `hex_terrain-0.1.0`,
`hex_world-0.1.2`, `glb-0.1.0`, `crypto-0.3.5`, `markdown-0.1.0`. Numeric ones (graphics/gridmesh/
hex_*) are the likely hits; text ones may hit A.

**Step E.** For each: run its tests under `LOFT_NULLFLOW=1`, convert (A/B/C) or benefit from D1,
bump the version, re-sign, publish (loft-ship skill). Tracks the `registry-validation` CI leg.

---

## Category F — goldens

**Cause.** New warnings change diagnostic goldens. **Sites:** `tests/runtime_warnings.rs`
(`wrong_field_guard_still_rejects`), `tests/features.rs` (`features_examples_interpret`), plus any
`@EXPECT_ERROR`/golden that pins a message A/B changed. **Step F.** Regenerate after A–D land;
grep inline `.error(...)` / `@EXPECT_ERROR` for pinned wording (a stale test binary can mask
message-assertion changes — force a rebuild or trust CI's clean build).

---

## Ordering + the flip itself

1. ~~**D1 first (recommended)** — vector-index range-tracking, so numeric code (in-tree D + much of
   E) needs no `??`.~~ **DONE 2026-07-11 (commit 21ec7837).** In-tree D closed; E's numeric libs
   benefit at the source.
2. ~~**A, then B+C** in-repo.~~ **A DONE 2026-07-11 (commit ccbac29f);** 43/44 converted, F5 at
   the flip. **B+C NEXT** (share the audience_crystal files; the nullable-scalar cast fix already
   clears C's `float?`-source casts). Verify each file `LOFT_NULLFLOW=1` on BOTH backends.
3. **E** — republish the ~9 registry libs (loft-libs-*), in parallel; this is the gating item for
   a green `registry-validation`.
4. **F** — regenerate goldens.
5. **The flip** — `nullflow_enabled()` → default-on with `LOFT_NO_NULLFLOW` opt-out
   (`src/keys.rs`); repoint the `tests/nullflow_phase*.rs` OFF cases to `LOFT_NO_NULLFLOW`; bump
   `CONTRACT_VERSION` 0 → 1 (`src/manifest.rs`). Land as ONE coordinated set so nothing stays red.

## See also

- [float-null-domain-typing.md](float-null-domain-typing.md) — the design + the phase implementation plan.
- [formal/types.md](../../formal/types.md) § Null-flow — the general laws.
- The five landed phases: commits 4b0c11e4, 5b3dd581, f8a307d7/0e135ab8/f582b6d8, c7caca54, 889e388b.
