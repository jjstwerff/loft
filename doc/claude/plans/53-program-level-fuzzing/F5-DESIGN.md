<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 F5 — OSS-Fuzz onboarding — precondition gate (F5.0)

**Step F5.0 of the F5 decomposition** ([`STEPS.md`](STEPS.md)). This is a
**gate**, not a build: F5 must not start until the targets run crash-clean for a
sustained window, because a crashy continuous run is noise, not signal. It also
blueprints F5.1 (the project skeleton) so the gate result feeds it directly.

## Gate criteria

A target is ready for continuous OSS-Fuzz running when, **in the exact
configuration it ships** (poison on/off as the target sets it), it runs
crash-clean over its seed corpus and a sustained local run finds no unfixed
crash.

## Evidence — the in-process sweeps (fresh, this session)

The stable in-process sweeps are the proxy for a sustained coverage-guided run
(no nightly / cargo-fuzz on this box, so the *real* run is still owed — see the
verdict). All green:

| Signal | Config | Result |
|---|---|---|
| F1.3 seed-corpus replay (`program_source`) | poison **off** | green — 0 new findings over 1333 `.loft` files (F1-1 fixed; env/harness classes classified) |
| F2.6 wide sweep (`program_keyed`) | poison **on** | green — 0 findings over 1500 seeded-random specs |
| F3.2 differential (`fuzz_differential`) | interp ≡ native | green — 24 generated programs agree |
| F2.1/2.2/2.4 lib sweeps | poison on/off | green |

## The one open crash-class (the gate's real finding)

`program_source` **ships with poison ON** (`fuzz_one_source` →
`classify_source_with(.., true)`). Under poison, the seed corpus is **not**
crash-clean: **F1-2** — the read-during-grow store use-after-free in
`walk.loft` — SIGSEGVs (a real bug, but store-lifetime, @PLN85's remit, not F1's
front-end one). So an OSS-Fuzz run of `program_source` with poison would crash
on a seed immediately and read as a broken target until @PLN85 lands.

This is why the gate distinguishes configuration:

| Target | Poison | Crash-clean? | OSS-Fuzz readiness |
|---|---|---|---|
| `program_keyed` | on | **yes** (F2.6: 1500 clean) | **ready** |
| `program_source` | **off** | **yes** (F1.3 replay green) | **ready** if shipped poison-off |
| `program_source` | on | **no** — F1-2 aborts on `walk.loft` | **gated on @PLN85** (store-UAF class) |
| `program_ownership` | on | assumed (shipped + triaged under @PLN85) | re-confirm before submit |

## Recommendation for F5.1 (the onboarding shape)

1. **Submit `program_keyed` (poison on) and `program_source` first**, but flip
   `program_source` to **poison-off** for the OSS-Fuzz build (front-end
   robustness — its actual remit), OR gate its poison-on variant behind @PLN85.
   Do **not** submit `program_source` poison-on while F1-2 is open.
2. **Seed corpus**: reuse `fuzz/seed_program_source.sh` (the ~1300 `.loft`
   files) for `program_source`; `program_keyed` is structure-generated (no seed
   needed, but a few `arbitrary` byte seeds help libfuzzer bootstrap).
3. **Dictionary**: a loft-keyword `.dict` (keywords, `hash`/`sorted`/`index`,
   operators) sharpens the mutator for `program_source`.
4. **Skeleton** (F5.1): `oss-fuzz/projects/loft/` — `project.yaml` (language
   `rust`, the fuzz build), `Dockerfile`, `build.sh` running `cargo +nightly
   fuzz build` and copying the target binaries. Build locally with
   `infra/helper.py build_fuzzers loft` (needs Docker).

## Gate verdict

**NOT yet a green light — two conditions remain:**

1. **A sustained coverage-guided run.** The in-process sweeps are strong but are
   not a real libfuzzer run. Install `cargo install cargo-fuzz` + a nightly
   toolchain and run `cargo +nightly fuzz run program_keyed` /
   `program_source` for a window with no new crash. (This is the F1.4 / F2.5
   "-run" residual — F5 depends on it.)
2. **Resolve the poison policy for `program_source`** given F1-2 (ship
   poison-off, or wait for @PLN85).

`program_keyed` (poison on) is the one target that clears the gate on today's
evidence. F5.1 may proceed for it once a sustained run confirms; the rest is
gated as above.
