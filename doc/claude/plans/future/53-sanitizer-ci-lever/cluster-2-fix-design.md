<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — Detailed fix design: full stack alignment via the shared field-layout mechanics

Chosen approach (user, 2026-05-29), superseding the reverted (B)
unaligned-accessor attempt.  See
[`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md)
for the decision + history.

**Status:** design — NOT implemented.  A first blunt "round every
`size()` to 8" spike is in the working tree and is **superseded by
this design — revert it first** (it builds but corrupts text
opcodes; it ignores the reusable machinery below).

## Core idea

Don't invent a new rounding.  **Reuse loft's existing
field/tuple alignment mechanics** — `element_offsets` /
`element_align` / `element_size` / `calculate_positions_with_groups`
/ `LinkedFieldGroup::group_size` — to lay out the stack, and feed
them the **context-correct type for text**: `String` (24 B, align 8)
in stack/`Variable` context, `Str` (16 B) in field context.  loft
already context-splits text *size* (`variables::size(tp,
Context::Variable)` → `String`=24, else `Str`=16); this design
extends the **same context-split to alignment**, then routes slot
layout and the currently-hand-rolled stack offsets through the
shared mechanic.  One alignment engine, two text types — which the
String/Str split requires anyway.

## Real sizes + alignment padding — NOT round-to-8 (decided)

**Keep every type's real size; achieve alignment by inserting
*padding between* values, never by inflating a value's own size.**
(User, 2026-05-29.)  Rationale: booleans / small enums (1 byte) are
*plentiful* on the stack; blunt round-to-8 would burn 7 bytes on
each.  `integer`/`float` are naturally 8 already; `char`/`single`
stay 4; `DbRef` stays 12.  So a `[bool, i64]` frame is
`bool@0` (1 B) → pad → `i64@8`, total 16 — the bool is **not** blown
up to 8.

This is precisely what `element_offsets` (`src/data.rs:1145`) already
does — "pad `pos` up to `align(t)`, place at `pos`, `pos += size(t)`"
— with the real `element_size`.  So:

- **Revert the round-to-8 spike** (it inflates `size()` itself —
  wrong).  `size()` keeps returning real sizes; alignment lives in
  the *positioning*, not the size.
- Per-type alignment (`bool`→1, `char`/`single`/`DbRef`→4,
  `i64`/`float`/`String`(stack)→8) packs small types densely and
  matches how records/tuples already lay out.
- Single source of truth: the hard-coded `pos - N` offsets in
  `text.rs` (the corruption landmine) become **derived** from the
  same mechanic — `N` = the padded span of the popped-arg sequence
  per `element_offsets`, computed, not hand-written.

## Pieces to change

### 1. Make alignment context-aware (mirror `size`)

Add `align(tp, &Context)` next to `variables::size` (or extend
`element_align` to take a context), with the text split:

| type | size (Variable) | align (Variable / stack) | size (field) | align (field) |
|---|---|---|---|---|
| `text` | `String` = 24 | **8** | `Str` = 16 | (record value, § Open) |
| `integer`/`float`/fn | 8 / 20 | 8 | same | 8 |
| ref/vec/hash/… (`DbRef`) | 12 | 4 | 12 | 4 |
| `character`/`single` | 4 | 4 | 4 | 4 |
| `boolean`/small enum | 1 | 1 | 1 | 1 |

The only value that *changes* vs today's `element_align` is
**stack-context text → align 8** (because the stack stores an
align-8 `String`, not the field `Str`).

### 2. Stack frame slot layout → through the mechanic

The slot allocator (`src/variables/slots.rs` `place_zone2` /
`assign_slots`, `slots_v2.rs assign_slots_v2`) computes byte
positions.  Route them through the context-aware `element_offsets`
equivalent so each typed slot is placed at its `align(tp, Variable)`
boundary.  Zone-2 `String` slots land 8-aligned; the existing
two-zone packing/reuse logic keeps working, just on aligned
positions.  `validate_slots` gains an alignment assertion.

### 3. Frame slots (static) vs eval-TOS (dynamic) — two regimes

The user's "real sizes + alignment" cleanly applies to **frame
slots** (named locals — where the booleans/enums accumulate); the
**eval-TOS** (transient expression scratch) has a LIFO subtlety.

- **Frame slots — DONE by reuse, and the gap-filling is the win.**
  Statically laid out by the allocator → route through
  `calculate_positions_with_groups` (`src/calc.rs:125`) / the
  context-aware `element_offsets` (real `size`, per-type `align`
  padding).  That positioner places by alignment and **backfills the
  alignment holes with smaller values** — so the alignment padding an
  `i64`/`String` forces gets filled by the *plentiful* `bool`/enum
  locals instead of wasted.  Per-type alignment is therefore close to
  free on frame slots, which is exactly the "lots of booleans" case.
  `bool`@1 packs into holes, `i64`/`String` land 8-aligned.
  Deterministic; no pop issue.  This is the core cluster-2 fix
  (the owned-`String` `string_mut` access is a frame-slot access).
  Note gap-filling **reorders** slots by alignment — fine for frame
  slots (positions are allocator-internal) but NOT applicable to the
  LIFO eval-TOS below, where push/pop order is fixed.
- **Eval-TOS — the genuine subtlety.**  `get_stack`/`put_stack`/
  `get`/`put` advance `stack_pos` by `size(T)` in LIFO order.
  Per-type alignment-padding here is **not reversible by a simple
  `-size` pop**: push `bool`@0 (→1), push `i64` (pad 1→8, write@8,
  →16); pop `i64` (→8 ✓); pop `bool` (→7 ✗ — reads the pad, the
  bool was at 0).  The leading pad isn't reclaimed.  So the TOS
  needs one of:
  1. **Uniform alignment step on the TOS** (round each push to the
     max align, 8) — symmetric pop, simple.  Costs padding on
     *transient* TOS values only (not the persistent frame slots
     the user cares about) — a small, scratch-lifetime cost.
  2. **Codegen-driven exact offsets** — codegen tracks the precise
     padded position of every value and emits it, so the runtime
     reads at the given offset instead of generic `±size`.  No TOS
     waste, but a larger change to the push/pop protocol.

  **Recommendation: (1) for the TOS, real-size+align for frame
  slots.**  The booleans-are-plentiful concern is a *frame-slot*
  concern (locals persist); TOS scratch padding is transient and
  cheap.  This keeps frame slots tight (per user) while sidestepping
  the LIFO-reversibility trap.  Revisit (2) only if TOS padding
  measurably matters.

### 4. The hard-coded `pos - N` offsets → derived

The `text.rs` format/append opcodes reach the string slot via
`string_mut(pos - N)`, where `N` is the byte-span of the popped
args (e.g. `format_single`: f32+i64+i64; `append_character`: char).
Replace each literal `N` with the span **computed via the same
`element_offsets`/`align` mechanic** over the popped-arg sequence,
so it tracks the layout automatically rather than being
hand-maintained.  The exact deltas depend on the §3 TOS regime
(under uniform-8 TOS: char-span 4→8, f32-format 20→24, f64-format
24 unchanged; `format_long` `pos-16` and the `size_ptr()` text
sites to be confirmed against the chosen regime).  Deriving them
through the mechanic — not re-hardcoding — is the point.

### 5. Codegen consistency (the linchpin)

`src/state/codegen.rs` emits the `pos` operands and slot offsets.
It already calls `variables::size`; make it call the context-aware
`align` for the same placements so codegen and runtime share one
layout. **Verify before editing offsets:** confirm the `pos`
operands for the format opcodes are computed from `size()`/the
mechanic (so they're already in aligned units) — if codegen
computes them another way, step 4's deltas change.

## Implementation steps — independent + verifiable

Ordered; each has a **binary verification** that stands on its own.
The full suite only goes green after S5, but S2–S4 each carry an
intermediate check (slot-trace / Miri-per-region) that confirms *that
step* in isolation, so progress is provable without waiting for the
end.  Revert target throughout: `origin/plan-53-sanitizer-ci-lever`.

| # | Step | Verification (binary) |
|---|---|---|
| **S0** | **Revert the round-to-8 spike.**  Restore `size()` to real sizes; drop the `stack_bytes` round-up edits in `variables/mod.rs`, `database/mod.rs`, `state/mod.rs`. | `git diff origin/plan-53-sanitizer-ci-lever -- src/variables/mod.rs src/database/mod.rs src/state/mod.rs` is **empty**; `cargo build` + `issues` + native suite green (back to checkpoint behaviour). |
| **S1** | **Confirm the codegen linchpin** (read-only).  Determine whether codegen emits the format-op `pos` operands + frame-slot offsets via `variables::size` / the positioner. | Documented finding citing the codegen site, with a `LOFT_LOG` trace of one format op's emitted `pos`.  **Binary:** derives-from-`size()` → proceed; computed-otherwise → re-plan S4/S5.  No code change. |
| **S2** | **Add context-aware `align(tp, &Context)`** (mirrors `size`; text Variable→8, field→`Str`-align).  No callers yet. | New unit tests assert `align(Text,Variable)==8`, `align(Text,field)==4`, `align(Integer,_)==8`, `align(Boolean,_)==1`, `align(Reference,_)==4`; `cargo build` clean; **full suite UNCHANGED/green** (pure addition). |
| **S3** | **Frame slots → gap-filling positioner** (`calculate_positions_with_groups` / context-aware `element_offsets`) using `(size, align)`; add the `validate_slots` alignment assertion. | (a) `validate_slots` asserts every slot at an `align`-multiple — debug-runs clean across the suite.  (b) Slot-trace (`LOFT_LOG=slots:<fn>`) on a fn with `[bool, i64, text-local]` shows the `String` slot offset `%8==0` and the `bool` backfilled into a hole.  (c) **Miri** on `s=""; s+=x` (owned-`String` local) → **no unaligned `&mut String`** at `string_mut`.  (d) interpreter suite green (frame access is internal; outputs unchanged). |
| **S4** | **Eval-TOS regime** (recommended: uniform-8 step in `get_stack`/`put_stack`/`get`/`put`/`put_var`; round frame-top to 8 so the TOS starts aligned). | (a) build + interpreter suite green.  (b) **Miri** on a text-pushing program → the `set_string` `&Str` write at the TOS (the original cluster-2 finding site, `store.rs:1366`) is **no longer unaligned**. |
| **S5** | **Derive the `text.rs` `pos - N` offsets** via the mechanic (per opcode — `append_character`, `format_single`, `format_long`, `append_stack_text`, …); replace each literal. | Per opcode: the `strings` + `format` suites pass; `a8_replace_into_var` passes; each touched opcode has a test exercising its output.  This is the corruption canary — **all text/format tests green**. |
| **S6** | **Full cross-backend + Miri validation.** | interpreter + native suites green **both backends**; `find_problems` full suite green (excl. pre-existing `macos-clippy-fixes` base failures); **Miri clean** on the cluster-2 probe + a text-heavy program (zero unaligned findings); `validate_slots` alignment invariant holds suite-wide. |
| **S7** | **Close.** Graduate a Miri-gated regression probe to `tests/scripts/`; cluster-2 doc Status → fixed + Miri-confirmed; record the rustc/nightly baseline. | Probe runs under both backends + clean under Miri; cluster catalogue row marked ✅; `make ci` green. |

**Independence notes:** S0/S1/S2 are fully independent (revert /
read-only / pure-addition).  S3 and S4 are each independently
*soundness*-verifiable via Miri on their own region (owned-`String`
frame local vs `Str` TOS push) even before S5 restores output
correctness.  S5 is the only step that flips the suite from
"sound but wrong text offsets" to "green".  S6/S7 are the gates.

### S1 result — CONFIRMED (2026-05-29)

Codegen computes the operand as
`var_pos = stack.position − function.stack(v_nr)`
(`src/state/codegen.rs:356 / 465 / 566`), where **both** terms are
`size()`-derived: slot offsets come from the allocator (uses
`size()`), and `stack.position` is advanced by
`size(_, Context::*)` as codegen tracks the eval stack
(`codegen.rs:99`).  The linchpin holds — everything routes through
`size()`.

This also pins down the `pos - N` literals.  A format op pops its
args (`get_stack` ×N) **then** calls `string_mut(pos - N)`; the
`- N` exactly **cancels those pops**, so:

> **`N` = total bytes the op's `get_stack` calls pop.**

Hence the round-to-8 spike's SIGSEGV: `format_single` pops
f32+i64+i64; rounding the f32 pop 4→8 (and `stack.position`) while
leaving the literal `pos - 20` left it off by 4 → wild pointer.

**Implications locked in:**
- Under alignment each `N` becomes `Σ aligned_size(popped arg
  types)` — derivable, not guessed (S5).
- Codegen's `stack.position` and the runtime `stack_pos` MUST
  advance identically, so the chosen aligned-advance rule (uniform-8
  on the TOS) applies in **lockstep** to `size()` / `stack.position`
  (codegen), the `get_stack`/`put_stack` steps (runtime), and the
  `N` literals (S5).  The three move together or corrupt.
- **Proceed to S2.**

### S3 investigation — frame-base coupling (S3 and S4 are NOT independent)

Slot position-assignment sites (where `align`-rounding goes):
zone-1 interval-colouring `candidate` (`slots.rs:232`) and zone-2
sequential `*tos += v_size` (`slots.rs:299 / 365 / 475`); `tos`
starts at `frame_base + zone1_size` (`:254`).

**Frame model (verified):** one stack record (`stack_cur` set once,
`state/mod.rs:192`, at an 8-aligned record base); a single running
`stack_pos`; `reserve_frame` advances it by raw `size` (`:1338`); a
call sets the callee frame base to `args_base = stack_pos −
args_size` (`:287`).  So a frame slot's **absolute** address is
`stack_cur.pos + args_base + slot_offset`.

**Consequence — overturns the design's "S3 independently
Miri-verifiable" claim:** aligning the slot *offset* (S3) only makes
the *address* aligned if `args_base` is a multiple of 8 — i.e. the
running `stack_pos` is kept 8-aligned (**S4 / eval-TOS**) **and**
`args_size` is rounded.  The stack alignment is **holistic**: slot
offsets + eval-TOS advance + args layout + `reserve_frame` must
align **together**, or no single frame slot is reliably aligned.

**Revised plan — merge S3+S4 (+args) into one coupled change:**
"align all stack layout consistently" — frame slots tight per-type
(gap-filled), eval-TOS + args + `reserve_frame` on the uniform-8
advance — validated **together** (Miri on owned-`String` local *and*
`Str` TOS push can only go clean once the frame base is aligned,
which needs the whole set).  S5 (the `N` offsets) then lands, and
the suite goes green.  The independence that survives: S0/S1/S2
stand alone; S3..S5 are one holistic unit with Miri+suite as the
joint gate.

## Out of scope

- **Records on `Str`**: `element_align(Text)=4` while runtime `Str`
  is align 8 → records *may* have the same latent unaligned access,
  not yet Miri-surfaced.  This design keeps records on their
  existing `Str` field layout (per the String/Str split); whether
  record `Str` fields need align bumped to 8 is a **separate
  follow-up** to verify (own cluster if real) — see § Open.
- **Cluster 1** (bytecode buffer) stays on `read_unaligned` —
  opcodes are legitimately byte-packed.

## Validation

1. `cargo build` (interpreter + native).
2. Interpreter suite (`issues`, `wrap`, `strings`, `format`) — text
   opcodes are the corruption canary; all must pass.
3. Native suite — codegen uses the same `size`/`align`, must stay
   green.
4. **Miri** — cluster 2's `&mut Str`/`&mut String` access no longer
   unaligned; peel any next finding.
5. `find_problems` full suite (excl. pre-existing base failures).

## Open questions

1. **Eval-TOS step:** tight per-type vs uniform 8-step (§ step 4\*).
2. **Record `Str` alignment:** latent (b) or unaligned-safe (a)?
   Verify; out of this cluster's scope but related.
3. **Codegen `pos` derivation** (§ 5 linchpin) — confirm first.

## Risk + rollback

Touches the size/align model + slot allocator + the `text.rs`
offset arithmetic + codegen — broad but uniform.  Corruption-based
validation (offsets aren't compiler-checked), so it's test/Miri
driven.  Rollback: `git reset --hard origin/plan-53-sanitizer-ci-lever`
(the pushed checkpoint).
