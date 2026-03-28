# Safety Analysis — Parallel Workers, Store Allocation, and Coroutines

This document records safety analyses of the runtime memory model with
design-level mitigations for each identified risk.

- **Part 1** covers the parallel worker system (`src/parallel.rs`,
  `src/database/allocation.rs`, `src/store.rs`).
- **Part 2** covers the coroutine system (`src/state/mod.rs`, `CoroutineFrame`,
  `stack_bytes`, `text_owned`).

---

## Contents

### Part 1 — Parallel Workers and Store Allocation
- [P1 Architecture Summary](#p1-architecture-summary)
- [P1 What is Safe](#p1-what-is-safe)
- [P1-R1 — Silent data loss in release builds](#p1-r1--silent-data-loss-in-release-builds)
- [P1-R2 — `out_ptr` lifetime not type-enforced](#p1-r2--out_ptr-lifetime-not-type-enforced)
- [P1-R3 — `claims` HashSet overhead in locked clones](#p1-r3--claims-hashset-overhead-in-locked-clones)
- [P1-R4 — `max` cascade panic with freed mid-slots](#p1-r4--max-cascade-panic-with-freed-mid-slots)
- [P1-R5 — No Rust-type-level proof of non-aliasing](#p1-r5--no-rust-type-level-proof-of-non-aliasing)
- [Part 1 Summary Table](#part-1-summary-table)

### Part 2 — Coroutines, Stores, and Strings
- [P2 Architecture Summary](#p2-architecture-summary)
- [P2 What is Safe](#p2-what-is-safe)
- [P2-R1 — Text argument `Str` dangles on first resume](#p2-r1--text-argument-str-dangles-on-first-resume)
- [P2-R2 — `String` objects leaked at exhaustion](#p2-r2--string-objects-leaked-at-exhaustion)
- [P2-R3 — Text locals have implicit "never freed" invariant](#p2-r3--text-locals-have-implicit-never-freed-invariant)
- [P2-R4 — `text_positions` inconsistent across yield/resume](#p2-r4--text_positions-inconsistent-across-yieldresume)
- [P2-R5 — Store-backed `Str` dangles on record delete](#p2-r5--store-backed-str-dangles-on-record-delete)
- [P2-R6 — Compiler check for `yield` inside `par()` missing](#p2-r6--compiler-check-for-yield-inside-par-missing)
- [P2-R7 — Exhausted frames never freed](#p2-r7--exhausted-frames-never-freed)
- [P2-R8 — `DbRef` locals outlive their store across suspension](#p2-r8--dbref-locals-outlive-their-store-across-suspension)
- [P2-R9 — `e#remove` on a generator iterator corrupts unrelated records](#p2-r9--eremove-on-a-generator-iterator-corrupts-unrelated-records)
- [P2-R10 — Yielded `Str` value lifetime is not enforced at the consumer](#p2-r10--yielded-str-value-lifetime-is-not-enforced-at-the-consumer)
- [Part 2 Summary Table](#part-2-summary-table)

### Combined
- [All Issues — Quick Reference](#all-issues--quick-reference)
- [See also](#see-also)

---

## All Issues — Quick Reference

Effort scale: **XS** < 4 h · **S** 1–2 d · **M** 3–5 d · **L** 1–2 wk · **XL** > 2 wk.
Where two values are shown, the first is the short-term fix and the second is the
full long-term design.

| ID | Severity | Effort | Key files | Short-term action |
|---|---|---|---|---|
| **P1-R1** | medium | S | `store.rs`, `parser/expressions.rs` | Remove `#[cfg(debug_assertions)]` guard on auto-lock; promote dummy-buffer to panic |
| **P1-R2** | low/medium | XS / S | `parallel.rs` | `// SAFETY:` comment + debug assert; replace `spawn` with `thread::scope` |
| **P1-R3** | low | XS | `store.rs`, `database/allocation.rs` | `clone_locked_for_worker` omitting `claims` |
| **P1-R4** | medium | XS / M | `database/allocation.rs` | LIFO debug assert (XS); free-bitmap replacing cascade (M) |
| **P1-R5** | low | M / L | `database/allocation.rs`, `parallel.rs`, `keys.rs` | `WorkerStores` newtype (M); `DbRef` origin tag (L) |
| **P2-R1** | critical | L † | `state/mod.rs` (`coroutine_create`) | Debug assert if text args present; implement `serialise_text_slots` at create |
| **P2-R2** | high | XS † | `state/mod.rs` (`coroutine_return`) | Drain `text_owned` before `stack_bytes.clear()` |
| **P2-R3** | high | L † | `state/mod.rs` (`coroutine_yield`, `coroutine_next`) | Debug assert on text slots; implement CO1.3d atomically |
| **P2-R4** | medium | S | `state/mod.rs`, `data.rs` (`CoroutineFrame`) | Save/restore `text_positions` set on yield/resume (debug only) |
| **P2-R5** | medium | S / S | `state/mod.rs`, `store.rs` | Document rule; pointer-range heuristic in `coroutine_yield` |
| **P2-R6** | medium | S | `parser/collections.rs`, `state/mod.rs` | `inside_par_body` flag + out-of-bounds guard in `coroutine_next` |
| **P2-R7** | low | M | `fill.rs`, `state/mod.rs`, `state/codegen.rs` | `OpFreeCoroutine` emitted at for-loop exit |
| **P2-R8** | medium | M / XL | `store.rs`, `database/`, `state/mod.rs` | Generation counter on `Store`; save+check in frame (M); flow analysis (XL) |
| **P2-R9** | medium | XS | `parser/fields.rs`, `database/search.rs` | Compiler rejection of `e#remove` on generator; guard in `remove()` |
| **P2-R10** | low | XS / XL | docs | Document ownership rule; `iter_text` type (XL language design) |

† P2-R1, P2-R2, and P2-R3 share the CO1.3d implementation (combined effort **L**, 1–2 weeks).
  They must land atomically — partial implementation is more dangerous than none.

---

## Part 1 — Parallel Workers and Store Allocation

---

## P1 Architecture Summary

Every `run_parallel_*` entry-point in `src/parallel.rs` creates **one fully
independent `Stores` clone per worker thread** via `stores.clone_for_worker()`
(`src/database/allocation.rs`).  That clone is moved into the spawned thread;
the main thread's `stores` is not touched while workers run.

Worker isolation flow:

```
main thread Stores
    └── clone_for_worker()          — deep-copies every active store
            ├── active slots  → clone_locked()   (locked: true, full byte-copy)
            └── freed slots   → Store::new(100)  (fresh, free: true)
    └── moved into thread::spawn(move || …)
            └── State::new_worker(worker_stores, …)
                    └── Stores::database()       — allocates worker stack at index max
```

No `Stores` instance, `Store`, or heap buffer owned by one worker is shared with
another worker or with the main thread (with the exception analysed in Risk 2 below).

---

## P1 What is Safe

### Store memory is fully deep-copied

`clone_locked` (`store.rs`) does:

```rust
std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8);
```

Every record, including string data, is copied into a fresh independent
allocation.  Strings are stored inline in the store as 32-bit word-offsets; after
byte-copy the offsets resolve correctly against the clone's own `ptr`.  Workers
never access the original store's memory.

### Worker-owned allocations never overlap cloned stores

When a worker function creates a new struct it calls `Stores::database` on its
private `Stores`.  At clone time `self.max` equals the original value (say *N*)
and `allocations.len() == N`, so the first worker allocation pushes a fresh
unlocked store at index *N* — beyond all locked clones at `0..N-1`.

### Locked clones enforce read-only access in debug builds

Every active store given to a worker is marked `locked: true`.  In debug builds,
`addr_mut`, `claim`, and `delete` all `debug_assert!(!self.locked)` and panic
immediately on any write attempt.  This converts accidental mutations into
fail-fast panics during development.

### Non-overlapping direct writes are safe

`run_parallel_direct` writes results via `out_ptr.add(row_idx * ret_sz)`.
Thread *t* owns indices `[t * n_rows / threads, (t+1) * n_rows / threads)`.
The last thread ends at `(threads * n_rows) / threads == n_rows`, so all ranges
tile `[0, n_rows)` without gaps or overlap.  All threads are joined before the
caller reads the buffer.

### `WorkerProgram` sharing is safe

Bytecode, text, and library are wrapped in `Arc` and never mutated after
construction.  The manual `unsafe impl Send + Sync` is justified by the
read-only invariant.

### Result collection is sequential

Channel-based paths send batches to the main thread after joining workers.
`copy_from_worker` (the store-graft deep-copy used for struct returns) is called
sequentially on the main thread — no concurrent store access.

---

## P1-R1 — Silent data loss in release builds

### Description

If a worker function writes to a field of its locked cloned input store in a
release build, the write is silently discarded into a thread-local 256-byte
dummy buffer.  The worker's computation may then observe stale or wrong data and
return incorrect results **with no error or panic**.

The auto-locking of `const` arguments is currently guarded by
`#[cfg(debug_assertions)]` (`parser/expressions.rs`), so release builds never
auto-lock.  A buggy worker that is supposed to be read-only can silently corrupt
results in production while passing all debug-mode tests.

### Mitigation design

**M1-a — Enable auto-locking unconditionally for `const` worker arguments**

Remove the `#[cfg(debug_assertions)]` guard on the auto-lock insertion in
`parse_code` and `expression` (the two sites that emit `n_set_store_lock` for
`const` parameters and local const variables).

Locking a store is a single flag write.  The only reason it was gated to debug
was to avoid the branch cost on every call; profiling shows the overhead is
negligible compared to the cost of a parallel dispatch.

**M1-b — Promote the write-to-locked-store path in release to a runtime error**

Change the release-build silent-discard path in `addr_mut` from a dummy buffer
return to an explicit `panic!` (or structured error).  The dummy buffer was
added to keep release builds from segfaulting; once M1-a ensures const stores
are always locked, no legitimate code path should hit it.  Making it a visible
failure removes the silent-corruption window.

**M1-c — Add a release-mode integration test**

Add a `#[test]` that compiles and runs with `--release` and asserts the result
of a `par(...)` loop whose worker accidentally writes to its input equals the
expected value.  If M1-a and M1-b are in place the test would panic with a
clear message instead of returning the wrong answer.

---

## P1-R2 — `out_ptr` lifetime not type-enforced

### Description

`run_parallel_direct` accepts a raw `*mut u8`.  The safety invariant — the
buffer must remain live until all threads are joined — is upheld today because
`thread::join` is called before the function returns.  But the Rust type system
does not enforce this.  A future refactor that moves or removes the join (e.g.
to allow early cancellation or deferred result collection) could introduce a
data race or use-after-free with no compile-time warning.

### Mitigation design

**M2-a — Wrap the output slice in a scoped-thread lifetime**

Replace `thread::spawn` with `std::thread::scope` (stabilised in Rust 1.63).
Scoped threads borrow data from the enclosing stack frame, so the compiler
enforces that the buffer outlives all threads:

```rust
std::thread::scope(|s| {
    for t in 0..threads {
        let out_slice = &mut out[t_start * ret_sz .. t_end * ret_sz];
        s.spawn(move || {
            // write into out_slice — lifetime enforced by scope
        });
    }
    // scope end: all threads joined here by the compiler
});
```

This eliminates `SendMutPtr`, the manual join loop, and the lifetime comment.
The only cost is that scoped threads cannot be detached, which is not a
requirement here.

**M2-b — Short term: add a safety comment with an invariant assertion**

Until M2-a lands, add a `// SAFETY:` block above every `SendMutPtr` use stating
the join invariant explicitly, and a `debug_assert!` after the join loop
verifying all handles are consumed.

---

## P1-R3 — `claims` HashSet overhead in locked clones

### Description

`clone_locked` copies `self.claims` (the set of all live record word-offsets,
used by `validate()`).  Workers with locked stores never call `validate()` and
never mutate claims, so the clone is wasted memory — O(*records*) allocation per
worker per `par(...)` call.

For programs with many long-lived records this can add measurable allocation
pressure when spawning many workers.

### Mitigation design

**M3-a — Skip `claims` in worker clones**

Add a constructor parameter or a dedicated `clone_for_worker` method on `Store`
that omits the `claims` clone:

```rust
pub fn clone_locked_for_worker(&self) -> Store {
    Store {
        claims: HashSet::new(),   // empty — workers never validate
        locked: true,
        free_root: 0,
        // … rest same as clone_locked
    }
}
```

`clone_for_worker` in `Stores` would call this variant instead of `clone_locked`.
The existing `clone_locked` (used elsewhere) is unchanged.

---

## P1-R4 — `max` cascade panic with freed mid-slots

### Description

**Reproducer:**
1. Original `Stores` has slots: 0 = live, 1 = freed, 2 = live (so `max = 3`,
   `allocations[1].free = true`).
2. `clone_for_worker` produces: 0 = locked_clone, 1 = fresh_free, 2 = locked_clone,
   `max = 3`.
3. Worker calls `database()` → pushes slot 3 (new, unlocked, `max = 4`).
4. Worker calls `free(slot_3)` → marks slot 3 free, cascade: `max` tries `4 → 3`,
   slot 2 has `free = false` (it is a locked clone), cascade stops at `max = 3`.
5. Worker calls `database()` again → `self.max (3) >= allocations.len() (4)` is
   false, so it calls `allocations[3].init()`.  Slot 3 was already freed (step 4
   set `free = true`), so `init()` succeeds and `max` becomes 4 again.

Wait — step 5 actually works if the slot is truly free.  The real failure path is
when the cascade overshoots into a locked-clone slot:

- Original: 0 = live, max = 1.  Worker slot push: max = 2 (slot 1 = worker).
- Worker frees slot 1: `max = 1`, cascade: slot 0 is `free = false` → stops.
  OK so far.
- Worker pushes again: `max (1) >= len (2)` → false → `allocations[1].init()`.
  Slot 1 (previous worker allocation, now freed) has `free = true` → assert
  passes.  `max = 2`.  OK.

The failing case requires the cascade to reach a slot with `free = false` that
is *below* `max` in the worker's view.  That happens when a worker frees its
*only* worker-created slot and the cascade hits slot `max-1` which is a locked
clone:

- Original: 0 = live (locked), `max = 1`.  Worker creates slot 1 (`max = 2`).
  Worker frees slot 1: `max → 1`.  Cascade checks slot 0: `free = false` →
  stops.  `max = 1`.
- Worker creates slot 1 again: `max (1) < len (2)` → `init()` slot 1.
  Slot 1 is `free = true` (from step above) → assert passes.  OK.

So the cascade itself is safe in the current logic.  The real panic would be:
- Worker frees a slot that has `store_nr < max - 1` (non-LIFO), triggering the
  LIFO debug assert `"Double free store"` or the `al == self.max - 1` check
  causing `max` *not* to decrement.

The LIFO-order requirement on `free()` is documented but not enforced in non-debug
builds.  When native-codegen code (`OpFreeRef`) frees stores out of order, `max`
stalls, slots leak, and subsequent `database()` calls eventually try to allocate
a slot that `free == false`.

### Mitigation design

**M4-a — Enforce LIFO order via a debug-build audit log**

The existing `LOFT_STORE_LOG` env-var logs alloc/free events.  Add a
`debug_assert` in `free_named` that verifies the freed slot equals `self.max - 1`
(strict LIFO), and emit the full alloc/free trace to a thread-local buffer on
violation so the error message shows which store broke ordering.

**M4-b — Replace LIFO scan with a free-bitmap**

Replace the `while max > 0 && allocations[max-1].free { max -= 1; }` cascade
with a bitset (`u64` array) tracking which slots are free.  `database` finds the
lowest free bit; `free` sets the bit.  `max` tracks the highest live slot for
boundary checks:

```
free_bits: [u64; MAX_STORES / 64]  — bit set = slot is free
max: u16                            — highest ever used index + 1
```

`database`:
1. Find lowest set bit in `free_bits` below `max` (first reuse slot).
2. If none, grow `max` and use the new slot.
3. Clear the bit, set `store.free = false`.

`free`:
1. Set bit for `store_nr`.
2. If `store_nr == max - 1`, trim `max` down to the highest cleared bit.

This eliminates the LIFO requirement entirely, makes store reuse O(1), and
removes the fragile cascade logic.  A worker creating and freeing stores in any
order would work correctly.

**M4-c — Short term: document the LIFO invariant prominently**

Until M4-b lands, add a `// INVARIANT: free() must be called in LIFO order`
comment in `free_named`, and assert it in debug builds (`al == self.max - 1`).

---

## P1-R5 — No Rust-type-level proof of non-aliasing

### Description

The architecture relies on:
1. The loft compiler enforcing `const` on worker arguments.
2. The runtime store lock catching violations in debug builds.
3. Convention that worker functions "may not write to shared state".

Rust's type system does not prevent a worker closure from capturing a `*mut`
pointer to main-thread data and writing through it, nor does it prevent a worker
from holding a `DbRef` whose `store_nr` belongs to the main thread.

This is acceptable for the current architecture but is an invariant that can
silently break if the parallel dispatch is extended (e.g. to allow workers to
receive mutable references for output accumulation).

### Mitigation design

**M5-a — Encode worker-store ownership in a newtype**

Introduce a `WorkerStores(Stores)` newtype that:
- Can only be constructed by `clone_for_worker` (private constructor).
- Exposes only `&Stores` (immutable) to the main thread after workers finish,
  never `&mut`.
- Is `Send` but not `Sync`, ensuring it cannot be shared across threads.

Worker closures receive `WorkerStores`; they can allocate into their private
portion but cannot be handed a raw pointer back to the main thread's stores.

**M5-b — Mark `DbRef` values from main-thread stores with a lifetime or tag**

Long term: add a `origin: StoreOrigin` field to `DbRef` (or an index range
`[0, worker_base)` vs `[worker_base, …]`) so that the runtime can assert in
debug mode that a worker does not store a main-thread `DbRef` into a result that
will be merged back, bypassing the `copy_from_worker` deep-copy path.

---

## Part 1 Summary Table

| Risk | Severity | Effort | Short-term fix | Long-term design |
|---|---|---|---|---|
| P1-R1 — Silent write-discard in release | **medium** | S | Remove `#[cfg(debug_assertions)]` guard on auto-lock | Promote dummy-buffer path to panic (M1-b), add release integration test (M1-c) |
| P1-R2 — `out_ptr` lifetime not type-enforced | **low/medium** | XS / S | ✓ S29: `thread::scope` (M2-a) + `// SAFETY:` comment (M2-b) in `run_parallel_direct` | Done |
| P1-R3 — `claims` cloned into locked workers | **low** | XS | ✓ S29: `clone_locked_for_worker` omits `claims` (M3-a) | Done |
| P1-R4 — LIFO violation stalls `max` / panic | **medium** | XS / M | ✓ S29: free-bitmap M4-b supersedes LIFO assert; non-LIFO frees now safe | Done |
| P1-R5 — No type-level non-aliasing proof | **low** | M / L | ✓ S30: `WorkerStores` newtype (M5-a) | `DbRef` origin tagging (M5-b) remains long-term |

---

## Part 2 — Coroutines, Stores, and Strings

---

## P2 Architecture Summary

A suspended `CoroutineFrame` lives in `State::coroutines: Vec<Option<Box<CoroutineFrame>>>`,
entirely outside the `Store`/`Stores` system.  It holds two Rust-heap structures
that reference loft memory:

- **`stack_bytes: Vec<u8>`** — raw byte copy of the generator's stack locals at
  the last suspension point.  For text variables this encodes inline `String`
  objects (`ptr + len + cap`).  For text arguments and yielded text values it
  encodes `Str { ptr: *const u8, len: u32 }` pointing into external storage.
- **`text_owned: Vec<(u32, String)>`** — designed to hold owned copies of all
  dynamic text slots after serialisation (SC-CO-1/SC-CO-8 mitigations), with the
  `u32` being the byte offset within `stack_bytes` to patch on resume.

`text_owned` is always empty in the current implementation.  The full
serialisation path (`serialise_text_slots` / `free_dynamic_str`) is described in
COROUTINE.md but is not yet implemented (deferred as CO1.3d).

Store records are referenced only via `DbRef` values serialised as raw bytes into
`stack_bytes`.

### Coroutine lifecycle

```
OpCoroutineCreate  →  frame.stack_bytes = raw copy of arg bytes; text_owned = []
                       (arg Str pointers unowned — dangles if caller frees text)
OpCoroutineNext    →  raw copy of stack_bytes → live stack; no Str patching
OpYield            →  raw copy of live stack → stack_bytes; no text serialisation
OpCoroutineReturn  →  stack_bytes.clear(); text_owned.clear()
                       (String objects in stack_bytes are dropped as raw bytes — LEAK)
```

---

## P2 What is Safe

**Re-entrant advance detection** — `active_coroutines.contains(&idx)` prevents a
running generator from being advanced again.  The `Vec<usize>` correctly tracks
all simultaneously active frames under `yield from` nesting (SC-CO-3, SC-CO-9
resolved). ✓

**Stack base relocation** — `frame.stack_base = self.stack_pos` is reset at every
resume, so restored bytes always land above the caller's current stack top.
Caller locals pushed after creation are never overwritten (SC-CO-7 resolved). ✓

**Null iterator guards** — `coroutine_next` and `coroutine_exhausted` check
`store_nr != COROUTINE_STORE || rec == 0` before touching any frame. ✓

**`text_owned` offset is `u32`** — `Vec<(u32, String)>` gives 4 GB headroom;
no truncation for deep frames (SC-CO-12 resolved). ✓

---

## P2-R1 — Text argument `Str` dangles on first resume

### Description

**Severity: critical — use-after-free**

`coroutine_create` copies argument bytes verbatim from the live stack into
`stack_bytes`:

```rust
std::ptr::copy_nonoverlapping(src, stack_bytes.as_mut_ptr(), args_size as usize);
// text_owned stays empty — CO1.3d will handle text serialisation.
```

Text arguments are passed as `Str { ptr: *const u8, len: u32 }` — a zero-copy
reference into the **caller's** owned `String`.  After `OpCoroutineCreate`, the
caller continues executing normally.  When the caller's text variable goes out of
scope, `OpFreeText` frees the `String`.  The `Str` bytes copied into `stack_bytes`
now hold a dangling pointer.

On the first `OpCoroutineNext`, those bytes are copied back to the live stack:

```rust
std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
```

The generator executes with a dangling `Str` in its parameter slot.  Any read of
that text parameter is a use-after-free.

Static string literals (`Str.ptr` into `text_code`) are permanently live and
are not affected.

### Mitigation design

**M6-a — Implement `serialise_text_slots` at `OpCoroutineCreate`**

The COROUTINE.md design (CO1.3d) already specifies the fix: after copying the
argument bytes, call `serialise_text_slots` to transfer ownership of every
dynamic-text `Str` slot:

```rust
// In coroutine_create, after the raw byte copy:
let text_owned = serialise_text_slots(
    &mut stack_bytes,
    &def.text_arg_slots,   // (byte_offset, Type) pairs for text parameters
    &mut self.database,
);
frame.text_owned = text_owned;
```

`serialise_text_slots` must:
1. Read each `Str` slot from the bytes.
2. Skip null `Str` (ptr == STRING_NULL) and static `Str` (ptr inside `text_code`).
3. Call `s.str().to_owned()` to make an independent `String`.
4. Call `database.free_dynamic_str(ptr)` to release the original allocation
   (matching `OpFreeText` semantics).
5. Write a `Str` pointing to the owned buffer into `stack_bytes`.
6. Record `(offset as u32, owned_string)` in `text_owned`.

**M6-b — Implement the pointer-patch step in `coroutine_next`**

Before copying `stack_bytes` to the live stack, patch each `text_owned[i].1`
buffer address back into the corresponding slot in `stack_bytes`:

```rust
for (offset, s) in &frame.text_owned {
    let patched = Str::new(s.as_str());
    write_str_at(&mut frame.stack_bytes, *offset as usize, patched);
}
```

The `String` buffer address is stable as long as the `String` is not pushed or
grown between the patch and the copy.  No extra allocation is required.

**M6-c — Implement `free_dynamic_str` in `Stores` / `State`**

This function must match how `OpFreeText` releases a dynamic string in `text.rs`.
The mechanism depends on whether the runtime uses a scratch/side-table of
`String` objects or direct heap addresses.  Determine the correct call and add it
to `database::Stores` or as a `State` helper before CO1.3d lands.

---

## P2-R2 — `String` objects leaked at exhaustion

### Description

**Severity: high — memory leak on every generator with text locals that yields**

`coroutine_return` clears the frame's saved state:

```rust
frame.text_owned.clear();
frame.stack_bytes.clear();
```

`Vec<u8>::clear()` drops the `Vec`'s own backing allocation but treats the
contained bytes as plain scalars — no element destructor is called.  If
`stack_bytes` encodes `String` objects (text local variables are stored inline
on the stack as `String { ptr, len, cap }` structs), those `String`s' internal
heap allocations are **never freed**.

This affects every generator that:
1. Has at least one text local variable, **and**
2. Yields at least once before exhausting (so `stack_bytes` was written with
   live `String` bytes at the last `OpYield`).

Additionally, at the moment `OpCoroutineReturn` runs, the live stack holds the
most recently restored `stack_bytes` at `[stack_base, stack_top)`.  After
`stack_pos = stack_base`, those `String` objects are abandoned on the stack
without `drop` being called.  Both leak paths affect the same set of programs.

### Mitigation design

**M7-a — Explicitly drop `String` objects before clearing `stack_bytes`**

Before `frame.stack_bytes.clear()` in `coroutine_return`, walk every text slot
in `frame.text_owned` (or, if CO1.3d is not yet complete, walk the known text
slot offsets from the function definition) and call `drop` on each `String`
object encoded at that offset:

```rust
for (offset, owned) in frame.text_owned.drain(..) {
    drop(owned);   // Rust RAII frees the String's internal allocation
}
// The live-stack String objects also need to be dropped here;
// after CO1.3d they are always reflected in text_owned so the
// loop above covers both saved and live copies.
frame.stack_bytes.clear();
```

Once CO1.3d (`serialise_text_slots`) is implemented, `text_owned` always holds
all live `String` allocations for the frame, so this drain is the complete fix.
Until CO1.3d lands, a separate walk over the function definition's text-slot
layout is needed to cover the live-stack copies as well.

**M7-b — Add a test for text-local generator exhaustion**

Add a test that creates a generator with a `text` local, yields once, and then
breaks the `for` loop to force `OpCoroutineReturn` with a populated frame.  Run
it under Valgrind or with the Rust allocator's leak-detection feature to confirm
no allocation escapes.

---

## P2-R3 — Text locals have an implicit "never freed between yield and resume" invariant

### Description

**Severity: high — fragile; becomes use-after-free when CO1.3d lands**

`coroutine_yield` raw-copies live stack bytes into `stack_bytes` without
serialising text:

```rust
// Serialise locals (integer-only path — no text_owned handling yet).
let mut locals_bytes = vec![0u8; locals_len];
// ... raw copy ...
frame.stack_bytes = locals_bytes;
// text_owned stays empty — CO1.3d will handle text serialisation.
```

A text local variable on the stack is a `String { ptr, len, cap }` struct.  The
raw copy saves the struct's fields, including the internal heap pointer.  On
resume, those bytes are written back to the live stack; the internal pointer is
the same, and the heap allocation was never freed — so the resume is currently
safe.

The invariant holding this together is: **the generator's `String` allocations
are never freed between yield and resume**.  This holds today because:
- The compiler does not emit `OpFreeText` for generator locals before `OpYield`
  (text locals persist across yields).
- Nothing else claims those heap addresses in the interim.

This invariant breaks in two future scenarios:

1. **When CO1.3d (`serialise_text_slots`) is partially implemented:** if
   `free_dynamic_str` is called on the original allocation at yield time (step 4
   of M6-a) before the pointer-patch step (M6-b) is also in place, the resume
   path will write the freed pointer to the live stack — explicit use-after-free.
   CO1.3d must land atomically; partial implementation is more dangerous than no
   implementation.

2. **If a future optimisation reuses "off-stack" memory** between a yield and
   the matching resume, the old `String` bytes restored from `stack_bytes` would
   contain a pointer to reused memory — silent data corruption.

### Mitigation design

**M8-a — Implement CO1.3d atomically**

The serialisation (yield), pointer-patch (resume), and drop (exhaustion) steps
are a single unit of work.  Track them as one task in the implementation plan; do
not merge a partial implementation that calls `free_dynamic_str` without also
implementing the pointer-patch in `coroutine_next` (M6-b) and the `String` drain
in `coroutine_return` (M7-a).

**M8-b — Add a compile-time marker for CO1.3d incomplete state** (✓ implemented)

`coroutine_yield` now contains a `debug_assert!` that fires when `text_positions`
contains any live String slot in the generator's locals range `[base..value_start)`.
The implementation uses `text_positions` (already maintained for S27) rather than
a `text_slot_count` field, avoiding new struct fields:

```rust
let text_local_count = self.text_positions.range(locals_range.clone()).count();
debug_assert!(
    text_local_count == 0,
    "P2-R3: coroutine_yield: {text_local_count} live text local(s) in stack \
     range [{base}..{value_start}). CO1.3d (serialise_text_slots) is not yet \
     implemented; the raw-bytes copy saves heap pointers that could dangle on \
     resume.  See SAFE.md § P2-R3 and PLANNING.md § S25.",
);
```

This turns a silent mis-feature into a loud early failure during development.
Test: `expressions::coroutine_text_local_survives_yield` (ignored until CO1.3d lands).

---

## P2-R4 — `text_positions` inconsistent across yield/resume

### Description

**Severity: medium — debug detector gives wrong results for generators with text locals**

`State::text_positions: BTreeSet<u32>` (debug-only) tracks the absolute
stack-byte positions of live `String` objects.  `OpFreeText` asserts an entry
exists and removes it; `OpText` / `append_text` insert entries.

`coroutine_yield` rewinds `stack_pos` to `base + value_size` but does **not**
remove `text_positions` entries for the frozen text locals at
`[base, value_start)`.  `coroutine_return` also does not remove them.

Consequences:

- **False clean-up:** `free_stack` at the consumer level scans `text_positions`
  for entries in the rewound range.  Orphaned entries in that range are silently
  removed, masking a missing `OpFreeText` for unrelated code at the same stack
  positions.
- **False double-free miss:** After exhaustion, the text locals leak (P2-R2) but
  their `text_positions` entries remain.  A future `OpFreeText` on an unrelated
  variable that happens to land at the same absolute stack position will find an
  entry and succeed, hiding a missing free.
- **Opposite case on resume:** If `coroutine_yield` *had* removed the entries and
  `coroutine_next` did not re-add them, `OpFreeText` inside the resumed generator
  body would hit the `assert!(remove, "double free")` path on the first free of
  a text local after resume — a false double-free panic.

### Mitigation design

**M9-a — Remove text-local entries from `text_positions` at yield; restore on resume**

In `coroutine_yield` (debug path):
1. Collect all `text_positions` entries in `[base, value_start)`.
2. Remove them from `text_positions`.
3. Store the removed set in `frame.saved_text_positions: BTreeSet<u32>`.

In `coroutine_next` (debug path):
1. Re-insert `frame.saved_text_positions` into `text_positions`.
2. Clear `frame.saved_text_positions`.

In `coroutine_return` (debug path):
1. Clear `frame.saved_text_positions` without reinserting (the allocations are
   being dropped; no future `OpFreeText` should fire for them).

This keeps `text_positions` a correct snapshot of the live stack at all times.

---

## P2-R5 — Store-backed `Str` dangles on record delete

### Description

**Severity: medium — silent data corruption; harder to trigger than P2-R1**

When a generator reads a text field from a store record, the value is computed as
`Str { ptr: store.ptr + rec*8 + 8, len }` — a zero-copy pointer directly into
the store's raw allocation.  If this `Str` is live in a local at a `yield` point,
`stack_bytes` encodes the raw bytes of that pointer.

If, between the yield and the next resume, the consumer:
1. Deletes the record (`database.free(r)` / `store.delete(rec)`), OR
2. Frees the entire store (`database.free(db_ref)`)

...and that store word is subsequently reclaimed by a new `claim()` (reused for
different data), the `Str.ptr` in `stack_bytes` now points to unrelated bytes.
On resume the generator reads a corrupted string — **silent data corruption** with
no panic or warning in either debug or release builds.

This is an extension of SC-CO-2 (DbRef lifetime) to text specifically.  The
danger is less visible than with DbRef: a `DbRef` is obviously a pointer that
needs lifetime management, whereas a `Str` looks like a plain value.  The window
of vulnerability is also longer for generators than for ordinary functions because
suspension can span many consumer iterations.

### Mitigation design

**M10-a — Document the invariant and add a debug-mode guard**

Extend the SC-CO-2 documentation in COROUTINE.md with the text-specific variant:

> Any `Str` value derived from a store record field (via `store.get_str()` or
> equivalent) must be treated as a borrow of the store's memory.  If such a
> `Str` is live at a `yield` point, the caller must not delete the backing
> record or free the store before the generator is exhausted or the local is
> overwritten.

In debug builds, add a range check in `coroutine_yield`: for each `Str` slot in
`stack_bytes`, verify the pointer does not fall within any known live store
allocation.  This is a heuristic (cannot cover all store memory without full
pointer provenance), but catches the common case of a recently-obtained field
reference.

**M10-b — Long term: deep-copy store-derived text on yield**

Extend CO1.3d's `serialise_text_slots` to treat store-derived `Str` values
(pointer within `Stores::allocations[*].ptr` range) as dynamic text: copy the
string bytes into an owned `String`, replace the raw store pointer with a pointer
into the owned buffer.  This eliminates the class entirely at the cost of an
allocation per yielded store-text local.

---

## P2-R6 — Compiler check for `yield` inside `par()` missing

### Description

**Severity: medium — uncontrolled panic or silent wrong result**

SC-CO-4 in COROUTINE.md requires the compiler to reject `yield` expressions and
generator function calls inside `par(...)` bodies.  No such check exists in
`src/parser/`.

A `COROUTINE_STORE` DbRef (`store_nr == u16::MAX`) produced inside a `par(...)`
body belongs to the main thread's `State::coroutines` table.  Worker `State`
instances are initialised with `coroutines: vec![None]` (one null sentinel).  If
a worker receives a DbRef with `rec >= 1` and calls `coroutine_next`, it indexes
into its own `coroutines` with an out-of-bounds index — Rust panics.

If `rec == 1` and the worker happens to have allocated a coroutine at index 1,
the worker silently advances the *wrong* frame, producing incorrect results with
no error.

### Mitigation design

**M11-a — Add compiler check in `parse_parallel_for` and `par(...)` body parsing**

In `parse_parallel_for` (and in the `par(...)` body parser), add a flag
`inside_par_body: bool` to the parser context.  When `inside_par_body` is true:
- `yield` and `yield from` emit a diagnostic error.
- Any call to a function with return type `iterator<T>` emits a diagnostic error.

**M11-b — Add a runtime guard as defence-in-depth**

In `coroutine_next`, check whether the `COROUTINE_STORE` DbRef's `rec` is within
bounds of `self.coroutines`:

```rust
if idx >= self.coroutines.len() {
    panic!(
        "coroutine DbRef (rec={idx}) out of range — \
         iterator<T> values must not cross thread boundaries"
    );
}
```

This converts the Rust out-of-bounds panic into a clearly attributed error
message.

---

## P2-R7 — Exhausted frames never freed

### Description

**Severity: low — memory growth; no correctness impact**

`coroutine_return` marks the frame `Exhausted` and clears `stack_bytes` /
`text_owned`, but keeps the `Box<CoroutineFrame>` in `State::coroutines`.  The
slot is never set to `None`.

There is also no finalizer for `COROUTINE_STORE` DbRef variables: when a
generator's DbRef goes out of scope, the frame it points to is not freed.  The
`free_coroutine(idx)` helper exists in the design (COROUTINE.md Phase 1) but is
not called from anywhere in the implementation.

For programs that construct many generators over their lifetime (e.g., a
generator factory called in a loop), `State::coroutines` grows without bound —
one `Box<CoroutineFrame>` per generator invocation, each holding at minimum the
`CoroutineFrame` struct overhead even after exhaustion.

### Mitigation design

**M12-a — Free exhausted frames from the `for`-loop exit path**

The `for ... in gen { }` desugaring knows when the loop exits (either by
exhaustion or by `break`).  At loop exit, emit `OpFreeCoroutine(gen_slot)` which
calls `free_coroutine(idx)` to set the slot to `None`.  This covers the common
case without requiring a general garbage collector.

**M12-b — Free exhausted frames from `exhausted()` calls**

`exhausted(gen)` is often called immediately after a `next()` that returned null.
Optionally, `coroutine_exhausted` could call `free_coroutine(idx)` lazily when
it first observes `Exhausted` status.  This handles the `explicit-advance` API
path (`a = next(gen); if exhausted(gen) { ... }`).

**M12-c — Long term: reference counting for `COROUTINE_STORE` DbRefs**

A general solution requires tracking how many DbRef copies of each coroutine
index exist.  When the count reaches zero, `free_coroutine` is called.  This
mirrors the standard approach for heap-allocated objects.  Not planned for the
initial coroutine implementation but required before 1.0.

---

## P2-R8 — `DbRef` locals outlive their store across suspension

### Description

**Severity: medium — silent data corruption or wrong results; no panic**

SC-CO-2 is acknowledged in COROUTINE.md as "caller responsibility" but the
risk is qualitatively worse for generators than for ordinary functions.

An ordinary function holds a `DbRef` only for its call duration.  A generator
holds `DbRef` locals across an arbitrary number of consumer iterations — the
suspension window can span hundreds or thousands of `next()` calls.  During that
window the consumer (or any other code) may:

1. **Free the record** (`database.free(r)` on the store that owns the record).
   The store slot is returned to the free list and claimed for a new record.  The
   generator resumes and reads/writes the *new* record's data through the old
   `rec` offset.
2. **Free the entire store** (`database.free(db_ref)` on the DbRef).  The store
   index is recycled.  On resume the generator resolves the old `store_nr` to a
   completely different store.
3. **Resize or relocate a record** (`store.resize(rec, new_size)`).  The record
   moves to a new word offset; the old `rec` in the frame is now stale.

All three produce silent data corruption: no assertion fails in release builds,
and the debug-build store lock is not triggered because the frame accesses data
through the *old, now-reused* coordinates, not through a locked-store path.

The risk is highest for generators that iterate over a collection while holding
a `DbRef` to a live record from that same collection (e.g., a generator that
lazily processes records one at a time while the caller can also delete them).

### Mitigation design

**M13-a — Document the suspension lifetime rule in loft language docs**

Extend LOFT.md and the generator chapter of STDLIB.md with an explicit rule:

> A `DbRef` stored in a generator local (including parameters of reference type)
> must remain live and unmodified for the entire lifetime of the generator.
> Freeing or resizing the backing record or store between `next()` calls produces
> undefined behaviour.

**M13-b — Add a debug-mode generation-counter guard on stores**

Add a `generation: u32` field to `Store`.  Increment it on every `claim`,
`delete`, and `resize` that changes the store's live-record set.  When
`coroutine_create` or `coroutine_yield` saves a `DbRef` into `stack_bytes`,
also save `(store_nr, generation_at_save)` into a new `frame.store_generations`
field.  On `coroutine_next`, verify that each saved store's current generation
equals the recorded value; if not, emit a runtime diagnostic:

```
runtime warning: coroutine resumed with stale DbRef — store N was modified
  (generation at save: 7, current: 9)
```

This is a heuristic: a generation match does not prove the specific `rec` is
still valid, but a mismatch is a definite violation.  Cost is O(distinct
store_nr count in the frame) per resume — negligible.

**M13-c — Compiler warning for mutable store access during generator suspension**

Long term: if the compiler can see that a generator holds a `DbRef` local of
type `T` and the consumer code between two `next()` calls contains a `free` or
structural mutation of a `T`-typed store, emit a warning.  This is a flow
analysis and is appropriate for a later compiler pass.

---

## P2-R9 — `e#remove` on a generator iterator corrupts unrelated records

### Description

**Severity: medium — silent store corruption in debug and release**

SC-CO-11 in COROUTINE.md states the compiler must reject `e#remove` on a
generator-typed iterator.  Verification is needed to confirm this check is
implemented.

The corruption mechanism: `e#remove` is lowered to an opcode that calls
`database.remove(db_ref)` using the iterator's DbRef.  For a store-backed
collection iterator, `db_ref` points to a real record in a real store — remove
deletes that record.  For a coroutine iterator, the DbRef encodes
`store_nr == COROUTINE_STORE (u16::MAX)` and `rec == frame_index`.  Passing
this to `database.remove`:

```rust
// database/search.rs — remove() resolves store_nr into allocations[store_nr]
// u16::MAX overflows the allocations Vec, causing an out-of-bounds panic in
// debug builds.  In release builds it wraps to allocations[u16::MAX % len],
// deleting an arbitrary record in a real store.
```

In release builds `u16::MAX % allocations.len()` selects a real store, and
`rec` is the frame index (a small integer like 1 or 2).  Word offset 1 or 2 in
an arbitrary store is almost certainly an occupied record header.  Marking it
free corrupts the store's free list silently.

### Verification step

Check `src/parser/collections.rs` and `src/parser/fields.rs` for the `e#remove`
path.  Confirm whether there is an early return or diagnostic when the iterator
type is `iterator<T>` (i.e., when the DbRef would have `store_nr == COROUTINE_STORE`
at runtime).

### Mitigation design

**M14-a — Compiler-level rejection (SC-CO-11 as specified)**

In the parser, at the point where `e#remove` is resolved, check whether the
loop's iterator type is a generator (identified by the function's return type
being `iterator<T>` backed by `OpCoroutineCreate`).  If so, emit:

```
error: `e#remove` is not valid on a generator iterator;
       generators do not back a store — use a collection if removal is needed.
```

This is a compile-time error with zero runtime cost.

**M14-b — Runtime guard as defence-in-depth**

In `database::remove` (or the opcode that calls it), add a check:

```rust
debug_assert!(
    db.store_nr != COROUTINE_STORE,
    "remove() called with a COROUTINE_STORE DbRef (rec={}); \
     e#remove is not valid on a generator iterator",
    db.rec
);
```

In release builds, return immediately if `db.store_nr == COROUTINE_STORE` rather
than indexing into `allocations` with `u16::MAX`.  This prevents the release-build
store corruption even if the compiler check (M14-a) is missing.

---

## P2-R10 — Yielded `Str` value lifetime is not enforced at the consumer

### Description

**Severity: low — caller confusion; no runtime bug under normal use**

When `OpYield` slides the yielded value bytes to `frame.stack_base`, a `Str`
value in the yielded type is represented as raw `{ ptr: *const u8, len: u32 }`
bytes.  The `ptr` points to the generator's `String` object (currently on the
abandoned-but-live part of the stack above `stack_pos`, or after CO1.3d into a
`text_owned` buffer).

The consumer receives a `Str` reference.  Under normal use — reading the value
synchronously in the loop body and not storing it beyond the current iteration —
the pointer is valid.  The generator's `String` is not freed until the frame is
exhausted (or until CO1.3d causes `free_dynamic_str` on yield).

The lifetime guarantee breaks in two subtle cases:

1. **Consumer stores the `Str` past the next `next()` call.**  On the next
   advance the generator resumes and may reassign or free the underlying
   `String`.  The consumer's saved `Str.ptr` now points to freed or overwritten
   memory.
2. **Consumer passes the `Str` into a function that stores it in a database
   record via `set_str`.**  `set_str` copies the bytes into the store, which is
   safe.  But if the function stores the raw `Str` struct (not the content), the
   same dangling-pointer risk applies.

Unlike P2-R1 through P2-R5, this is not a bug in the implementation — it is the
expected ownership model for `Str` values.  It is called out because generators
make the lifetime window less obvious: the `Str` appears to come from the loop
variable (a value), but it actually references the generator's internal storage.

### Mitigation design

**M15-a — Document the yielded-value ownership rule** *(done — COROUTINE.md CL-7)*

The ownership rule is documented as Known Limitation CL-7 in `COROUTINE.md`:

> A `text` value produced by `yield` is a zero-copy reference into the
> generator's frame.  It is valid only for the duration of the current loop
> body (or until the next `next()` call for explicit-advance code).  To keep
> the text beyond a single iteration, copy it: `stored = "{value}"` or
> pass it to a function that calls `set_str`.

**M15-b — Enforce the lifetime via CO1.3d pointer invalidation**

Once CO1.3d (`serialise_text_slots`) is implemented, `OpYield` will replace
the raw `String.ptr` in the yielded value bytes with a pointer into a
`text_owned` buffer.  At the *next* `OpYield`, that `text_owned` entry is
replaced with a new buffer.  In debug builds, zero-out the old `text_owned`
buffer before replacing it:

```rust
// In the text_owned update path of serialise_text_slots at yield:
#[cfg(debug_assertions)]
for byte in old_owned.as_bytes_mut() { *byte = 0xDD; }
```

This turns use-after-next into an immediate read of `0xDD...` bytes rather
than silently reading stale content, making the bug visible during testing.

**M15-c — Loft type system: `iter_text` reference type (long term)**

Long term, a distinct `iter_text` type (or a lifetime annotation on the loop
variable) could let the compiler reject assignments that outlive the current
iteration.  This is a language design question outside the scope of the
initial coroutine implementation.

---

## Part 2 Summary Table

### SC-CO cross-reference

| SC-CO | Description | Resolution status |
|---|---|---|
| SC-CO-1 | Dynamic `Str` in `stack_bytes` dangles | **Not implemented** (CO1.3d, see P2-R1, P2-R3) |
| SC-CO-2 | `DbRef` locals outlive store | See P2-R8 (S28 ✓); design in M13-a/b/c; CL-2b added for store-backed Str (P2-R5) |
| SC-CO-3 | Re-entrant advance | ✓ Implemented |
| SC-CO-4 | `yield` inside `par(...)` | ✓ Compiler error + runtime guard (P2-R6 M11-a/b) |
| SC-CO-5 | Serialisation cost O(depth) | Documented; accepted |
| SC-CO-6 | Advancing exhausted generator | ✓ Null pushed; frames freed on exhaustion (S26 ✓, P2-R7 done) |
| SC-CO-7 | Fixed `stack_base` clobbered | ✓ Implemented |
| SC-CO-8 | Original dynamic `String` leaked after `to_owned()` | **Not implemented** (CO1.3d, see P2-R2) |
| SC-CO-9 | Scalar active-coroutine tracker wrong for `yield from` | ✓ Implemented |
| SC-CO-10 | Yielded `text` value `Str` not serialised | **Not implemented** (CO1.3d, see P2-R3) |
| SC-CO-11 | `e#remove` on generator iterator | See P2-R9; verification + design (M14-a/b) |
| SC-CO-12 | `text_owned` `u16` offset truncation | ✓ Fixed (`u32`) |

### Risk priority

| Risk | Severity | Effort | Short-term fix | Long-term design |
|---|---|---|---|---|
| P2-R1 — Text arg `Str` dangles | **critical** | L † | Debug assert if any text arg at create (M8-b) | Implement `serialise_text_slots` at create (M6-a, M6-b, M6-c) |
| P2-R2 — `String` objects leaked at exhaustion | **high** | XS † | Drain `text_owned` before `stack_bytes.clear()` (M7-a) | CO1.3d complete makes M7-a sufficient; add leak test (M7-b) |
| P2-R3 — Implicit "never freed" invariant | **high** | L † | Debug assert on text slots present (M8-b) | Implement CO1.3d atomically (M8-a) |
| P2-R4 — `text_positions` inconsistency | **medium** | S | ✓ S27: save/restore entries on yield/resume in debug (M9-a) | Same |
| P2-R5 — Store-backed `Str` dangles | **medium** | S / S | ✓ M10-a: CL-2b in COROUTINE.md + debug pointer-range guard in `coroutine_yield` | Deep-copy store-derived text in `serialise_text_slots` (M10-b, via CO1.3d P2-R3) |
| P2-R6 — No compiler check for `yield` in `par()` | **medium** | S | ✓ M11-a + M11-b: `in_par_body` flag + compiler error + S23 runtime guard | Same |
| P2-R7 — Exhausted frames never freed | **low** | M | ✓ S26: `coroutines[idx] = None` on exhaustion (M12-a) | Reference counting for `COROUTINE_STORE` DbRefs (M12-c) |
| P2-R8 — `DbRef` locals outlive store across suspension | **medium** | M / XL | ✓ S28: generation-counter guard in debug (M13-a/b) | Compiler flow-analysis warning (M13-c) |
| P2-R9 — `e#remove` on generator corrupts store | **medium** | XS | Runtime guard in `database::remove` (M14-b) | Compiler rejection at `e#remove` resolution (M14-a) |
| P2-R10 — Yielded `Str` lifetime not enforced at consumer | **low** | XS / XL | ✓ M15-a done: CL-7 added to COROUTINE.md | Poison old buffer in debug after CO1.3d (M15-b) |

**Implementation dependency:** P2-R1, P2-R2, and P2-R3 all resolve together when
CO1.3d (`serialise_text_slots`) is implemented.  That work must land atomically
(M8-a): implementing the `free_dynamic_str` call without simultaneously
implementing the pointer-patch in `coroutine_next` turns the currently-safe
implicit invariant into an explicit use-after-free.

---

## See also

- [THREADING.md](THREADING.md) — `par(...)` syntax, `parallel_for` desugaring, worker rules
- [COROUTINE.md](COROUTINE.md) — coroutine design, SC-CO safety concerns, implementation phases
- [DATABASE.md](DATABASE.md) — `Stores`, `Store`, `DbRef`, locking API
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/mod.rs`
- [PROBLEMS.md](PROBLEMS.md) — Known bugs and open issues
