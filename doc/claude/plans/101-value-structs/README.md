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

**Locked decisions (user, 2026-07-08):** a value struct is *just bytes in the parent's Store,
addressed by offset* — like a scalar field, but multi-field.

1. **Part of ONE Store — no separate record, no DbRef.** A value struct's fields are packed
   **contiguously inside the parent's single Store allocation** (the allocator already
   supports a sub-record living inside a parent allocation; value structs always take that
   path). `field_ref` (`structures.rs:193`) hands back a fat `{store_nr, rec, pos+offset}`
   pointer for a struct field today even though the bytes are inline — **drop that inline
   DbRef**: a value-struct field is read/written **directly at `base + offset`**, no DbRef
   materialised.
2. **Non-null, enforced by initialisation.** Value structs have NO null. The compiler
   **forbids an uninitialised value struct** (require a value at declaration or fill a
   declared default) — no inline null sentinel, no `value struct?`. (Q4 closed.)
3. **Distinct, opt-in kind:** `value struct T { … }`. Value semantics (copy-on-assign, no
   aliasing) are *observable*, so NO silent size-based auto-promotion — it would change
   mutation/aliasing/@PLN85–90 ownership. Reference `struct` unchanged.
4. **Representation — value structs STAY `Type::Reference`; copy is an ISOLATED pass (DECIDED
   2026-07-08, user — "less complex, don't wire it in directly").** A distinct `Type::Value`
   variant threaded through the type system was tried and **reverted** (commit 1240f459): it
   caused a broad `if let Type::Reference` blast radius across many runtime sites plus an
   unbounded embedded-size refinement obligation. Instead, a value struct is an ordinary
   `Type::Reference` record, marked only by `Data.value_structs`. Its **storage is already
   inline** out-of-the-box (record fields via `finish_type`; vector elements via the backing
   stride). The ONLY behavioural change — value (copy) semantics — is a single self-contained
   IR pass (`value_struct_copy` in `scopes.rs`): it rewrites a value-struct local bind from a
   view into `OpDatabase` + `OpCopyRecord` so the local owns a copy. No `Type` match, no size
   function, no assignment-path surgery, no ownership-oracle change. Full recipe: STEPS §1.2.
5. **Zero-cost inside records + collections falls out** of "part of one Store": a
   value-struct field/element is packed inline in the parent record / vector backing — no
   per-field, per-element record (a `vector<V>` becomes contiguous like `vector<scalar>`).
6. **No lifetime/free** for pure-value structs (`has_lifetime_concern` = false). A value
   struct with a `text`/`vector`/reference field is lifetime-bearing → Slice 4 or disallowed.
7. **Both backends:** native rides the Rust struct/tuple ABI (the T1.8a pure-value path);
   interpret does inline packed reads/writes at offsets.

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
   before touching representation code (this is a core change; measure before cutting). **Done
   — and it corrected the cost map (see STEPS § Slice 0 findings):** the payoff is in transient
   locals/temporaries (a scratch store each), NOT collections (already inlined).
2. **Slice 1 is the highest-value slice** (not moot): one `value struct`, all-scalar, as a
   local — construct→field→operator→format→conversion — built inline, **`stores_allocated`
   delta 0** vs the reference-struct baseline (which is ~1 store per construction). This is
   where most of the win is, and it proves the model on the simplest cell.
3. **Slice 2** (record fields) — storage is already inline; the win is dropping the `field_ref`
   access DbRef + value semantics. **Slice 3** (`vector<V>`) — already mostly inlined; residual
   win is eliminating the per-element construction scratch (build in place, → single backing).
4. **Slice 4** handles the lifetime-bearing case (or defers it with a stated trigger).
5. **Slice 5** closes with native ABI parity + the alloc benchmark.

## Open design questions

1. **~~Nullable `value struct?`~~ — CLOSED (user, 2026-07-08):** value structs are non-null.
   The compiler forbids an uninitialised value struct (require a value or a declared
   default); no inline null sentinel, no `value struct?`.
2. **`&`-ref params** for in-place mutation of a value struct — what do tuples do today?
3. **Ownership** — make the @PLN85–90 deps analysis treat value structs as non-heap (no
   deps edges, no free); reuse `has_lifetime_concern`.
4. **@PLN97 layout contract** — value structs change record layout; the conformance tests
   must cover the inline embedding.
5. **Field access rewrite** — replace the `field_ref` inline-DbRef fat pointer with direct
   `base + offset` read/write for value-struct receivers (drop the DbRef materialisation).
5. **Keyword** — `value struct` vs `inline struct` vs a modifier on `struct`.

## See also

- Supersedes **@PLN99 Arc D** (the deferred "someday-perf" framing); @PLN99 still closes on semantics.
- Cooperates with @PLN25 (nullable), @PLN85/@PLN90 (ownership), @PLN97 (layout contract).
- Mechanisms: `src/data.rs`, `src/variables/mod.rs`, `src/typedef.rs`, `src/store.rs`;
  DESIGN_DECISIONS.md C65; [`lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md).
- Reference: [INTERMEDIATE.md](../../INTERMEDIATE.md), [DATABASE.md](../../DATABASE.md), [SLOTS.md](../../SLOTS.md), [TUPLES.md](../../TUPLES.md).
- Tracker: [@PLN101](https://github.com/loft-lang/plans/issues/101).
