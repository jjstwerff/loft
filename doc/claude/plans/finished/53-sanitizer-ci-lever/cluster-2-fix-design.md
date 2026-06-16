<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — Detailed fix design: full stack alignment via the shared field-layout mechanics

Chosen approach (user, 2026-05-29), superseding the reverted (B)
unaligned-accessor attempt.  See
[`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md)
for the decision + history.

**Status (2026-05-30):** BOTH HALVES SUBSTANTIALLY DONE.  Frame-slot
half (aligned V2 allocator) complete; EVAL-TOS half (S4) implemented
(E1–E7 + R2 native marshalling + frame-base + working alignment guards
+ a8 + c60/R6) — the aligned suite runs from the start through ~`p117`
with two characterized crashers left.  **The authoritative current
state + how-to-test + the three hard-won process rules are in
[`cluster-2-S4-progress.md`](cluster-2-S4-progress.md) — READ THAT
FIRST.**  The handoff below is the older (2026-05-29) frame-slot-half
snapshot, kept for history.

---

## SESSION HANDOFF (2026-05-29) — read this first

The session that did this work hit a **broken Bash tool** (every
shell command errors after a heavy concurrent test sweep; `Read`/
`Write`/`Edit` still work).  Session is being cleared.  Everything
below is the state for a fresh session to resume.

### Branch / commits
- Branch **`plan-53-sanitizer-ci-lever`**, rebased onto the new
  `main` **`a9bc23fa`** (PLAN52 closed via #230 — the macos-clippy-fixes
  base was dropped; its content is in main).  Pushed to
  `origin/plan-53-sanitizer-ci-lever`.
- HEAD = **`8abfb8e1`** (`feat(@PLAN53 cluster 2, S3): complete the
  aligned V2 allocator`).  Working tree was CLEAN at that commit; only
  this doc edit (the handoff) is uncommitted — commit it once Bash works.
- NO open PR on the branch.  Pushing is OK (branch policy).

### What is DONE (committed + verified)
- **Cluster 1 — unaligned bytecode buffer: FIXED + Miri-confirmed.**
  `code_add`/`code_put`/`code<T>` in `src/state/mod.rs` →
  `write_unaligned`/`read_unaligned`; `code<T>` returns `T` by value;
  241 `*x.code::<T>()` deref sites destarred.  Miri got past `code_add`
  into execute.  (Catalogue: `cluster-1-unaligned-bytecode.md`.)
- **Cluster 2 — FRAME-SLOT half: the aligned V2 allocator is complete
  and validates CLEAN corpus-wide.**  `assign_slots_v2`
  (`src/variables/slots_v2.rs`) is now a scope-blind, kind-aware,
  ALIGNED interval-graph greedy allocator: lowest `align(tp)`-aligned
  slot with no conflict (conflict = overlapping range AND (life-overlap
  OR incompatible kind/size OR loop-straddle OR non-exact partial
  overlap)); **pins argument/SRet slots** as fixed always-live
  intervals so locals never collide.  Validated via the per-function
  `LOFT_SLOT_V2=mode[:filter]` shadow (`v2_mode_for`): `scopes.rs`
  resets slots, applies V2, runs `validate_slots(scope_blind=true)` +
  `validate_alignment`, then restores V1 (validate/report) so
  **execution is unchanged**.  Supporting: `validate_alignment` + I8
  (`check_i8_total_claim`) + `scope_blind` I7-skip in
  `src/variables/validate.rs`; `align(tp)` + `reset_local_slots` in
  `src/variables/mod.rs`; `dump_v1_v2_slots` (report mode).
- **Full-sweep result** (`LOFT_SLOT_V2=validate cargo test --no-fail-fast`):
  **ZERO `[Ix]`/`[ALIGN]` violations corpus-wide.**  The only failures
  were native `cc`-link + `p254_cache_poisoning` panics = the **stale
  `target/release/libloft.rlib`** false-failure (dev-built src vs
  un-rebuilt release rlib), NOT V2 problems.

### What REMAINS for cluster 2 (in order)
1. **Native re-sweep**: `cargo build --release --lib`, then re-run the
   native portion to clear the stale-rlib false-failures (confirms
   native execution unaffected).
2. **S4 — eval-TOS / frame-base alignment (THE LOAD-BEARING OTHER
   HALF).**  `validate_alignment` checks slot offsets RELATIVE to the
   frame; the runtime address is `stack_cur.pos + args_base + slot`
   and `args_base = stack_pos − args_size` DRIFTS, so absolute
   alignment (what the original Miri `Str`-at-TOS finding is about) is
   NOT achieved by frame-slot alignment alone.  S4 = round the eval-TOS
   step to 8 in lockstep (runtime `get`/`put`/`get_stack`/`put_stack`/
   `put_var` AND codegen `stack.position` advances) + `frame_hwm`→8 +
   args→8 + recompute the `text.rs` `pos − N` offsets (N = Σ
   `round8(popped sizes)`).  The hard part: codegen's `bump`/`advance =
   slot_end − stack.position` couples the eval-TOS to slot positions
   (~dozen sites).  A "round-8 everywhere" spike was tried and REVERTED
   in S0 (commit history) — the design here keeps tight slots + a
   round-8 eval-TOS step.  See § 3 + § S1 result for the codegen
   linchpin (pos operand = `stack.position − function.stack(v)`, both
   `size()`-derived).
3. **Drive + Miri**: `LOFT_SLOT_V2=drive:<fn>` to switch one function
   onto V2's layout, run its tests (execution correctness), then Miri
   (`MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --test
   issues <test>`) to confirm the original `Str`-at-TOS UB clears.

### Sanity commands (once Bash works)
- `cargo test --test issues --quiet` → 681/0 (normal, V1 drives).
- `LOFT_SLOT_V2=validate cargo test --test issues --quiet 2>&1 | grep -oE '\[(I[0-9]|ALIGN)\]'` → empty (clean).
- `LOFT_SLOT_V2=report:<fnname> cargo test <test-using-it> -- --nocapture` → V1-vs-V2 slot dump.
- `cargo clippy --lib` → clean.

### Cleanup the inspection agent flagged (LOW, do anytime)
- Remove the now-dead `WalkState` + `walk_node` (the OLD TOS-reset
  allocator) from `slots_v2.rs`; drop the unused `_code` param from
  `assign_slots_v2`.
- `v2_validate_enabled`/`v2_report_enabled` exports are unused (only
  `v2_mode_for` is wired) — remove or add a caller.

### Decision history (so a fresh session doesn't relitigate)
- **Full alignment chosen over (B) unaligned-access**: `read_unaligned`
  doesn't fault but trap-emulates on RISC-V Linux SBCs (a real future
  target); user wants real sizes + gap-fill, NOT blunt round-to-8.
- **Reuse the field/tuple mechanics** (`element_offsets`/`element_align`
  /`calculate_positions_with_groups`) with a context-correct text type
  (`String` stack / `Str` field) — the String/Str split forces it.
- **I7 skipped for V2** (scope-blind); V1 keeps I7.

---

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

---

## Stage B mechanism findings — per sub-cluster (2026-05-30)

After the [`probes/`](probes) suite landed (35 probes, all 27 aligned-mode
failures reproduced), each sub-cluster's minimal reproducer was run through the
`stack_align_guard` release binary
(`cargo build --release --bin loft --features stack_align_guard`, then
`LOFT_ALIGN=1 LOFT_SLOT_V2=drive loft --interpret <probe>`).  The guard fires
at the exact site for a *raw misalignment* and stays silent for a *span-logic*
miscount that happens to land on an 8-aligned boundary — which cleanly forks
the four sub-clusters into two fix shapes:

| Sub-cluster | Guard | Mechanism class | Site | Design state |
|---|---|---|---|---|
| **2a** generator-arg | **FIRES** | raw misalignment (frame drift) | `coroutine_next` (op 252) | **pinned** → Stage C |
| **2b** sorted-iter | silent | span-logic (dead cursor) | `step()` raw deltas (io.rs) | **pinned via trace** → Stage C |
| **2c** hash-iter | silent | span-logic (off-by-one) | `step()` case 3 (shares 2b) | **pinned via trace** → Stage C |
| **2d** composite-format | **FIRES** | raw misalignment (handle read) | `text.rs` 489/499 | **pinned** → Stage C |

(The "span-logic, not yet pinned" prose for 2b/2c just below was the
guard-pass state; the follow-up `LOFT_ITERATE_TRACE` pass pinned both to
`step()`'s raw `state_var - N` deltas — see § Stage C.)

### 2a — `coroutine_next` frame drift (PINNED)

Guard output: `S4 alignment broken: op_code=252 ... stack_pos=84 (not
8-aligned), fn_d_nr=588`.  Op 252 is `coroutine_next` (the RESUME path).  After
resume, `stack_pos` is left at a non-multiple-of-8 because the restore advances
by the RAW saved-frame byte length (`self.stack_pos += bytes.len()`,
src/state/mod.rs ~1043) rather than a stepped length — so the generator's
argument region, which the body then reads at its stepped var offset, sits 4
bytes off (`n=42` → `42<<32`).  **Design seed:** step the resume advance (and/or
ensure the saved `stack_bytes` / `locals_bytes` length is captured stepped in
`coroutine_create` / `coroutine_yield`), so TOS after `coroutine_next` is
8-aligned and the args/locals boundary matches codegen's stepped layout.  This
is the p117 template (value in a stepped slot, advanced by a raw width) applied
to the coroutine frame round-trip.

### 2d — composite handle read at unaligned offset (PINNED)

Guard output: `S4 unaligned stack access: alloc::string::String at abs offset
68 (align 8)`.  The format path (`"{v}"` / `"{v:j}"`) reads the composite
value's `String`/`Str` handle from a stack slot whose offset (68) is computed
at a raw width, not stepped — so under alignment it reads a misaligned handle
and renders empty.  **Design seed:** route the format opcode's value-slot read
through `stack_step` (the same `text.rs` offset arithmetic the fix-design §5
already targets); confirm the `pos-N` derivation for the format read is stepped.

### 2b / 2c — span-logic miscounts (NOT yet pinned)

The guard is SILENT for both: the sorted/hash gather lands on aligned
boundaries but advances the iteration cursor by the wrong *number of elements*
— 2b skips them all (dead cursor → empty), 2c starts one early (phantom
leading element → count+1).  Because there's no misalignment, the guard cannot
localize these; the next step is the wrong-value-diff: instrument the sorted /
hash `OpNext` (and the iterator materialisation / `gather_key` span) under
`LOFT_ALIGN` vs flag-OFF and diff the per-element cursor advance.  Likely a
stepped-vs-raw mismatch in the element stride that, unlike 2a/2d, preserves
alignment while corrupting the count.  **No fix design yet** — site must be
pinned first.

### Summary

Of the four sub-clusters, **2a and 2d have pinned sites + fix-design seeds**
(both are the stepped-slot/raw-width pattern); **2b and 2c have a verified
symptom and mechanism class but no pinned site** — they need one more
investigation pass (cursor-stride diff) before a fix design.

---

## Stage C — fix designs, all four sub-clusters (2026-05-30)

2b/2c were pinned by tracing `LOFT_ITERATE_TRACE` (the `iterate()` setup is
byte-identical aligned vs flag-OFF, so the bug is downstream in the per-element
`step()`), 2a by the frame guard (op 252), 2d by the access guard (`String` at
offset 68).  Two of the four share one root cause.

### 2a — coroutine resume advances TOS by a raw frame length

**Site:** `State::coroutine_next` — `src/state/mod.rs` ~1043
(`self.stack_pos += bytes.len()`), with the saved frame produced by
`coroutine_create` (~865, `stack_bytes = vec![0u8; args_size]`) and
`coroutine_yield` (~1176, `locals_bytes` of length `value_start - base`).

**Mechanism (verified):** the guard fires
`op_code=252 … stack_pos=84 (not 8-aligned)`.  On resume, TOS is advanced by
the RAW byte length of the saved frame.  The generator body then reads its
argument at the codegen-stepped var offset (8-aligned), but the restored TOS /
arg region sits 4 bytes off, so `n=42` reads as `42<<32`.  No-arg generators
have no argument to mis-read (2a-02/03 PASS).

**Fix:** make the saved-frame length and the resume advance agree with
codegen's stepped frame layout.  Two options:

- **(A) recompute TOS from the stepped frame extent** — after the restore
  `copy_nonoverlapping`, set `self.stack_pos = stack_base + <stepped frame
  size>` using the same stepped `local_start` / `generator_zone2_size` math
  already in the file, instead of `+= bytes.len()`.  Preferred — single
  authoritative advance, independent of how the bytes were captured.
- **(B) capture the frame at a stepped length** — pad `stack_bytes` /
  `locals_bytes` to `stack_step(len)` in create/yield so `bytes.len()` is
  already 8-aligned.  Simpler diff but spreads the invariant across three
  functions.

Recommend (A).  Identity when `aligned_stack` is off (`stack_step` = identity),
so flag-OFF is byte-for-byte unchanged.

**Validation:** `probes/run.sh 2a` → all 11 PASS (incl. the HANG/CRASH
variants); frame guard silent; flag-OFF `issues` 681/0.

### 2b — iterator `step()` walks a stepped state block at raw byte deltas

**Site:** `State::step` — `src/state/io.rs` 963–1052.  Lines 968, 989, 997,
1011, 1013, 1017, 1020, 1041 (and the sibling `remove`/reverse paths at
~1092, 1119, 1133, 1135, 1153, 1172) access the iterator-state block at
HARD-CODED raw deltas: `state_var - 4` (finish), `state_var - 8` (next cur),
`state_var - 12` (done flag), `state_var - 16` (reverse finish).

**Mechanism (verified):** `iterate()` writes the state words with
`put_stack(start); put_stack(finish)` (`io.rs` ~934) — under `LOFT_ALIGN` each
`put_stack` advances `stack_step(4) = 8`, so the state words are 8-spaced.
`step()` then reads `finish = get_var(state_var - 4)` — the RAW 4 lands on the
wrong (padding) slot.  In the sorted case (`on&63 == 2`, line 1018) `finish`
reads as a small/zero value, so `pos >= finish` is immediately true →
`pos = i32::MAX` → done before the first element → **every element dropped**.
The `iterate()` bounds themselves are correct (trace: start=MAX, finish=1),
confirming the fault is purely the `step()` delta arithmetic.

**Fix:** replace the raw constants with stepped deltas.  The block is a
sequence of logical 4-byte words each occupying `stack_step(4)` bytes:

```
finish  : state_var - 1*stack_step(4) as u16   // was -4
next cur: state_var - 2*stack_step(4) as u16   // was -8
done    : state_var - 3*stack_step(4) as u16   // was -12
rev fin : state_var - 4*stack_step(4) as u16   // was -16
```

Introduce a small local `let w = self.stack_step(4) as u16;` and express the
offsets as `state_var - w`, `state_var - 2*w`, … .  Identity when off (`w==4`).
Apply uniformly across `step()` AND the `remove`/reverse siblings that share
the layout.

**Validation:** `probes/run.sh 2b` → all 8 PASS; flag-OFF unchanged.

### 2c — hash iteration shares 2b's `step()` (via the scratch rec-nr vector)

**Site:** hash iteration does NOT walk the hash directly: the parser
substitutes a scratch rec-nr vector (`{id}#hash_scratch`,
`src/parser/collections.rs` ~828) and iterates THAT through the SAME
`step()` (`on&63 == 3`, `io.rs` 1032–1047, which also writes
`state_var - 8` raw).

**Mechanism (high-confidence shared root, one open check):** the off-by-one
"phantom leading element" is the case-3 manifestation of the same raw-delta
read — the cursor/finish bookkeeping is shifted by a slot, so the scratch
iteration starts one early.  Because 2b and 2c go through the same function,
the 2b fix above is expected to close 2c as well.

**Fix:** apply the 2b `step()` stepping fix (case 3 inherits the same
`state_var - 2*w` for its `put_var(state_var - 8, …)`).  THEN re-run
`probes/run.sh 2c`.  **Open check:** if a phantom survives, inspect the
`{id}#hash_scratch` BUILD (does the scratch vector itself get a spurious
leading entry under aligned?) and the case-3 start-sentinel (`cur` init) — but
the leading hypothesis is that 2b's fix closes 2c with no extra change.

**Validation:** `probes/run.sh 2c` → all 7 PASS after the shared fix.

### 2d — composite format reads the value handle at a raw offset

**Site:** `src/state/text.rs` — the composite/ref format ops at lines 489 and
499 (`string_mut(pos - 8 - size_ptr() as u16)` /
`string_ref_mut(pos - 8 - size_ptr() …)`), reached when interpolating a
vector / struct / enum value (`"{v}"` / `"{v:j}"`).  The access guard fires
here: `String at abs offset 68 (align 8)`.

**Mechanism (verified misalignment):** SCALAR interpolation works (2d-02
PASSES), so the pure `format_int` (`pos - 16`) path is already correct.  The
COMPOSITE path pops a value + a pointer and addresses the destination/handle at
`pos - 8 - size_ptr()` — a RAW composite of two widths.  Under `LOFT_ALIGN` the
popped value and the pointer each occupy a stepped span, so `8 + size_ptr()`
(=16 raw) under-counts the real popped distance and the `String`/`DbRef` handle
is read off its 8-boundary → misaligned read → empty render.

**Fix:** step the composite-format offsets:
`pos - stack_step(8) - stack_step(size_ptr())` (or the appropriate
`stack_step` of the combined popped layout) at lines 489/499 — mirroring the
already-stepped read at line 146 (`pos - self.stack_step(4)`).  Leave the
scalar `format_int`/`format_float` offsets alone (they pass).  Confirm by
re-arming the access guard: it must fall silent on 2d-01.

**Validation:** `probes/run.sh 2d` → all 9 PASS; access guard silent on the
composite probes.

### Landing order

1. **2b+2c together** — one `step()` stepping commit closes both families
   (15 probes); re-run 2c to confirm the shared fix suffices.
2. **2a** — coroutine_next stepped resume advance (11 probes; clears the
   HANG/CRASH variants — the dangerous family).
3. **2d** — text.rs composite-format stepping (9 probes).

Each is the same one-line-family pattern (raw width where a stepped width
belongs), each gated behind `aligned_stack` so flag-OFF stays byte-identical,
each validated by `probes/run.sh <id>` flipping its aligned column to PASS.
One sub-cluster per commit per the plan's fix-application discipline.

---

## 2b+2c — LANDED 2026-05-30

**Fix (single root cause, NOT the Stage-C "step the deltas" design above — a
read-only investigation refuted that and pinned the bug one slot earlier):**
the iterator state is a single packed I64 var (`{id}#iter_state`, "cur<<32 |
finish", `parser/collections.rs:384`).  `iterate()` produced it as TWO separate
`put_stack::<u32>` pushes (`io.rs` ~934).  Under `LOFT_ALIGN` each `put_stack`
advances a stepped 8-byte slot, so `start@P` and `finish@P+8` with a 4-byte
gap; the consumer's single `get_stack::<i64>()` then reads `[finish | padding]`
and DROPS `start` — killing the sorted cursor (2b) and shifting the hash cursor
(2c, via the `{id}#hash_scratch` vector that walks the same path).

Two-line change, gated by `stack_step` (identity flag-OFF, so byte-identical):

- `src/state/io.rs` ~934 — replace `put_stack(start); put_stack(finish)` with a
  single `put_stack((u64::from(finish) << 32) | u64::from(start))`.
- `src/state/codegen.rs` ~2314 — `OpIterate` `was_stack`:
  `step(4) + step(4)` → `step(8)` (one i64 push, not two u32s).

**Result (verified):**
- `probes/run.sh` exit 0 — all 8 `2b-*` and all 7 `2c-*` aligned columns flip
  FAIL/HANG → PASS; references unchanged; flag-OFF all PASS.
- Aligned `issues` sweep (per-test isolation): **27 → 14 failures**, a STRICT
  SUBSET (zero regressions).  Closed: all of 2b (inc02, inc12×2, p190, p277,
  p295, p300, p4d_b) + all of 2c (c60×4) + `p193`/`2d-09` (a hash-iter case
  mis-grouped into 2d).
- Flag-OFF `issues` 681/0; clippy clean; full flag-OFF suite no new failures
  (native-lib `ring`/`rustls`/`ureq` rlib errors are pre-existing environment).

### Follow-up findings (in-plan, recorded NOT filed as P-issues)

- **`n2_sorted_field_content_type_registered_first` is a SEPARATE mechanism.**
  It was loosely grouped under 2b but the iterate fix did NOT close it (still
  FAILs aligned), and it is about sorted-field *content-type registration
  order*, not iteration.  Needs its own probe + investigation — currently has
  no dedicated probe.  Re-bucket: 2e (or fold into 2d).
- **Latent `remove()` aligned corruption (NOT exercised by any test/probe).**
  The investigation found `State::remove()` (`src/state/io.rs` ~1059) reads
  cur/finish AFTER popping the DbRef and uses `get_var`/`put_var::<i64>` deltas
  (`state_var-4` at :1092, `state_var-16` at :1135) whose raw constants do NOT
  survive alignment the way `step()`'s post-pop `-8`/`-12` do.  `#remove` during
  keyed iteration has no probe and no `issues`/script test, so it is OUT OF
  SCOPE for 2b/2c — but should get a probe + fix before keyed iteration is
  declared alignment-clean.  Candidate sub-cluster 2f.

---

## 2a — LANDED 2026-05-30

**Fix (corrected by a read-only investigation; the Stage-C Option (A) was WRONG
— it would have broken Suspended resumes).**  The defect is NOT
`coroutine_next`'s `stack_pos += bytes.len()` (correct as-is) but the
return-address-slot append in `coroutine_create` (`src/state/mod.rs` ~877),
which reserved a RAW 4 bytes.  Codegen lays that slot out at a STEPPED span
(`local_start = Σ step(arg) + step(4)`), so a raw-4 append makes the captured
`bytes.len() = args_size + 4`, which is `step(4)-4 = 4` bytes short of
`local_start`.  On a `Created`-status resume `coroutine_next` advances TOS by
that short length, so the generator body reads every argument 4 bytes high
(`n=42` → `42<<32`; both args of a 2-arg generator shift uniformly because the
single short boundary under-advances TOS by exactly 4 regardless of arg count).
Integer args (8 B = `step(8)` both modes) don't themselves diverge — the
return-slot was the only divergent term.

One-line change, identity flag-OFF (`step(4)==4`):

- `src/state/mod.rs` ~877 — `extend_from_slice(&[0u8; 4])` →
  `let ret_slot = self.stack_step(4) as usize; stack_bytes.resize(len + ret_slot, 0)`.

`coroutine_next` (`+= bytes.len()`) and `coroutine_yield` are left unchanged —
yield already steps (`value_start = stack_top - step(value_size)`;
`stack_pos = base + step(value_size)`), and a Suspended frame's `bytes.len()`
is the true stepped extent (includes live locals above `local_start`, which is
exactly why Option (A)'s fixed-extent recompute would have dropped them).

**Result (verified):**
- `probes/run.sh 2a` exit 0 — all 11 `2a-*` aligned columns flip FAIL/HANG/CRASH
  → PASS; `stack_align_guard` binary SILENT on 2a-01/07/11 (genuine alignment,
  not coincidence).
- Aligned `issues` sweep: **14 → 8 failures, and 0 CRASH / 0 HANG** (was 3+2 —
  the dangerous coroutine family is entirely closed: p210, p211, p218×2, p225,
  p328).  Strict subset, zero regressions.
- Flag-OFF `issues` 681/0; clippy clean.

### Follow-up found (in-plan, recorded NOT filed): `serialise_text_args` raw offset

`serialise_text_args` (`src/state/mod.rs` ~826-857) advances `byte_offset` by
RAW `var_size(attr, Argument)`, not stepped.  Harmless for the 2a probes (text
16 B + integer 8 B args are both 8-multiples), but a generator with a <8-byte
arg (`character`/`boolean`/`single`/small `enum`) positioned BEFORE a `text`
arg would mis-locate the captured Str under LOFT_ALIGN.  Same root pattern,
different fix site, no probe exercises it — candidate sub-cluster 2g
(reproducer: `fn g(c: character, s: text) -> iterator<text>`).

---

## 2d — LANDED 2026-05-30

**Fix (investigation corrected BOTH the file and the divergent term in the
Stage-C design).**  The Stage-C design named `text.rs:489/499`
(`format_text`/`format_stack_text`, which pop `Str(16)+i64(8)` — all
8-multiples, so alignment-safe and NEVER the bug) and blamed `size_ptr()=16`.
The real site is `format_database`/`format_stack_database`
(`src/state/io.rs:647`/`653`): `OpFormatDatabase` pops the composite value's
12-byte `DbRef` (`format_db` → `get_stack::<DbRef>`), then addresses the
destination `String` at `pos - size_ref()`.  `size_ref()=12` is the LONE
divergent term (`stack_step(12)=16` aligned vs `12` off); the raw `pos - 12`
backs up too little under alignment, so the destination `String` is read 4
bytes low and the composite renders empty.  Scalars (`format_int`, `pos-16`
from `i64+i64`) are 8-multiples → unaffected (2d-02 passes).

One-term change at two sites, identity flag-OFF (`stack_step(12)==12`):

- `src/state/io.rs:647`/`653` — `pos - size_ref()` →
  `pos - self.stack_step(size_ref())`.

No codegen co-change: `Stack::operator()` already decrements `position` by
`step(size_ref())` for the mutable `reference` param, so only the runtime
destination-offset literal was unstepped.

**Result (verified):**
- `probes/run.sh 2d`: 2d-01,03,04,05,06,07 (+2d-09) PASS aligned;
  `stack_align_guard` SILENT on 2d-01.
- Aligned `issues` sweep: **8 → 1 failure** (684 ok).  Closed all six composite
  shapes (n4, n5, n8×2, p145, p159) PLUS **n2** (`n2_sorted_field_content_type_
  registered_first` was NOT a separate mechanism after all — it formats a
  composite and the same fix closed it; the earlier "separate" note is
  retracted).  Strict subset, zero regressions.
- Flag-OFF `issues` 681/0, `format` 11/0, `wrap` 49/0, `strings` 11/0; clippy
  clean.

### Only remaining aligned failure: `p189c` (2d-08, tuple-in-`par`)

`2d-08-vector-tuple-par` / `p189c_vector_tuple_element_bytes_written` is a
SEPARATE root cause — the `stack_align_guard` fires `i64 at offset 132` in
`get_var` ← `var_int` ← `execute_at_raw_primitive_input_wide` inside a parallel
worker (a tuple-element read in the par-marshalled worker frame), NOT the
composite-format path.  Candidate sub-cluster **2h** (par/tuple worker-frame
alignment); needs its own investigation.  After 2a+2b+2c+2d the cluster-2
aligned `issues` surface is down to this single case.

---

## 2h + 2i — LANDED 2026-05-30 (aligned `issues` suite now 685/0)

Tuple-in-`par` had TWO facets, both in the worker-arg setup
(`State::execute_at_raw_primitive_input_wide`, `src/state/mod.rs`):

**2h (byte smear).** The wide worker-arg buffer was pushed BYTE-BY-BYTE via
`put_stack::<u8>`; under LOFT_ALIGN each byte advanced `stack_step(1)=8`,
smearing the packed tuple across 16 separate 8-byte slots → the worker read
padding zeros → result collected as 0.  Fix: one contiguous `copy_nonoverlapping`
block copy (mirrors the coroutine-restore at ~1041).

**2i (frame shortfall).** The worker frame reserved `args_size = input_size`
(RAW tuple total), but the body's codegen lays the tuple arg at a STEPPED span
(`stack_step(size)`), so a non-8-multiple tuple (e.g. `(integer,character)`=12)
left the frame 4 bytes short and underflowed the worker stack.  Fix: reserve
`stack_step(input_bytes.len())` for `args_size` and advance TOS by the stepped
span; the copied DATA stays the raw bytes (trailing slack unread).

Both identity flag-OFF (`stack_step(1)=1`, `stack_step(n)=n`).

**Result:** all 6 `2h-*` and all 5 `2i-*` probes PASS aligned; full probe sweep
ALL CLEAN; **aligned `issues` per-test sweep = 685 / 0** (zero failures, crashes,
hangs — down from 27 at session start); flag-OFF `issues` 681/0, `wrap` 49/0;
clippy clean; full flag-OFF regression no new failures; zero regressions.

### NOT yet switch-ready — two validation gaps remain

The S4 DEFINITION OF DONE also requires the `stack_align_guard` SILENT and a
Miri run.  Neither holds yet:

- **2j — par-worker entry base (PRE-EXISTING, guard-cleanliness, NOT functional).**
  Both worker dispatchers (`execute_at_raw_primitive_input` ~2508 scalar,
  `execute_at_raw_primitive_input_wide` ~2592 wide) hardcode `stack_pos = 4` /
  `args_base = 4` instead of `aligned_stack_step(4)` = 8 (what the real entry
  `execute_argv` ~2066 uses).  The frame is self-consistent at base 4 so results
  are CORRECT (685/0), but every access lands 4-off an 8-boundary, so the
  access guard fires (`mod.rs:1403`) on ALL par workers — scalar included
  (predates 2h/2i).  Fixing it (set both bases to the stepped `aligned_stack_step(4)`
  and place args there) is what makes the par path guard-clean.  Needs its own
  probe/verification — candidate sub-cluster 2j.
- **Miri** — the gold-standard detector has not been pointed at the aligned
  interpreter yet (`MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test
  --test issues <pure-compute-test>` under `LOFT_ALIGN=1 LOFT_SLOT_V2=drive`).

So: aligned mode is FUNCTIONALLY green, but the switch (flip default / declare
guard-clean) waits on 2j (guard silent) + a Miri pass.

### Other in-plan follow-ups recorded this session (unexercised, separate fixes)

- **2e** sorted content-type registration (`n2` — turned out to be closed by 2d).
- **2f** `remove()` keyed-iteration aligned deltas (no test exercises `#remove`).
- **2g** `serialise_text_args` raw offset (sub-8-byte arg before a text arg).
- **Separate flag-OFF bugs (NOT alignment):** char-first / boolean-first tuple
  `par` (e.g. `(character, integer)`) returns 0 in production; and sequential
  `for p in pairs { f(p) }` tuple-arg fails `expected (int,int), got
  __tuple<int,int>` flag-OFF.  Surfaced while probing 2i; recorded here.

---

## 2j — LANDED 2026-05-30 (aligned interpreter now GUARD-CLEAN)

**Fix:** the par-worker dispatchers hardcoded the frame base `args_base: 4` /
`stack_pos = 4` instead of the stepped `aligned_stack_step(4)` = 8 that the real
fn entry (`execute_argv`) uses.  The frame is self-consistent at base 4 (output
correct — hence 685/0 functional) but every typed slot lands at `base + slot ≡
4 (mod 8)` → off its 8-boundary → the access guard fires.  Var addressing is
purely frame-relative (`get_var` reads `stack_cur.pos + stack_pos − pos`; the
`base` term cancels), so bumping the base 4 → 8 shifts the whole frame up 4
bytes uniformly: output unchanged, every slot now 8-aligned.  Existence proof:
`execute_argv` ALREADY runs at base 8 and the suite passes.

**Investigation under-scoped (caught by checking).** The agent hypothesised 2
dispatchers; reading the file showed **8** sharing the identical
`args_base: 4` / `stack_pos = 4` pattern: `execute_at`, `execute_at_raw`,
`execute_at_raw_primitive_input`, `execute_at_raw_primitive_input_wide`,
`execute_at_raw_to`, `execute_at_raw_text_input`, `execute_at_ref`,
`execute_at_text`.  All eight bumped to `self.stack_step(4)` (the wide one also
at its post-copy `stack_pos = … + stepped_size` advance).  `args_base` is u32
and consumed only by `stack_trace()` introspection (`debug.rs`) — bumped in
lockstep so an in-worker stack trace stays correct.  Identity flag-OFF
(`aligned_stack_step(4,false)=4`).

**Left untouched:** `execute_at_void_with_snapshot` (L2985, the `parallel {}`
block worker) — it overlays the PARENT's stack snapshot with offset-4 semantics
tied to the parent layout; distinct from the `par()` dispatchers and not part of
2j.  The full `issues` suite is guard-clean including its parallel cases; the
`tests/scripts` `parallel {}` suites were not run under the guard here, so the
snapshot path's guard-cleanliness there is unverified — candidate follow-up if a
guard run over scripts surfaces it.

**Result — the S4 guard criterion is MET:**
- `run_guard.sh 2j`: all reproducers FIRES → **SILENT**; references stay SILENT;
  functional PASS/PASS throughout.
- **Full `issues` suite under the `stack_align_guard` test binary, aligned,
  per-test isolated: 685 / 0 — ZERO guard fires.**  The homegrown
  Miri-for-the-stack is fully silent across the whole interpreter.
- Aligned functional 685/0; flag-OFF `issues` 681/0; clippy clean (with and
  without the `stack_align_guard` feature); full functional probe sweep CLEAN.

### Switch-readiness after 2j

Three of the four S4 DoD criteria now hold: aligned 685/0, flag-OFF 681/0,
**guard-clean (685/0 armed)**.  The LAST gate is the **Miri** run — the
gold-standard external detector — against the now-guard-silent aligned
interpreter.  Only after Miri is clean is the switch (flip default vs keep
behind `LOFT_ALIGN`) a tool-validated decision.

---

## Miri validation 2026-05-31 — cluster-2 alignment confirmed; two successor UB clusters surfaced

Ran `cargo +nightly miri test --test issues p213_struct_field_basic_int` (a
pure-compute struct test) under four configs (`-Zmiri-disable-isolation`):

| Config | Result |
|---|---|
| aligned, default (Stacked Borrows ON) | UB: `from_mut(&mut self.allocations)` reborrow, `structures.rs:208` (`claim_child_rec`) |
| aligned, `-Zmiri-disable-stacked-borrows` | UB: uninit read, `[u8; 20]` at [18] (fn-ref slot) |
| flag-OFF, `-Zmiri-disable-stacked-borrows` | **UB: uninit read, `[u8; 20]` at [18] — IDENTICAL** |

**Conclusion — cluster 2 (eval-stack alignment) is VALIDATED.**  With the
aliasing model off (isolating the hard memory-safety UB Miri checks — alignment,
OOB, UAF, uninit), aligned and flag-OFF produce the **byte-identical** finding.
The alignment work introduced **no new hard UB**, and Miri reports **no
alignment-class UB** in aligned mode — corroborating the homegrown guard's
685/0-zero-fires result with the gold-standard external tool.  There is NO
remaining eval-stack alignment UB; the original cluster-2 finding (unaligned
`Str`/`DbRef` at TOS) is cleared.

**Two SUCCESSOR clusters the Miri lane surfaced** (both PRE-EXISTING and
mode-independent — present identically in flag-OFF production; NOT alignment;
NOT introduced by this plan).  These are the "next layer" Stage-A2 was meant to
find:

- **Cluster 3 — store-aliasing reborrow.**  `Stores::claim_child_rec`
  (`src/database/structures.rs:208`) does `&mut *std::ptr::from_mut(&mut
  self.allocations)` — a `&mut [Store]` reborrow Miri's Stacked Borrows rejects.
  One of the ~426 intentional-aliasing `unsafe` store blocks the plan's baseline
  flagged.  Blocks a default-Miri clean run for ANY store-allocating program.
  Fix surface: the store's aliasing discipline (or adopt Tree Borrows / narrow
  the reborrow).  Own cluster — needs its own investigation.

- **Cluster 4 — uninit-padding typed read.**  A 20-byte fn-ref slot is read as
  `[u8; 20]` while only 16 bytes (d_nr 4 + closure `DbRef` 12) are written,
  leaving bytes 16..20 uninitialised; Miri rejects the uninit read.  Harmless on
  real hardware (the live bits are valid) but real UB per Rust's model.
  Pre-existing fn-ref representation quirk, mode-independent.  Fix surface:
  zero-fill the fn-ref slot tail (or read only the written 16 bytes).

### Switch-readiness — final assessment

- **Aligned `issues` 685/0**, **flag-OFF 681/0**, **guard-clean (685/0 armed,
  zero fires)**, **Miri: no alignment UB, no new UB vs flag-OFF** — the eval-stack
  alignment work (2a–2j) is complete and triple-validated (functional + homegrown
  guard + Miri differential).
- A *literally* clean Miri run is blocked by clusters 3 & 4 — but both are
  PRE-EXISTING in flag-OFF, so they do NOT gate the alignment switch (flipping
  to aligned adds no UB).  The honest Miri gate "no NEW hard UB under aligned vs
  flag-OFF" is **MET**; the absolute "Miri clean" gate awaits clusters 3 & 4
  (successor work, tracked above).
- **Recommendation:** the @PLAN53 deliverable is the CI lever, not making V2 the
  default.  Cluster 2 can be closed as "fixed + guard-clean + Miri-differential-
  clean behind `LOFT_ALIGN`"; clusters 3 & 4 become the next sanitizer-lane
  targets (they are what a Miri CI job would gate on going forward).

---

## Clusters 3 & 4 — LANDED + Miri-validated 2026-05-31; cluster 5 (leak) surfaced

**Cluster 3 (store-aliasing reborrow) — FIXED** (commit bca761fe).  All 4
`from_ref::<[Store]>`/`from_mut::<[Store]>` sites (`claim_child_rec`,
`vector_add`, `vector_add_array`, `copy_claims`) replaced by one sound
`Stores::copy_block_cross_store` using stable `<[Store]>::get_disjoint_mut`.
Miri-confirmed: the cluster-3 run no longer aborts at `claim_child_rec` — it
proceeded to cluster 4.

**Cluster 4 (uninit fn-ref padding) — FIXED** (commit 8ba675eb).  Root cause:
the 20-byte fn-ref slot = i64 d_nr (0..8) + closure `DbRef` (8..20); `DbRef` has
no `#[repr(C)]`, reorders to `{rec,pos,store_nr}`, leaving 2 bytes of tail
padding at slot 18..20 that the typed `*m = DbRef` store never defines.
`OpVarFnRef`/`OpPutFnRef` read the whole slot as `[u8; 20]` (integer array → all
bytes must be init) → Miri uninit abort at [18].  Fix: read/write the slot as
`[MaybeUninit<u8>; 20]` (propagates bytes without an init requirement),
byte-identical.  Edited the generator source (`default/02_files.loft`) and
regenerated `src/fill.rs`.

**Miri validation (p213, aligned, `-Zmiri-disable-stacked-borrows`):
`test result: ok. 1 passed; 0 failed`** — the test BODY executes with ZERO
hard UB.  Clusters 2 (alignment), 3 (aliasing), 4 (uninit) are all cleared for
the interpreter execution path.

**Cluster 5 (NEW, surfaced now) — a memory LEAK, not UB.**  After the test
passes, Miri's leak checker reports a 20-byte `String` leaked during teardown:
`free_text` (`src/state/text.rs:316`, `String::shrink_to`) ← `Test::drop`'s
cleanup `execute`.  PRE-EXISTING (masked until clusters 3/4 let Miri reach the
leak check), mode-independent, lower severity than UB (a leak cannot corrupt
data), and suppressible with `-Zmiri-ignore-leaks`.  NOT introduced by clusters
3/4 (they don't touch text/String allocation).  Candidate next sanitizer-lane
target — investigate whether it's a real teardown leak (text not freed on
program exit / store drop) or a test-harness `Test::drop` artifact.

### Validation status after clusters 3 & 4

The loft interpreter's execution path is now Miri-clean for hard UB
(alignment + aliasing + uninit) on the p213 reproducer.  The only Miri signals
left are (a) the cluster-5 teardown leak (leak-class, suppressible) and (b)
Stacked-Borrows reports on the remaining intentional-aliasing store blocks when
SB is ON (the store layer's design; out of scope for a hard-UB gate).  A Miri
CI job would run with `-Zmiri-disable-stacked-borrows` (hard-UB gate) and either
`-Zmiri-ignore-leaks` or after cluster 5 lands.

---

## Cluster 5 (leak) — LANDED; Miri CI gate SHIPPED (D-final) 2026-05-31

**Cluster 5 (free_text leak) — FIXED** (commit 11064863).  `free_text` released
the String's heap buffer with `shrink_to(0)`, but the `clear()` that makes it
free ran only under debug-assertions; in release/Miri a non-empty String hit
`shrink_to(0)` which shrinks capacity only down to `len` → buffer leaked (the
store holds the String as raw bytes and never runs its Drop).  Fix: `clear()`
unconditionally before `shrink_to(0)`.  A real production text leak, not just a
Miri artifact.  Verified: flag-OFF issues 681/0, stores-leak gate 34/0, clippy
clean, and **Miri p213 fully clean: test ok, no leak, EXIT 0**.

**Miri CI gate — SHIPPED** (commit 79428a50).  `.github/workflows/miri.yml` +
`make ci-miri`.  Runs the curated `issues` test(s) on the aligned interpreter
under `-Zmiri-disable-isolation -Zmiri-disable-stacked-borrows` (hard-UB gate).
Dedicated workflow (interpreter-under-Miri is ~15 min/test); triggers on
UB-relevant path changes + nightly + manual.  Curated set = `p213` (validated
Miri-clean after clusters 2/3/4/5).

### Final state — the @PLAN53 lever is in place

The loft interpreter's execution path is Miri-clean for hard UB on the p213
reproducer (alignment + store-aliasing + uninit-padding + leak all closed,
clusters 2–5), and a Miri CI job now gates the UB-relevant surface so the next
latent UB lands red on `main` instead of surfacing months later via a toolchain
bump (the @P383 failure mode this plan was triggered by).

Remaining lower-priority follow-ups (not blockers; tracked above): the
intentional store-aliasing blocks under Stacked Borrows ON (a design question —
the gate runs SB-off by choice); the in-plan items 2e/2f/2g (unexercised
keyed-iter `remove`, `serialise_text_args`, the char-/bool-first tuple-par and
sequential-tuple-arg flag-OFF bugs); and growing the Miri curated set as more
tests are validated clean.
