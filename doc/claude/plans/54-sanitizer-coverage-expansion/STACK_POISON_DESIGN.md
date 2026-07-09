<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# S3 second half — stack-slot poison, the sound way (design + closeout)

> **Part of [@PLN54](README.md) S3.** Written as a `design-protocol` hypothesis:
> the design is a claim about ONE invariant, probed to falsify before it is built.
> **Status: ✅ BUILT + VALIDATED (2026-07-09).** The reserve-time poison is live
> in `State::reserve_frame` (gated on `keys::poison_enabled()`, interpreter-only —
> native uses Rust's own stack, so it does not call `reserve_frame`). Validation:
> the `LOFT_POISON=1` interpreter suites are **green (1498/1498, both the
> soundness prediction — no false positive — and the definite-assignment claim
> hold)**, and the positive control `frame_vars::reserve_poison_fires_on_uninit_slot_read`
> fires (reads the sentinel from an unwritten reserved slot) under the flag and
> skips without it. The design below is preserved as the record of why *reserve*,
> not *free*, is the sound hook.

## The goal, and the reframe that makes it sound

S3's store-record half fills a freed **store record** with `0xDEADBEEF` so a
dangling-`DbRef` read hits loud garbage. The second half was filed as *"poison
freed STACK slots too"* — turn a stale read of a dead **eval-stack / frame** slot
loud the same way.

Taken literally — *poison at free* — the design is **unsound**, and the failure
paths say why (this is the doc doing its job):

- **FP1 — the pop primitive returns a reference INTO the vacated bytes.**
  `get_stack<T>` (state/mod.rs:1725) lowers `stack_pos` by `step(size)` **then
  returns `&T` at the new `stack_pos`** — the just-"freed" region *is* the value
  being read. Poison-on-pop corrupts the read.
- **FP2 — the return value transiently lives in the about-to-be-vacated region.**
  On return, `stack_pos` is restored to the caller (`reenter_ret`: `stack_pos =
  saved_sp`), but the result is read from the **callee frame** (`base` offset)
  *before* the restore. Poison the vacated `[saved_sp, callee_high)` and you erase
  the return value.

Both hazards share a root: **at *free* time the vacated region still holds live
data** (the popped value, the return value). So *free* is the wrong hook.

**The reframe (read off the plotted answer): poison at *reserve*, not at free.**
`reserve_frame` (state/mod.rs:1591) advances `stack_pos` into fresh space and
**leaves the reserved region holding prior garbage** (it never zeroes). That
region is, by the stack discipline, **above the old TOS — provably dead: nothing
live is ever above `stack_pos`.** Filling it with the sentinel at reserve cannot
touch any live value (FP1/FP2 both vanish — the return value is written *after*
reserve, over the poison), yet it achieves the identical detection goal: a read
of a slot the current frame **has not yet written** returns the sentinel, not a
stale occupant.

This is the sound dual of the store poison: the store poisons **on free** (its
freed record is genuinely dead); the stack poisons **on reserve** (its freed
region is not yet dead — but its *freshly reserved* region provably is).

## The one invariant

> **At the instant a stack frame is reserved, every byte of its not-yet-written
> slot region holds the poison sentinel. A correct program writes each slot (an
> `OpInit*` / push) before it reads it (definite assignment), so any read that
> observes the sentinel is a definite-assignment violation — an uninitialized or
> cross-frame-stale slot read.**

Definite assignment is the property that makes this non-vacuous: SLOTS.md's frame
model already guarantees every first assignment is a positional init
(`OpInitText` / `OpInitRef` / `OpInitRefSentinel` / `OpInitCreateStack`), and a
nullable slot is *written* with a sentinel, not left blank. So on a correct
program the sentinel is never observed; on a stale read it always is.

## Re-assertion sites — the prospective tell (design-protocol step 2)

**One chokepoint for the main path: `reserve_frame`.** Every ordinary function /
block frame flows through `OpReserveFrame` → `State::reserve_frame`, so a single
edit there covers all user-code frames. **N = 1** for the class that matters.

**The residual reserve paths are ENUMERATED, not silent** (they set `stack_pos`
directly, bypassing `reserve_frame`): the par-worker entries (`stack_pos =
stack_step(4)` at state/mod.rs:4524/4575/4646/4737/4831/4912…), `reenter_ret`
(4152), and the coroutine frames. They are a *known* coverage gap listed here,
not a forgotten one — extend to them only if the green-drive shows a real
uncovered read there. (This is the step-2 discipline: N small, omission loud
*because it is written down*.)

## Soundness — falsifying the cleanest claim (steps 3–4)

The cleanest claim is **"poison at reserve never corrupts a live value."** Probes:

1. **Is the reserved region ever holding a live value?** No — `reserve_frame`
   advances `stack_pos`; the region `[old_stack_pos, new_stack_pos)` is *above the
   old TOS*, which the stack discipline defines as free. Nothing reads free space
   before writing it. **Holds.**
2. **Does it touch the args?** No — args sit *below* the frame base
   (`[args_base, args_base+args_size)`); reserve poisons *above* it. Args stay
   live and untouched.
3. **Does it touch the return value?** No — the return value is written into the
   frame *after* reserve and read *after* the callee returns; the reserve-time
   poison is overwritten by the real write. (This is exactly FP2, dissolved.)
4. **Does any *correct* program read a reserved-but-uninitialized slot** (relying
   on zero-init — e.g. an accumulator with no explicit `= 0`)? This is the one
   claim the desk cannot settle — it is what the **green-drive** (build step 4)
   probes. `sum = 0` is an explicit init; nullable slots are sentinel-*written*;
   so the prediction is *no such read exists*. A surviving break is then either a
   real uninit-read bug (fix it, stability rule) or a codegen zero-init assumption
   to make explicit. **Deliberately left to the build — the build is the last probe.**

## Coverage — what it catches, what it does not (explicit)

- **Catches:** a read of a frame slot the current frame never wrote — the
  uninitialized-slot read, and the **cross-frame stale read** (a new frame whose
  slot still holds a prior frame's dead bytes; the new frame's reserve re-poisons,
  so the stale read hits the sentinel).
- **Does NOT catch (documented residual):** *within-scope zone-1 slot reuse* — A
  dies, B reuses A's slot, a mis-emitted read of B before B's init reads A's real
  value (A wrote over the reserve poison), not the sentinel. Catching this needs a
  re-poison at A's interval end, for which there is **no runtime event** (slot
  reuse is a compile-time `assign_slots` decision). Left out; it is largely a
  codegen slot-assignment bug the `stack_align_guard` + the cross-backend
  differential already pressure.
- **Complementary to the DbRef detectors, not redundant:** `LOFT_UAF_GEN`
  (state/mod.rs:1756) catches a DbRef whose target *store* was freed+reused
  between push and pop; reserve-poison catches an *uninitialized/cross-frame slot*
  read of **any** type. Different classes; both wanted.

## Sentinel + detection — reuse the existing guards

Fill with the same `0xDEADBEEF` byte pattern as the store poison. Detection is
free on the load-bearing type: a `DbRef { store_nr: u16, rec, pos }` read of
poison bytes gets `store_nr = 0xBEEF` (48879) → the existing `get_stack<DbRef>`
OOB guard (state/mod.rs:1740) fires a named panic. A poisoned `Str`/text read
derefs a wild pointer → loud. A poisoned scalar surfaces as a wrong value the
cross-backend differential catches. No new guard needed.

## Build steps (cheap-prototype-first — the build pins the invariant)

1. **Prototype at the one chokepoint.** In `State::reserve_frame`, under
   `crate::keys::poison_enabled()`, fill `[old_stack_pos, new_stack_pos)` of
   `stack_bytes` with the `0xDEADBEEF` pattern (via `stack_cur` addressing, the
   same as the store poison writes past its header). ~10 lines, gated, off by
   default. Both backends inherit it (native calls the same runtime).
2. **Positive control (prove it can fire — non-vacuous).** A unit test that
   reserves a frame with poison on and reads an un-initialized slot as a `DbRef`,
   asserting the OOB guard panics; plus, if the green-drive surfaces a real one, a
   graduated `tests/scripts/85-*.loft`. A silent detector is worthless — this is
   the injected fault it must catch.
3. **Falsify claim 4 — the green-drive.** Run `LOFT_POISON=1` over the interpreter
   suites (the S3 CI-gate command) with reserve-poison on. Expected: green. Each
   real surfaced uninit/cross-frame read is FIXED in-session with a guard (the
   stability rule — no filing); a benign zero-init reliance is made an explicit
   init. Re-verify on BOTH backends.
4. **Decide the residual reserve paths.** If step 3 is clean via the main
   chokepoint, leave par/coroutine/`reenter_ret` as the documented gap. If it
   shows an uncovered read there, extend the same fill to that path.
5. **Wire it standing.** The existing nightly `poison` job already runs
   `LOFT_POISON=1`, so reserve-poison ships *inside* it the moment it lands — no
   new CI. Update this doc + the README S3 row to DONE with the green reading.

## Done criterion

`LOFT_POISON=1` interpreter suites green with reserve-poison **on**, both
backends; the positive control fires; the residual (within-scope slot reuse; the
enumerated non-`reserve_frame` reserve paths) is documented, not silent. At that
point S3 is closed on its own terms — a stale frame-slot read is loud, not silent
— rather than deferred.

## See also

- [README.md](README.md) § Concrete steps S3.3 — the plan row this closes.
- [fuzz-proof-gate.md](../85-store-lifetime-retirement/fuzz-proof-gate.md) — the
  store-record poison half + the 23-bug campaign this mirrors.
- [SLOTS.md](../../SLOTS.md) — `assign_slots`, `OpReserveFrame`, the definite-
  assignment frame model the invariant rests on.
