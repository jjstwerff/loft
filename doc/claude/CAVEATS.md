
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

### P135 / C58 — Canvas Y direction is not locked in
Three compensating flips (upload row-reverse, UV, 2D projection) that
don't cancel on non-square atlases.  **Decision:** canonical `(0, 0) =
screen-top-left`, `y` grows down — matches HTML canvas, PNG files, and
how users think about 2D drawing.  The 3D pipeline's internal OpenGL
texture-coordinate math stays internal.  Lock this as a language-level
guarantee in LOFT.md so future backends cannot drift.  Rebake Brick
Buster's atlas (the only loft program with a non-trivial layout).
**Test:** extend `snap_smoke.sh` with a 2×2 atlas corner check.

---

## Scheduled — 0.9.0

### C54 — `integer` representation
Silently returning null when arithmetic lands on `i32::MIN` (and
debug-aborting) is actively hostile in a language pitched as "reads
like Python".  **Decision:** switch `integer` from `i32` to `i64`.
At 8 bytes per field the overhead is rounding error on today's
machines; `i64::MIN` is `-9.2e18`, so accidental sentinel collisions
effectively vanish.  `long` becomes a historical alias; future code
writes `integer` and Just Works for any arithmetic.  Breaking change,
so 0.9.0 is the right window (pre-1.0 stability contract).
Bumping struct field sizes requires revisiting `size(Type::Integer)`
in `src/database/mod.rs` and every schema layout test.

### C60 — Hash iteration
A collection type you can't iterate breaks the "vector, hash, sorted,
index" promise.  **Decision:** `for (k, v) in hash` returns `(K, V)`
tuples in **unspecified order**.  Insertion-order iteration (Python)
would cost an extra vector per hash — wrong trade for a "fast lookup"
collection.  Users who need ordered iteration pair the hash with a
`vector<K>` (the current workaround, now explicit and documented).
Tuples shipped already, so the API shape is natural.

### ~~C61.local~~ — Outer-local shadow — DONE
`x = 5; for x in …` now rejected on pass 1 via the `was_loop_var`
flag on `Variable` — a slot that exists in `names` but has never
served as a loop variable is unambiguously a plain local, so the
shadow is flagged with a rename-or-drop hint.  Same-typed shadow
only (the existing type-mismatch check handles the different-typed
class with a clearer message).  Sequential same-name loops stay
legal because the prior slot carries `was_loop_var = true`.

Unblocked by PROBLEMS.md #139's `OpReserveFrame` fix, which made the
stdlib rename sweep possible without tripping the slot-allocator
TOS assertion.

**Tests:** `tests/parse_errors.rs::c61_local_shadow_rejected`,
`c61_local_shadow_renamed_ok`, `c61_local_dropped_outer_ok`, plus
the flipped-to-reject `shadow_same_type_ok`.
**Files cleaned up:** `lib/graphics/src/mesh.loft` (dropped dead
`row = 0; col = 0` inits), `lib/parser.loft` (renamed `p` / `f` →
`param` / `fld`), `tests/docs/01-keywords.loft` (renamed `for a`
→ `for i`), `tests/scripts/05-enums.loft` (two loops renamed),
`tests/scripts/39-diagnostics-passing.loft` (flipped the
once-permissive test), `lib/graphics/examples/25-brick-buster.loft`
(renamed `br_rt` → `br_pti`).

### P91 — Default-from-earlier-parameter
`fn make_rect(w: integer, h: integer = w)` is an idiomatic default.
**Semantics decision:** the default expression is evaluated at
*function entry*, after earlier arguments are bound, using their
actual call-site values.  That is different from struct-field
`init(expr)`, which evaluates once at construction.  Required shape:
parse default as a `Value` tree (already stored in `Attribute.value`);
codegen emits a prologue guarded by "was this argument supplied?"
using a per-call supplied-args bitmap.  **Scope honestly: M.**
Three moving parts (parser, call site, prologue) all need updates.

### P54 — `json_items` returns opaque `vector<text>`
The typeless API contradicts loft's type-system promise, but the full
`JsonValue` enum (Object / Array / String / Number / Boolean / Null)
is a large design surface for a problem that's mostly "distinguish
'valid JSON body' from 'garbage' at the type level".  **Decision
(revised):** introduce a typed newtype `JsonBody` that wraps `text`,
returned by `json_items` and accepted only by `MyStruct.parse`.  Adds
`.is_object() / .is_array() / .is_null()` for cheap shape checks.
Dynamic shape-unknown access (`v["users"][0]["name"]`) is deferred to
1.1+ when someone asks for it with a concrete use case.  80% of the
type-safety gain for 20% of the design surface.

### ~~C7 / P22~~ — `spacial<T>` diagnostic — DONE
Diagnostic updated to surface the 1.1+ timeline: *"spacial<T> is
planned for 1.1+; until then use sorted<T> or index<T> for ordered
lookups"*.  A user who typed `spacial` now knows when the feature
ships and which substitute to reach for.  Keyword retained (more
helpful than a generic "unknown type" would be).  **Tests:**
`tests/parse_errors.rs::spacial_not_implemented`,
`spacial_not_implemented_in_local` (new regression guard for the
local-variable path).

---

## Verification log

Last retested: **2026-04-12** against commit `2aaba5a` (main branch).

| Caveat | Milestone | Decision |
|--------|-----------|----------|
| C3     | 1.1+      | Accepted — WASM threading deferred (Web Worker pool cost > benefit today) |
| ~~C7/P22~~ | — | **Done** — diagnostic now references 1.1+ timeline; regression guard added |
| C38    | —         | Accepted — value-semantic capture by design (like Rust `move`) |
| C54    | 0.9.0     | **Switch `integer` from i32 to i64.** `long` becomes historical alias. Breaking change, pre-1.0 window |
| C58/P135 | 0.8.5   | Canonical `(0, 0) = screen-top-left`; lock in LOFT.md; re-bake brick-buster atlas |
| C60    | 0.9.0     | `for (k, v) in hash` returning `(K, V)` tuples in unspecified order |
| ~~C61.local~~ | — | **Done** — pass-1 reject via `was_loop_var`; stdlib docs cleaned up; unblocked by #139 |
| P54    | 0.9.0     | `JsonBody` newtype + `.is_object/array/null()`; full `JsonValue` deferred to 1.1+ |
| P91    | 0.9.0     | Default evaluated at function entry via prologue; call-site supplied-args bitmap. Scope M |
| P137   | 0.8.5     | Browser WASM `unreachable` panic fix |

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
