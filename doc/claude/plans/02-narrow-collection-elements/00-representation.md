<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Representation

**Status:** open.

**Goal:** extend `Type::Integer` so the `size(N)` annotation from an
integer alias (`i32`, `u16`, `u8`, ...) flows through the full type
tree, including `Box<Type>` inside `Type::Vector`, `Type::Hash`,
`Type::Sorted`, `Type::Index`.

Phase 0 is a no-op refactor: the new field defaults to `None`, no
emission sites read it yet.  Acceptance is "full test suite is
still green."

---

## Rationale — why not the other options

See `README.md § Representation choice` for the full comparison.  In
short: attribute plumbing doesn't cover locals; `type_elm` remapping
can't disambiguate `i32` from `integer` (identical bounds post-2c);
`Type::Integer` extension is invasive but once-and-done.

---

## Shape of the change

```rust
// src/data.rs

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Type {
    // ...
    /// (min, max, not_null, forced_size)
    ///
    /// `forced_size` holds the explicit `size(N)` value from the integer
    /// alias the user typed (e.g. `Some(4)` for `i32`, `Some(1)` for
    /// `u8`).  `None` means "use the bounds heuristic in `Type::size()`"
    /// — the default for plain `integer` and for compiler-generated
    /// Type::Integer values without an originating alias.
    Integer(i32, u32, bool, Option<NonZeroU8>),
    // ...
}
```

Alternative: a named struct `IntegerSpec { min, max, not_null,
forced_size }` referenced by `Type::Integer(Box<IntegerSpec>)` or
`Type::Integer(IntegerSpec)`.  That's cleaner at pattern-match sites
but makes the `I32` / `I64` constants awkward.  **Recommended:
stick with the 4-tuple** — uglier at pattern sites but zero
allocation and minimal diff churn.

### The I32 / I64 constants

```rust
// src/data.rs — before
pub static I32: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false);
pub static I64: Type = Type::Integer(i32::MIN + 1, u32::MAX, false);

// after
pub static I32: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false, None);
pub static I64: Type = Type::Integer(i32::MIN + 1, u32::MAX, false, None);
```

`None` is correct for both — these are the *primitive* `integer`
type (and an internal wide alias).  The forced-size comes from
actual user aliases like `i32`, `u8` which the parser captures at
resolution time (Phase 1).

---

## Sites to touch

`rg 'Type::Integer\(' src/ | wc -l` → **125 occurrences across 22
files** (2026-04-21).

The edits fall into three buckets:

### Bucket A — construction (add `None` as 4th arg)

Uses that build a fresh `Type::Integer(...)` with explicit bounds:

```bash
rg 'Type::Integer\([^,]+,[^,]+,\s*(true|false)\)' src/ -l
```

Representative hits:
- `src/data.rs:32` — `I32` static constant.
- `src/data.rs:42` — `I64` static constant.
- `src/parser/control.rs:1318` — `Type::Integer(i32::MIN + 1, i32::MAX as u32, false)`.
- `src/generation/emit.rs:371` — `Some(Type::Integer(i32::MIN + 1, i32::MAX as u32, false))`.
- `src/variables/slots.rs:459` — `const INT: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32, false);`
- `src/parser/definitions.rs` — parse_type's `Type::Integer(min, max, nullable)` emission.

All mechanical: append `, None`.

### Bucket B — pattern match (add `_` for 4th field)

Uses that destructure `Type::Integer(...)`:

```bash
rg 'Type::Integer\([^)]*_[^)]*\)' src/
```

Representative hits:
- `src/data.rs:596` / `:1831` — bounds-identity check for I32.
- `src/parser/definitions.rs:1655` — `if let Type::Integer(_, _, true) = &tp`.
- `src/scopes.rs:*` — type-match helpers.
- `src/extensions.rs:218` / `:239` — FFI arg-type dispatch.
- `src/generation/mod.rs:479,499,1657` — native codegen type emitters.
- `src/main.rs:463,527,662` — CLI type-mapping.
- `src/state/debug.rs:474` — dump formatter.
- `src/variables/mod.rs:1264` — size lookup.

All mechanical: `Type::Integer(a, b, c)` → `Type::Integer(a, b, c, _)`;
`Type::Integer(_, max, _)` → `Type::Integer(_, max, _, _)`.

### Bucket C — rare: sites that pass the tuple by value

Only a handful if any — `rg '= Type::Integer\(' | rg -v '= Type::Integer\('`
to find bindings that destructure into a local variable.  Should
be zero today; if any exist, they need the same update.

---

## Execution recipe

1. **Branch from the current feature branch** (don't start Phase 0
   until the reverting commit `a100bfb` is the baseline).
2. Edit `src/data.rs` first:
   - `Type::Integer` variant: add `Option<NonZeroU8>`.
   - `I32` / `I64` constants: append `None`.
   - Any `impl` on `Type` that matches `Integer(...)`: add `_`.
3. Run `cargo check --release`.  rustc will list every site that
   needs updating; fix each one by adding `None` (construction) or
   `_` (match).
4. Repeat `cargo check` until clean.
5. Run `cargo test --release --no-fail-fast`.  Expected: all green
   (no behaviour change yet).
6. Run `cargo clippy --tests -- -D warnings` — might flag
   `Option<NonZeroU8>` patterns that would read better as `if let
   Some(n)`; accept the suggestions.
7. Commit as `refactor: Type::Integer carries optional forced_size
   (P184 Phase 0)`.

---

## What `Type::size()` does with the new field

**Nothing, yet.**  The current heuristic stays:

```rust
pub fn size(&self, nullable: bool) -> u8 {
    if let Type::Integer(min, max, _, _) = self {
        let c_min = i64::from(*min);
        let c_max = i64::from(*max);
        if c_max - c_min < 256 || (nullable && c_max - c_min == 256) {
            1
        } else if c_max - c_min < 65536 || (nullable && c_max - c_min == 65536) {
            2
        } else {
            8
        }
    } else {
        0
    }
}
```

Phase 1 is the one that teaches `Type::size()` (or a new helper) to
prefer the forced-size when present.

Rationale: staging the behaviour change into Phase 1 means Phase 0
can land on its own without any regression risk — if Phase 1
reveals a deeper problem, Phase 0 stays in and the project isn't
worse off.

---

## Risk log

- **NonZeroU8 vs u8.**  `NonZeroU8` gives free niche packing
  (`Option<NonZeroU8>` is 1 byte, not 2) but the parser has to
  filter `size(0)` with an explicit check.  That's fine — `size(0)`
  isn't a valid annotation anyway.  If this is painful, use
  `Option<u8>` (2 bytes) — the size bloat is negligible.
- **Debug / Display of Type::Integer.**  Existing code prints like
  `Integer(-2147483647, 2147483647, false)`; the new field adds
  `, None` or `, Some(4)` to the formatted output.  Check
  `src/data.rs::Type::name()` and any `Display` impl — they skip
  the bounds for user-facing output, so no user-visible change.
- **Serialisation paths.**  If anything binary-serialises `Type`
  (the old `.loftc` cache is removed per C54 Phase 2c; confirm no
  new serialiser has since been added).  `rg 'impl.*Type.*serial'`.

---

## Rollback

`git revert <phase-0-commit>`.  Every call site change is
mechanical; the revert is a pure no-op undo.  No schema versioning
to worry about because Phase 0 doesn't change behaviour.

---

## Acceptance

- [ ] `cargo build --release --all-targets` green.
- [ ] `cargo test --release --no-fail-fast` green (same pass count
      as pre-Phase-0).
- [ ] `cargo clippy --tests --release -- -D warnings` green.
- [ ] `cargo fmt -- --check` green.
- [ ] `make test-packages` green (pre-existing imaging / random /
      web failures unchanged — those are unrelated).
