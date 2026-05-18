<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN31 — W1.1 — Single-file HTML Export (CLOSED)

**Reference for the shipped pipeline moved to
[`doc/claude/HTML_EXPORT.md`](../../../HTML_EXPORT.md)**
on 2026-05-09 per the closure rule.

This file is a closure record only.

## Status

**SHIPPED.**  All 10 implementation steps landed in production.

Evidence:
- `loft --html program.loft` — CLI flag in `src/main.rs`.
- `make game` — produces `doc/brick-buster.html`.
- `make wasm-html-test` — E2E gate via `tests/html_wasm.rs`
  (460 lines) drives the export pipeline against fixture
  programs through headless chromium.
- `/usr/local/share/loft/wasm32-unknown-unknown/libloft.rlib`
  — installed by `make install` per Step 1.
- `wasm-opt` integration: see CHANGELOG_TECHNICAL.md
  "loft --html switched to wasm-opt -O1".
- CHANGELOG.md notes "`loft --html program.loft` produces a
  single folder you can drop on a static server".

## Step-by-step build sequence (historical)

The 10-step build sequence used to ship W1.1 is preserved
in git history (commits prior to the 2026-05-09 plan
promotion).  For the **shipped pipeline reference** — how
each piece works today — see
[`HTML_EXPORT.md`](../../../HTML_EXPORT.md).

For archaeology — "how was this built" — `git log
src/main.rs Makefile tests/html_wasm.rs` between 0.8.0
and 0.8.4.  Each step landed in its own commit per the
plan's "commit after each green step" discipline.

The 10 steps were:

1. Build libloft.rlib for `wasm32-unknown-unknown`.
2. Compile a trivial program to browser WASM.
3. cdylib entry-point codegen.
4. Minimal HTML loader (no GL).
5. Route `println` through WASM import.
6. GL functions as WASM imports.
7. Frame yield for game loops.
8. HTML assembly with GL bridge.
9. `wasm-opt` integration.
10. End-to-end test.

## Subsequent W1.x work

W1.x continued past W1.1:
- W1.15 — function references in WASM (CallRef).
- W1.16 — file I/O wired to VirtFS.
- W1.17 — store locks.
- W1.18-1..5 — Node.js Worker Thread infrastructure for
  parallel `par()` (W1.18-6 test enablement still open).
- W1.19 — random bridge.
- W1.20 — time bridge.
- Frame yield refinements.

All shipped (W1.18-6 excepted).  See
[WASM.md](../../../WASM.md) for the runtime reference.

## See also

- [`doc/claude/HTML_EXPORT.md`](../../../HTML_EXPORT.md) —
  shipped pipeline reference (where you should be reading
  if you want to know how the HTML export works today)
- [WASM.md](../../../WASM.md) — broader WASM runtime
- CHANGELOG_TECHNICAL.md — W1.x history
