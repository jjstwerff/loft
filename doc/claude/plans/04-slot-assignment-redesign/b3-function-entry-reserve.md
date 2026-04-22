<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase B.3 — Function-entry frame reserve (slot-aware refactor)

**Status:** designed, not yet implemented.  Requires care — the
slot-move in `gen_set_first_at_tos` is load-bearing under the
current per-block-reserve model, and Phase B.2.a's `_copy` rewrites
silently depend on it.

## Context

Phases B.1–B.2 replaced the 4 compound push-and-init opcodes
(`OpText`, `OpConvRefFromNull`, `OpNullRefSentinel`, `OpCreateStack`)
with a `OpReserveFrame(n) + OpInit*(pos)` decomposition.  The
architectural principle — separate "advance stack pointer" from
"write init value" — is now threaded through every codegen call
site and through `generate_call`'s interception of parser-emitted
IR.

The next step is **Phase B.3**: emit one `OpReserveFrame(frame_hwm)`
at function entry, delete the per-block
`OpReserveFrame(block.var_size)` in `generate_block`, and remove
the now-redundant slot-move + gap-fill preamble from
`gen_set_first_at_tos` so V1's slot placement becomes authoritative
end-to-end.  The bytecode footprint shrinks from N per-block
reserves + matching `OpFreeStack` pairs to 1 per function.

## The architectural problem

Attempting this naively — just the two codegen edits — produces a
SIGSEGV in `fn_ref_basic_call` (reproduced with
`cargo test --release --test issues fn_ref_basic_call`).  The root
cause is that B.2.a's four `gen_set_first_ref_*_copy` rewrites
(codegen.rs:1222 / 1263 / 1297 / 1336) and the reassign-deep-copy
branch in `generate_set` (codegen.rs:988) all hard-code
`OpReserveFrame(12) + OpInitRef(12) + OpDatabase(12, tp)`.

That `12` addresses `v.stack_pos` only because the slot-move at
the top of `gen_set_first_at_tos` (codegen.rs:1125) forces
`v.stack_pos == stack.position` before these functions run.  Under
function-entry reserve, `stack.position` jumps to
`local_start + frame_hwm` at function entry — every local's
V1-placed slot is now below TOS.  Slot-move fires for every
first-Set, moves `v.stack_pos` to the current TOS (above
`frame_hwm`), and the hard-coded `OpInitRef(12)` writes the null
DbRef at `stack.position - 12` — past the reserved memory →
SIGSEGV on the next stack-crossing.

## Design

Make V1's slot placement authoritative end-to-end.
`gen_set_first_at_tos` stops moving slots.  Each
`gen_set_first_*` function computes
`slot_offset = stack.position - v.stack_pos` at its entry
(bumping TOS with a gap-fill if `v.stack_pos > stack.position`)
and uses it uniformly for every slot-addressing positional op it
emits.

### Per-function refactor

Each of the 4 `gen_set_first_ref_*_copy` functions + the
reassign-deep-copy branch in `generate_set` follows the same
pattern today:

```rust
// Before (B.2.a, load-bearing on slot-move):
OpReserveFrame(12)            // push 12 bytes
stack.position += 12
OpInitRef(12)                 // null DbRef at stack.position - 12 (= old TOS)
OpDatabase(12, tp)            // alloc store; DbRef at stack.position - 12
generate(OpCopyRecord(src, v, tp))   // OpVarRef(v) reads at v's slot
```

This works only because slot-move forces `old TOS == v.stack_pos`.

```rust
// After (slot-aware):
if stack.function.stack(v) > stack.position {
    let gap = stack.function.stack(v) - stack.position;
    stack.add_op("OpReserveFrame", self);
    self.code_add(gap);
    stack.position += gap;
}
let slot_offset = stack.position - stack.function.stack(v);
stack.add_op("OpInitRef", self);        // null DbRef at v's slot
self.code_add(slot_offset);
stack.add_op("OpDatabase", self);       // alloc store; DbRef at v's slot
self.code_add(slot_offset);
self.code_add(tp_nr);
self.generate(&copy_val, stack, false); // unchanged
```

`OpDatabase(pos, tp)` is already positional — `pos` is an offset
from TOS, and the runtime implementation at `src/state/io.rs:628`
reads + writes the DbRef at `stack_pos - pos`.  We just reuse it
with `slot_offset` instead of the hard-coded `12`.

### Signature changes

`gen_set_first_ref_copy(stack, d_nr, value)` at codegen.rs:1222
currently does not take `v`.  Rename + add `v` parameter:

```rust
fn gen_set_first_ref_copy(&mut self, stack: &mut Stack, v: u16, d_nr: u32, value: &Value)
```

Its caller in `gen_set_first_at_tos` (codegen.rs:1146) already has
`v` — just pass it.  Inside, compute `slot_offset` as above.

The other three (`_var_copy`, `_tuple_copy`, `_call_copy`) already
take `v` — only the body changes.

### `gen_set_first_at_tos` simplification

Delete the slot-move
(`set_stack_pos(v, stack.position)` for `pos < TOS`, codegen.rs:1125).
Delete the gap-fill (moved into each `gen_set_first_*` callee's
own preamble).  The function becomes a pure type-dispatcher.

### Function-entry `OpReserveFrame(hwm)`

After argument layout in `def_code`, emit once:

```rust
let frame_hwm = stack.function.frame_hwm(&Context::Variable);
if frame_hwm > stack.position {
    let reserve = frame_hwm - stack.position;
    stack.add_op("OpReserveFrame", self);
    self.code_add(reserve);
    stack.position += reserve;
}
```

`Function::frame_hwm(&Context)` returns
`max(v.stack_pos + size(v, ctx))` over non-argument placed locals.
The B.3 attempt used `Context::Variable` for correct Text sizing
(24-byte `String`, not 16-byte `Str`) — reuse that.

### Per-block reserve deletion

In `generate_block` (currently codegen.rs:2010–2015), delete:

```rust
if block.var_size > 0 {
    stack.add_op("OpReserveFrame", self);
    self.code_add(block.var_size);
    stack.position += block.var_size;
}
```

The per-block `OpFreeStack` later in `generate_block` stays — it
only discards eval-stack residue above the block's result value,
not frame memory.

### Reassign-deep-copy branch

In `generate_set`, at the reassignment deep-copy code around
codegen.rs:973–988, the same pattern appears:

```rust
self.emit_push_null_ref(stack);          // OpReserveFrame(12) + OpInitRef(12)
stack.add_op("OpDatabase", self);
self.code_add(size_of::<DbRef>() as u16);
self.code_add(tp_nr);
let var_pos = stack.position - stack.function.stack(v);
stack.add_op("OpPutRef", self);
self.code_add(var_pos);
```

Rewrite slot-aware (drop the trailing `OpPutRef` — `OpDatabase`
already writes back to v's slot when its `pos` arg targets v):

```rust
// gap-fill if v.stack_pos > stack.position (unusual for reassign
// but handled uniformly).
if stack.function.stack(v) > stack.position {
    let gap = stack.function.stack(v) - stack.position;
    stack.add_op("OpReserveFrame", self);
    self.code_add(gap);
    stack.position += gap;
}
let slot_offset = stack.position - stack.function.stack(v);
stack.add_op("OpInitRef", self);
self.code_add(slot_offset);
stack.add_op("OpDatabase", self);
self.code_add(slot_offset);
self.code_add(tp_nr);
```

## Commit sequence

```
B.3.a  Function::frame_hwm(&Context::Variable) helper
         │ (reuse the impl from the broken attempt; it was correct
         │  in isolation.)
         ▼
B.3.b  gen_set_first_ref_copy:       add v param, use slot_offset
B.3.c  gen_set_first_ref_var_copy:   use slot_offset
B.3.d  gen_set_first_ref_tuple_copy: use slot_offset
B.3.e  gen_set_first_ref_call_copy:  use slot_offset
B.3.f  generate_set reassign branch: use slot_offset, drop OpPutRef
         │ (B.3.a–f are additive / behavior-preserving — slot-move
         │  still fires, so slot_offset == 12 in practice; every
         │  step green.)
         ▼
B.3.g  Verify: full issue suite + LOFT_SLOT_V2=validate shadow.
         │ (Gate: if anything regresses here, a `*_copy` path still
         │  has a hidden slot-move dependency; fix before
         │  proceeding.)
         ▼
B.3.h  Delete slot-move + gap-fill from gen_set_first_at_tos
       (~lines 1122–1131).
         │ (First destructive removal; bisectable against B.3.b–f.)
         ▼
B.3.i  def_code: emit OpReserveFrame(frame_hwm) at entry.
       generate_block: delete per-block OpReserveFrame(block.var_size).
         │ (The per-block `OpFreeStack` below the deleted reserve
         │  stays.)
         ▼
B.3.j  Delete dictionary entries + runtime impls for the 3 dormant
       compound ops: OpConvRefFromNull, OpNullRefSentinel,
       OpCreateStack (OpText already gone in B.2.b).
         │ - default/01_code.loft: remove 3 fn declarations.
         │ - src/fill.rs: regen (OPERATORS: 239 → 236).
         │ - src/state/text.rs: remove `create_stack` impl.
         │ - src/state/codegen.rs: remove the 3 interception cases
         │   in generate_call.
         │ - Rewrite parser `cl(…)` call sites to emit the
         │   decomposed form directly (verify via
         │   `grep 'cl("Op(CreateStack|ConvRefFromNull|NullRefSentinel)"'`).
```

Steps B.3.a–g are additive / refactoring, every one green.  Step
B.3.h is the first destructive removal but it's scoped: if a
regression appears, bisect tells us which path still depends on
slot-move, and B.3.b–f can be patched.  Step B.3.i is the main
win.  Step B.3.j is cosmetic cleanup.

## Verification per step

After each commit:

- `cargo check` clean.
- `cargo test --release --test issues` — 500 passed, 2 ignored.
- `cargo test --test issues fill_rs_up_to_date` — green (critical
  for B.3.j).
- Canaries:
  - `p162_return_match_struct_enum_native` — outer-scope /
    inner-Set shape.
  - `p178_is_capture_slot_alias` — orphan placer.
  - `p181_inline_field_access_format_string` — `slot != TOS`
    sensitive.
  - `fn_ref_basic_call` — the broken-attempt canary.
- `LOFT_SLOT_V2=validate cargo test --test slot_v2_baseline` —
  shadow validator green.
- Before claiming B.3 done: `./scripts/find_problems.sh --bg
  --wait`.

## Files touched

| File | Change | Step |
|---|---|---|
| `src/variables/mod.rs` | `Function::frame_hwm` helper | B.3.a |
| `src/state/codegen.rs::gen_set_first_ref_copy` | Take `v`, compute + use `slot_offset` | B.3.b |
| `src/state/codegen.rs::gen_set_first_ref_var_copy` | Use `slot_offset` | B.3.c |
| `src/state/codegen.rs::gen_set_first_ref_tuple_copy` | Use `slot_offset` | B.3.d |
| `src/state/codegen.rs::gen_set_first_ref_call_copy` | Use `slot_offset` | B.3.e |
| `src/state/codegen.rs::generate_set` reassign path | Use `slot_offset`; drop trailing `OpPutRef` | B.3.f |
| `src/state/codegen.rs::gen_set_first_at_tos` | Delete slot-move + gap-fill | B.3.h |
| `src/state/codegen.rs::def_code` | Emit `OpReserveFrame(frame_hwm)` | B.3.i |
| `src/state/codegen.rs::generate_block` | Delete per-block `OpReserveFrame(block.var_size)` | B.3.i |
| `default/01_code.loft` | Delete `fn OpConvRefFromNull`, `OpNullRefSentinel`, `OpCreateStack` | B.3.j |
| `src/state/*.rs` | Delete `conv_ref_from_null` / `null_ref_sentinel` / `create_stack` impls | B.3.j |
| `src/fill.rs` | Regenerate (OPERATORS: 239 → 236) | B.3.j |
| `src/state/codegen.rs::generate_call` | Remove 3 interception cases | B.3.j |
| `src/parser/{mod,control,objects,operators}.rs` | Rewrite `cl(…)` call sites | B.3.j |

## Risk and rollback

- **B.3.a–g**: additive.  Full test suite gates each step; no
  regression possible without catching it immediately.
- **B.3.h**: deletion of slot-move.  Medium risk; bisectable
  against B.3.b–f.  If any path still expects
  `v.stack_pos == stack.position` post-call, regression.
- **B.3.i**: function-entry reserve.  Conceptually clean;
  bytecode changes at every function.  Snapshot tests
  (`tests/dumps/*.txt`) may need regeneration.
- **B.3.j**: cosmetic.  Zero behavior change (interception
  already routes everything through decomposed helpers).

Each step is its own commit.  Revert is narrow per step.  If
B.3.j hits parser friction, defer indefinitely — the 3 ops are
dead runtime code after B.3.i, so keeping dictionary entries is
harmless.

Each `_copy` function has historical subtleties worth re-reading
before editing:

- `gen_set_first_ref_call_copy` — P143 store-lock bracket around
  the call; `n_set_store_lock` calls are void and leave
  `stack.position` unchanged, so `slot_offset` stays valid.
- `gen_set_first_ref_var_copy` — O-B1 last-use move (skips the
  deep copy entirely when the source is used exactly once).
- `gen_set_first_ref_copy` — O-B2 fresh-store adoption (skips
  the allocation entirely when the callee has no Reference
  params).
- `generate_set` reassign — P150 tolerance for work-refs aliased
  across loop iterations.

## Estimated effort

**2–3 focused days** to land B.3.a–j cleanly.  Realistically
4–5 if any of B.3.c–f hits an unexpected edge case.

## What ships without B.3

Phase A + B.1 + B.2 (current `develop`).  The 3 compound ops
remain dictionary entries — parser-callable, codegen-decomposed,
runtime-dormant.  Per-block `OpReserveFrame` + `OpFreeStack`
still run.  `gen_set_first_at_tos` keeps slot-move.  The
architectural win of positional primitives is fully delivered;
only the bytecode slimming and full opcode deletion are
deferred.
