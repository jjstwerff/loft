<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — Detailed fix design (path B: unaligned typed stack access)

Design for the chosen path **(B)** from
[`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md).
Path (A) (align slots in variable-positioning) is retained there as
the future pivot; this document specifies (B) only.

**Status:** design — NOT implemented.  References function *names*
(stable) rather than line numbers, which will shift after the
PLAN52 PR merges to main (this work lands on that clean slate).

## Goal

Make every typed read/write of the byte-packed eval stack use the
unaligned intrinsics (`ptr::read_unaligned` / `write_unaligned`)
instead of forming a `&T` / `&mut T` into a possibly-unaligned
address.  Forming the reference is the UB (cluster 2); the value
intrinsics are defined at any alignment.  No slot-allocator change,
no record-layout change, no on-disk-format change.

## Why the read side must return BY VALUE

You cannot soundly hand out a `&T` / `&mut T` to an unaligned
address — the reference is UB the moment it is constructed,
regardless of how the caller uses it.  So the read accessors that
currently return `&T` must return `T` (an owned copy).  This is the
exact shape that fixed cluster 1 (`State::code<T>`): bound `T: Copy`,
read by value, callers drop their leading `*`.  Every value on the
eval stack is `Copy` (`Str`, `DbRef`, `i64`, `u32`, …; owned
`String`s live in *records*, never the stack), so this is always
sound.

## The seam (the funnel — these ARE the named accessors)

All typed stack access already funnels through a small set of
accessors.  These become the seam: their bodies switch to unaligned
value access, and the future pivot to (A) is a single-site flip of
these bodies back to aligned access.

| Accessor | File / fn | Today | After (B) |
|---|---|---|---|
| Stack pop (DbRef-based) | `Stores::get<T>` (`src/database/mod.rs`) | `-> &T` via `store.addr::<T>` | `<T: Copy> -> T` via `store` unaligned read |
| Stack push (DbRef-based) | `Stores::put<T>` (`src/database/mod.rs`) | `*addr_mut::<T> = val` | `store` unaligned write (signature already takes `val: T`) |
| Stack pop (State frame) | `State` typed-pop `<T>` (`src/state/mod.rs`, the `stack_cur.pos + stack_pos` reader) | `-> &T` via `addr::<T>` | `<T: Copy> -> T` via unaligned read |
| Stack push (State frame) | `State` typed-push `<T>` (`src/state/mod.rs`) | `*addr_mut::<T> = val` | unaligned write |
| String push | `State::set_string` (`src/state/text.rs`) | `*addr_mut::<Str> = Str{…}` | unaligned write of `Str` |
| String pop/read | `State` Str reader (`src/state/text.rs`) | `addr::<Str>` ref | unaligned read of `Str` (value) |

### Underlying primitive

Add two value accessors to `Store` (the single place the unaligned
intrinsic lives — the rest of the seam calls these):

```rust
/// Read a `T` from a byte offset that may be unaligned (the eval
/// stack is byte-packed).  Defined at any alignment; see @PLAN53
/// cluster 2.
#[inline]
pub fn read_at<T: Copy>(&self, rec: u32, fld: u32) -> T {
    // bounds debug_assert identical to addr::<T>
    unsafe {
        self.ptr
            .offset(Self::checked_offset(rec, fld))
            .cast::<T>()
            .read_unaligned()
    }
}

/// Write a `T` to a possibly-unaligned byte offset.
#[inline]
pub fn write_at<T>(&mut self, rec: u32, fld: u32, value: T) {
    // read_only / bounds asserts identical to addr_mut::<T>
    unsafe {
        Arc::make_mut(&mut self.bytecode_or_ptr_owner) // (mut access as addr_mut)
            ...
            .offset(Self::checked_offset(rec, fld))
            .cast::<T>()
            .write_unaligned(value);
    }
}
```

(`read_at`/`write_at` mirror `addr`/`addr_mut`'s bounds + read-only
asserts exactly — only the final reference construction changes to
the value intrinsic.  `addr`/`addr_mut` stay unchanged for the
ALIGNED record-field accesses, which still legitimately hand out
references.)

## What stays UNTOUCHED

- **`u8` stack access** — `align_of::<u8>()` = 1, never unaligned.
  The many `addr::<u8>` / `addr_mut::<u8>` block-copy/memcpy paths on
  the stack are left exactly as they are.
- **Record-field `addr`/`addr_mut`** — records are 8-byte aligned by
  design (store base + field layout), so those references are valid;
  do not route them through the unaligned seam (would lose the
  `&mut String`-in-place mutation the records rely on, and would be
  needlessly slow on strict-alignment targets).

## Debug-assert preservation

`Stores::get` / the State pop currently inspect the returned `&T` as
`&DbRef` to run the stack-corruption OOB check.  With a value return,
keep the check by taking a reference to the *local* value:

```rust
let r: T = self.store(stack).read_at::<T>(stack.rec, stack.pos);
#[cfg(debug_assertions)]
if TypeId::of::<T>() == TypeId::of::<DbRef>() {
    let db: &DbRef = unsafe { &*(&r as *const T as *const DbRef) };
    debug_assert!( /* …same OOB check… */ );
}
r
```

`&r` is a reference to a stack-local `T` — properly aligned by the
Rust compiler — so the check itself introduces no unaligned ref.

## Caller migration

Mechanical, identical to cluster 1: the read accessors now return
`T`, so call sites that did `*x.get::<T>(...)` / `*pop::<T>()` drop
the leading `*`.  The compiler enumerates every site (E0614 "cannot
be dereferenced"), so the migration is exhaustive and self-checking.
Push/write accessors already take the value by parameter — their
call sites are unchanged.

## Verification

1. `cargo build` — resolve every E0614 deref site (exhaustive).
2. `cargo test` interpreter suite (`issues`, `wrap`, …) + native
   suite — behaviour is identical (value vs reference for `Copy`).
3. **Miri** (`cargo +nightly miri test … production_mode_…`) — cluster
   2 no longer trips at the stack accessor; Miri proceeds and peels
   the next execution-phase finding (if any) → new cluster.
4. `find_problems` full suite green (excluding the pre-existing
   `macos-clippy-fixes` failures, which the clean-slate main rebase
   resolves anyway).

## Risk + rollback

- **Risk: low.**  Localized to ~6 accessors + their (compiler-
  enumerated) deref sites.  No slot-allocator / `validate_slots`
  touch.  No format change.  Zero perf delta on loft's shipping
  targets (x86-64 / AArch64 / wasm — `read/write_unaligned` lower to
  the same single instruction).
- **Rollback:** revert the accessor bodies + return types; the deref
  sites revert with them.

## Future pivot to (A)

When loft gains a strict-alignment **interpreter** target (RISC-V
SBC interpreter build), pivot: align the slots in the
variable-positioning code (`src/variables/slots.rs` /
`slots_v2.rs`) **and** flip `read_at`/`write_at` (+ the seam
accessors) back to the aligned `addr`/`addr_mut` reference form.
Because all typed stack access funnels through the seam, that flip
is single-site — the call sites do not move.  Full (A) detail in
[`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md)
§ Fix path.
