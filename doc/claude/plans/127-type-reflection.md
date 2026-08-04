<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN127 — Type reflection from loft code

## Status

**Open — no implementation.** The substrate is built and validated; nothing projects it
into loft.  Arc A (the two mainline defects below) is independently shippable and should
land first, because it repairs the only field enumeration loft has today.

**Issue:** [loft-lang/plans#127](https://github.com/loft-lang/plans/issues/127).

## Goal

A loft program can ask, at runtime, for the **declared shape of a type** — fields with
their names, declared types and offsets, enum variants, keys — without holding a value of
that type and without a JSON round-trip.

## Effort + design

- **Effort:** M (arc A is XS/S; arcs B–D are the M)
- **Design:** ~ (partial — the substrate is settled, the entry points are not)
- **Last touched:** 2026-08-04

## What already exists

loft has three levels of reflection.  Two of them are reachable from loft code; the one an
ORM or a generic serialiser actually needs is not.

| level | what | reachable from |
|---|---|---|
| **value** | `{x:j}` out, `Type.parse(text)` in, `json_parse` → `JObject.fields` | **loft code** |
| **frame** | `stack_trace()` → `StackFrame.variables[].{name, type_name, value}` | **loft code** |
| **schema** | `LayoutDesc` — per-type nodes, field name + byte position + content type-id, base kinds, enum variant ids + names, narrow-scalar nullability, keyed-collection kinds and key lists | Rust only |
| **compile-time** | `loft introspect` — IR + bytecode, field offsets, type ids | CLI only |

The schema row is the whole point, and it is **already built**.  `LayoutDesc`
([`src/database/descriptor.rs`](../../../src/database/descriptor.rs), shipped with @PLN105)
reproduces `Stores::layout_dump` byte-for-byte — so its hash *is* `layout_algo_hash`, the
@PLN97 layout identity — and it already crosses a language boundary: the JS reader walks
any loft value from it with no copy and no serialization.

**A foreign reader in JavaScript can enumerate a loft type's fields today.  A loft program
cannot.**

`stack_trace` ([`default/04_stacktrace.loft`](../../../default/04_stacktrace.loft), @F88) is
the shipped precedent for the *shape of the answer*: a Rust-side fact surfaced as an
ordinary loft struct family (`StackFrame` / `VarInfo` / `ArgValue`), filled by a native
builtin.  Measured — it returns the declared type names of live locals.

## The gap, stated precisely

From loft code it is impossible to obtain:

- the field list of a type when no value is in hand;
- a field's **declared type** (the JSON route gives names only, and erases `integer` vs
  `i32`, `float` vs `single`);
- whether a field is nullable, `const`, or a hash/index key;
- a type's enum variants;
- byte offsets or record size.

## The workaround is broken, not merely awkward

`{x:j}` + `json_parse` → `JObject.fields` is the only field enumeration loft has, and it
fails on two ordinary shapes.  Both reproduce on `--interpret`, on `--native`, and on a
pre-branch installed binary, so both are mainline:

- **[loft#768](https://github.com/loft-lang/loft/issues/768)** — a struct field of an enum
  type renders a bare unquoted variant name (`{"kind":Circle {"r":2}}`), which is not JSON,
  so `json_parse` rejects the **whole** document.  A bare enum renders correctly, so the
  defect is specific to the field position.  Any struct containing an enum field cannot be
  reflected over at all.
- **[loft#769](https://github.com/loft-lang/loft/issues/769)** — a `text?` holding null
  renders as an escaped-NUL string instead of JSON `null`, so absent becomes
  present-but-corrupt.

## Why this should be cheap

Every fact the API needs is already computed, already walked (`layout_closure`), already
serialised (`layout_dump`, the `.dschema` sidecar) and already delivered across an FFI
boundary.  The work is a **projection**, not a new analysis: walk `LayoutDesc` into a loft
struct family, exactly as `stack_trace` walks frames.

## Composition matrix — Stage A

Reflection adds an operation over *every* type, so its matrix is the type space itself.
Each cell asks the same question — does reflecting this type report the shape the source
declares — and every cell must be green on **both backends** before the API is called done.

| axis | cells |
|---|---|
| scalar width | `integer`, `i32`/`u8`/narrow, `long`, `float`, `single`, `boolean`, `character` |
| nullability | `T` vs `T?`, on each of the above and on `text` |
| composition | flat struct · nested struct · value struct · struct in vector · vector of struct |
| enum | value enum · struct-enum · enum as a struct field · recursive enum |
| collection | `vector` · `hash` · `index` · `sorted` (the `Iterated` kinds) |
| declared vs storage | narrow int fields, value structs — where the two answers differ |
| generics | a monomorphised generic struct: which name does reflection report |
| identity | a type reflected via `type_of` vs via `type_named` must answer identically |

The last row is the one that catches the real hazard: `--native` **replays** the
parse-time type order in generated `init()` rather than minting ids
(`LOFT_STRICT_SCHEMA_IDS`, loft#739), so a name→id lookup is exactly where the two
backends can silently disagree.  Probe it with the strict flag on.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — repair the fallback: loft#768 + loft#769 | [#768](https://github.com/loft-lang/loft/issues/768), [#769](https://github.com/loft-lang/loft/issues/769) | Open |
| **B** — the type-info struct family + `type_of(value)` | this doc | Open |
| **C** — reflection with no value: `type_named(text)` | this doc, Q1 | Open |
| **D** — the declared-vs-storage contract | this doc, Q2/Q3 | Open |
| **E** — a real consumer built only through the API | this doc | Open |

## Phase ordering

1. **A** first, alone.  It is independently valuable, it is the smallest thing here, and it
   unblocks the workaround everyone is told to use while B–E are still unbuilt.
2. **B** — `default/07_reflect.loft` plus a native filler over `LayoutDesc`, mirroring
   `stack_trace`.  Read-only, `type_of(value)` only.  This is where the Stage-A matrix is
   built and run.
3. **D** before **C**: what the API *answers* (declared or storage type) has to be settled
   before adding a second way to ask the question, or the two entry points will disagree.
4. **C** — the name→id lookup, with the identity row of the matrix as its gate.
5. **E** — a generic serialiser or a small ORM mapping written *only* through the
   reflection API.  This is the dogfood gate that decides whether the API is sufficient;
   until it exists the design is a hypothesis.

## Open design questions

1. **Entry points.** `type_of(value)` is easy.  `type_named("Row")` needs a name→type-id
   lookup that survives `--native`, where the type table is replayed rather than minted.
   This is the one genuinely load-bearing question.
2. **What `LayoutDesc` does not carry.**  Verified present: names, sizes, field positions,
   enum variants, narrow-scalar nullability, key lists.  Verified **absent**: `const`-ness,
   and nullability for non-narrow types.  Either the descriptor grows those, or reflection
   reads them from a second source — and a second source means the same fact derived twice,
   which is the failure this repo has paid for before.  Decide before building.
3. **Storage type vs declared type.**  The descriptor is a *storage* layout; a caller asking
   "what type is this field" wants the *declared* type.  They differ for narrow ints and
   value structs.  Which one the API answers is a contract decision, not an implementation
   detail.
4. **Write access.**  Read-only reflection first.  Constructing or mutating a value by field
   *name* is a separate and larger question — out of scope until read ships.

## Cross-arc dependencies

- **@PLN105** (closed) — where `LayoutDesc` came from.  This plan is its loft-side sibling:
  the same descriptor, projected to loft instead of to JS.
- **@PLN97** — the layout contract the descriptor is pinned against.  Reflection must not
  become a second, drifting description of the same layout.
- **@PLN128** — unrelated in subject, but shares the lesson: check what the substrate
  already does before designing on top of it.

## See also

- [`src/database/descriptor.rs`](../../../src/database/descriptor.rs) — the substrate.
- [BROWSER_INTEROP.md § The binary bridge](../BROWSER_INTEROP.md) — the shipped reader that
  already consumes it.
- [`default/04_stacktrace.loft`](../../../default/04_stacktrace.loft) — the precedent for the
  answer's shape.
- [`doc/claude/DATABASE.md`](../DATABASE.md) — stores, `Parts`, and the schema sidecar.
