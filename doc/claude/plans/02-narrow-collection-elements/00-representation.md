<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Representation

**Status:** open.  (Scoped-up from "add a fourth tuple field" to
"replace `Type::Integer(i32, u32, bool)` with a named struct
`Type::Integer(IntegerSpec)`" after an in-progress build-break
surfaced the tuple's readability debt — 2026-04-21.)

**Goal:** give `Type::Integer` a named struct carrier
(`IntegerSpec`) that holds the bounds, the not-null flag, AND the
forced-size annotation — so the annotation travels through
`Box<Type>` in `Type::Vector` / `Hash` / `Sorted` / `Index`
naturally, and so 125+ read sites stop counting tuple positions.

Phase 0 is a near-no-op refactor: the new field defaults to `None`
(no forced size), and no consumer reads it yet.  Acceptance is
"full test suite is still green and every pattern site reads
better."

---

## Why this grew from "add a 4th tuple field" to a struct

First attempt (2026-04-21) added `Option<NonZeroU8>` as a fourth
tuple field — `Type::Integer(i32, u32, bool, Option<NonZeroU8>)`.
The mechanical refactor worked (compiled, zero warnings), but the
readability dropped sharply: every pattern site now looks like
`Type::Integer(_, _, _, _)` or `Type::Integer(min, max, _, _)`,
and guards like `if *min == i32::MIN + 1 && *max == i32::MAX as u32`
require the reader to remember positional meaning at 125+ sites.

A named struct with named fields lets every call site read as
`spec.min` / `spec.forced_size` / etc. and makes future additions
(e.g. a diagnostic origin span, a `kind: IntegerKind` tag) cost
only the sites that actually care about the new field.

### Fields the code actually reads today

From `grep -oE 'Type::Integer\([^)]+\)' src/ | sort -u`:

| Field       | Read sites | Purpose                                                       |
|-------------|-----------:|---------------------------------------------------------------|
| `min: i32`  | ~15        | Storage offset for `Parts::Byte/Short/Int`; size heuristic.   |
| `max: u32`  | ~20        | "Is this a long?" (`max > i32::MAX as u32`); size heuristic.  |
| `not_null: bool` | ~2    | Mostly `_` — the attribute-level `nullable` flag covers it.   |
| `forced_size` | NEW     | Storage layout override (P184).                               |

Also ~10 construction sites repeat magic bound constants
(`Type::Integer(0, 255, ...)` for u8, `Type::Integer(-32768, 32767, ...)`
for i16).  `IntegerSpec::u8()` / `signed32()` helpers consolidate
these.

### Decoupling bounds and storage

Pre-C54 Phase 2c, bounds *determined* size via a heuristic
(`max - min < 256` → 1 byte, etc.).  Post-C54, the heuristic still
runs but a `forced_size` annotation on the alias can override.  The
two concerns have diverged — bounds are now mostly a *diagnostic*
concern (validation / range-check / display), while storage width
is a *layout* concern.  The named struct keeps them clearly
labelled in the same place.

---

## Shape of the change

### The new carrier

```rust
// src/data.rs

use std::num::NonZeroU8;

/// Specification of an `integer`-family type — bounds, nullability,
/// and optional forced storage width (from a user-typed alias's
/// `size(N)` annotation, P184).
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct IntegerSpec {
    /// Inclusive lower bound of the value range.  `i32::MIN` is
    /// reserved as the null sentinel; the plain-integer template
    /// uses `i32::MIN + 1`.
    pub min: i32,
    /// Inclusive upper bound.  `u32` to allow the wide
    /// (`integer` / former `long`) template to use `u32::MAX` as a
    /// "wider than i32" sentinel that size heuristics recognise.
    pub max: u32,
    /// If true, the value cannot be null — saves the null sentinel
    /// bit pattern and widens the usable range by 1 on narrow types.
    pub not_null: bool,
    /// When `Some(n)`, storage width is `n` bytes regardless of
    /// bounds.  Set by the parser from an integer alias's `size(N)`
    /// annotation (`i32` → `Some(4)`, `u8` → `Some(1)`).  `None`
    /// means "use the bounds heuristic in `Type::size()`".
    pub forced_size: Option<NonZeroU8>,
}

impl IntegerSpec {
    // ── Canonical templates (constructors) ──────────────────────────────

    /// Plain `integer` / former `long` (post-2c): full i64 range,
    /// 8-byte storage via the default heuristic.  No forced size.
    pub const fn wide() -> Self {
        IntegerSpec { min: i32::MIN + 1, max: u32::MAX, not_null: false, forced_size: None }
    }
    /// The I32 template — i32 bounds, `not_null = false`, no forced size.
    pub const fn signed32() -> Self {
        IntegerSpec { min: i32::MIN + 1, max: i32::MAX as u32, not_null: false, forced_size: None }
    }
    /// `u8` alias — `0..=255`, forced 1-byte storage.
    pub fn u8() -> Self {
        IntegerSpec { min: 0, max: 255, not_null: false, forced_size: NonZeroU8::new(1) }
    }
    /// `i8` alias — `-128..=127`, forced 1-byte storage.
    pub fn i8() -> Self {
        IntegerSpec { min: -128, max: 127, not_null: false, forced_size: NonZeroU8::new(1) }
    }
    /// `u16` alias — `0..=65535`, forced 2-byte storage.
    pub fn u16() -> Self {
        IntegerSpec { min: 0, max: 65535, not_null: false, forced_size: NonZeroU8::new(2) }
    }
    /// `i16` alias — `-32768..=32767`, forced 2-byte storage.
    pub fn i16() -> Self {
        IntegerSpec { min: -32768, max: 32767, not_null: false, forced_size: NonZeroU8::new(2) }
    }
    /// `i32` alias — full i32 range, forced 4-byte storage.
    pub fn i32() -> Self {
        IntegerSpec { min: i32::MIN + 1, max: i32::MAX as u32, not_null: false, forced_size: NonZeroU8::new(4) }
    }
    /// `u32` alias — `0..=u32::MAX - 1`, wide (8-byte) storage (per CAVEATS.md § C54).
    pub fn u32() -> Self {
        IntegerSpec { min: 0, max: u32::MAX - 1, not_null: false, forced_size: None }
    }

    // ── Query methods (consolidate scattered bounds arithmetic) ─────────

    /// Storage width in bytes — honours `forced_size` first, falls back
    /// to the bounds-range heuristic otherwise.  Single source of truth
    /// for layout width; use everywhere instead of hand-rolling the
    /// `forced_size(alias).unwrap_or_else(|| tp.size(nullable))` chain.
    /// Phase 3 relies on this method — its existence is what lets the
    /// read path stop threading `alias_d_nr` through every call.
    pub fn byte_width(&self, nullable: bool) -> u8 {
        if let Some(n) = self.forced_size {
            return n.get();
        }
        let range = self.range();
        if range <= 256 || (nullable && range == 257)      { 1 }
        else if range <= 65536 || (nullable && range == 65537) { 2 }
        else                                                { 8 }
    }

    /// True when the value range exceeds the signed-32-bit range
    /// (`max > i32::MAX as u32`).  Used by native codegen to pick
    /// between i32 and i64 Rust types.  Consolidates 6+ sites.
    pub fn is_wide(&self) -> bool {
        self.max > i32::MAX as u32
    }

    /// Number of distinct representable values (inclusive range + 1).
    /// Used internally by `byte_width` and by a handful of size heuristics.
    /// i64 return accommodates the wide template's ~4.3B-value range.
    pub fn range(&self) -> i64 {
        i64::from(self.max) - i64::from(self.min) + 1
    }

    /// True when this is the I32 template the parser hands out for
    /// plain `integer` (pre-2c I32 shape, post-2c still emitted by
    /// `typedef.rs::complete_definition`).  Used by `Display` / `name()`
    /// to print `"integer"` instead of `"integer(-2147483647, 2147483647)"`.
    pub fn is_signed32_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == i32::MAX as u32
    }

    /// True when this is the wide / I64 template (`max == u32::MAX`).
    /// Used to distinguish plain `integer` from a bounded `integer limit(...)`
    /// in the "is this wide" diagnostic path.
    pub fn is_wide_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == u32::MAX
    }
}
```

### Why these five methods

Grep of the current tree showed these guard / expression patterns
repeating across 22 files:

| Pattern                                                                   | Sites | Replaced by |
|---------------------------------------------------------------------------|------:|-------------|
| `*max > i32::MAX as u32`                                                  | 6     | `s.is_wide()` |
| `data.forced_size(alias).unwrap_or_else(\|\| tp.size(nullable))`          | 5     | `s.byte_width(nullable)` |
| `i64::from(*to) - i64::from(*from)` + threshold compare                   | 5     | `s.range()` + `byte_width()` |
| `*min == i32::MIN + 1 && *max == i32::MAX as u32`                         | 3     | `s.is_signed32_template()` |
| `*min == i32::MIN + 1 && *max == u32::MAX`                                | ~2    | `s.is_wide_template()` |

All five are behavioural constants — changing the "what's wide?"
definition in one place propagates everywhere, instead of hunting
six sites.

Methods **not** added (evaluated and rejected):

- `is_i8 / u8 / i16 / u16` — derivable from `range()` + `min` sign.
  Adding them invites callers to check the wrong identity (an
  `integer limit(0, 255)` should behave like u8 storage-wise, so
  the method name would be misleading).  Let callers inspect
  `byte_width()` / `min` / `max` directly.
- `rust_elem_type() -> &'static str` — tempting (consolidates
  `vector_elem_rust_type` in `generation/mod.rs:1652`), but it's
  a generation-layer concern that shouldn't bleed into `Data`.

### Type variant

```rust
// src/data.rs

pub enum Type {
    // ...
    Integer(IntegerSpec),
    // ...
}
```

### Constants

```rust
// src/data.rs — replace 4-tuple forms
pub static I32: Type = Type::Integer(IntegerSpec::signed32());
pub static I64: Type = Type::Integer(IntegerSpec::wide());
```

---

## Pattern-site migration

The 125+ sites fall into these rewrite patterns:

### Pattern: `Type::Integer(_, _, _)` → `Type::Integer(_)`

When a site ignores all inner fields, collapse to a single wildcard:

```rust
// before
if let Type::Integer(_, _, _) = tp { ... }
// after
if let Type::Integer(_) = tp { ... }
```

### Pattern: `Type::Integer(min, max, _)` with guards → `Type::Integer(s)` + field access

```rust
// before
Type::Integer(min, max, _) if *min == i32::MIN + 1 && *max == i32::MAX as u32 => {
    // ...
}
// after
Type::Integer(s) if s.min == i32::MIN + 1 && s.max == i32::MAX as u32 => {
    // ...
}
```

Less line noise; fewer spurious `*` dereferences.

### Pattern: `Type::Integer(_, max, _) if *max > i32::MAX as u32`

```rust
// before
Type::Integer(_, max, _) if *max > i32::MAX as u32 => ...
// after — use the method, not direct field access
Type::Integer(s) if s.is_wide() => ...
```

### Pattern: range-based guards → `s.range()` + `s.byte_width()`

```rust
// before
Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 255 => "i8",
Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 65536 => "i16",
// after
Type::Integer(s) if s.byte_width(false) == 1 => "i8",
Type::Integer(s) if s.byte_width(false) == 2 => "i16",
// or for the raw range check:
Type::Integer(s) if s.range() <= 256 => ...,
```

### Pattern: I32-template detection → `s.is_signed32_template()`

```rust
// before
Type::Integer(min, max, _) if *min == i32::MIN + 1 && *max == i32::MAX as u32 => { ... }
// after
Type::Integer(s) if s.is_signed32_template() => { ... }
```

### Pattern: `Type::Integer(min, _, not_null)`

```rust
// before
Type::Integer(min, _, not_null) => { /* use min, not_null */ }
// after
Type::Integer(s) => { /* use s.min, s.not_null */ }
```

### Construction: bound literals → `IntegerSpec::u8()` / etc.

```rust
// before
Type::Integer(0, 255, true)
// after
Type::Integer(IntegerSpec { not_null: true, ..IntegerSpec::u8() })
// or if the helper already matches
Type::Integer(IntegerSpec::u8())
```

### Construction: parsed bounds

Parser sites that compute min/max from literals:

```rust
// before
Type::Integer(min, max, not_null)
// after
Type::Integer(IntegerSpec { min, max, not_null, forced_size: None })
```

---

## Sites to touch

`rg 'Type::Integer\(' src/ tests/ | wc -l` → **~130 occurrences
across 22 src files + ~5 test files** (2026-04-21 estimate).

Rough split:
- ~90 patterns (destructuring in `match` / `if let` / function signatures)
- ~40 constructions (literal bounds, cloned templates, parser output)
- ~10 construction sites eligible for the new helper constructors

Test files (`tests/testing.rs`, `tests/data_import.rs`) also
contain `Type::Integer(...)` pattern sites — caught by
`cargo test --release --no-fail-fast` after the main src changes
land.

---

## Execution recipe

1. **Stay on the current feature branch** per the user's memory
   rule.  No new branch.
2. Edit `src/data.rs`:
   - Add `use std::num::NonZeroU8;`.
   - Define `pub struct IntegerSpec` with `#[derive(Debug,
     PartialEq, Eq, Clone, Copy, Hash)]`.
   - Add constructor helpers (`wide`, `signed32`, `u8`, `i8`,
     `u16`, `i16`, `i32`, `u32`).
   - Change `Type::Integer(i32, u32, bool)` → `Type::Integer(IntegerSpec)`.
   - Update `I32` and `I64` constants to use helpers.
3. Run `cargo check --release --lib` — rustc lists every site.
4. For each site:
   - **Pattern with full destructure** → `Type::Integer(s)` +
     `s.field` access inside.
   - **Pattern that ignores all fields** → `Type::Integer(_)`.
   - **Construction with literal bounds that match a helper** →
     use the helper.
   - **Construction with computed bounds** → explicit
     `IntegerSpec { .. }` literal.
5. Run `cargo test --release --no-fail-fast` — includes test
   files, which rustc's `check --lib` misses.
6. Run `cargo clippy --tests --release -- -D warnings` and
   `cargo fmt -- --check`.
7. Commit as `refactor(data): IntegerSpec carrier + helpers (P184
   Phase 0)`.

### Semi-automated rewrite script

A Python script can do most of this.  Key insight from the first
attempt: positional heuristics (third arg is `true`/`false` →
construction; third arg is `_` or identifier → pattern) misclassify
`return Some(Type::Integer(min, max, not_null, _))` as a pattern
(construction's third arg is a bound-variable-named `not_null`,
looks like a pattern).  Use rustc's error output to re-classify
sites that fail to compile, or hand-edit the ~10 misfires.

Rough skeleton:

```python
# 1. Collapse every `Type::Integer(a, b, c)` to `Type::Integer(/* TEMP */(a, b, c))`.
# 2. Run cargo check; for each E0023 (pattern) site, rewrite to `Type::Integer(s)`.
# 3. For each E0061 (construction) site, rewrite to `Type::Integer(IntegerSpec { min: a, max: b, not_null: c, forced_size: None })`.
# 4. Iterate until cargo check is clean.
```

Hand edits after the bulk pass:
- Replace repeated literal-bound constructions with helper calls
  (`Type::Integer(IntegerSpec::u8())` etc.).
- Rewrite guard expressions to use `s.field` in place of `*min` / `*max`.

---

## Risks

- **Derive incompatibility.**  `IntegerSpec` needs `Copy` to
  avoid adding `.clone()` at existing call sites.  `NonZeroU8` is
  `Copy` so this works.  Verify the derives compile cleanly.
- **`NonZeroU8` vs `u8` in the Option.**  `Option<NonZeroU8>` is
  1 byte (niche-packed) vs `Option<u8>` at 2 bytes.  Cosmetic but
  worth keeping the niche.  Constructors use `NonZeroU8::new(n)`
  which returns `Option<NonZeroU8>` directly.
- **`Type::size()` helper**: currently reads bounds from the tuple.
  Adapt to `if let Type::Integer(s) = self { s.min, s.max, ... }`.
  Keep the bounds heuristic unchanged — Phase 1 is the one that
  teaches it to prefer `s.forced_size` when present.
- **Display / Debug output**.  The existing `Display` impl prints
  `integer({min}, {max})`; with the struct it reads
  `s.min, s.max`.  No user-visible change.

---

## Rollback

`git reset --hard <pre-phase-0-ref>` works if nothing downstream
has landed.  Once Phase 1 onwards depend on `IntegerSpec.forced_size`,
revert becomes a multi-commit revert instead of a single one —
standard git workflow, no data-loss risk because Phase 0 touches
no persistent state.

---

## Acceptance

- [ ] `cargo build --release --all-targets` green.
- [ ] `cargo test --release --no-fail-fast` green (same pass count
      as pre-Phase-0).
- [ ] `cargo clippy --tests --release -- -D warnings` green.
- [ ] `cargo fmt -- --check` green.
- [ ] `make test-packages` green (pre-existing imaging / random /
      web failures unchanged — those are unrelated).
- [ ] Every `Type::Integer(...)` pattern site reads `spec.field`
      instead of positional `*min`, `*max`.
- [ ] Every `Type::Integer(0, 255, ...)`-style literal construction
      uses the matching `IntegerSpec::u8()` / etc. helper.
- [ ] Every `*max > i32::MAX as u32` guard uses `s.is_wide()`.
- [ ] Every `i64::from(*to) - i64::from(*from) <= N` guard uses
      `s.range()` (or `s.byte_width(...)`).
- [ ] Every `*min == i32::MIN + 1 && *max == i32::MAX as u32` guard
      uses `s.is_signed32_template()`.
- [ ] `grep 'i32::MAX as u32' src/` returns zero hits outside
      `IntegerSpec` itself and its unit tests.
