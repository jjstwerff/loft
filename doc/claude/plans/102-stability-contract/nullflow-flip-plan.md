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

## The categories at a glance

| | Category | Fix | In-tree sites | Scope |
|---|---|---|---|---|
| **A** | bare `text as numeric` → error (N-Cast, Phase 4) | `as τ?` or `?? d` | 44 (13 files) | this repo |
| **B** | nullable returned/stored into a non-null slot → warning | `?? d`, or declare `τ?` | 18 (5 files) | this repo |
| **C** | `possibly-null as non-null` (e.g. `float? as integer`) → error | `as τ?` or `?? d` | 18 (5 files) | this repo |
| **D** | matrix / vector arithmetic `v[i]*v[j]` → `τ?` (DN3-index × N-Prop) | range-track, or `?? d` | 3 (2 files) in-tree; the bulk is EXTERNAL | this repo + range-tracking |
| **E** | external registry libs break | republish | ~9 libs | loft-libs-* (out of repo) |
| **F** | goldens shift (new warnings) | regenerate | 2 test files | this repo |

The one-way blocker is **E** — the suite cannot go fully green in this repo until the registry
libs are republished. A/B/C/F are tractable in-repo; **D** is where a *language* fix (better
vector-index range-tracking) beats a mechanical `?? d` sweep.

---

## Category A — text-parse assertion (`text as numeric` → error)

**Cause.** Phase 4: a cast `as τ` is an assertion, and a text parse can't be proven, so bare
`s as integer` / `s as float` is a compile error. **Fix per site:** the checked `as τ?` (when the
value is used nullably or the parse may fail) or `s as τ ?? <default>` (assert-or-default). For
test data known to parse, `as τ ?? 0` is usually right.

**Sites (44):**
- `tests/scripts/01-integers.loft` (7) · `03-text.loft` (6) · `25-tuple-nstore.loft` (4) ·
  `52-single.loft` (2) · `02-floats.loft` (1)
- `tests/docs/16-parser.loft` (4) · `15-lexer.loft` (4) · `03-integer.loft` (2) ·
  `05-float.loft` (1) · `features/F5.loft` (1)  ← `features/` is GENERATED: fix the @F5 issue body, then `make features-gen`
- `lib/parser.loft` (4) · `lib/lexer.loft` (4) · `lib/docs.loft` (4)

**Step A.** Convert each site; re-run `LOFT_NULLFLOW=1 loft <file>` until clean, both backends.

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

**In-tree sites (3, small):**
- `tests/scripts/81-iterator-protocol.loft` · `85-yield-resume.loft`

**External (the bulk):** `graphics-0.3.0/src/math.loft` (mat4 multiply), and any registry lib
doing matrix/vector math (see E).

**Step D (decision — do FIRST if chosen).** Either (D1) extend index range-tracking so
`v[<provably-in-bounds>]` is non-null (removes the friction at the source, no consumer churn — the
right fix for numeric code), or (D2) accept the friction and `?? d` the 3 in-tree sites + document
it. Recommend **D1** before the flip; it also shrinks E's conversion burden.

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

1. **D1 first (recommended)** — vector-index range-tracking, so numeric code (in-tree D + much of
   E) needs no `??`. Biggest leverage; least churn.
2. **A, then B+C** in-repo (B+C share the audience_crystal files). Verify each file
   `LOFT_NULLFLOW=1` on BOTH backends.
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
