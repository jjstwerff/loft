<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# CI budget — what runs when, where the time goes, what to split

> **Status: DESIGN (2026-07-26).** Measured against real runs, not estimates:
> PR run `30203257738` and nightly run `30191161652`. Every duration below is
> observed. The owner's rule is the premise: **a normal PR must settle within
> 20 minutes**, and anything that pushes past it gets parallelised or moved to a
> slower cadence until we are consistently back under.

## The rule that decides placement

**The PR gate tests what THIS DIFF can break. The nightly tests what THE WORLD
can break.**

Toolchain drift, published-library rot, registry decay and time-passing checks
are "the world": running them per-PR says nothing about the diff, costs the
author minutes, and trains everyone to ignore red. Memory safety, codegen
correctness and internal invariants are "the diff": they must fail on the PR that
introduced them, when the author still has the context to fix it.

A second rule follows from the first: **a gate that cannot fail because of a diff
does not belong on a PR, however cheap it is.**

## What runs when — today

| cadence | jobs | trigger |
|---|---|---|
| **per PR** (`ci.yml`) | full suite ubuntu + macOS, ASan UAF/OOB (ubuntu), `stack_align_guard`, browser build+probe, Clippy, Format, Doc hygiene, CodeQL, feature catalogue, contract-goldens drift, API compat, several advisory doc jobs | `pull_request` |
| **push to main** | everything above **plus the real `Test (windows-latest)` leg** (~53 min) | `push: main` |
| **nightly 04:00** (`miri.yml`) | Miri ×2, ASan UAF/OOB ×2, ASan interpreter leak ×2, POISON arena-UAF, TSan, native-backend ASan, debug-assertions, toolchain matrix (beta+nightly), doc index hygiene, library health, stale-plan audit | `schedule` |
| **nightly 04:30** | `registry-validation` — every published package installed + tested on both backends | `schedule` |
| **nightly 06:17 + on `src/**`,`default/**`** | `revalidate-libs` — every published lib against this loft, plus the warning dashboard | `schedule`, `push`, `pull_request` |
| **nightly 07:00** | `lib-branch-report` — unmerged branches across the library repos | `schedule` |
| **daily 03:00** | the Windows leg (mirrored onto PRs as the non-blocking `Windows (daily)` check) | `schedule` |
| **library repos** | one `library-ci` per repo, all callers of `library-ci-reusable.yml` | `push: main`, `pull_request` |

## Where the time goes — measured

### The PR critical path is the test suite, not the build

| leg | total | build steps | **test step** | nextest wall | tests |
|---|---|---|---|---|---|
| `Test (macos-latest)` | **31m40s** | 8m20s | **22m18s** | 1152s | 3481 |
| `Test (ubuntu-latest)` | **21m46s** | 5m05s | **14m30s** | 734s | 3481 (1 flaky) |
| `ASan UAF/OOB (ubuntu)` | 11m39s | — | — | — | — |

Both legs exceed the 20-minute rule; macOS by 11m40s. Build is **not** the
problem — it is a ~8m (macOS) / ~5m (ubuntu) floor. Test execution is 70 % of the
macOS leg.

### Two tests dominate everything

Time by test binary (macOS / ubuntu):

| binary | macOS | ubuntu | tests |
|---|---|---|---|
| `ir_schema_roundtrip` | **866s** | **744s** | 8 |
| `codegen_emitter` | 268s | 215s | 21 |
| `exit_codes` | 251s | 207s | 24 |
| `native` | 240s | 134s | 10 |
| `deliver_wasm` | 216s | 235s | 17 |
| `html_gl_imports` | 107s | 113s | 1 |

And inside `ir_schema_roundtrip`, **two tests are the whole story**:

| test | macOS | ubuntu |
|---|---|---|
| `stdlib_load_compares_equal_to_fresh` | **379s** | **308s** |
| `stdlib_whole_data_round_trip` | **375s** | **308s** |
| `tests_scripts_round_trip` | 83s | 99s |

Those two are 754s / 616s — about 65 % of the largest binary, and **~11 % of all
per-test time on their own**.

### The floor nobody can shard away

A parallel suite cannot finish faster than its **slowest single test**. Today
that is **379s (6m19s)** on macOS. So:

```
minimum PR leg  =  build floor  +  slowest single test
macOS           =  8m20s        +  6m19s   =  14m39s   (before any other test runs)
ubuntu          =  5m05s        +  5m08s   =  10m13s
```

Sharding buys nothing below that line. **Any plan that leaves those two tests on
the PR gate is capped at ~15 minutes on macOS**, which leaves almost no room for
the gates we want to adopt. They are the first thing to move.

## What to optimise and split

### A. Move the two stdlib round-trip tests to nightly — biggest single win

`stdlib_load_compares_equal_to_fresh` and `stdlib_whole_data_round_trip` verify
that the whole parsed stdlib survives a serialise/deserialise cycle byte-identical.
That is a **format-stability** property: it breaks when the IR schema or the
serialiser changes, which is rare and always deliberate.

- **PR keeps** `tests_scripts_round_trip` (83s/99s) — the cheap canary that still
  fails if round-tripping breaks at all.
- **Nightly gains** the two exhaustive ones.
- **Effect:** removes 754s/616s of work and drops the sharding floor from 379s to
  **194s** (macOS) / 139s (ubuntu).

Risk: an IR-schema diff that breaks only the exhaustive pair lands and is caught
that night rather than on the PR. Acceptable — it is precisely a "format did not
change by accident" check, and the canary still guards the common case.

### B. Shard the suite with `nextest --partition`

`cargo nextest run --partition count:N/M` splits a suite across runners with no
source changes. Each shard pays the build floor, so shards are cheap in wall-clock
and linear in machine minutes (this repo is public — GitHub minutes are free).

| config | macOS projected | ubuntu projected |
|---|---|---|
| today | 31m40s | 21m46s |
| after **A** only | ~20m | ~15m |
| after **A** + 2 shards | **~14m** | **~11m** |
| after **A** + 3 shards (macOS) | **~12m** | — |

### C. Asymmetric platform coverage

macOS is ~50 % slower than ubuntu for identical work and duplicates it exactly.
Options, in order of preference:

1. **Full suite on ubuntu per PR; macOS runs the platform-sensitive families**
   (`native`, `codegen_emitter`, `exit_codes`, `deliver_wasm`, `html_*`) — the
   ones where an ARM/Darwin difference can actually appear. Full macOS stays
   nightly. Saves ~10 minutes of the worst leg.
2. Keep both full but shard macOS harder (B).

Option 1 is the honest one: a pure-interpreter test cannot fail on macOS alone —
and when it did (@P383), it was a *memory* bug the ASan-macOS leg is there to catch.

### D. Adopt the cheap nightly gates onto the PR — free under the ceiling

Once A–C put the critical path near ~14 minutes, these all fit **in parallel**
without touching it, and each one can be broken by a diff:

| gate | cost | catches |
|---|---|---|
| Miri hard-UB ×2 | 1m15s / 1m07s | hard UB in the store/unsafe layer |
| ThreadSanitizer | 2m57s | `par` data races |
| POISON arena-UAF | 3m43s | store-internal use-after-free |
| Native-backend ASan | 3m57s | codegen-only memory bugs |
| Debug-assertions | 5m46s | violated internal invariants |
| ASan UAF/OOB **macOS** | 10m38s | the @P383 class (ARM-only) |

**Stays nightly:** ASan interpreter leak gate (23m29s / 25m28s — over the ceiling
by itself, and deliberately the slow single-threaded pass), toolchain matrix
(tests rustc drift, not the diff), `registry-validation`, `revalidate-libs`,
library health, stale-plan audit, `lib-branch-report`.

### E. Build floor (secondary)

macOS spends 8m20s building before a test runs. `scripts/sccache_env.sh` exists
and is unused in CI. Worth measuring after A–D; it is the next constraint once the
suite stops dominating, not before.

## The daily overview

Two problems today: a red nightly is **undifferentiated** (six nights of
`registry-validation` failure looked identical to the five before it, so nobody
reached the cause), and **every** non-success files a GitHub issue — which is how
a jq crash in a matrix selector became a ticket.

### Narrow the auto-issue to the blocking class

`notify` currently watches nine gates and files on any non-success. It should file
**only** when the language itself is unsound:

- Miri hard-UB · POISON arena-UAF · ASan UAF/OOB · debug-assertions

Everything else reports without ticketing. Once those four are gated per-PR
(section D), a nightly red in them means something slipped past the PR gate —
exactly the case worth interrupting a human for.

### One "Daily status" run, not an issue list

A single scheduled workflow, after the others, writing **one job summary**:

- every gate with conclusion + link
- the library warning dashboard (`lib_warning_scan.py collect`)
- `registry-validation` result, per package
- in-flight library branches
- anything INCONCLUSIVE — a gate that did not run proves nothing

Its own conclusion goes red **only** on the blocking class, so a README badge for
that one workflow answers "is anything blocking?" without opening anything. Cost
is API calls — well under a minute.

## Phasing

| phase | change | effect |
|---|---|---|
| **1** | move the two stdlib round-trip tests to nightly (A) | macOS ~20m, ubuntu ~15m; floor 379s → 194s |
| **2** | shard both legs 2× (B) | macOS ~14m, ubuntu ~11m — **under the rule** |
| **3** | adopt the six cheap gates (D) | large coverage gain, no wall-clock change |
| **4** | daily digest + narrowed auto-issues | one place to look, no ticket noise |
| **5** | asymmetric macOS (C) / sccache (E) | headroom for the next thing to adopt |

Phases 1–2 are the ones that satisfy the rule; 3 is what the rule buys us.

## See also

- [TESTING.md](TESTING.md) — the test framework, `LOFT_LOG`, targeted-suite map
- [DEVELOPMENT.md](DEVELOPMENT.md) — workflow and where changes land
- [PERFORMANCE.md](PERFORMANCE.md) — runtime benchmarks (not CI cost)
