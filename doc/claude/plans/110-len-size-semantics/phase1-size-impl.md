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

### ⚠ Finding — the plan's "array = N × 4 references" prose is wrong for loft

The README's illustrative rule ("an array counts each member as its 4-byte reference (N × 4)")
assumes vector elements are references. **They are not — loft stores vector elements INLINE:** a
scalar at its width (int 8, bool 1, narrow at its narrow width), a **struct at its full record size**
(`Point`/`VPoint` = 16, counted fully), a **text at its 4-byte handle** (the one genuine ref case),
a **nested vector at 8** (the #477 stride `element_size(inner).max(4)`, *not* 4). The faithful
`size` = N × real-stride is correct against the plan's **core** definition ("occupied bytes of this
allocation" — inline sub-records count fully) — it is only the plan's *illustration* that was based
on a layout loft doesn't use. **Action (Phase 2 doc pass):** correct the README's rule-#1 array
bullet to "each member at its in-buffer stride — inline value fully, a heap element (text, nested
collection) as its handle/stride," and keep the concrete table above as the reference.

## Remaining sub-arcs — formulas + open questions

Order: **1e (scalar) → 1b (struct) → 1c (hash) → 1d (sorted/index/spatial) → 1g (character, deferred
on a contract decision — below).** Rationale: scalar is a trivial constant (validates nothing new
but is free); struct reuses the record-size the vector stride already exposed; hash/sorted/index/
spatial each need their collection's table/tree byte accounting.

- **1e `size(<scalar>)`** = `element_size(τ)` (L-Scalar/L-Narrow): integer 8, float 8, boolean 1,
  single 4, character 4, u8 1, u16/i16 2, i32 4. A compile-time constant — the parser can emit a
  literal `Value::Int(width)` (no runtime op needed). Simplest; purely additive.
- **1b `size(struct)`** = the struct's finished **record size** (`database.size(type)` — the packed
  L-Struct total, with `text`/reference/collection fields at their 4-byte stored width, inline
  sub-records counted fully). Parser fills the record-size const from the struct type. **Open:** an
  enum value — 1-byte tag + the (packed) active variant's fields (L-Enum); decide whether `size(enum)`
  reports the max-variant record size (the allocated footprint) or the active variant's — recommend
  the **allocated record size** (max variant), since that is the bytes actually reserved.
- **1c `size(hash<…>)`** = the **full bucket table, holes included** (open addressing IS the format).
  Needs the hash allocation's table byte span from `src/hash.rs` (bucket count × bucket stride), not
  the live entry count. Parser/op reads the table size from the record.
- **1d `size(sorted / index / spatial)`** = the collection's table/tree bytes (sorted: the
  length-prefixed sorted buffer like vector; index: the red-black tree nodes; spatial: the radix/
  Morton tree). Each reads its structure's allocated byte span. **Open:** confirm whether "holes/
  spare nodes" count (recommend: the allocated structure bytes, mirroring hash's "full table").
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
