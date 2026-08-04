<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN127 — Type reflection from loft code

## Status

**Arcs A, B, C, D and E are BUILT.** Arc D shipped the nullability half; `const` is
deliberately left out.

Arc B projects `LayoutDesc` into loft: `default/07_reflect.loft` declares `TypeInfo` /
`FieldInfo` / `VariantInfo` / `TypeKind`, `type_of(x)` is intercepted in
`src/parser/control.rs` and lowered to `n_reflect_type(<type-id>)`, and one filler
(`native::reflect_type_into`) serves both backends — the interpreter through
`src/native.rs`, `--native` through a `codegen_runtime` wrapper onto the SAME function.
A second implementation there would be exactly the drift this plan exists to avoid.

Four things the build settled that the design had not:

- **Q1 dissolves for `type_of`.** The type id is resolved where the call is WRITTEN, so
  it is a parse-time constant — the mechanism `to_json` already uses. `--native` replays
  the type table rather than minting it, and a parse-time id is replayed with it. Q1 is
  only load-bearing for a name given at RUNTIME, which is arc C's real question.
- **Q3 answers itself for two scalars, and only two.** `get_type` — the one existing
  storage derivation — reports `integer` for a `character` (which is how it is stored)
  and has no entry at all for a `boolean` (`#65535`). Reflection names those two
  directly and keeps the single derivation for everything else, because a second
  derivation is a second thing to drift. Narrow ints still report storage, and `size`
  is where that shows.
- **Reflection inside a generic is NOT reachable this way.** A generic body is parsed
  ONCE against its type variable, so `type_of(v)` there answers `__typevar_T`. The same
  mechanism makes `"{v:j}"` in a generic body render `{}`. It needs the body parsed per
  instantiation, which is a different plan; the doc comment says so rather than
  implying otherwise.
- **Arc A was a prerequisite in fact, not just in order.** A `TypeInfo` holds an enum in
  a struct field, which is precisely the shape that made `json_parse` reject a whole
  document. `"{t:j}"` on a `TypeInfo` renders complete JSON only because arc A landed.

Arc C is `type_named(name) -> TypeInfo?` — reflection with no value in hand, the shape an
ORM needs when the name arrives from a config file or a catalogue. No parser intercept: the
name is a RUNTIME value, and the lookup works on `--native` because the generated `init()`
replays the type registrations, names included, and `Stores::name` is a TOTAL lookup that
answers absent rather than minting a type for a typo. Measured on both backends, including
a name held in a variable. **That is Q1, answered rather than worked around.**

Arc E is `tests/scripts/pln127-reflect-consumer.loft`: an ORM's schema half — `CREATE TABLE`
generated from a loft struct, written only through the API, with the table name arriving as
a runtime value. It passes on both backends, and being used as a gate it found two things:

- **It cannot emit `NOT NULL`.** Nullability is not in the answer because it is not in the
  STORE: `Field` carries a name, a content type-id, a position and a default, and nothing
  else. A narrow scalar does record a nullable flag; `text` and a record reference spell an
  absent value with a SENTINEL instead — `text?` is stored as `"\0"`, which is precisely
  what arc A had to repair in the JSON writer. The same holds for `const`.
- **It had to be a schema generator, not the plan's "generic serialiser".** Reflection
  describes a TYPE; there is no way to read a VALUE's field by name, and a serialiser needs
  both. That is the write-access question the plan deferred, arriving from the read side.

### Arc D — nullability yes, `const` no

The plan asked whether `LayoutDesc` should grow `const`-ness and non-narrow nullability, or
read them from a second source. Measuring found neither was the real choice, because
**neither is a storage fact**: `text?` and `text` share a content type and spell absence with
a SENTINEL, so nothing in the stored bytes implies either. (A NARROW int is the exception —
it registers a distinct content type per nullability, which is why the descriptor already
reported nullable for those and only those.) So D was really: *does reflection report facts
that exist only in the source?*

**The line drawn: what a VALUE can be, yes; what CODE may do to it, no.** Nullability is the
first kind and is load-bearing — arc E's generator was complete, correct, and could not emit
`NOT NULL`, which does not make a DDL less detailed, it makes it accept rows the loft type
would refuse. `const` is the second kind: it constrains loft code, not data, so an ORM has no
use for it and admitting it would make reflection a mirror of the source text.

Built as a deposit at the one parse-time site that knows (`typedef.rs`, where `Optional(τ)`
is peeled), **replayed by the native generator**, carried by `LayoutField` and *not* rendered.

Two things this cost that reading the code would not have shown:

- **`--native` reported `nullable=false` for every field** until the generator emitted the
  deposit too, because it rebuilds the schema by REPLAYING `init()`. The parity probe caught
  it; the fix wraps `emit_field` rather than sitting in either caller, because there are two
  call sites and the one that mattered was not the obvious one.
- **The IR-store round trip had to carry it too**, or a schema read back from a store
  answered "not nullable" for every field. That grew `DbField` by a byte, which needed
  `CACHE_FORMAT_VERSION` bumped — the stdlib cache key does not fold in the binary's mtime,
  so a cache written at the old stride was read at the new one and panicked. 25 LSP tests
  failed on that one cause.
- **The @PLN97 layout identity is unchanged, and that is measured rather than argued.** A
  store written by the pre-arc-D binary loads under the arc-D binary through both the
  whole-image and keyed paths with `ok=true`, and the same gate still REFUSES a genuinely
  reshaped layout. `layout_hash` hashes `render_dump()`, and nullability is carried there
  but never rendered.

Also settled, and it removes the plan's stated reason for ordering D before C: the two entry
points cannot disagree, because `type_of` and `type_named` are two ways to reach ONE filler.

Arc A landed first because it repairs the only field enumeration loft has today, and it
was a repair rather than a feature: both defects were WHOLE-DOCUMENT failures, so a
struct holding either shape could not be read back at all.

- **loft#768** — an enum-TYPED position (a struct field, a vector element) wrote its tag
  bare, which is not JSON. Two writers render an enum and only one knew about JSON:
  `Parts::EnumValue` wrapped as `{"Circle":{…}}`, `Parts::Enum` did not. The typed
  position now wraps the same way, and `walk_parsed_into` already accepted that shape —
  so writer and reader name ONE shape between them rather than two.
- **loft#769** — an absent `text?` is stored as the sentinel `"\0"`, not as a null
  pointer, so it reached the escaper and came back as a present one-character string. It
  is the same absence the null-pointer branch beside it already rendered as `null`.

Seven cells in `tests/scripts/57-json.loft`, both backends, each proven able to fail
first. The debug form (`{x}`) is unchanged — only the re-parseable forms make a
round-trip claim.

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
| **A** — repair the fallback: loft#768 + loft#769 | [#768](https://github.com/loft-lang/loft/issues/768), [#769](https://github.com/loft-lang/loft/issues/769) | **Built** — `src/database/format.rs`, cells in `tests/scripts/57-json.loft` |
| **B** — the type-info struct family + `type_of(value)` | this doc | **Built** — `default/07_reflect.loft`, `native::reflect_type_into`, `tests/scripts/pln127-reflect.loft` |
| **C** — reflection with no value: `type_named(text)` | this doc, Q1 | **Built** — `native::type_named_in`, both backends, runtime-valued names |
| **D** — the declared-vs-storage contract | this doc, Q2/Q3 | **Built** — nullability reported, `const` deliberately not; cross-version store load proves the layout identity is untouched |
| **E** — a real consumer built only through the API | this doc | **Built** — `tests/scripts/pln127-reflect-consumer.loft` (CREATE TABLE from a struct) |

## Phase ordering

1. ~~**A** first, alone.~~  **Done.**  It was independently valuable, it was the smallest
   thing here, and it unblocks the workaround everyone is told to use while B–E are still
   unbuilt.  It also corrected the plan's own framing: the fallback was not merely
   awkward, both defects rejected the WHOLE document, so "enumerate a value's fields with
   `{x:j}` + `json_parse`" was not a degraded path but an unavailable one for any struct
   holding an enum or an absent `text?`.
2. ~~**B** — `default/07_reflect.loft` plus a native filler over `LayoutDesc`, mirroring
   `stack_trace`.~~  **Done**, read-only and `type_of(value)` only.  The matrix is
   `tests/scripts/pln127-reflect.loft`: a record with hand-checked byte offsets, an enum
   whose tags start at 1 (0 is how the store spells ABSENT), a struct-enum variant, a
   nested record, a vector's element, all five scalars, and the `TypeInfo` itself
   serialising.
3. **D** before **C**: what the API *answers* (declared or storage type) has to be settled
   before adding a second way to ask the question, or the two entry points will disagree.
   Arc B settled the part it could not avoid — declared for `boolean` and `character`,
   storage for narrow ints — so D is now about `const`-ness and non-narrow nullability,
   the two facts `LayoutDesc` verifiably does not carry.
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
