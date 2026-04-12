
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Caveats

Real edge cases that bite loft programmers today.  Each entry either has
a decided fix (with a milestone) or is an accepted trade-off we intend
to keep.  Entries that merely document shipped diagnostics or internal
compiler details belong in CHANGELOG.md / LOFT.md / SLOTS.md, not here.

**Maintenance rule:** when an entry is fixed, delete it; when it becomes
a design-accepted fact, move it to LOFT.md § Design decisions.

---

## Accepted trade-offs (not scheduled for change)

### C3 — WASM backend: `par()` runs sequentially
A Web Worker pool has real bundle-size and startup cost that most
browser loft programs don't want.  Revisit only when a concrete game
demands it (W1.18 / 1.1+).  **Workaround:** use native for CPU-bound
parallel work.

### C38 — Closure capture is copy-at-definition
Captured values are copied into the closure at definition time, like
Rust `move`.  Mutations to the outer variable after capture are not
visible inside the lambda.  Reference capture would require either GC
or borrow tracking, neither of which fits the "simple, fast, no
lifetime annotations" ethos.  **Test:**
`tests/scripts/56-closures.loft::test_capture_timing`.

---

## Scheduled — 0.8.5

### P137 — `loft --html` Brick Buster: runtime `unreachable` panic
The headline browser build wedges on the first call to `loft_start`;
native mode works.  Blocks the "share a link, anyone plays" story and
the Moros editor ships on the same WASM path.  Fix path: phase-C
bisection of `#native` functions (detailed in PROBLEMS.md #137).

### P135 / C58 — Canvas Y-flip is a three-flip compensation
`loft_gl_upload_canvas` reverses row order, `draw_sprite`'s UV does it
again, and the 2D projection flips a third time — the three don't
cancel on non-1-tall atlases, so the 2×2 (and larger) case off-by-ones
by a row.  **Decision:** normalise to screen-top-left `(0,0)`
throughout — remove the upload-side row reversal and re-bake any loft
program that depended on the previous convention (currently only
Brick Buster's atlas).  **Test:** extend `snap_smoke.sh` with a 2×2
atlas corner check.

---

## Scheduled — 0.9.0

### C54 — `i32::MIN` as null sentinel for `integer`
Silently returning null on arithmetic that happens to land on
`-2147483648` (and debug-aborting in that case) is actively hostile in
a language pitched as "reads like Python".  **Decision:** keep `i32`
for `integer` but *warn when `integer` is used for arithmetic likely
to overflow* — teach users to use `long` for wide arithmetic.  Stdlib
indexing continues to use `integer` (1-based counter → small range,
safe).  File a compiler-warning enhancement.

### C60 — Hash iteration
A collection type you can't iterate breaks the "vector, hash, sorted,
index" promise.  **Decision:** implement I13 iterator protocol —
`for (k, v) in hash` returns pairs in unspecified order (matches
Python dict / Rust HashMap).  Users who need ordered iteration pair a
hash with a vector (documented pattern).  Stopgap `hash.keys()` /
`hash.values()` not needed if I13 lands on schedule.

### C61.local — Outer-local silently clobbered by a for-loop
`x = 5; for x in …` silently reassigns `x` to the loop's last value.
The naive "any defined outer local" reject was tried and reverted —
it broke stdlib doc examples that reuse a dead local for iteration.
**Decision:** liveness-aware diagnostic — reject only when the outer
`x` has a live read after the loop.  Infrastructure
(`Variable::was_loop_var` + `Function::was_loop_var`) already landed.
**Pin for the future fix:**
`tests/parse_errors.rs::c61_local_shadow_still_silent_tracked_as_c61_local`.

### P91 — `init(expr)` / default-from-earlier-parameter
`fn make_rect(w: integer, h: integer = w)` is an idiomatic default.
Currently fails with *"Unknown variable 'w'"* because earlier
arguments aren't yet bound when the default expression parses.
**Decision:** extend `parse_arguments` to inject each parsed argument
into `self.vars` before parsing the next default (tried once; hit
two-pass-parser interactions — needs a careful second attempt).

### P54 — `json_items` returns opaque `vector<text>`
`vector<text>` where each element is "either a JSON object body or
junk" is exactly the kind of typeless API loft rejects elsewhere.
**Decision:** ship the `JsonValue` enum (Object / Array / String /
Number / Boolean / Null) in 0.9.0 (promoted from 1.1+).  It's the
difference between "language has JSON" and "language has a JSON
escape hatch".

### C7 / P22 — `spacial<T>` keyword reserved but unimplemented
Claiming the namespace while always erroring is user-hostile and
misleading.  **Decision:** remove the keyword entirely for 0.9.0
(treat `spacial<T>` as a plain unknown-type error) and re-add when A4
actually implements the radix/R-tree backing.  Existing tests for the
"not implemented" error flip to "unknown type".

---

## Verification log

Last retested: **2026-04-12** against commit `2aaba5a` (main branch).

| Caveat | Milestone | Status |
|--------|-----------|--------|
| C3     | 1.1+      | Accepted — WASM threading deferred |
| C7/P22 | 0.9.0     | Remove keyword until implemented |
| C38    | —         | Accepted — value-semantic capture by design |
| C54    | 0.9.0     | Warn on likely-overflowing `integer` arithmetic |
| C58/P135 | 0.8.5   | Normalise to screen-top-left; re-bake brick-buster atlas |
| C60    | 0.9.0     | Implement I13 iterator protocol for hash |
| C61.local | 0.9.0  | Liveness-aware diagnostic (infra landed) |
| P54    | 0.9.0     | `JsonValue` enum (promoted from 1.1+) |
| P91    | 0.9.0     | Earlier-param-ref in default expressions |
| P137   | 0.8.5     | Browser WASM unreachable panic |

---

## Moved out of this document

- **C12** (null + `??` instead of exceptions) → design fact, see LOFT.md
- **C45** (zone-2 slot reuse text-only) → internal allocator detail, see SLOTS.md
- **C56, C57** (clean diagnostics for stdlib-name clash / nested file-scope decls)
  → shipped in 0.8.4, see CHANGELOG.md
- **C51, C53, C55, C61-nested** → fixed and deleted
- **P55** (thread-local `http_status`) → design reject, not an open item
- **P90** (per-call HashMap lookup) → premature optimisation, see PERFORMANCE.md

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker (severity, fix paths)
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT.md](LOFT.md) § Design decisions — accepted language-level trade-offs
