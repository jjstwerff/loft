<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 cluster — write+read struct residual (`p9`)

**Status: FIXED (2026-07-09).** Interp-only struct binary-I/O corruption+leak:
`f += s` then `f#read as S` mis-targeted the eval-stack slot instead of the
record on BOTH the write (`assemble_write_data`) and read (`dispatch_read_data`)
paths — the write serialised the record's DbRef bytes as "fields" (garbage to
file), the read filled the stack (record orphaned → leak) and delivered a store
whose number equalled the first field value (right only by coincidence; garbage
under `LOFT_POISON`, wrong for inline-literal / one-field structs). **Fix:** deref
the slot to the record in both paths (mirroring the proven Vector arm) — native
already did this via `FileVal for DbRef`, so it was purely an interp divergence.
All 13 probes pass the poison oracle (value == native + leak-free) on BOTH
backends; `binary_io_matrix` 32/32; full suite green (bar a websocket flake);
regression `tests/scripts/86-writeread-struct.loft`. No switch was needed — the
poison oracle proved the deref exact, so it landed directly.

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

## The oracle (defineable NOW — use it before + through the fix)

Two independent, already-available oracles make the whole matrix a DETERMINISTIC
falsification instrument, defeating the "correct-by-coincidence" trap:

1. **`LOFT_POISON=1` + cross-mode value equality.** Poison overwrites freed
   stores, so a struct read that delivers a FREED/stale store yields garbage
   instead of the coincidentally-correct value. Under poison, **native is
   uniformly correct** (the reference) and **interp diverges on EVERY p9 probe**
   — a1/a2/a4/b1/d1 read poison (`-2401…`), c2 reads `-559038737`, c3 reads
   `239`, a3 `16`, c1 `null` — vs native's true values. Baseline captured
   2026-07-09. The oracle assertion: *interp-under-poison value == native value*.
   This exposes defect (1) (stale delivery) universally, not just the a3/c1
   flukes the no-poison baseline happened to show.
2. **Leak-check (`collect_store_leaks` empty).** Exposes defect (2) (the orphaned
   read record) — already the suite leak-gate.

So the fix is DONE when, for every probe, on BOTH backends: value matches native
**under poison** AND zero leaks. Poison also proves the harness can fail (all red
now), and it will flip green only when the read genuinely delivers + owns its own
live record — the exact correctness the fix must produce.

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

### Step 1 — localize the divergence ✅ DONE (mechanism below; fix needs the debugger)

**Mechanism (poison oracle, both backends):** the struct `f#read` emits
`OpReadFile(OpCreateStack(temp_var))` — the SCALAR mechanism. `OpCreateStack`
yields a ref to the temp's eval-stack SLOT; correct for a scalar (value lives in
the slot) but wrong for a struct (slot holds a DbRef to the record), so
`OpReadFile` writes the field bytes into the eval stack and the allocated record
is never filled. Compounding it, `temp_var`'s frame slot reads back a STALE
freed store (`#5`, the write `_wf`) at the block tail, so the consumer `q` is
delivered that stale store — correct only by coincidence (`#5` = a freed-not-
zeroed copy of the written struct); garbage under `LOFT_POISON`, garbage in `a3`.
So p9 = TWO coupled defects: (1) the read targets the eval stack, not the record
(→ orphan/leak); (2) `temp_var`→`q` delivery picks up a stale store (→ wrong
value, masked by coincidence). Native does neither (FileVal-for-DbRef
dereferences correctly) — interp-only.

**The fix is NOT guessable — two attempts corrupted (kept the oracle honest):**
- parser `Var(temp_var)` instead of `OpCreateStack` → native E0308 (OpReadFile
  needs `&mut`), interp still garbage.
- interp `dispatch_read_data` dereference-to-record → fixed the LEAK but broke
  the VALUE (a1 `5`→`34359738369`): it filled the record while `q` still
  received the stale `#5`, so defect (2) turned the coincidence into garbage.

Both reverted. The coupling means the read-target and the delivery must be fixed
TOGETHER, and the `temp_var`-slot-→-`#5` step is not visible to eprintln tracing.
**Step 4 requires the loft debugger** (`loft debug --rpc`: breakpoint the block
tail, watch `temp_var`'s slot DbRef from `OpDatabase` through `OpReadFile` to the
delivery `PutRef`) to see exactly where the slot acquires `#5` — then fix
read-target + delivery as one change, gated, verified against the poison oracle.

<!-- superseded partial notes: -->
#### (earlier partial)
Slot-trace instrumentation (`put_ref`/`var_ref`/`read_file`/`free_ref_db`, since
removed) on interp, cache-off, established:

- **The stale delivery is NOT the leak cause.** `OpReadFile` fills the eval-stack
  store `#0` (via `OpCreateStack` — normal), and `q` is delivered a STALE store
  (`#5`, the freed write `_wf`) in BOTH the leaking (`a1`) AND the clean
  (`rdloop`, write-outside) cases. So `q ← #5` is common to clean runs — it is
  the reason struct-read values are right only by COINCIDENCE (`#5` = a copy of
  the written struct, freed but not zeroed), and why `a3` (inline literal, no
  matching `_wf`) reads garbage `16`. This is a real correctness defect but a
  SEPARATE axis from the leak.
- **The leak is the read RECORD (`#2` from `OpDatabase(_read_1)`) not freed**, in
  the same-scope write+read shape (`a1`) but freed when the write is outside the
  loop (`rdloop`). i.e. the leak is shape-dependent free-accounting of the read
  record, not the `q` delivery.

So p9 is TWO intertwined defects: (i) the read-record free is shape-dependent
(the leak), and (ii) the struct read delivers a stale store rather than the read
result (correctness-by-coincidence; garbage in `a3`). eprintln tracing hit its
limit here — the precise free-accounting for (i) needs the loft debugger
(`loft debug --rpc`: breakpoint the read-record free, watch its store across the
`a1` vs `rdloop` shapes). Chokepoints refined: (ii) is the read block's
`OpCreateStack`/delivery plumbing in `src/parser/objects.rs` (the struct-read
block never copies `#0`/`#2` into the consumer); (i) is the read-record's
free-emission in `src/scopes.rs`. **Exit remaining:** the ONE free-site for (i)
named via the debugger; decide if fixing (ii) [deliver the read result, not a
stale alias] subsumes (i).

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

### Step 4 — the fix 🔶 DIRECTION CONFIRMED, last-mile open (reverted; no code shipped)

Full mechanism now understood (op-level `put_var`/`read_file`/deref traces):
- The struct read emits `OpReadFile(OpCreateStack(temp_var))`. `OpReadFile` fills
  the eval-stack slot, NOT the record `#2` (`OpDatabase(temp_var)`). The block
  then delivers that stack slot; **reinterpreted as a DbRef its `store_nr` = the
  first field value** (`x=5` → "store #5"), so the consumer lands on whatever
  store number equals the field — correct only by that coincidence, garbage
  under poison / in `a3`.
- **Fix direction (confirmed correct):** mirror the working **Vector arm** in
  `dispatch_read_data` — deref the slot to the record ref and fill THAT. With it:
  the delivery is FIXED (`put_var` shows `q ← #2`, the real record, not the
  bytes-as-DbRef) and the write targets the record (`[deref] rec_ref=#2@1,8
  data.len=16`, the right file bytes). So the structural fix is the deref, same
  as vectors.
- **Last-mile open:** even with `rec_ref = #2` and 16 correct bytes, `write_data`
  does not land them — `#2` reads back stale (`0x8_0000_0001`) though `q == #2`.
  So `write_data(&#2, struct, data)` isn't filling the record here (offset /
  reused-store / copy-back interaction), a record-fill question distinct from the
  now-solved delivery. Needs a raw byte-dump of `#2` before/after `write_data`.

**Closed by the byte-dump.** Dumping `#2` around `write_data` showed `data0`
itself was garbage (`34359738369`) — i.e. the write had already put DbRef bytes
in the FILE. So the read deref alone couldn't help; the WRITE
(`assemble_write_data`) needed the SAME deref. Landing BOTH derefs makes all 13
probes pass the poison oracle + leak-free, both backends (see status header). No
gate/switch needed — the oracle proved the two-line deref exact.

### Steps 5-6 ✅ DONE
Poison-oracle validated (all 13 green both backends); `binary_io_matrix` 32/32;
full suite green modulo a known websocket parallel-flake. Regression graduated to
`tests/scripts/86-writeread-struct.loft` (a1/a3/c1/c2/c3 shapes, value asserts +
leak-gate), green both backends. Cluster CLOSED.

### Step 5 — oracle gate + flip
Gate the fix, then assert the **poison + cross-mode oracle** (above): every probe
matches native under `LOFT_POISON=1` and is leak-free, both backends. Full suite
with the gate ON stays 2721/2721 both backends (also run a `LOFT_POISON` pass over
the struct-IO tests); the only diffs vs OFF are the 13 probes turning green. Then
default the gate ON.

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
