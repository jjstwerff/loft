<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 F1 — mutational raw-source fuzz target — design gate (F1.0)

**Step F1.0 of the F1 decomposition.** No code. This pins the oracle's
pass/fail contract — the rule every future F1 finding is judged against — via
Design Protocol 1. Written before F1.1 so the invariant surfaces on the page,
not by whack-a-mole in the driver.

## What F1 is

A `fuzz_target!` that takes raw bytes as loft source and drives
`parse → byte_code → execute` in-process, seeded from the ~2000 existing
`.loft` test files. Its job is the **panic-triage wave**: malformed input
surfaces unhardened `unwrap`/`panic!`/`unreachable!` paths in the front-end;
each surfaced path is either fixed or recorded.

The one architectural choice (see the parent README's step table): the driver
logic lives in a **library function `fuzz_one_source(&[u8])`** behind a
`fuzzing` cargo feature, and the `fuzz_target!` is a 2-line shim over it. The
same function is replayed over the seed corpus by an ordinary `#[cfg(test)]`
test — so the *tested* code IS the *fuzzed* code, and F1's correctness oracle
runs under `cargo test` on stable with no nightly/cargo-fuzz.

## The one invariant

> **`fuzz_one_source` panics (or aborts) if and only if the loft front-end
> executed an unhardened path on that input.** The driver contributes **zero**
> panics of its own — every harness-side failure is either impossible or
> converted to a clean return — and it swallows **zero** language panics.

The untested-case property this buys: any malformed input never tried before
gets a correct verdict *for the same reason* the tried ones do — because the
driver is a transparent conduit. Its verdict reflects the language, not the
harness. Naming this is what makes the seed-corpus replay **constitutive** (the
2000 green cells are the only reason we trust the target) rather than merely
confirmatory.

### The three terminal states (the classifier)

A run of `fuzz_one_source` ends in exactly one of:

| Terminal state | How the front-end got there | F1 verdict |
|---|---|---|
| **Diagnostics rejection** — `p.diagnostics` has a non-warning line | intended rejection of bad input | **clean** (the common case; F1 feeds garbage) |
| **`had_fatal`** — a loft-level `raise`/`assert`/index-OOB fault | the program compiled and faulted at runtime | **clean** ← *the sharp F1↔F2 flip* |
| **Rust `panic!` / abort** | an unhardened native path | **FINDING** |

The `had_fatal` row is the load-bearing difference from F2
(`program_ownership`): F2's programs are total/valid-by-construction, so
`had_fatal` there is a miscompile finding. F1's programs are arbitrary, so a
runtime fault is the *correct* behavior of a total interpreter on a bad
program. Verified: `src/state/mod.rs:3638` — a loft fault sets
`had_fatal = true` and returns; it is **not** a Rust panic.

## Failure paths (the generative enumeration)

Every way the oracle can return a **wrong verdict**:

1. **False negative — swallowed panic.** `run_oracle` (F2) gates two panics by
   `msg.contains("H5: definition COUNT diverged")` string-match in two places.
   Every such gate silently hides a class of real findings. → *F1 must not
   inherit this speculatively* (see re-assertion sites).
2. **False negative — wrong-answer defect.** A program that mis-compiles but
   runs to completion with wrong output: no panic, no `had_fatal` → clean. F1
   is a **panic/abort** oracle; silent-wrong-answer is out of scope (that is
   F3's differential domain). Documented limit, not a bug.
3. **False positive — harness panic.** The trap the parent README names
   ("harness crashes indistinguishable from real bugs"): a temp-file write
   `.expect(...)` panicking on a full disk, a bad stdlib clone, mishandled
   non-UTF-8 — any of these reads as a finding but is noise.
4. **False positive — state bleed.** If `Data`/`Stores` is not truly fresh per
   input, input N's residue makes N+1 panic, mis-attributed to N+1.
5. **Non-termination.** Arbitrary source produces an infinite loop; the run
   never returns → a hung `cargo test`, or a libfuzzer timeout reported as a
   finding when the program is merely slow-but-correct.
6. **Abort, not panic.** A pathologically nested input overflows the native
   parser stack → `SIGSEGV`/abort, uncatchable by `catch_unwind` → aborts the
   whole `cargo test` binary.

## Re-assertion sites — the brittleness, counted now (Protocol step 2)

The invariant "driver contributes zero panics / swallows zero language panics"
must hold at **N = 6** independent sites, and omission at any is **silent** (a
wrong verdict, not a compile error):

| # | Site | Silent-failure mode | Collapse / loud-omission cure |
|---|---|---|---|
| 1 | temp-file write | `.expect` panics on I/O error → false positive (path 3) | convert I/O `Err` to a clean return; I/O is never a language finding |
| 2 | temp-file remove | leak, not a wrong verdict | keep `let _ =` (non-panicking) |
| 3 | stdlib clone freshness | state bleed (path 4) | clone fresh per call, as `run_oracle` does — do not share |
| 4 | UTF-8 gate | harness panic on non-UTF-8 (path 3) | non-UTF-8 → early clean return (lexer's contract is `&str`) |
| 5 | **panic-gate allowlist** | **swallowed finding (path 1) — the dangerous one** | **one** outermost `catch_unwind` boundary; the allowlist is an explicit, enumerated set pinned by the F1.2 falsification test — never scattered `msg.contains` |
| 6 | non-termination bound | hang / timeout-noise (path 5) | see bounding policy below |

**`N × silence` reduction.** Site 5 is where F2's design is brittle and F1
must not copy it. The collapse: F1 starts with an **empty** panic allowlist —
every panic propagates. If the seed-corpus replay (F1.3) trips the known H5
def-count assert (a filed latent asymmetry, not a new bug), it is added to the
allowlist in **one** place *at that point*, tagged with its issue number, and
the F1.2 test asserts the allowlist is exactly that set and no more — so any
drift is **loud** (a failing test), not a silently-growing denylist.

## Load-bearing claims + falsification probes (Protocol steps 3–4)

- **Claim A — "every intended rejection is a `Diagnostics`, never a panic."**
  Probe (run): `grep` the front-end panic surface. Result: **36** panic-shaped
  constructs (27 `panic!`, 9 `unreachable!`, 0 `unwrap`/`expect`) in
  `lexer.rs` + `parser/`. Claim A is **false as an absolute** — and that is the
  point: the reachable subset of those 36 *is* the F1 finding surface. The
  probe calibrates the expected triage-wave size; it does not refute the
  design.
- **Claim B — "the driver contributes zero panics."** Probe (audit of the 6
  sites): the temp-file write `.expect("write fuzz program")` in `run_oracle`
  **falsifies B** — it panics on I/O error. F1 fixes it (site 1). All other
  driver `.expect`s are one-time stdlib init: input-independent, so a panic
  there is a genuine setup error, acceptable.
- **Claim C — "non-UTF-8 and non-termination give no wrong verdict."** Probes
  become F1.2/F1.3 tests: feed non-UTF-8 bytes (must clean-return, site 4);
  feed a non-terminating program (must be bounded, site 6).
- **Claim D — the cleanest, attacked hardest (over-unification guard):
  "a panic is a *sufficient* signal of a language defect."** Two exceptions
  found by attacking it:
  - a loft `raise`/`assert` is **`had_fatal`, not a Rust panic** — verified,
    already handled by the three-state classifier (clean for F1);
  - a deeply-nested input **aborts** (stack overflow), uncatchable by
    `catch_unwind` (failure path 6). So "panic = finding" is *nearly* clean but
    not total: the abort class is a real defect the libfuzzer run still catches
    (it reports the crash), but the `cargo test` replay form is fragile to it
    until a nesting bound exists.

**What F1 therefore does NOT detect** (written down so F1 is not oversold): the
silent wrong-answer class (path 2 → F3's domain), and — in its `cargo test`
form — the stack-overflow-abort class until bounded.

## Bounding policy (the non-termination rule, pinned)

There is **no in-execution step budget** in `State`; the only mechanism is the
`LOFT_TIMEOUT` watchdog, which hard-kills the whole process — fatal for both a
fuzzer and a `cargo test`. Decision, evidence-gated (do **not** build a fuel
budget speculatively):

- **F1.3 seed-corpus replay:** the ~2000 seed files all terminate (they are
  passing tests), so the replay needs **no bound** and ships on stable now.
- **F1.4 live fuzzing:** mutated programs may not terminate; rely on
  **libfuzzer's own `-timeout` / `-rss_limit_mb`** initially.
- **Promote to a real `State` step-budget** (a deterministic instruction
  counter that converts non-termination to a clean bounded return, usable in
  both the test and the fuzzer) **only if** triage shows timeout-noise drowning
  findings. That is a runtime change routed through the loft-codegen skill when
  its trigger fires — not part of F1.0.

Same for the abort class (failure path 6): add a parser recursion-depth guard
only if the corpus/fuzz run actually aborts on nesting; until then it is a
documented limit of the `cargo test` form.

## Build divergences — what F1.1–F1.3 taught the contract (Protocol step 6)

Two axes the design gate did not foresee, surfaced by the F1.3 replay (the
build is the last probe):

1. **Poison belongs in the isolated fuzzer, not the in-process replay.** F1.0
   baked F4 poison-on-free into the oracle. The first replay SIGSEGV'd on
   `walk.loft` — a real read-during-grow store UAF (finding F1-2) that poison
   correctly amplified. But a poison abort is *uncatchable*, so it killed the
   whole 1332-file sweep with no isolation, and the class it surfaces
   (store-lifetime UAF) is @PLN85 / `program_ownership`'s remit, not F1's
   front-end one. Refinement: `classify_source_with(src, poison)` — the
   **libfuzzer** target runs poison ON (each crash is an isolated recorded
   artifact — pure upside); the **replay** runs it OFF by default (front-end
   remit) with `LOFT_F1_POISON=1` opt-in.

2. **Two false-positive classes the observer must filter** (they live in the
   replay's triage, NOT in `classify_source` — the oracle stays transparent):
   - **Environment** — a program calling a native-extension function whose
     cdylib is not built in the fuzz context panics "native function not
     loaded" (28 files, incl. `tests/integration/multiplayer/*`). Keyed on the
     message, counted as `env-skipped`, never a finding.
   - **Harness artifact** — an index-OOB (`51-tuple-as-arg.loft`) that fires
     only under the preloaded-stdlib-cache parse path, not a fresh CLI parse
     (the clone-cache asymmetry `program_ownership` also documents). Allowlisted
     as a known harness limitation, not a language bug.

   The `TRIAGED` allowlist stays the F1.0 site-5 shape: explicit, referenced to
   a catalogue id, and anything unlisted fails loudly.

## Exit criterion (F1.0)

Met: the invariant, the three-state classifier, the six re-assertion sites with
their cures, the four probed claims, and the bounding policy are all on the
page. F1.1 (reify `fuzz_one_source` behind `feature="fuzzing"`) may proceed
against this contract; F1.2 is the falsification test that proves the classifier
can fail before F1.3 trusts it over the corpus.
