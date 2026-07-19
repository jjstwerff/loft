<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc E — remove the tolerated-warnings filter (test hygiene)

> **Status: ✅ BUILT (2026-07-19).** The warning-surface half of arc E's "be strict
> now" one-way-door audit — DONE. The `is_runtime_warning` filter is deleted; the
> `code!` harness now asserts exactly what loft emits, guarded by a meta-lock test.
> Measured reality (test-hygiene-buckets.md): **7 of 9 clauses were dead** (0 fixtures
> — better than the "5 dead" estimate: the two N-Store nudges also had no `code!`
> fixture), so only **2 live families** needed fixture work — redundant-`&` (12,
> dropped) and `not null` (110, deleted; 2 genuine-feature tests keep it + assert the
> deprecation). Commits: measure+dead-clauses, `not null`, `&`+delete+lock.
> Reference: [test-hygiene-buckets.md](test-hygiene-buckets.md).

## The hazard

`tests/testing.rs::assert_diagnostics` carries an `is_runtime_warning` predicate
(around lines 519–542) that **silently absorbs** a fixed set of warnings so the
`code!(...)` diagnostic-comparison harness never counts them as unexpected output. A
`code!` fixture that emits one of these therefore PASSES *without asserting it*. The
consequence for the freeze: a semantic change to *what warns* — the null-flow
default-on cutover, the redundant-`&` lint, the `not null` retirement, now the arc-C
steer — can change the warning surface and **no `code!` test notices**. Before we
freeze the diagnostic surface, the suite must describe the diagnostics loft *actually*
emits, not tolerate a frozen legacy set.

The absorbed clauses (verbatim, `testing.rs:541–564`, applied at `:567`). **Crucial
finding (2026-07-19 research): 5 of the 9 are already DEAD** — retired under the DN1
default-on flip, they cannot fire during `cargo test` (they need `LOFT_PLN25_OFF`,
which also stops stdlib load) — so removing them is a **zero-delta no-op**. The real
blast radius is only the 4 LIVE categories, dominated by `not null`.

| Clause (`testing.rs`) | Warning family | Live? | Emitter / gate |
|---|---|---|---|
| `:541` `"division may produce null"` | @PLN25 runtime-null (`÷`) | **DEAD** | `operators.rs:3057`, early-return under `pln25_dn1_enabled()` |
| `:542` `"modulus may produce null"` | same (`%`) | **DEAD** | same |
| `:543` `` `v[i]` may produce null`` | same (vector index) | **DEAD** | same |
| `:544` `` `s[i]` may produce null`` | same (text index) | **DEAD** | same |
| `:545` `"field "` | `not null` field-read hint | **DEAD** | `mod.rs:1363`, early-return under `pln25_dn1_enabled()` |
| `:551` `` `&` on parameter `` | @PLN87 redundant-`&` lint | **LIVE — small** (single-digit–low-tens) | `operators.rs:2889`, on (`LOFT_NO_WARN_RUNTIME` opts out) |
| `:556` `` `not null` is deprecated`` | F2 retirement advisory | **LIVE — largest** (~150–180 fixtures) | `definitions.rs:25`, ungated pass-2 |
| `:563` `` a nullable `…` is stored into`` | N-Store nudge | **LIVE — tens** | `mod.rs:2179`/`2214`, gated `nullflow_enabled()`; narrow target is a hard *Error*, never this warning |
| `:564` `` `null` is stored into`` | N-Store null form | **LIVE — tens** | same |

There are **0** explicit `.warning(...)` assertions for any of these strings today — the
filter is the sole suppressor. (Re-confirm the clause set at edit time; the ladder is
per-clause, so it absorbs a changed set naturally.)

## The invariant (design-protocol step 1)

> **After this work, every `code!` fixture's diagnostic set is EXACTLY what loft emits
> for that source — no more, no less.** A warning a fixture emits is either
> `.warning("…")`-asserted (the fixture means to exercise it) or eliminated by fixing
> the `.loft` (the fixture didn't mean to). Zero silent absorption.

## What must NOT be lost

End-to-end coverage of these warnings already lives OUTSIDE the `code!` harness —
`tests/runtime_warnings.rs` asserts every one of the 4 LIVE families at the binary
level (`Command::new`): the redundant-`&` lint (`w4_*` — fires on reassign, quiet on
writeback/scalar), the N-Store nudge **with correct file:line**
(`nstore_return_position_names_offending_fn`, `nstore_tail_arg_position_names_offending_fn`),
and it also pins the 5 DEAD families as *retired* (`div_by_var_no_warn_retired_dn3`,
`vec_index_by_var_no_warn_retired`, `hint_4h_high_read_count_hint_retired`). The
`not null` retirement is covered by `definitions.rs`'s own path (it is a deprecated
no-op headed for hard-error). So deleting the `code!`-harness filter removes *silent
tolerance*, not *coverage* — every family already has an e2e home; no new one is
needed first.

## The ladder — one clause per commit (small, isolated blast radius)

Sequence by risk: the **dead clauses first (a proven no-op)**, then the live
categories smallest-to-largest, `not null` last (its own sweep). Each step is a commit.

| # | Step | Proof |
|---|---|---|
| 0 | **Measure (confirm the research).** Temporarily make `is_runtime_warning` return `false` wholesale, run `cargo test` over the `code!` users (`parse_errors`, `issues`, `expressions`, `n2_cdylib`, `use_analysis`, `engine_host_kernel`, `slot_v2_baseline`, …), bucket every newly-`FAILED` fixture by clause. Expected: buckets `541–545` empty, `551`/`563`/`564` small–tens, `556` ~150–180. Revert the probe. | a bucketed list matching the dead/live table; no code committed |
| 1 | **Delete the 5 DEAD clauses (`541–545`) — one zero-delta commit.** They cannot fire under the test env (DN1 default-on), so the suite is byte-identical. This shrinks the risky diff to 3 live categories before touching a single fixture. | suite green + **no fixture newly asserts/fixes** (the proof it was dead); `runtime_warnings.rs` already pins them retired |
| 2 | **`&` on parameter (`551`) — smallest live bucket.** For each tripping fixture: **prefer dropping the redundant `&`** (the field is only read/mutated, never reassigned) — assert only if the `&` deliberately exercises the RefVar path. Then delete clause `551`. ⚠️ do NOT `.warning(...)`-assert this one casually: the message is `LOFT_NO_WARN_RUNTIME`-gated, so an assertion is env-fragile (spuriously "expected-but-not-found" if a wrapper ever sets that env). | the bucket clears; suite green with `551` gone; a re-added redundant `&` now FAILS |
| 3 | **N-Store (`563`+`564`) — tens.** For each: **discharge the nullable** (`?? d`) or store into a correctly-typed slot so the nudge is genuinely gone; assert only where storing-a-nullable IS the point. (Narrow-target stores were never covered — they are a hard *Error* — so those fixtures are untouched.) Delete both clauses. | the bucket clears; suite green; positions are irrelevant to `code!` (see traps) |
| 4 | **`not null` deprecation (`556`) — the big sweep, its own commit(s).** For ~all ~150–180 fixtures the fix is **DELETE `not null` from the `.loft`** — it is a deprecated no-op (the field is non-null by default), and it is headed for a *hard error* (`definitions.rs:18`), so asserting it would entrench a doomed construct. Mechanical + low-risk per fixture; split across a few commits by test file (`issues.rs` is ~100 of them) to keep each diff reviewable. Delete clause `556`. | each file's fixtures pass with `not null` removed; suite green; `556` gone |
| 5 | **Delete the `is_runtime_warning` mechanism entirely** (the predicate + its call-site branch at `:567`). The harness now fails on ANY unexpected warning. | grep shows no residual filter; suite green; an injected stray warning in any fixture fails |
| 6 | **Lock it.** Add one meta-fixture: a `code!` whose source emits a known warning WITHOUT a matching `.warning(...)` must fail the harness (guards a future silent-tolerance re-introduction). | the meta-fixture fails when the assertion is omitted, passes when present |

## Ordering rationale + traps

- **Dead-first is the whole risk-reduction.** Step 1 clears 5 of 9 clauses at zero
  cost, so the human-review surface is only the 3 live categories — and `not null`
  (~150–180 fixtures) is isolated to its own late, mechanical sweep.
- **Position is IRRELEVANT to a `code!` assertion.** `assert_diagnostics` runs every
  line through `normalize_loft_loc` and matches on message text only (location-agnostic
  after normalisation) — so a `.warning(...)` cannot (and need not) pin a column, and
  the N-Store position bug (guarded separately in `runtime_warnings.rs`) can never break
  a `code!` fixture. (Contrast the E2-B *parse_errors* guards, which DO pin `:L:C` —
  a different harness path.)
- **The `&`-param assertion is env-fragile — prefer dropping the `&`.** Its message is
  `LOFT_NO_WARN_RUNTIME`-gated, so a `.warning("`&` on parameter …")` would spuriously
  fail if any wrapper ever set that env (the `runtime_warnings.rs` header still *claims*
  the harness sets it — stale/incorrect today, but a latent trap). Drop the redundant
  `&` unless it deliberately exercises the RefVar path.
- **Never assert `not null` — delete it.** It is a no-op headed for a hard error;
  a `.warning(...)` would entrench a construct we are removing. The only correct fix is
  to strip it from the `.loft`.
- **Do not silence by env.** The fix is delete-or-assert, never "set
  `LOFT_NO_WARN_RUNTIME`/`LOFT_NO_NULLFLOW` in the harness" — that recreates the
  tolerance by another name and hides the very surface we are freezing.
- **Backend uniformity holds.** These are parse-time diagnostics emitted once; unlike
  the steer's interpret-vs-native divergence, the filtered warnings don't vary by
  backend in the `code!` path.

## Falsification (design-protocol steps 3–4)

- **"The buckets are stable."** Attacked: removing clause A might change a fixture that
  also trips clause B, shifting B's bucket. Mitigation: re-run step 0's measure after
  each clause deletion, not just once — the worklist is regenerated, not trusted.
- **"Assert is always safe."** A `.warning()` pins prose that arc E may still improve.
  Mitigation: these warning texts are our own goldens (E1: prose is improvable, the
  goldens update with it) — acceptable, and the same policy the ~100 existing
  `parse_errors` assertions already accept. Where a warning has a code, the
  `strip_diag_code` normalisation keeps the assertion prose-only.
- **"Coverage preserved."** Vacuous if `runtime_warnings.rs` doesn't actually exercise
  a family. Mitigation: step "What must NOT be lost" verifies each family's e2e home
  *before* dropping its tolerance; a missing home is added first.

## See also
- `tests/testing.rs` — `assert_diagnostics` / `is_runtime_warning` (the filter to remove).
- `tests/runtime_warnings.rs`, `tests/steer_warning.rs` — the e2e homes that keep coverage.
- [flip-gate.md](flip-gate.md) — this is the Test-hygiene precondition row of the flip gate.
- CLAUDE.md § `LOFT_LOG` — the default-on/opt-out status of each warning family.
