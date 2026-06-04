<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Unified store-backed yield channel

**Status: SHIPPED 2026-05-23 (first user: tuple yields, closing @P327
native).  Identified mid-session while attempting to land @P327 native
the hardcoded way; the per-shape channel approach (one trait method +
one runtime helper per yield type — `next_i64`, `next_text`,
`next_dbref`, then a hypothetical `next_tuple_2i64`,
`next_tuple_3i64`, …) reproduces the closure combinatorial problem
that @PLAN15 spent significant effort recovering from.  This phase
introduces the unified channel; legacy channels remain for migration
one yield-shape at a time.**

### What shipped

- `LoftCoroutine::next_into(&mut self, &mut Stores, &mut [i64]) -> bool`
  trait method (`src/codegen_runtime.rs`).
- `coroutine_next_into(gen_ref, stores, dest) -> bool` runtime helper.
- `value_size` packs (low byte = byte size, high byte = channel tag).
- Three call sites encode the high-byte tag for tuple-of-(integer|float)
  yields:
  * `parser/collections.rs::iterator` (for-loop driver)
  * `parser/control.rs` (manual `next()`)
  * `generation/coroutine.rs::emit_next_i64` (state-machine impl)
- Interp masks the tag off in TWO places (caught mid-session): the
  bytecode op (`fill.rs::coroutine_next` via the `& 0xFF` template in
  `default/02_images.loft`) AND the SLOT ALLOCATOR's
  `OpCoroutineNext` arm in `state/codegen.rs` — the latter is where
  the half-finished first attempt missed a mask and corrupted
  consumer-frame var offsets (`Variable 280 outside stack 84`).
- Native dispatch (`generation/ops/coroutine.rs::OpCoroutineNextEmitter`)
  reads both bytes, emits `let mut _loft_yield_buf: [i64; N] = [0; N];
  coroutine_next_into(...); (buf[0], buf[1], ...)`.
- State machine impl writes per-slot `dest[i] = (val) as i64;` and
  returns `true` (exhaustion: `false`).
- Regression: `tests/issues.rs::p327_native_iterator_of_tuple_for_loop`
  + `p327_native_iterator_of_tuple_manual_next` +
  matrix cells `y4_x1_tuple_for_loop` + `y4_x2_tuple_manual_next`
  (16/16 matrix cells now green).

## Why

Currently the coroutine state machine dispatches on the yielded value's
shape via per-type trait methods:

| Yield type    | Channel   | Runtime helper            | Shipped |
|---|---|---|---|
| integer / float / boolean / char | `next_i64` | `coroutine_next_i64` | yes |
| text                             | `next_text`  | `coroutine_next_text`  | yes (@P211) |
| Reference / vector / struct-enum | `next_dbref` | `coroutine_next_dbref` | yes (@P326) |
| `(integer, integer)` tuple       | (none)       | (none)                 | OPEN (@P327 native) |
| `(integer, integer, integer)`    | (none)       | (none)                 | OPEN |
| `(text, integer)`                | (none)       | (none)                 | OPEN |
| `(Struct, integer)`              | (none)       | (none)                 | OPEN |
| `iterator<closure>`              | (uses dbref) |                        | @PLAN16 Y5 phase 05 |

The pattern that almost shipped on 2026-05-23: add `next_tuple_2i64`
returning `(i64, i64)`, encode tuple yields in the high byte of
`value_size`, dispatch in `OpCoroutineNextEmitter`.  This works for
2-tuples of ints — and only for 2-tuples of ints.  Each new arity (3,
4, 5) or element-mix (with text, with DbRef) needs another trait
method, runtime helper, dispatch arm, and codegen path.  That is
exactly the shape @PLAN15 ran into for closures before the closure
record + DbRef-based dispatch landed.

The closure resolution was: instead of typing closures by their
captured shape, allocate a closure RECORD (a struct in a Store) and
pass DbRefs around.  Every closure shape gets the same trait dispatch;
the captured state lives in the record.

## What

Introduce ONE trait method that covers all current and future yield
shapes by writing the yielded value's bytes into a caller-provided
buffer:

```rust
pub trait LoftCoroutine {
    /// Existing per-type channels — kept for migration.
    fn next_i64(&mut self, _stores: &mut Stores) -> i64 { COROUTINE_EXHAUSTED }
    fn next_text(&mut self, _stores: &mut Stores) -> String { STRING_NULL.to_string() }
    fn next_dbref(&mut self, _stores: &mut Stores) -> DbRef { /* null sentinel */ }

    /// Unified channel — writes the yielded bytes into `dest` (sized
    /// by the consumer for the yielded type's layout) and returns
    /// true.  Returns false on exhaustion.  Each yield shape encodes
    /// its OWN layout in this single method — no new trait method per
    /// shape.  This is the migration target for the per-type channels
    /// above; each migrates one shape at a time, eventually leaving
    /// `next_into` as the sole channel.
    fn next_into(&mut self, _stores: &mut Stores, _dest: &mut [i64]) -> bool {
        false
    }
}
```

Runtime helper:

```rust
pub fn coroutine_next_into(
    gen_ref: DbRef,
    stores: &mut Stores,
    dest: &mut [i64],
) -> bool {
    NATIVE_COROUTINES.with(|c| {
        let mut coroutines = c.borrow_mut();
        let idx = gen_ref.rec as usize;
        if let Some(slot) = coroutines.get_mut(idx)
            && let Some(coro) = slot.as_mut()
        {
            let ok = coro.next_into(stores, dest);
            if !ok { coroutines[idx] = None; }
            ok
        } else { false }
    })
}
```

Consumer codegen (for `iterator<(integer, integer)>`):

```rust
let mut tuple_buf: [i64; 2] = [0; 2];
'l3: loop {
    if !coroutine_next_into(gen, stores, &mut tuple_buf) { break; }
    let var_p: (i64, i64) = (tuple_buf[0], tuple_buf[1]);
    // ... body ...
}
```

State-machine impl per coroutine:

```rust
impl LoftCoroutine for NPairsGen {
    fn next_into(&mut self, stores: &mut Stores, dest: &mut [i64]) -> bool {
        match self.state {
            0 => { self.state = 1; dest[0] = 1; dest[1] = 10; true }
            1 => { self.state = 2; dest[0] = 2; dest[1] = 20; true }
            ...
            _ => false,
        }
    }
}
```

## Scope of phase 01

1. Add `next_into` + `coroutine_next_into` to the trait + runtime.
2. Route tuple-of-i64 yields through `next_into`.
3. **Crucially:** preserve interp behaviour byte-for-byte — the interp
   uses bytecode dispatch (not the trait), so the IR's `value_size`
   field must still tell it the EXACT byte size to push for the
   exhaustion null sentinel.  The native channel discriminator lives
   in a SEPARATE arg, not in `value_size` (the half-finished 2026-05-23
   attempt used the high byte of `value_size` for the channel and
   broke the interp's slot allocator — the unified design must keep
   `value_size` strictly as byte size).
4. Cross-mode harness `tests/coroutine_matrix.rs::y4_*` cells un-ignore
   for tuple-of-i64 arities 2, 3, 4 once the channel is in.
5. NO migration of existing per-type channels in this phase — they
   stay parallel.  Migration is phase 02+.

## Open design questions

- **IR encoding.** `OpCoroutineNext` currently takes `[gen, value_size]`.
  Adding a 3rd arg (`channel_tag: const u8`) is the cleanest way to
  pass the discriminator without touching `value_size`.  Opcode arg
  changes need careful staging — fill.rs is regenerated, callers
  emit the new arg.
- **Heterogeneous tuples.** `(integer, text)` has variable-size
  payload (text is a String allocation).  Pure `[i64]` buffers can't
  carry strings without indirection.  Options: (a) defer hetero
  tuples and stay on the per-type channels for them; (b) encode an
  out-of-band String table per yield; (c) write the String into a
  caller-allocated text slot in `stores`.
- **DbRef migration.** `next_dbref` already returns 12 bytes (one
  DbRef).  Migrating to `next_into` means writing those 12 bytes into
  `dest[0..2]` (with one slot of slack).  Mechanically straightforward
  but breaks the `next_dbref` callsites in `@P326`'s regression tests
  — needs simultaneous codegen + test update.

## Acceptance

- `next_into` trait method + `coroutine_next_into` helper land.
- `IR::OpCoroutineNext` gains a 3rd arg for the channel tag (or
  equivalent encoding — but NOT in `value_size`).
- `tests/coroutine_matrix.rs::y4_x1_tuple_for_loop` passes cross-mode.
- The doc-hygiene gate (`tests/index_hygiene.rs`) stays green.
- @P327 native row in PROBLEMS.md closes; the fast-index entry
  describes the unified channel as the actual fix.
