<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 101 — Zero-cost value structs

> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN101](https://github.com/loft-lang/plans/issues/101) ← lifecycle label. This README is the source of truth for **per-slice status + design**.

## Status

**`status:active` — filed 2026-07-08. TOP PRIORITY; other language-feature work held for
this (user directive).** Today a `struct` value is a 12-byte `DbRef` + heap record +
`OpFreeRef` lifetime — standalone AND as a field (the DbRef sits inline in the parent
record, pointing to a *separate* record), so `event.when.ms` is a double indirection. This
plan adds a `value struct` kind stored **inline / by value** (packed fields, no DbRef, no
heap, no free), zero-cost as a local, a temporary, and **inside another record**. It is the
zero-cost-abstraction half of first-grade types (@PLN99 shipped the *semantics*; this ships
the *cost*), and it is core to why loft exists — the ergonomic wrapper must be as cheap as
the raw field, or people fall back to raw `integer`/`single` and the abstraction is dead.

## Goal

`value struct DateTime { ms: integer }` lays out inline like a tuple, carries full
first-grade semantics (operators/format/conversions), allocates nothing, frees nothing, and
embeds inline when used as a field — validated by flipping `tests/scripts/515` and proving
its heap-alloc count goes to zero on both backends.

## Effort + design

- **Effort:** VH (foundational value-representation change; months-scale, phased)
- **Design:** ~ (direction settled — reuse the tuple inline-aggregate machinery; per-slice design as we go)
- **Last touched:** 2026-07-08

## Why feasible (not "technically impossible")

The mechanism already exists and is proven:

| Fact | Location |
|---|---|
| A struct value / field is a 12-byte `DbRef` → heap record (the cost to remove) | `data::element_size`, `variables::size` → `size_of::<DbRef>()` for `Type::Reference` |
| **Tuples are already inline zero-cost aggregates** (pure-value → Rust tuple ABI, no heap/free) | `data.rs` tuple arm of `element_size`; `has_lifetime_concern` (`data.rs:1863`) |
| Inline layout machinery | `element_size` / `element_offsets` / `element_align` / `calculate_positions_with_groups` |
| The door was left open on purpose | DESIGN_DECISIONS.md **C65** ("a future feature that introduces inline value structs would re-open the row") |
| First-grade dispatch is representation-independent | `Data::find_op_method` resolves `t_<len><Type>_Op…` by type-def, not layout (@PLN99) |

**A value struct = a named tuple with methods.** We generalize this, not invent heap surgery.

## Design shape

1. **Distinct, opt-in kind:** `value struct T { … }`. Value semantics (copy-on-assign, no
   aliasing) are *observable*, so NO silent size-based auto-promotion — it would change
   mutation/aliasing/@PLN85–90 ownership. Reference `struct` unchanged. (Auto-inlining POD
   structs is a later option once the kind is proven.)
2. **Type representation:** a `Type::Value(def)` variant (or a kind-flag on the struct def)
   so `size()`/`align()`/`element_size()` return the packed inline size, not `DbRef`.
3. **Zero-cost inside records falls out of the inline layout:** a value-struct field embeds
   its packed fields directly in the parent record, exactly like a tuple field already does.
4. **No lifetime/free** for pure-value structs (`has_lifetime_concern` = false). A value
   struct with a `text`/`vector`/reference field is lifetime-bearing → Slice 4 or disallowed.
5. **Both backends:** native rides the Rust struct/tuple ABI (the T1.8a pure-value path);
   interpret does inline packed reads/writes.

## Composition matrix — Slice 0 (probe-first, before any code)

Vary ONE axis per probe, hand-compute each cell, assert **value AND heap-alloc-count AND
no-leak** on **both backends**:

- **placement** × {local, temporary, field-in-reference-struct, field-in-value-struct, `vector<V>` element, fn arg, fn return}
- **operation** × {construct, field read, field write, `<`/`==`/`-` operator, `{x}` / `{x:spec}` format, `as` conversion}
- **field shape** × {all-scalar (pure-value), nested value struct, (later) text/vector element}
- **backend** × {interpret, native}

The feature is done not when the demo runs but when every cell is green on both backends AND
the alloc-count cells read zero; probes graduate to `tests/scripts/`.

## Sub-arcs (slices)

| Slice | Ships | Status |
|---|---|---|
| **0** — composition matrix as `/tmp` probes | the acceptance spec | Open (next) |
| **1** — `value struct` scalar fields as a LOCAL: construct, field r/w, operators, format, conversions, ZERO alloc, both backends. Driver: flip `DateTime`/`Duration` in `515` | the kind exists end-to-end | Open |
| **2** — zero-cost INSIDE records: value-struct field inline in a reference struct / another value struct | "inside records too" | Open |
| **3** — collections: `vector<V>` stored inline (the DB-column win) | bulk zero-cost | Open |
| **4** — lifetime-bearing value structs (text/vector fields), or explicit deferral | full generality | Open |
| **5** — native ABI parity + perf validation (515 alloc benchmark → 0) | proof | Open |

## Phase ordering

1. **Slice 0** first — the matrix is the spec; write the probes and the alloc-count harness
   before touching representation code (this is a core change; measure before cutting).
2. **Slice 1** — the smallest complete vertical: one `value struct`, all-scalar, as a local,
   through construct→field→operator→format→conversion, zero alloc. Prove the model on the
   simplest cell before generalizing.
3. **Slices 2–3** widen placement (records, then collections) — where the real payoff lives.
4. **Slice 4** handles the lifetime-bearing case (or defers it with a stated trigger).
5. **Slice 5** closes with native ABI parity + the alloc benchmark.

## Open design questions

1. **Nullable `value struct?`** — no DbRef `store_nr` sentinel; needs an inline null
   representation (ties into @PLN25's null model).
2. **`&`-ref params** for in-place mutation of a value struct — what do tuples do today?
3. **Ownership** — make the @PLN85–90 deps analysis treat value structs as non-heap (no
   deps edges, no free); reuse `has_lifetime_concern`.
4. **@PLN97 layout contract** — value structs change record layout; the conformance tests
   must cover the inline embedding.
5. **Keyword** — `value struct` vs `inline struct` vs a modifier on `struct`.

## See also

- Supersedes **@PLN99 Arc D** (the deferred "someday-perf" framing); @PLN99 still closes on semantics.
- Cooperates with @PLN25 (nullable), @PLN85/@PLN90 (ownership), @PLN97 (layout contract).
- Mechanisms: `src/data.rs`, `src/variables/mod.rs`, `src/typedef.rs`, `src/store.rs`;
  DESIGN_DECISIONS.md C65; [`lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md).
- Reference: [INTERMEDIATE.md](../../INTERMEDIATE.md), [DATABASE.md](../../DATABASE.md), [SLOTS.md](../../SLOTS.md), [TUPLES.md](../../TUPLES.md).
- Tracker: [@PLN101](https://github.com/loft-lang/plans/issues/101).
