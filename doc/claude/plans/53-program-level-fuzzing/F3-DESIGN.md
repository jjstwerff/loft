<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 F3 — differential on fuzzed programs — design gate (F3.0)

**Step F3.0 of the F3 decomposition** ([`STEPS.md`](STEPS.md)). No production
code. This pins what makes a *generated* program's output comparable across
backends — the one open question the plan flagged — read off a concrete
instance (design-protocol § constructive instrument).

## What F3 is

@PLN89's differential oracle (`tests/differential_oracle.rs`) already runs a
program on `--interpret`, `--native`, and (gated) wasm and flags any divergence
in normalised stdout / exit code / leak-freedom — but over a **fixed ~29-program
corpus**. F3 extends that same oracle to **generated** programs (F2's
`generate_keyed`, and later F1's mutated source), so a cross-backend codegen bug
the hand-written corpus never hits is caught.

## The one invariant

> **For a generated program in the DETERMINISTIC-OUTPUT SUBSET, every backend
> (`--interpret` ≡ `--native` ≡ wasm) produces byte-identical *normalised*
> stdout and the same exit code. Any divergence is a codegen finding.**

The load-bearing qualifier is "deterministic-output subset" — without it the
oracle drowns in false divergences (a fuzzer can trivially emit programs whose
output legitimately differs across backends).

## The deterministic-output subset rule (the open question, pinned)

A generated program is in the subset iff **its stdout is a pure function of the
program's abstract semantics** — nothing backend-incidental. Concretely, for the
keyed-collection generator:

- **In**: the canonical summary — population, then the `key=value;` pairs in the
  collection's *declared key order*. Iteration order is deterministic on every
  backend (loft's `hash` iterates a sorted key snapshot; `sorted`/`index` follow
  the declared key), and with distinct keys the values are fixed. Verified by
  construction: `print_hash` (population + ordered pairs) emits
  `pop=2;k000=1;k002=15;` **byte-identical on `--interpret` and `--native`**.
- **Out** (must never be printed): memory addresses / `DbRef` values, timings,
  iteration over an *unordered* structure, `sizeof`, or anything whose value is
  an implementation incident rather than a program result.

Two comparable signals, both deterministic:
1. **stdout** of the canonical summary — a *direct value* comparison (catches a
   wrong value / wrong order even if it doesn't crash).
2. **exit code** of the F2 *self-checking* programs — their baked expectations
   mean exit 0 ⟺ the backend computed the correct map, so interp-passes /
   native-fails is a divergence. Free differential on the F2 corpus.

## Failure paths (the generative enumeration)

1. **Non-semantic output → false divergence.** A generated program prints an
   address or a timing → backends differ legitimately → noise. Cured by the
   subset rule: the F3 generator emits ONLY the canonical summary.
2. **Backend-incidental stdout formatting** (CRLF vs LF, trailing whitespace) →
   false divergence. Cured by reusing @PLN89's `normalise_stdout` at the single
   comparison chokepoint — never a hand-rolled compare.
3. **rustc-per-native is heavy** (~seconds/program) → cannot run the 1500-spec
   sweep on native. F3 runs a **small curated corpus** (like @PLN89's ~29). This
   is a real coverage cap: `log` how many specs the differential leg covers — a
   silent top-N reads as "all backends agree" when it only checked a slice.
4. **A real codegen divergence** → the true finding (interp≠native output, or
   one backend crashes/leaks).

## Re-assertion sites — brittleness (Protocol step 2)

The invariant "backend-legitimate stdout differences are normalised away" must
hold at exactly one place: the comparison. @PLN89 already centralises it
(`normalise_stdout` + `divergences`). F3 **calls that**, adding no second
normaliser — N = 1, so there is no drift between "how F3 compares" and "how the
fixed-corpus oracle compares".

## Over-unification guard (Protocol step 4)

The tempting clean claim — "run *any* fuzzer-generated program through the
differential oracle" — is **false**: only the deterministic-output subset is
comparable. A program that prints non-semantic data is genuinely outside the
family; forcing it in produces noise, not findings. So F3's generator is a
*restricted* emitter (canonical summary only), not the full F1/F2 space. The
self-checking exit-code signal (2) sidesteps this — it needs no stdout subset —
so F3 can also just run the existing F2 corpus on `--native` and compare exit
codes, which is the cheapest first cut.

## The build (F3.1, against this contract)

1. A **printing variant** of `generate_keyed` (a `Summary` emit mode: build the
   collection, then `print` the canonical population + ordered `key=value;`
   summary — no asserts, so both backends run to completion and stdout is the
   signal).
2. A **small curated corpus** of specs (hash/sorted/index × a few sizes × a
   remove pattern × closures) — bounded because native shells out to rustc.
3. Run each on `--interpret` and `--native` (wasm via @PLN89's gated leg) with
   `run_mode`; compare with `divergences`. `log` the corpus size covered.
4. Cheapest first cut needing no printing variant: run the **existing F2
   self-checking corpus** on `--native` and assert exit 0 == interp's exit 0.

Home: an `#[ignore]`, `#[cfg(feature = "fuzzing")]` integration test (it shells
out to the `loft` binary and needs `loft::fuzz_keyed`), run with
`cargo test --features fuzzing --test <name> -- --ignored`.

## Exit criterion (F3.0)

Met: the invariant, the deterministic-output subset rule (verified byte-identical
on both backends via `print_hash`), the failure paths, the single-normaliser
chokepoint, and the restricted-emitter guard are pinned. F3.1 may build the
printing variant + curated differential against this contract.
