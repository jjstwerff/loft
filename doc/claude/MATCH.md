
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Match Expression Design (T1-4)

> **Status: fully implemented** — shipped 2026-03-16 through 2026-03-18.
> Only planned extension remaining: **L2 Nested patterns in field positions**.

Pattern-matching expression for dispatching on enum values with
compiler-checked exhaustiveness.

---

## Syntax

```loft
// Plain enum
label = match direction {
    North => "N",
    East  => "E",
    South | West => "other",
}

// Struct enum with field binding
area = match shape {
    Circle { radius }        => PI * radius * radius,
    Rect   { width, height } => width * height,
    _ => 0.0,
}

// Guards, scalars, ranges
match code {
    200 => "ok",
    300..399 => "redirect",
    n if n >= 500 => "server error",
    _ => "other",
}
```

### Exhaustiveness

Without a wildcard `_`, every variant must appear exactly once (compile error
if missing). Duplicate variant arms produce a warning.

### Result type

All arm bodies must produce compatible types (same unification as `if`/`else`).

---

## Supported pattern types

| Pattern | Example | Since |
|---|---|---|
| Plain enum variant | `North =>` | T1-4 |
| Struct enum with binding | `Circle { radius } =>` | T1-4 |
| Wildcard | `_ =>` | T1-4 |
| Guard | `x if x > 0 =>` | T1-16 |
| Scalar literal | `200 =>` | T1-14 |
| Or-pattern | `Paid \| Refunded =>` | T1-15 |
| Range | `0..9 =>` | T1-17 |
| Struct destructuring | `Point { x, y } =>` | T1-18 |
| Null pattern | `null =>` | T1-20 |
| Binding pattern | `n =>` (captures value) | T1-20 |
| Slice/vector | `[first, ...rest] =>` | T1-21 |

---

## Relationship to polymorphic dispatch

| Mechanism | Trigger | Use case |
|---|---|---|
| Polymorphic dispatch | `shape.method()` | Reusable, named per-variant behaviour |
| `match` expression | `match shape { ... }` | One-off dispatch with optional field binding |

Both coexist. `match` does not replace polymorphic methods.

---

## Remaining work: L2 Nested patterns in field positions

Allows sub-patterns inside struct field bindings:

```loft
match event {
    Http { status: Paid } => ...,          // enum sub-pattern
    Http { code: 200 }    => ...,          // scalar sub-pattern
    Http { status: Paid | Refunded } => ..., // or in field
}
```

Implemented via recursive `parse_sub_pattern` in the pattern parser.
See [PLANNING.md](PLANNING.md) for target milestone.

---

## See also

- [LOFT.md](LOFT.md) — match expression syntax reference
- [TUPLES.md](TUPLES.md) — tuple destructuring in match arms
- [MATCH_PEG.md](MATCH_PEG.md) — L3 design: PEG-style sequence/alternation/optional with anchor-revert captures
- [REGEX.md](REGEX.md) — standalone regex library for rich text matching (not a match-pattern kind)
- [PLANNING.md](PLANNING.md) — L2 backlog entry
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — #26 (guarded arms and exhaustiveness)
