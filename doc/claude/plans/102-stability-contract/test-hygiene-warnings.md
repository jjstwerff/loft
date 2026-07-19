<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc E — remove the tolerated-warnings filter (test hygiene)

> **Status: DESIGN (2026-07-19).** The warning-surface half of arc E's "be strict
> now" one-way-door audit. Bounded, in-repo, both backends. The ladder removes the
> filter **one category at a time** so each category's blast radius is isolated and
> committed separately. Reference: README § *Pre-freeze test-hygiene*.

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

The absorbed clauses (verbatim, `testing.rs`):

| Clause | Warning family | Default-on? |
|---|---|---|
| `"Warning: division may produce null"` | @PLN25 runtime-null advisory (`÷`) | on (`LOFT_NO_WARN_RUNTIME` opts out) |
| `"Warning: modulus may produce null"` | same (`%`) | on |
| ``"Warning: `v[i]` may produce null"`` | same (vector index) | on |
| ``"Warning: `s[i]` may produce null"`` | same (text index) | on |
| `"Warning: field "` | `not null` field-read hint | on (`LOFT_NO_HINT_NOT_NULL`) |
| ``"Warning: `&` on parameter "`` | @PLN87 redundant-`&` lint | on |
| ``"Warning: `not null` is deprecated"`` | @PLN25 F2 retirement advisory | on |
| ``"Warning: a nullable `…` is stored into"`` | @PLN25 N-Store nudge | on |
| ``"Warning: `null` is stored into"`` | N-Store null form | on |

(Confirm the exact current clause set at edit time — the list drifts as lints are
added; the ladder below is per-clause, so it absorbs a changed set naturally.)

## The invariant (design-protocol step 1)

> **After this work, every `code!` fixture's diagnostic set is EXACTLY what loft emits
> for that source — no more, no less.** A warning a fixture emits is either
> `.warning("…")`-asserted (the fixture means to exercise it) or eliminated by fixing
> the `.loft` (the fixture didn't mean to). Zero silent absorption.

## What must NOT be lost

End-to-end coverage of these warnings already lives OUTSIDE the `code!` harness —
`tests/runtime_warnings.rs` and `tests/steer_warning.rs` assert them at the binary
level (`Command::new`) / parser-unit level. So deleting the `code!`-harness filter
removes *silent tolerance*, not *coverage*. Before deleting a clause, confirm its
family is still covered by one of those files; if a family has no e2e home, add one
there first (that is the coverage, the `code!` assertions are just per-fixture
correctness).

## The ladder — one clause per commit (small, isolated blast radius)

Do the **lowest-fixture-count clause first** to shake out the mechanics on a small
surface, then work up. Each step is one commit.

| # | Step | Proof |
|---|---|---|
| 0 | **Measure.** Temporarily make `is_runtime_warning` return `false` wholesale, run `cargo test` (parse_errors + the `code!` users: expressions, issues, …), and bucket every newly-`FAILED` fixture by clause. This is the worklist + per-category counts (drives the ordering). Revert the probe. | a bucketed list: clause → {test fns}; no code committed |
| 1 | **Pick the smallest bucket. For each fixture in it:** decide *assert* vs *fix*. **Assert** when the warning is correct for what the fixture tests (add `.warning("<text> at <fn>:L:C")`, exact — the harness is exact-match, cf. the E1 `strip_diag_code` note). **Fix** when the `.loft` tripped it incidentally (e.g. discharge the nullable with `?? d`, drop a dead `&`, rename a `not null`) so it no longer emits. Prefer *fix* for incidental trips, *assert* only where the warning is the point. | each edited fixture passes; the clause's bucket is empty |
| 2 | **Delete that clause** from `is_runtime_warning`. Run the full `code!` suite green. Commit `test-hygiene: assert-or-fix <family>; drop its tolerance`. | suite green with the clause gone; a re-introduced trip now FAILS (positive control: the tolerance is truly gone) |
| 3 | **Repeat** steps 1–2 per remaining clause, smallest-first. The families are independent, so a mistake in one never widens another's diff. | — |
| 4 | **Delete the `is_runtime_warning` mechanism entirely** once the last clause is gone (the predicate + its call site + the now-dead `is_empty()`/filter branch). The harness now treats ANY unexpected warning as a failure. | grep shows no residual filter; suite green; an injected stray warning in any fixture fails |
| 5 | **Lock it.** Add one meta-fixture: a `code!` whose source emits a known warning WITHOUT a matching `.warning(...)` must fail the harness (guards against a future silent-tolerance re-introduction). | the meta-fixture fails when the assertion is omitted, passes when present |

## Ordering rationale + traps

- **Smallest bucket first** is not cosmetic: it proves the assert-vs-fix mechanics
  (exact-string `.warning()`, position columns) on ~a handful of fixtures before the
  large buckets (the null-flow "may produce null" family is likely the biggest — many
  `÷`/index fixtures incidentally trip it).
- **Position columns are exact.** `.warning("… at <fn>:L:C")` must match the emitted
  column; capture it from the failing-test diff (as done for the E2-B parse_errors
  guards), do not hand-count.
- **Default-on vs cache.** These are parse-time warnings, so a WARM whole-program
  cache hit skips them — the `code!` harness compiles cold, so they always fire there
  (no cache confound). No `LOFT_LOG`/env interaction to worry about in-harness.
- **Do not silence by env.** The fix is assert-or-correct, never "set
  `LOFT_NO_WARN_RUNTIME` in the harness" — that would recreate the tolerance by another
  name and hide the very surface we are freezing.
- **A family with no e2e home** (step "What must NOT be lost") → add the
  `runtime_warnings.rs` coverage in the SAME commit that drops its `code!` tolerance,
  so coverage never dips between commits.

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
