<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 0 — Tooling decision (Miri vs ASan vs both)

Stage A1 deliverable.  Decision: **both**, with disjoint roles.
The spike below shows the two detectors catch *disjoint* UB
classes — running only one leaves a real, demonstrated blind spot.

## Spike setup

- **Host:** Linux, **rustc 1.95.0** stable (the box where loft is
  developed).  This matters: 1.95 *masks* the UB the lever targets
  (the @P383 family surfaced on rustc 1.96 / macOS).  Nightly
  **1.98.0-nightly (2026-05-28)** + `miri` + `rust-src` are
  installed.
- **Probe:** a minimal cluster-I-shaped program
  (`/tmp/p53/clusterI.loft`): `a = h.items[0] ?? "fallback"` on a
  `vector<text>`.  On stable interpret it prints the correct
  `a=present` — i.e. the UB is masked here, by construction.
- **Miri:** `cargo +nightly miri test --test issues <one test>`
  with `MIRIFLAGS=-Zmiri-disable-isolation` (for the `default/*.loft`
  file reads).  Interpreter path only.
- **ASan:** `RUSTFLAGS=-Zsanitizer=address cargo +nightly run
  -Zbuild-std --target x86_64-unknown-linux-gnu --bin loft --
  --interpret --path <repo> <probe>`, `ASAN_OPTIONS=detect_leaks=0`.

## Results

| | Miri | ASan |
|---|---|---|
| **What it caught** | **Cluster 1** — unaligned `&mut u16` in `code_add` (`src/state/mod.rs:1386`), inside `byte_code` | **PLAN52 cluster-I** — `heap-use-after-free` in `copy_nonoverlapping::<u8>` reading the freed `_ncc_N` buffer |
| **What it could NOT catch** | The heap-UAF — Miri **aborts at the compile stage** on cluster 1 before execution begins | The unaligned reference — ASan **does not detect alignment UB** at all; it ran straight past it |
| **Runtime (this spike)** | **minutes** for one test — dominated by `parse_dir("default")` interpreting the whole stdlib under MIR (10–100×) | **sub-second** run after build; the binary executes near-native (~2–3× with instrumentation) |
| **First-build cost** | builds a miri sysroot once (cached `~/.cache/miri`) | `-Zbuild-std` recompiles std with ASan once (heavy, ~minutes, cached in `target/<triple>`) |
| **FFI / native reach** | interpreter only.  Binary path needs `crash_report::install` gated behind `#[cfg(not(miri))]` (`libc::sigemptyset` is unshimmed).  `--native` (rustc-spawn + dlopen) is a structural non-target | full surface — the static `--native` binary (loft runtime rlib + package rlibs, no dlopen) is one cleanly-instrumented image |
| **Detects** | UAF, dangling, uninit, **strict aliasing / Stacked-Borrows, alignment, type-punning** | UAF, heap/stack overflow, use-after-return, double-free |

## The decisive observation

The two detectors caught **different bugs on the same program**,
and *each was blind to the other's bug*:

- Miri found an **alignment-class UB** (cluster 1) that ASan
  cannot see.
- ASan found an **execution-phase heap-UAF** (PLAN52-I) that Miri
  could not reach.

This is not redundancy to trim — it is the empirical justification
for the **Combination** option in the README's matrix.  A
single-detector gate would have a demonstrated hole.

## Sequencing finding (blocks a Miri gate today)

Cluster 1 fires inside `byte_code`, which runs for **every**
program.  Under Miri this masks *all* execution-phase UB — the
entire PLAN52 family included.  Therefore:

1. **A Miri CI gate is not viable until cluster 1 is fixed**
   (`write_unaligned`/`read_unaligned`, ~2-3 sites — see
   `cluster-1-unaligned-bytecode.md`).  Until then Miri red-flags
   on the first compiled program.
2. **ASan is gate-viable now** — it runs past cluster 1 and
   exercises the full surface at affordable cost.

## Decision

| Detector | Role | Surface | When |
|---|---|---|---|
| **ASan** | Primary CI gate | Full suite incl. static `--native` binary | Wireable now (D-final); affordable at ~2–3× |
| **Miri** | Deep gate on a curated, **minimal-fixture** subset | Interpreter only; the alignment / aliasing classes ASan misses | After cluster 1 is fixed; subset must reuse `cached_default()` and avoid per-test full-stdlib reloads |

## Affordability rules for the Miri subset (hard-won here)

- **Never call `parse_dir("default")` per test** — that full-stdlib
  parse under MIR interpretation is what turned one test into
  minutes.  Use `cached_default()` (load once per process) or
  minimal fixtures that pull only the stdlib the probe needs.
- A persistent precompiled-stdlib cache would help generally but is
  **off this plan's critical path** and carries a Miri-provenance
  caveat (mmap-reinterpret is Miri-hostile) — filed as
  [@PLAN54 stdlib-fast-start](../../future/54-stdlib-fast-start/README.md).

## rustc baseline (per close-criterion 5)

- Miri / ASan spike green-or-finding against **nightly 1.98.0
  (2026-05-28)**, host stable **rustc 1.95.0**.  Record the nightly
  pin whenever the gate is re-confirmed so future bumps have a
  known-good anchor.

## Progress (peeling, 2026-05-29)

- **Cluster 1 (unaligned bytecode buffer) — FIXED + Miri-confirmed.**
  Miri now traverses the whole `byte_code` pass and into execute.
- **Cluster 2 (unaligned store accessor) — surfaced by that fix**,
  catalogued, M+/design-sensitive (on-disk-format coupling), not
  started.  This is the peeling pattern: each fix reveals the next
  gating finding.

## Detector lanes beyond Miri/ASan (informational)

- **Valgrind Memcheck** — overlaps ASan for libc-heap errors but
  ~10-50× (slower than ASan's 2-3×) and **misses alignment UB**
  (would NOT have caught cluster 1).  Unique value: uninitialized-
  read detection on the full native binary with **no rebuild**.
  Worth a third informational lane, not a gate.
- **The shared blind spot:** loft's store is a custom arena; its
  internal use-after-free (the @P377/@P378 family) is invisible to
  Miri, ASan, AND Valgrind unless the allocator is annotated.  See
  README § Case-finding strategy lane 3 (`LOFT_POISON` poison-on-
  free) — the homegrown keystone that covers it.

## Open for Stage A2

- Decide cluster 2's (A)/(B)/(C) fix with the @PLAN38 layout owner,
  then re-run Miri to peel the next execution-phase finding.
- Full ASan sweep over `tests/` (not just the one probe) to
  enumerate the rest of the cluster catalogue.
- Stand up the case-finding lanes (README § Case-finding strategy),
  prioritising `LOFT_POISON` (lane 3) for the arena blind spot.
- Decide gating policy (required vs informational) at D-final.
