<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN110 Phase 1 — the additive `size(x)` implementations

Phase 1 builds `size` on the structures where it does not exist today (`size` is text-only). Each
lands **green and independent** — nothing existing calls these, so implementing them breaks nothing;
only the *text* redefinition (Phase 2a) is a flip. This doc is the build spec: the uniform
mechanism, the per-type formula verified against the @PLN97 layout contract, and the build order.

## The uniform mechanism (established by 1a)

`size(x)` for a non-text type needs a **type-derived constant** the loft level can't see (a stride,
a record size), so it can't be a plain stdlib overload. It follows the `len(index)` pattern exactly:

1. **Declare a native op** in `default/01_code.loft` with a `#rust"…"` body — e.g.
   `fn OpSizeVector(r: vector, stride: const u16) -> integer;`. The `#rust` template feeds **both
   backends** (interp via generated `src/fill.rs`; native via `src/generation`), so they cannot
   diverge on the op body.
2. **Regenerate `fill.rs`:** `make fill` (enforced fresh by `issues::n9_generated_fill_matches_src`).
3. **Add a parser special-case** in `src/parser/mod.rs` (the fallback chain after the `len(ix)` case
   at ~`mod.rs:3014`): match `name == "size" && types.len()==1 && named_args.is_empty()` on the
   target type, compute the type-const, rewrite `*code = Value::Call(op_d_nr, args)`, return `I64`.
   Text keeps its existing stdlib `size(both: text)` overload — the special-case only catches the
   non-text types, so `size("…")` is unaffected (verified).

Inserting an op shifts downstream op indices — **safe**: indices are recomputed every build and
`make fill` keeps `fill.rs` in lockstep; no test pins absolute op numbers (checked).

## Sub-arc 1a — `size(vector<T>)` ✅ DONE

`size(v)` = **element count × the element's in-buffer stride** =
`length_vector(v) × vector_elem_iter_stride(T)`. The stride is loft's REAL per-element storage
width — the *same* one iteration walks the buffer with — so this reports the bytes the buffer
actually occupies. Excludes the buffer header (length/size words) and excludes spare capacity (uses
the logical length, not the allocation's capacity) — matching the plan's "content, not reserve" rule.

Verified on **both backends** (`tests/scripts/pln110-size-vector.loft`):

| vector | stride | size |
|---|---|---|
| `vector<integer>` [1,2,3] | 8 | 24 |
| `vector<boolean>` [t,f,t] | 1 | 3 |
| `vector<u8>` [10,20,30,40] | 1 (narrow) | 4 |
| `vector<text>` ["x","yy","zzz"] | 4 (handle) | 12 |
| `vector<Point>` (2× {int,int}) | 16 (**inline**) | 32 |
| `vector<VPoint>` value struct | 16 (**inline**) | 32 |
| `vector<vector<integer>>` [[1,2],[3]] | 8 (**#477**) | 16 |
| empty | — | 0 |
| after one `+= [7]` | 8 | 8 (not the over-allocated capacity) |

### The two representations — `Vector` (inline) vs `Array` (by-reference)

loft's surface `vector<T>` compiles to **two distinct internal representations**, and `size`
correctly reflects whichever is in play — this is exactly the plan's rule #1 ("inline sub-records
counted fully" AND "an array counts each member as its 4-byte reference, N × 4"):

- **`Parts::Vector<T>` — inline.** `T` is stored inline in the buffer at `size(T)`: a scalar at its
  width (int 8, bool 1, narrow at its narrow width), a **struct at its full record size** (`Point` =
  16, counted fully), a **text at its 4-byte handle**. size = `size(T) × len`.
- **`Parts::Array<T>` — by-reference.** When `T` is *linked* (shared with a keyed collection — it is
  an element of a `hash`/`index`/`radix` AND a vector, so both refer to the same records),
  `Stores::finish_type` (@P376, `types.rs:372`) promotes `vector<T>` to `Array<T>`: each element is a
  **4-byte rec-id** pointing at a separate record. size = `4 × len` (the plan's "N × 4 references").

The choice is a whole-program property decided at `finish()` (before pass-2 parse), and
`vector_elem_iter_stride` reads it via `is_linked(T)` — returning 4 for a linked/Array element and
`size(T)` for an inline one — so the parse-time stride const is **not** stale (verified: a genuine
Array gives `size = 4 × len` on both backends; `tests/scripts/pln110-size-vector.loft` covers both).
Either way `size` is allocation-local: for the Array it counts the 4-byte pointers, NOT the separate
Node records they reference. (Nested `vector<vector<T>>` is a third case: the element is an 8-byte
descriptor, the #477 stride `element_size(inner).max(4)` — `size` reports it faithfully.)

> **Correction:** an earlier draft of this finding claimed the plan's "array = N × 4" prose was
> wrong; that was a mistake — it had only tested the *inline* case. The plan's rule #1 describes both
> representations correctly, and the implementation handles both. No README change is needed.

## Remaining sub-arcs — formulas + open questions

Done so far: **1a (vector), 1b (struct), 1e (scalar), 1c (hash), 1d (sorted/index/spatial)**.
Remaining: **1g (`len(character)` contract call, deferred).** (enums ✅ done; the `size` half of 1g
is also done via 1e — only the `len(character)` decision is left.) Rationale: hash/sorted/index/
spatial each need their collection's table/tree byte accounting; enums split into simple (scalar-
like) and data (struct-like); 1g's *size* half is already done (via 1e, `size(character)`=4), leaving
only the deferred `len(character)` decision.

- **1e `size(<scalar>)` ✅ DONE** = the value's storage width (L-Scalar/L-Narrow): integer/float 8,
  boolean 1, character/single 4, u8 1, u16 2, i32 4. `OpSizeScalar(v: integer, sz: const u16)` — the
  arg is evaluated (side effects run: **every scalar is one 8-byte eval-stack slot**
  (`aligned_stack_step = size.next_multiple_of(8)`), so a single op reading an 8-byte slot consumes
  *any* scalar and discards it). Two width-source gotchas (both fixed): `element_size(Type::Integer)`
  and `Type::size(false)` both **over-report a narrow / `forced_size` integer** — `i32` reads as 8
  (its value range), not its declared 4 — so integers take the width from the FINISHED type
  (`get_type` → `database.size`, narrow-aware); the non-integer scalars use `element_size` (correct:
  bool 1, char/single 4, float 8). NOT a bare `Value::Int` rewrite — that would drop a side-effecting
  argument, inconsistent with the other `size` ops. Verified both backends
  (`tests/scripts/pln110-size-scalar.loft`), incl. `size(calc())` runs the call and the cross-check
  `size(inline vector<T>) == len × size(T)`. **`size(character) = 4` — this settles the *size* half of
  1g** (the code-point slot width, consistent with `size(vector<character>)` striding 4); it does NOT
  touch `len(character)`, so it does not pre-empt the deferred `len(character)` contract call.
- **1b `size(struct)` ✅ DONE** = the struct's finished **record size** (`database.size(type)`), a
  compile-time constant. `OpSizeStruct(r: reference, sz: const u16)` — the arg `r` is evaluated (so
  `size(make())` runs its argument, verified) but only its type feeds the const `sz`; parser
  special-case matches `Type::Reference` gated by `is_struct`. Verified both backends
  (`tests/scripts/pln110-size-struct.loft`): `Point`{int,int}=16, `Mixed`{text,int}=**12** (text =
  4-byte handle, **no tail padding**), `WithVec`{vector,int}=12 (collection field = 4-byte pointer),
  `Nested`{Point,int}=**24** (inline sub-record counted fully), `Flags`{bool,bool,int}=10,
  `OneByte`{bool}=1. Cross-check holds: `size(inline vector<Point>) == len × size(Point)`.
- **enums ✅ DONE** — dispatch splits three ways, all reusing existing ops:
  - a **simple** enum (`Type::Enum(_, false, _)`) is a **1-byte inline** discriminant (scalar-like) →
    `OpSizeScalar`, width 1.
  - a **data** enum referred to by the ENUM type (`Type::Enum(_, true, _)`) is a `DbRef` to a record
    = 1-byte tag + the **max variant's** packed fields → `OpSizeStruct`, `database.size(enum)`. A
    `Shape` slot reports the same size (24) for every variant — it must hold any of them.
  - a **bare variant** value (`Circle { r: 2.0 }`) has the VARIANT's type — a `Type::Reference` to an
    `EnumValue`, a struct-like record — so it reports the *variant's own* record (Circle 16, Rect 24),
    which is ≤ the enum's max-variant record. **This fixed a silent-empty bug in 1b:** the
    `size(struct)` branch matched `Type::Reference` broadly and returned `Unknown(0)` (→ silent empty
    output) for a non-struct reference; it now also accepts `is_enum_value` records and emits the
    standard `Unknown function` **error** (never a silent Unknown) for any other reference.
  Verified both backends (`tests/scripts/pln110-size-enum.loft`): simple 1; Shape-typed 24 (any
  variant); bare Circle 16 / bare Rect 24.
- **1c `size(hash<…>)` ✅ DONE** = the **full bucket table, holes included** = `elms × 4` where
  `elms = (record_words(claim) − 2) · 2` — each bucket a 4-byte `u32` rec-id, the two reserved
  header/seed words excluded (mirrors `size(vector)` counting content, not the length prefix). An
  empty slot (zero rec-id) still counts — open addressing's spare capacity IS the format, so unlike a
  vector's spare capacity it is NOT reserve. Allocation-local: the entry records are separate
  allocations, not counted. Needs no type-derived const, so it is a clean **stdlib overload**
  (`size(both: hash) → OpSizeHash → hash::table_bytes`), not a parser special-case — like
  `len(hash)`. Verified both backends (`tests/scripts/pln110-size-hash.loft`): empty → 0; initial
  table (room 10) → 16 slots × 4 = 64; after rehashes (room 10→19→37, load 0.75) → 136 → 280. The
  step sizes encode the resize policy, so a policy change flips the test (a conscious layout change,
  @PLN97).
- **1d `size(sorted / index / spatial)` ✅ DONE** — these split by whether there IS an aggregate
  structure (owner guidance 2026-07-17):
  - **`sorted<T[key]>`** shares vector's length-prefixed BUFFER → `size = len × stride` (reuse
    `OpSizeVector`, stride = `vector_elem_iter_stride(content())` = the node record inline). A real
    aggregate: `size(sorted) == len × size(node)`.
  - **`index<T[key]>`** (red-black tree) and **`spatial<T[keys]>`** (radix/Morton tree) keep their
    ordering as bookkeeping embedded IN each element record — **there is NO separate structure
    allocation to sum**, so `size` reports a **SINGLE node record** (a compile-time constant,
    independent of the element count; reuse `OpSizeStruct` with `database.size(element node type)`).
    An index node = the element + its RB bookkeeping (left/right/colour), so a plain `{int,int}` node
    is 25 (16 + 9). `size(index)` stays constant as `len` grows; an empty index still reports the
    node size.
  Verified both backends (`tests/scripts/pln110-size-sorted-index-spatial.loft`): sorted 3 → 75 =
  3 × 25; index (2 then 5 elements) → 25 (unchanged); spatial → 24 (one `{int,int,int}` record).
  Reusing `OpSizeVector`/`OpSizeStruct` for `Sorted`/`Index`/`Radix` args works on both backends (no
  dedicated op needed). ⚠ Note: a struct used as a keyed-collection element carries the bookkeeping
  everywhere, so `size(K{…})` is 25 (not 16) in a program where `K` is an index element — a
  whole-program layout property, like the vector/array promotion.
- **1f `s[p]`** — no-op (already correct; just don't regress). Covered by the 0e golden fixture § B.

## 1g `size(character)` / `len(character)` — DEFERRED, needs a contract call

Building 1a **corrected** the earlier 0f finding-7 recommendation. A `character` is stored as a
**fixed 4-byte slot everywhere it lives inline** — a `vector<character>` strides 4, a struct
`character` field is 4. So there are three distinct quantities, and no single assignment is clean:

1. **logical count** = 1 (a character is one character)
2. **slot / storage width** = 4 (how it is stored inline — used by `vector<character>` stride, struct layout)
3. **UTF-8 encoded width** = 1–4 (bytes when encoded into text) — **today's `len(character)`**, and
   the quantity the byte-world identity `c#next == c#index + len(c)` (STDLIB.md:168) depends on.

- The plan table says `size(scalar) = scalar width` → **`size(character) = 4`** (quantity #2). This
  is **consistent** with `size(vector<character>) = N × 4` and with `size(<scalar>)` — recommended.
- **Do NOT redefine `len(character)` to 1** (my earlier 0f rec — now retracted): that destroys the
  useful, *used* quantity #3 for nothing. `len(character)` = UTF-8 encoded width is the byte-world
  companion to loft's byte-addressed text surface (it pairs with `#index`/`#next`, `s[p]`, `find`),
  so returning a byte quantity there is *consistent with the byte world*, even though `len(text)` is
  a char count — the two live in different worlds by design.
- Net recommendation: **add `size(character) = 4`; keep `len(character)` = UTF-8 byte width
  unchanged.** The surface stays coherent (byte world: `size`=slot, `len(character)`=encoding width,
  `#index`/`#next`; char world: `len(text)`=count). **Flag for owner** — this reverses the 0f note
  and touches the `character` contract; it is a genuine judgment call, so it does not ship until
  confirmed. (0f finding 7 in phase0-inventory.md is annotated with this correction.)

## Validation gate (every sub-arc)

Hand-compute each value from the layout rules, then confirm **identical on both backends**
(`--interpret` and `--native`) via a `tests/scripts/pln110-size-<type>.loft` fixture, and run
`make fill` + `n9_generated_fill_matches_src` + `wrap loft_suite` + `native` before landing.
