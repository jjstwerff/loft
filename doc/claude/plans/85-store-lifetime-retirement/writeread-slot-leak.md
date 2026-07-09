<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 cluster — write+read struct residual (`p9`)

**Status: OPEN — matrix done (Step 0), root-cause + fix designed below.** An
interp-only store leak: `f += s` (struct write) then `f#read as S` (struct read)
in one program leaks one read-buffer record **per read**, iff a struct write ran
earlier. Native is clean. Distinct from block-return-move (the read block-return
in isolation is clean). This doc is the stepped implementation plan; it reuses
the oracle/switch migration proven on block-return-move.

## The defect (measured, not hypothesised)

The 13-probe boundary matrix ([`probes/writeread-slot-leak/`](probes/writeread-slot-leak/README.md))
pins it and **refuted** the first guess. Facts:

- Leak = one record **per struct read**, only after a struct write earlier in
  the program. Independent of read type (a2), of ANY use of the result (b2 — no
  use, no call, still leaks), of call-vs-non-call use (b1/b3), of live locals
  (e1). Scales per read (d1=2, d2=5). Interp only.
- The read-buffer free IS emitted: the read block returns `_read_1` and the
  consumer `q` adopts it (empty-dep), so `OpFreeRef(q)` covers the buffer. But
  at runtime that free reclaims the STALE store the slot held before (the freed
  write `_wf` temp, `#5`) instead of the adopted read record (`#2`), so `#2`
  leaks. `q`'s stack slot is REUSED from `_wf`'s (both `[80,92)` in `a1`) and is
  never re-`InitRef`-ed at read-scope entry; the block-delivery `PutRef(q,#2)`
  runs (q.x reads correctly) yet the slot is back to `#5` by the free — with NO
  intervening call (b2), so it is not a call/frame-teardown revert of a
  correctly-set slot. Native re-uses slots too but frees correctly ⇒ the
  divergence is in the interp free/slot path, not the shared IR.

Two more interp-only bugs the matrix surfaced (same neighbourhood — likely the
same read-buffer / write-serialise root, to confirm in Step 2):

- **a3** — inline-literal struct write (`f += P{…}`) reads back `16` on interp
  vs `5` native (write serialises wrong bytes for a non-`Var` operand).
- **c1** — one-field struct read returns `null` on interp vs `42` native (single-
  field records take a different, and the only leak-FREE, read path).

## The invariant (to enforce)

> A struct read's buffer store is owned by the consumer and freed exactly once,
> regardless of any earlier write. The consumer's slot holds the adopted read
> record at the free — never a stale DbRef from a prior occupant of that slot.

## Chokepoint (to confirm in Step 1)

Leading candidates, in order:
1. **Reused-slot init** — `q`'s slot is not `InitRef`-ed when it reuses a freed
   slot at a new scope, so it enters holding a stale freed DbRef. Home:
   `src/scopes.rs` slot assignment / `InitRef` emission (SLOTS.md zone-2).
2. **Block-delivery persistence** — the read block's `PutRef(q, #2)` does not
   persist to the outer slot across the block's `FreeStack`/frame teardown.
   Home: `src/parser/objects.rs` read-block build + `src/state/` block exec.
3. **Free-site store identity** — `OpFreeRef` frees the slot's CURRENT DbRef;
   the fix may be to null the slot on adopt so a stale value can't be freed.

The native path (clean) is the reference oracle for the correct sequence.

## Implementation steps (oracle/switch migration)

### Step 0 — boundary matrix ✅ DONE
13 probes, both backends, in `probes/writeread-slot-leak/`. Refuted the first
root-cause; pinned the boundary. Proven to fail (a1/…/e1 red on interp).

### Step 1 — localize the divergence
Instrument `q`'s slot DbRef at three points on interp: right after the block
`PutRef` (expect `#2`), at scope exit before `OpFreeRef(q)` (observed `#5`), and
compare the native emission for the same program (clean). Use the loft debugger
(`loft debug --rpc`: breakpoint at the free, `eval`/`getValue` the slot) rather
than sprinkled prints. Decide between chokepoints 1–3 by WHERE `#2`→`#5` (or
never-`#2`) happens. Exit: the ONE site named, with a trace proving it.

### Step 2 — a3 / c1 shared-root check
Instrument a3 (inline-literal write bytes) and c1 (one-field read path) in
isolation. Classify: same read-buffer/slot root as p9, or distinct. Fold in if
shared; otherwise spin each into its own row here (do not scope-creep the p9
fix). Exit: each of a3/c1 tagged shared|distinct with evidence.

### Step 3 — the switch (inert)
Add `use_analysis::writeread_slot_fix()` gate (env, default OFF — mirrors the
block-return-move Step 1). Thread to the Step-1 chokepoint; prove IR/behaviour
byte-identical OFF via `loft introspect` (cache-off / `LOFT_NO_CACHE=1` — the
per-script cache keys on source, not binary).

### Step 4 — the fix behind the gate
Enforce the invariant at the chokepoint: null/`InitRef` the consumer slot on
adopt (chokepoint 1/3) or persist the delivery (chokepoint 2). Match the native
sequence. When ON: all 13 probes leak-free AND correct-valued on interp; native
unchanged.

### Step 5 — oracle gate + flip
Full suite with the gate ON must stay 2721/2721 both backends (the block-return-
move oracle standard); the only diffs vs OFF are the 13 probes turning green.
Then default the gate ON.

### Step 6 — graduate + retire
Regression `tests/scripts/86-writeread-slot.loft` (minimal a1 + d1 + b2 shapes —
leak-gate + value asserts; keep minimal so the deterministic interp signal
survives, per the block-return-move layout-fragility lesson). Delete the switch
after a green cycle. Close the cluster on @PLN85.

## Acceptance

- All 13 `writeread-slot-leak` probes leak-free AND correct on BOTH backends
  (incl. a3 value `5`, c1 value `42` if folded in).
- Full suite green + leak-clean, switch defaulted ON then removed.
- Regression in `tests/scripts/`; @PLN85 cluster closed.

## Cross-references

- [block-return-move.md](block-return-move.md) — the sibling cluster (DONE);
  its oracle/switch migration is the template here.
- [SLOTS.md](../../SLOTS.md) — slot assignment / zone-2 / `InitRef`.
- `src/scopes.rs` (slot/`InitRef`) · `src/parser/objects.rs` (`f#read` block) ·
  `src/database/allocation.rs` `free_named` (free-site).
