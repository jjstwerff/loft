<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — Unaligned typed access to the byte-packed eval stack

**Detector:** Miri (`cargo +nightly miri test`).
**Surfaced:** 2026-05-29, immediately after cluster 1 was fixed —
removing the `code_add` abort let Miri reach execution.
**Status:** mechanism VERIFIED; root cause pinned (user-confirmed:
record fields ARE aligned, the **stack** is not; owned `String`
locals live on the stack and can't be value-copied).  **Fix path
chosen: FULL STACK ALIGNMENT** (user, 2026-05-29) — round every
stack footprint up to 8 so `stack_pos` stays 8-aligned and all
`addr`/`addr_mut` on the stack are aligned.  The earlier (B)
attempt was reverted (couldn't cover owned `String`).
Implementation in progress.  Disjoint from PLAN52's surface.

## Root cause (corrected — NOT a record-field-layout bug)

An earlier draft of this doc blamed record field layout and feared
an on-disk-format change.  **That was wrong.**  Record fields are
8-byte aligned by design (and the store base is 8-aligned —
`Layout::from_size_align(size*8, 8)`, `src/store.rs:281`).  The
unaligned access is on the **eval stack**, which is byte-packed:

- `State::set_string` writes a `Str` via
  `addr_mut::<Str>(stack_cur.rec, stack_cur.pos + stack_pos)`
  (`src/state/text.rs:27-30`) — i.e. onto the stack at the current
  `stack_pos`.
- `stack_pos` advances by each pushed value's raw byte size
  (`self.stack_pos += size`, no alignment padding — `src/state/mod.rs`
  stack primitives), so a value can land at any byte offset.
- `Str` is `{ ptr: *const u8, len: u32 }` → `align_of` = 8.  At a
  non-8-aligned `stack_pos`, `addr_mut::<Str>` constructs an
  unaligned `&mut Str` → UB.

The same applies to any ≥2-byte-aligned type pushed to the stack
(`DbRef`, `i64`, `f64`, `u16`, …).  This is the **same shape as
cluster 1** (the bytecode buffer): a *byte-packed* region accessed
through typed references.  The store's *record* region is the one
aligned area; the two byte-packed regions — bytecode buffer
(cluster 1) and eval stack (cluster 2) — are where the UB lives.

## Severity

| Axis | Rating |
|---|---|
| Corruption / panic / hang | Latent UB (unaligned reference); masked on x86-64 rustc 1.95.  Fires on any program that pushes a `Str`/`DbRef`/8-byte value to a non-aligned stack offset — i.e. essentially every non-trivial program. |
| Leak | none |

## Backend asymmetry

Interpreter (the eval stack is interpreter runtime; the
`--native` backend does not use this byte-packed eval stack).
Confirmed via the interpreter Miri run.

## Verified mechanism

| Statement | Status | Evidence |
|---|---|---|
| `Store::addr`/`addr_mut::<T>` return `&T`/`&mut T` at a byte offset into the store buffer | ✅ VERIFIED | `src/store.rs:1322-1325` / `:1364-1367` |
| The failing call writes a `Str` to the **stack** (`stack_cur.pos + stack_pos`), not a record field | ✅ VERIFIED | `src/state/text.rs:27-30` (`set_string`) |
| `stack_pos` is byte-packed (advances by raw value size, no alignment) | ✅ VERIFIED | `self.stack_pos += size` in the stack primitives, `src/state/mod.rs` |
| `Str` is 8-byte aligned (`ptr: *const u8`) | ✅ VERIFIED | `src/keys.rs:49` |
| Record fields + store base ARE aligned (so records are NOT the bug) | ✅ VERIFIED (user-confirmed) | `Layout::from_size_align(size*8, 8)` `src/store.rs:281`; field layout aligned by design |

## Fix path — CHOSEN: full stack alignment

**Decision (user, 2026-05-29): full alignment.**  The earlier (B)
attempt (unaligned accessors) was **reverted** — it cannot cover the
owned `String` stack locals (a `std::String` owns its heap
allocation and can't be value-copied/`read_unaligned`'d; only
*slices* are `Str`), and the owned `String` is reached through the
*drifting* `stack_pos` (`stack_cur.pos + stack_pos - pos`), so
selectively aligning just its slot is impossible without aligning
the whole addressing chain — at which point you've done full
alignment anyway.  One uniform rule beats a special case fighting
the slot allocator.

### Design — reuse the field/tuple alignment mechanics

**Detailed design: [`cluster-2-fix-design.md`](cluster-2-fix-design.md).**

Refined from the first "blunt round-to-8" spike: instead of a new
rounding, **reuse loft's existing field/tuple alignment machinery**
(`element_offsets` / `element_align` / `calculate_positions_with_groups`
/ `group_size`) to lay out the stack, feeding it the **context-correct
text type** — `String` (align 8) in stack context, `Str` in field
context.  `size(tp, Context::Variable)` already context-splits text
*size*; extend the same split to *alignment*.  Then every stack
`addr`/`addr_mut::<T>` is aligned on all targets (incl. RISC-V),
`String` included, and the hard-coded `pos - N` offsets get *derived*
from the mechanic rather than hand-maintained.  (The blunt round-to-8
spike currently in the tree is superseded — revert before
implementing.)

**Two size notions to change (they agree by construction today):**

1. **loft `size(tp, Context::Variable)`** (`src/variables/mod.rs:1462`)
   — the slot allocator's per-type frame footprint.  Round the
   result up to a multiple of 8 (1→8, 4→8, 12→16, 20→24; 8/16/24
   unchanged).
2. **Rust `size_of::<T>()` at the eval-stack push/pop** —
   `get_stack`/`put_stack`/`get`/`put`/`get_var`/`put_var` advance
   `stack_pos` by `size_of::<T>()`.  Route through an
   `aligned_stack_size::<T>() = (size_of::<T>() + 7) & !7` helper so
   they advance in 8-aligned steps consistently with (1).

**Blast radius (the real work):**

- **`size()` `Context::Constant` cases must NOT round** (lines
  1464-1465) — those are bytecode operand widths (cluster-1 /
  byte-packed bytecode buffer), nothing to do with the stack.
- **Hard-coded `pos - N` offset arithmetic** in `fill.rs` / `text.rs`
  (`string_mut(pos - 4)`, `pos - 16/20/24`, etc.) is byte-exact
  today and is **not** compiler-checked — it must be recomputed for
  the rounded sizes, or it silently corrupts.  This is the part that
  needs test + Miri validation, not just a green build.
- **Codegen** (`src/state/codegen.rs`) must emit slot offsets / push
  widths from the *same* rounded sizes the runtime uses — codegen
  and runtime share `size()`, so rounding there propagates, but any
  codegen site using a raw width needs the rounded one.
- **`validate_slots`** should assert 8-alignment of every slot once
  the rule is in.
- **Cost:** small stack-footprint growth (sub-8 values padded to 8;
  `DbRef` 12→16) + marginally larger frame copies.  No per-access
  cost; aligned access is the fast path.  See `cluster-2-fix-design.md`
  for the worked padding/perf analysis.

**Out of scope:** cluster 1 (bytecode buffer) stays on
`read_unaligned`/`write_unaligned` — opcodes are legitimately
byte-packed and must not be padded.

### History — (A)/(B) options considered

The reverted (B) design + the original (A)/(B) trade-off remain
below and in [`cluster-2-fix-design.md`](cluster-2-fix-design.md)
for the record.  Full alignment is essentially (A) generalised from
"align named slots" to "align the whole stack", which is what the
owned-`String`-via-drifting-`stack_pos` analysis showed is actually
required.

Rationale: (B) is quick and localized (~4-6 funnel helpers, all
whole-value read/write, no stack RMW), uses the proven cluster-1
idiom, keeps clear of the fragile slot allocator, and unblocks the
Miri gate immediately.  Its only cost — slow on strict-alignment
targets — is deferred: loft's shipping targets (x86-64 / AArch64 /
wasm) all tolerate unaligned access, and a RISC-V SBC would more
likely run loft via cross-compiled `--native` (no byte-packed eval
stack) than via the interpreter.

**Seam requirement (makes the future pivot a single-site flip).**
Route all typed stack access through named helpers — e.g.
`stack_get::<T>` / `stack_set::<T>` — whose bodies use
`read_unaligned` / `write_unaligned` today.  Then the pivot to (A)
is exactly: align the slots in the positioning code **and** flip
those two helper bodies back to aligned/reference access — the call
sites never move.  (The flip is required because `read_unaligned`
keeps emitting byte-reassembly on strict targets even when the
runtime address is aligned — the compiler can't know — so aligning
the data alone doesn't recover RISC-V perf without also dropping the
intrinsic.)

**Pivot trigger.**  Revisit (A) when loft gains a strict-alignment
**interpreter** target (RISC-V Linux SBC interpreter build), or when
the slot allocator is being reworked for another reason anyway.

---

### Option detail (both retained)

Everything on the eval stack is `Copy` (`Str`, `DbRef`,
primitives — owned `String`s live in *records*, not the stack), so
the `&mut String`-in-place problem does **not** apply here.

- **(A) Align the stack at variable-positioning time (future pivot).**
  The slot/variable-positioning code is the **single source of
  truth** for stack byte offsets — the bytecode and `addr`/`addr_mut`
  merely consume whatever offsets it emits.  So rounding each typed
  slot up to `align_of::<T>()` *there* makes every stack value land
  aligned and the existing reference accessors valid everywhere,
  with **no change to `addr`/`addr_mut`, no `read_unaligned`
  scattering, and no format change**.  This is the principled fix:
  the byte-packed stack becomes alignment-packed.
  - **Site:** `src/variables/slots.rs` — `assign_slots` /
    `place_zone2` (the SLOTS.md two-zone allocator that assigns
    `tos` byte positions); and `src/variables/slots_v2.rs`
    `assign_slots_v2`.  Add an `align_up(pos, align_of::<T>())`
    step where a slot's byte position is chosen.
  - **Interactions to handle:** slot *reuse* across disjoint
    lifetimes (`find_reusable_zone2_slot`) and the `validate_slots`
    invariants (`src/variables/validate.rs`) — an aligned slot must
    stay aligned when reused, and the validator should assert
    alignment.  Modest, self-contained; cost is a little stack
    padding.
- **(B) Unaligned typed stack access (cluster-1 idiom) — CHOSEN.**
  Add `Store` value accessors using `read_unaligned`/`write_unaligned`
  for the stack paths (`set_string` + the typed stack push/pop
  helpers), leaving `addr`/`addr_mut` for aligned record fields.
  Localized and format-neutral.  **Funnel through a named seam**
  (`stack_get`/`stack_set`) per the Seam requirement above so the
  pivot to (A) stays single-site.  The known cost — unaligned access
  spread across the stack accessors, slow on strict-alignment
  targets — is accepted and deferred.
  - **Surface:** the ~4-6 typed (non-`u8`) stack funnel helpers:
    `src/database/mod.rs:1195/1230`, `src/state/mod.rs:1462/1666`,
    `src/state/text.rs:30/63`.  Leave `u8` stack access (alignment 1,
    never unaligned) and the aligned record `addr`/`addr_mut`
    untouched.

**Trade-off analysis (the basis for choosing (B) now / (A) later):**

- **Platform reach.**  `read_unaligned`/`write_unaligned` (B) never
  *fault*, but on strict-alignment targets they lower to byte-wise
  reassembly, and on current RISC-V Linux SBCs (VisionFive/JH7110-
  class) misaligned access is **trap-and-emulated in firmware** —
  hundreds of cycles per access, on the interpreter's hot stack
  path.  loft's present targets (x86-64 / AArch64 / wasm) all
  tolerate unaligned access, so (B) is free *today*; the cost is
  hypothetical-future (RISC-V SBC), which is "effort to port to,
  not off the table" (user, 2026-05-29).
- **(A)** aligns the data so a single fast load works on every
  platform present and future, and avoids rare cache-line-straddle
  on x86 — at the cost of touching the slot allocator +
  `validate_slots` (a known-fragile subsystem).
- **(B)** is smaller (~4-6 funnel helpers, all whole-value
  read/write, no stack RMW) and keeps the slot machinery untouched,
  at the cost of being slow on strict-alignment targets loft
  doesn't ship to yet.
- **Native cross-compile angle** changes the RISC-V calculus — see
  README discussion: `--native` does NOT use the byte-packed eval
  stack, so the RISC-V *deployment* path may sidestep this entirely,
  weakening (A)'s perf argument (the interpreter UB still needs
  fixing for correctness + the Miri gate regardless of which).
- **Bare-metal MCUs / hacker badges** (RP2040/Cortex-M0+, ESP32
  Xtensa — all strict-alignment) do NOT constrain this: loft is a
  `std` program and cannot run on them regardless.  The only
  strict-alignment platform loft could run on is the `std`/Linux-
  capable **RISC-V SBC**.

## Reproducer

```bash
MIRIFLAGS=-Zmiri-disable-isolation \
  cargo +nightly miri test --test issues production_mode_no_error_had_fatal_false
# → Undefined Behavior: unaligned reference (&mut Str) at store.rs:1366,
#   via set_string → string_from_code → const_text → execute
#   (any program that pushes a text/ref value to an unaligned stack slot)
```

Native (rustc 1.95) masks it: programs run clean.

## Next step

Implement (B): introduce the `stack_get`/`stack_set` seam over
`read_unaligned`/`write_unaligned`, route the ~4-6 typed stack
funnel helpers through it (leave `u8` and record accessors alone),
then re-run Miri to confirm cluster 2 clears and peel the next
execution-phase finding (the way cluster 1's fix revealed this one).
Disjoint from PLAN52 — can land off-gate.  Implementation awaits an
explicit go.
