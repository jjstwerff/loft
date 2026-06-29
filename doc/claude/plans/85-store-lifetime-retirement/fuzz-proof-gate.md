<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Fuzz-proof gate — prove the store-lifetime class closed *by construction*

> **Part of [@PLN85](README.md)** (store-lifetime retirement). **Status:** SLOT OPEN —
> design, not built. **This is wide-release gate 1** (the floor that does not betray you,
> [GOALS.md § The deeper aim](../../GOALS.md); the bar lives in
> [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md)). Written as a
> `design-protocol` hypothesis: the fuzz-proof is the **falsification instrument** for the
> claim "the store-lifetime class is closed," designed to BREAK the invariant, not confirm it.

## Why a separate slot when @PLN85 already closed each cluster

The investigation reached outcome (b): the store-lifetime bugs are *independent mechanisms*,
each fixed at its own chokepoint with a per-cluster regression guard (clusters II / III / V /
C / 462 — see [README.md](README.md)). Those guards prove the **known shapes stay fixed**.
They do **not** prove an **unknown composition** can't violate the invariant. "No new report
this week" is anecdotal silence, not proof — and at one dogfooding agent the class kept
spawning bugs precisely because each new *composition* found a hole the last fix didn't cover.

This slot closes that gap: turn "every known dangerous shape is fixed + guarded" into "the
**class** is closed by construction," proven by a standing generative instrument run at scale.
That is the difference between *quiet* and *sealed* — and only *sealed* clears the gate.

## The one invariant being proven

(From [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md); the `deps` ownership chokepoint.)

> At every program point each heap store has **exactly one owner**; all mutation flows through
> that owner; a non-owning alias is **read-only** and **never outlives** its owner.

The whole class is this one invariant violated four ways — the instrument must catch all four:

| Violation | Observable signal the oracle checks |
|---|---|
| **leak** — ownership dropped, no free | store count grows across runs (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) |
| **double-free** — ownership duplicated | native sanitizer hit / store-table corruption |
| **use-after-free** — alias outlives owner | sanitizer hit / OOB store index (the `65535` family) |
| **silent corruption** — two owners mutate one store | cross-backend **value + length divergence** (the #437 NRVO shape) |

Silent corruption is the dangerous one: no crash, no leak — only a wrong value. So a leak-and-
crash oracle is **insufficient**; the cross-backend value diff is the load-bearing check.

## The instrument (what to build)

**What already EXISTS — reuse, do not rebuild (inventory 2026-06-29).** The cross-backend ORACLE
is built: `tests/differential_oracle.rs` (@PLN89) — `divergences(interp, native)` checks normalised
stdout + exit-code + leak, *with its own positive-control test*; the leak signal is the identical
string `"stores not freed"` on both backends (`LOFT_NATIVE_LEAK_CHECK=1` for native). A `fuzz/`
cargo-fuzz crate exists (libfuzzer + `arbitrary`) with *store-level* targets. **The genuine gaps are
three:** (1) a **program-level generator** (@PLN53 F1/F2, unbuilt); (2) the **`LOFT_POISON`** arena
poison-on-free detector (@PLN54 S3, unbuilt) — the *only* thing that catches the store double-free/UAF
class, because loft's arena reuse defeats stock ASan/Miri/Valgrind; (3) **native-backend ASan**
(@PLN54 S6, unbuilt). So this gate = build the generator (1) + add the arena UAF detector (2), feeding
the existing oracle.

A generative harness that emits **valid loft programs** over the ownership-composition space, runs
each through the oracle, and turns any finding into a minimized regression.

- **Generator grammar — seed from the existing corpus.** The `probes/` directory already
  encodes the known dangerous shapes (matrix A–F, borrowed-view, adopt-free, 462, coalesce);
  the generator's grammar must *reach at least those*, then mutate/compose beyond them.
  Composition axes (the bug-bearing ones from the clusters):
  `delivery {return | bind | arg-pass | field-store}` ×
  `source {local | param | borrowed-view (v[i] / match-arm / if-arm) | nested}` ×
  `value {dense vector | nullable | struct | enum | hash}` ×
  `churn {none | reuse-slot | par}` × `backend {interpret | native+ASan}`.
- **Oracle (all four, every program):** (a) cross-backend **value + length** equality, interp
  as the reference; (b) **zero leak** on both backends; (c) **zero sanitizer finding** on the
  ASan/UBSan native build; (d) **clean process exit** (the teardown-crash trap — "PASSED prints"
  is not enough, check the exit code).
- **Minimize + graduate.** Each counterexample shrinks to a `tests/scripts/85-*.loft` regression
  (the same per-cluster guard mechanism, now fed by the fuzzer instead of by hand).
- **Don't reinvent the harness — focus the existing instrument plans.** @PLN53 (program-level
  fuzzing) and @PLN54 (sanitizer coverage expansion) are the standing instruments; the store-level
  fuzz harness (store.rs LLRB / coalesce / claims) is the layer below. This slot is the
  **@PLN85-specific focusing** of those onto the ownership invariant + the cross-backend oracle —
  it consumes them, it does not duplicate them.

## Build-order dependency — BLOCKED BY @PLN25 (the load-bearing constraint)

The generator's value grammar and the oracle's ownership model **both depend on the settled
value/null model**: what a value *is* (dense vs nullable) and how it copies vs borrows is exactly
what @PLN25 decides, and ownership flows through the `deps` facts that model defines. Fuzzing a
moving value model proves nothing. This is why earlier @PLN85 attempts flailed — there wasn't
enough of @PLN25 settled to know what to build.

Consequence: **@PLN25 leads.** The vectors-half is settled (dense default, merged), so the
fuzz-proof can **start now on the vectors-settled subset** and **expand as scalars land**
(scalars in flight — see [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md)). Do not gate the
whole instrument on @PLN25 being 100% done; gate each composition axis on its value-model piece
being settled.

## Done criteria — what "gate met" means

1. The harness runs as a standing job (CI or scheduled) with **zero findings across all four
   oracles, both backends**, over a meaningful budget (N programs / M cpu-hours — set the number
   when the generator exists; record it, no silent cap).
2. **Coverage is non-vacuous:** every historical cluster shape (II / III / V / C / 462) is
   provably within the generator's reachable space, so zero-findings means "covers the known
   class," not "the grammar is too narrow to express the bug."
3. Then the class is **closed by construction** — the wide-release gate-1 definition of
   *stabilized*. Until (1)+(2) hold, the memory model is *quiet*, not *sealed*.

## Status — first increment BUILT (2026-06-29): `fuzz/ownership_fuzz.py`

A first generator + runner: [fuzz/ownership_fuzz.py](fuzz/ownership_fuzz.py). Two-stage per the perf
split (native compiles via rustc — too slow for a tight loop): **interp fast-loop** with leak check,
**native replay** on every flagged program (cross-backend divergence + native leak). Mutates the
**churn axis** — scales every `0..N` loop/pressure bound — because the over-free class only corrupts
once a freed slot is REUSED. Violation modes: `CRASH` (signal only — a clean `exit=1` that *agrees*
across backends is not a bug, only a `DIVERGENCE` if they disagree), `LEAK`, `DIVERGENCE`.

**Positive control proven (engineering-rigor: the harness can fail).** `--self-test` requires
`probes/over-free-sweep/P14-enum-field-vec.loft` to be flagged — interp **SIGSEGV (signal 11)**, native
passes. It is. A harness silent on P14 would be vacuous.

**Calibration overturned the dated sweep table** — verify against observation, not a stale doc:

| Shape | over-free-sweep/README verdict | ACTUAL on current build (harness) |
|---|---|---|
| **P14** (enum-field vector via match arm + churn) | 🔴 open, interp-only | **LIVE** — interp SIGSEGV, native ok (CRASH + DIVERGENCE) |
| **P10** (accumulate borrowed-view results) | "PASS" | **LIVE** — interp `len(t)=7` (corrupt, own assert fails), native ok (DIVERGENCE) |
| **P3** (mon_one-cond native leak) | 🟠 leak open | **FIXED** — clean both backends, no leak |
| **P9** (struct-field vector borrow + churn) | 🔴 open (interp r=0, native crash) | **FIXED** — clean both backends |
| adopt-re-return NRVO leak (leak-462) | leak open | **FIXED** — no `stores not freed` either backend |

So the live positive controls are **P14 (crash) + P10 (value divergence)**; the leak arm is validated
by the existing oracle's own positive-control test. Baseline run: **2/28 over-free-sweep probes
flagged** (P10, P14), churn-mutation reproduces P14 across all variants.

### Increment 2 — full grammar BUILT (2026-06-29): `fuzz/grammar_gen.py`

Widens past churn to the cross-product **shape (source × delivery) × value × churn**: 6 grounded
shapes × {struct, scalar} × {none, heavy} = **24 self-checking programs**, each asserting the
source-length + delivered-length invariant the over-free violates. Generated programs are valid loft
(the compiler is the validity oracle; agreed-compile-errors drop out). **3/24 flagged — and the
grammar LOCALIZES the live class precisely:**

| cell | result |
|---|---|
| `match_return × struct × heavy` | CRASH(interp **SIGSEGV**) + divergence — the P14 class, regenerated from a clean grammar (not a mutated probe) |
| `match_return × struct × none` | CRASH(interp **SIGABRT**) + divergence — **new:** the match-arm borrow is unsound even WITHOUT slot-reuse |
| `elem_accumulate × struct × heavy` | DIVERGENCE (interp assert-fail, native ok) — the P10 class, regenerated |
| field_return / field_local / field_reassign / if_return (any value) | clean — the **field-view** shapes are FIXED |
| every `scalar` cell | clean — the class is **record-store-specific**; `vector<integer>` does not hit it |

So the live over-free is precisely **struct-value × {match-arm, element-accumulate} source**. The
generator reproduced both live classes from a clean grammar and pinned a new no-churn abort variant —
this is the boundary-matrix the chokepoint fix (Cluster C) must be measured against.

**Next, in order:**
1. **Correct the stale catalog** — over-free-sweep/README P10 verdict PASS → divergence; mark P3/P9/leak fixed. ✅ done.
2. **Graduate the generator in-process** — port `grammar_gen.py` to a `fuzz/fuzz_targets/program_ownership.rs` cargo-fuzz target (the full grammar is built; this makes it run in-process/fast under libfuzzer coverage-guidance, no rustc-per-program) and add the `value` axis pieces (enum-payload, nested, hash) as @PLN25 settles them.
3. **Build `LOFT_POISON`** (@PLN54 S3) — the arena UAF/double-free detector stock sanitizers miss; wire it into the native replay.
4. **Minimize** P14 + P10 to `tests/scripts/85-*.loft` regressions; wire the harness into the differential-oracle corpus as a standing job (the done-criteria budget).
- **Method gate:** every M+ step runs the `design-protocol` skill; this doc IS the hypothesis.
- **Blocked-by reminder:** widen the value axis only as @PLN25 settles it (above).

## See also

- [README.md](README.md) — the @PLN85 clusters this generalizes.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the `deps` invariant the oracle checks.
- [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md) — gate 1, and the
  @PLN25 dependency.
- @PLN53 (program-level fuzzing) · @PLN54 (sanitizer coverage expansion) — the instruments this consumes.
- [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md) — the value model this is blocked by.
