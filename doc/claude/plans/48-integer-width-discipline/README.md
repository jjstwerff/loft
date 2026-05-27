<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 48 — Integer width discipline: `integer` is i64, `i32` is explicit, no implicit narrowing

## Status

In flight, partially landed (uncommitted).  Surfaced re-validating the external
`loft-libs-core/random` package: a native `n_rand(i64,i64)->i64` returning the
`i64::MIN` null sentinel works on `--native` but returns `0` on `--interpret`,
because the interpreter's FFI auto-marshal mapped a plain loft `integer` to
**i32** and truncated the i64 return (filed **@P370**).  The maintainer's rule:
**a plain loft `integer` is 64-bit; the only 4-byte int is an explicit `i32`;
and `integer`→`i32` (data-losing narrowing) must NEVER be implicit — it requires
`as i32`.**  This plan makes the whole toolchain obey that, end to end.

- **P1 (FFI marshal) — DONE, uncommitted.**  `src/extensions.rs` now maps a
  plain `integer` (`forced_size.is_none()`) → `ArgT::I64`; only an explicit
  narrow int (`u8/i8/u16/i16/i32`, which carry `forced_size`) or `Character`
  → `ArgT::I32`.  Fixes @P370.  `--native` was already i64-correct.
- **P2 (compiler enforcement) — NOT STARTED.**  Make implicit `integer`→`i32`
  (and `integer`→narrow) a compile error suggesting `as i32`.  Widening
  (`i32`→`integer`) stays implicit.
- **P3 (library consequences) — IN FLIGHT.**  web/server `#native` decls moved
  `integer`→`i32` (done); the i32 must be carried end-to-end (struct fields +
  wrapper params) so no internal narrowing remains.
- **P4 (@P368 follow-up) — NOT STARTED.**  The divide-by-zero warning still
  fires on a *named-constant* divisor (`const K = 2.0; x / K`).

## Goal

`integer` behaves as a 64-bit value everywhere (stack, fields, arithmetic, FFI);
narrowing it to `i32`/`u8`/… always requires an explicit `as`, so no conversion
silently loses data and no backend disagrees on width.

## The rule (design)

This is the existing conversion-mode framework ([INCONSISTENCIES.md #17](../../../INCONSISTENCIES.md),
[LOFT.md § Type-conversion rules](../../../LOFT.md)) applied to integer widths.
Rule of thumb already in LOFT.md: *infallible/widening = implicit; fallible/
narrowing = explicit `as`.*  The integer-width rows that follow from it:

| From → To | Mode | Why |
|---|---|---|
| `i32`/narrow → `integer` | **Implicit** | widening, lossless (already works) |
| `integer` → `i32` | **Explicit `as i32`** | NARROWING — loses the high 32 bits (currently wrongly implicit — the bug) |
| `integer` → `u8`/`u16`/`i8`/`i16` | **Explicit `as`** | narrowing |
| FFI: native fn declares `integer` | marshals **I64** | impl must be i64 (P1) |
| FFI: native fn declares `i32` | marshals **I32** | impl is i32; marshal widens the C-i32 return into the 8-byte slot |

LOFT.md's conversion table has no `integer → i32` row today; P2 adds it as
Explicit `as` and the typer enforces it.

## Sub-arcs

| Item | Status | Files |
|---|---|---|
| **P1** — FFI marshal: plain `integer` → I64 | **Done (uncommitted)** | `src/extensions.rs` (arg + return ArgT inference, `forced_size.is_none()` gate) — fixes @P370 |
| **P2** — compiler: implicit `integer`→`i32` is an error (suggest `as i32`); add the LOFT.md table row | Not started | the typer's `convert`/`can_convert`/assignment + arg coercion (`src/parser/`); LOFT.md; INCONSISTENCIES.md; `inc17_*` + new `inc_*` regression tests |
| **P3** — libraries: i32 end-to-end for the 4-byte values | In flight | `lib/web/src/web.loft` + `lib/server/src/server.loft` (`#native` decls → i32: DONE), their public struct fields (`HttpSession.id`, `WsHandler.id`, `WebSocket`/`Server` handles) + wrapper params (`client_id`, `status`, …) → i32, plus explicit `as i32` at any genuine `integer`→i32 feed |
| **P4** — @P368 const-divisor | Not started | `src/parser/operators.rs::is_easy_proof` — const-fold a named-constant divisor via `src/const_eval.rs` so `const K = 2.0; x / K` doesn't warn |
| _(related)_ @P370 | Fixed by P1 | the interpret/native integer-ABI divergence that motivated this plan |

## Why i32-on-exit was the right call (P3 rationale)

Returns are the **safe** direction: an `i32` return widens to `integer` implicitly
and losslessly, so a native fn declaring `i32` flows into any `integer` context
with no cast (`status = http_do(...)` → `HttpResponse.status: integer`).  Entries
are the lossy direction: an `i32` param fed an `integer` value narrows — so the
fix is to carry i32 end-to-end (struct fields + wrapper params hold the same
4-byte handle/status), making the internal flow `i32`→`i32` (no coercion) and
only widening (`i32`→`integer`) when *user* code drops a handle into an integer.
Only values genuinely bounded to 4 bytes use `i32` (HTTP status, slot handles,
opcodes, byte values, counts); anything that can exceed i32 stays `integer`.

## Phase ordering

1. **P1** (done) — unblocks the external `random` package on both backends and
   stops the silent i64→i32 truncation.  Commit with P3 (they're coupled: P1
   broke the web/server libs until their decls moved to i32).
2. **P3** — finish i32 end-to-end so the libs are self-consistent BEFORE P2 turns
   implicit narrowing into an error (otherwise P2 flags the lib wrappers).
3. **P2** — enforce in the compiler; fix the resulting fallout across stdlib /
   lib / tests (add `as i32` where narrowing is intended).  Largest blast radius
   — scope it first (grep the typer's integer→i32 coercion sites + count breaks).
4. **P4** — independent; land any time.

## Open questions

1. **P3 shape — i32 end-to-end vs. all-i64.**  Option (b) chosen (explicit i32
   for 4-byte values); the alternative (a) keeps everything `integer` and moves
   the ~30 web/server Rust impls to i64 — zero i32, zero narrowing, but more
   `.rs` churn + new I64 dispatch arms.  (b) is in flight; revisit if the
   end-to-end i32 surface (public struct fields + wrapper params + scattered
   `as i32`) proves larger than (a).
2. **P2 blast radius.**  How much existing loft (stdlib `default/*.loft`, `lib/*`,
   `tests/`) relies on implicit `integer`→`i32` today?  Must be measured before
   enforcing, so the fallout (`as i32` insertions) is bounded.
3. **P2 narrow aliases.**  Does `integer`→`u8`/`u16`/`i8`/`i16` get the same
   explicit-`as` treatment, or only `i32`?  (The rule-of-thumb says all
   narrowings; confirm no stdlib pattern depends on implicit byte-narrowing.)

## See also

- [INCONSISTENCIES.md #17](../../../INCONSISTENCIES.md) — the implicit/format/explicit
  conversion-mode framework this plan extends.
- [LOFT.md § Type-conversion rules](../../../LOFT.md) — the conversion table P2 adds the
  `integer → i32` row to; also § `integer` (64-bit end-to-end) and the null-sentinel table.
- [PROBLEMS.md](../../../PROBLEMS.md) — @P370 (fixed by P1), @P368 (partial; P4).
- `src/extensions.rs` (FFI auto-marshal), `src/parser/operators.rs` (@P368),
  `lib/web/src/web.loft` + `lib/server/src/server.loft` (P3).
- External: `loft-lang/loft-libs-core/random` (the consumer that surfaced @P370;
  its i64 `n_rand` impl is the *correct* model).
