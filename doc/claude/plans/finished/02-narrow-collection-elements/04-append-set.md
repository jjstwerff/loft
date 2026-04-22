<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4a — Short-encoding gate + u16 round-trip guard

**Status:** open — uncommitted work in-tree, ready to land.

**Goal:** lock down the `Parts::Short` legacy-encoding hazard so
Phase 2/3's narrowing can NEVER accidentally cover a 2-byte element
type until Phase 4b replaces the encoding.  Guarantee that
`vector<u16>` / `vector<i16>` fields stay at wide (8-byte) storage
with reads aligned to writes — correct but not optimal.

Original Phase 4 plan: "narrow storage + reads + writes for every
alias".  Scope reduced after discovering `Parts::Short` uses
`raw = val - min + 1` encoding while the raw-byte vector-copy path
in `src/database/structures.rs::vector_add` moves bytes directly
(no +1 shift).  The mismatch was already excluded from Phase 2 at
commit `3b6fd43` (2-byte sizes skipped in `fill_database`'s narrow
branch); Phase 4a closes the matching gap on the read side.

---

## The narrow-width gate

`IntegerSpec::vector_narrow_width(&self) -> Option<u8>` on
`src/data.rs` returns `Some(n)` iff `forced_size = Some(n)` and
`n ∈ {1, 4}`.  Anything else (including `Some(2)` for `u16`) returns
`None` — callers fall back to the default wide stride.

```rust
#[must_use]
pub fn vector_narrow_width(&self) -> Option<u8> {
    match self.forced_size?.get() {
        1 => Some(1),
        4 => Some(4),
        _ => None,  // 2-byte deferred to Phase 4b
    }
}
```

This is the **single point of policy** for what widths Phase 2 will
narrow.  If Phase 4b replaces the short encoding, the only change
needed is adding `2 => Some(2)` here.

---

## Read-side gate

Two sites were using `byte_width()` directly in Phase 3 and needed
the gate:

### `src/parser/fields.rs::parse_vector_index`

```rust
// Before: narrow whenever forced_size is present (includes 2-byte).
let elm_size = if let Type::Integer(spec) = etp {
    i32::from(spec.byte_width(true))
} else { /* heuristic */ };

// After: narrow only for sizes Phase 2 actually registers.
let elm_size = if let Type::Integer(spec) = etp
    && let Some(n) = spec.vector_narrow_width()
{
    i32::from(n)
} else { /* heuristic */ };
```

### `src/parser/mod.rs::get_val`

Three-way dispatch: struct-field path keeps its captured alias
lookup; vector-element path (alias = `u32::MAX`) uses the narrow
gate; everything else falls back to `byte_width(nullable)` for the
plain-integer / `integer limit(...)` cases.

```rust
let s = if alias != u32::MAX {
    // Struct field with captured alias (u8/u16/i32/...): use it.
    self.data.forced_size(alias).unwrap_or_else(|| spec.byte_width(nullable))
} else if spec.forced_size.is_some() {
    // Vector-element context: mirror Phase 2's narrow-or-wide decision.
    spec.vector_narrow_width().unwrap_or(8)
} else {
    // Plain `integer` / `integer limit(...)`: heuristic.
    spec.byte_width(nullable)
};
```

Without the `forced_size.is_some()` check, `integer limit(0, 255)`
struct fields (which use `alias = u32::MAX`) would mis-dispatch to
the vector narrow gate and always read 8 bytes — breaking
`tests/scripts/06-structs.loft::Point`.  The three-way split keeps
narrow struct-field reads (bounds-heuristic path) and narrow
vector-element reads (Phase 2 gate) independently correct.

---

## Test matrix landed in Phase 4a

| Test                                     | Purpose                                                       |
|------------------------------------------|---------------------------------------------------------------|
| `p184_vector_i32_narrow_read`            | i32 vector field reads correct values (Phase 3 guard).        |
| `p184_vector_u8_narrow_read`             | u8 vector field uses 1-byte narrow storage + reads.           |
| `p184_vector_u16_round_trip` *(new 4a)*  | u16 stays wide (8-byte), reads + writes agree.  Non-optimal but correct. |
| `p184_vector_integer_wide_control`       | Plain `vector<integer>` unchanged.                            |

---

## Why Phase 4a is a separate commit from 4b

Phase 4a is a **correctness** fix: without the gate, mid-session
work in Phase 3 had narrow reads against wide storage for u16
fields (`v[0] = 0` for stored value 1).  The gate closes that
window AND adds a regression guard.

Phase 4b is an **optimisation**: switch u16/i16 to 2-byte storage.
Not required for any user-visible bug — the glTF case that triggered
P184 resolves via `vector<i32>` narrowing, which is already working
for struct fields and lands fully after Phase 5.

Separating them lets Phase 4a ship with an immediate regression
benefit and keeps Phase 4b's encoding-replacement risk isolated.

---

## What Phase 4a does NOT unlock

- `lib/graphics/src/glb.loft::glb_write_indices` workaround can NOT
  revert yet.  That requires Phase 5 — the helper uses a local
  variable (`result: vector<i32> = []`), and locals don't narrow
  until Phase 5.  A TODO note has been added to the workaround.
- `vector<u16>` / `vector<i16>` memory density.  Users with
  bit-packed 2-byte protocols will keep using the explicit
  `f += val as u16` cast idiom documented in CAVEATS.md § C54.

---

## Acceptance

- [x] `vector_narrow_width()` method exists and gates 1-byte / 4-byte only.
- [x] `parse_vector_index` and `get_val` use the gate.
- [x] `get_val`'s three-way alias / vector / struct-field dispatch is correct
      (no regression in `06-structs.loft`).
- [x] `p184_vector_u16_round_trip` passes; u16 values round-trip through
      wide storage.
- [x] `p184_vector_i32_narrow_read` and `p184_vector_u8_narrow_read` still
      pass.
- [x] Full `cargo test --release --no-fail-fast` green.

---

## Rollback

Single-commit revert is safe — the gate is additive and defaults
to "wide" for every unlisted size.  No data-format change, no
schema migration.
