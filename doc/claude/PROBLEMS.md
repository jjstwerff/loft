
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

**Before opening a new issue here, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** — the closed-by-decision
register holds items explicitly evaluated and declined (C3 / C38 /
C54.D / …).  If your symptom maps onto one of those, the fix is to
produce new evidence (reproducer, incident, measurement) on the
existing entry, not re-open it as a bug.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 192 | `len()` not defined for `hash<T[key]>` or `index<T[key]>`.  Only `vector` and `sorted` have `len()` overloads (via `OpLengthVector` / `OpLengthSorted` which both delegate to `vector::length_vector`'s length-prefix read).  Hash needs `src/hash.rs::records().len()` (walks the bucket array, O(n)); index needs a tree traversal (O(n)).  Adding a `len()` overload for these would require a new runtime helper per kind that returns the count by full traversal. | Low | **Workaround:** count via iteration: `count = 0; for _ in h { count += 1; }` |
| 191 | `for x in <local index/hash>` produces wrong results.  Sequential iteration over a local-var `index<T[key]>` returns **0 elements** (`for s in ix { sum += s.value }` over `ix` with two entries gives `sum=0`).  Sequential over a local-var `hash<T[key]>` returns wrong sums (`sum=195` instead of `30` for `{a:10, b:20}`).  Struct-field iteration works for both (`for s in db.items` returns the right values).  P190 fixed the on-demand db type registration so the iteration codegen no longer panics, but the underlying iteration (OpStep / fill_iter on=1 for index, n_hash_sorted-driven for hash) still doesn't produce the right element sequence for local-var keyed collections.  **Blocks par_index_input_t4 + par_hash_input_t4** which use the local-var pattern in their test setup. | Low | **Workaround:** put the index/hash inside a `struct` field (`struct Db { items: index<T[key]> }`) — works correctly today. |
| 189b | `vector<(T1, T2, …)>` element field access via DbRef returns garbage.  `pairs[0]` returns a 16-byte `DbRef` to the heap record holding the tuple bytes; `.0` / `.1` parses to `OpTupleGet` which reads 8 bytes from the local slot directly — but the slot holds the DbRef, not the inline tuple.  Result: reading `pairs[0].0` returns `(store_nr \| (rec << 32))` masquerading as `i64` (saw `21474836482` instead of `1` for `(1, 10)`).  Iterating with `for p in pairs { … }` reports `Field access not supported on type tuple([…])` instead of unboxing.  P189 (literal construction + `len()`) is fixed — this is the access-side follow-up. | Low | **Workaround:** wrap the tuple in a `struct` (`struct Pair { a: integer, b: integer }`) and use `vector<Pair>`.  Struct field access via DbRef is correct. |
| 189d | `vector<(T1, text)>` element write returns 0 length for the text element.  P189c closed the per-attribute write path for primitive tuple elements (`(int, int)` works), but text elements within a vector-of-tuple read back as empty / zero-length.  Surfaced via `par_tuple_input_int_text` — workers see `len(p.1) == 0` instead of the expected `3, 3, 5`.  Likely root cause: `set_field`'s `Type::Text` arm in `src/parser/mod.rs:1716` writes via `OpSetText` which interns the string into the field's local store, but a vector-element write needs a different routing because the "field" is a tuple element inside a vector record (different store / different position computation). | Low | **Workaround:** wrap the tuple in a `struct` containing a text field — works correctly via the standard struct path. |

## Interpreter Robustness

### 192. `len()` missing for hash and index

**Symptom:** `len(h)` where `h: hash<T[key]>` errors with
`Unknown function len — did you mean the method x.len(…) on
text / character / vector / sorted / JsonValue?`.  Same for
`len(ix)` on `index<T[key]>`.

```loft
struct Score { name: text not null, value: integer }
struct Db { items: hash<Score[name]> }
fn test() {
  db = Db { items: [Score { name: "a", value: 10 }] };
  println("count = {len(db.items)}");   // ← parse error
}
```

**Where:** `default/01_code.loft` only declares:
- `len(both: text)` (line 636)
- `len(both: character)` (line 651)
- `len(both: vector)` → `OpLengthVector` (line 825)
- `len(both: sorted)` → `OpLengthSorted` (line 836, added
  earlier this session for sorted parity).

`OpLengthSorted` delegates to `vector::length_vector` which
reads the length-prefix word at offset 4 of the backing
record.  Sorted shares this layout with vector.  Hash and
index do NOT — hash uses a bucket array (count requires
walking via `src/hash.rs::records()`), index uses a red-black
tree (count requires traversal via `src/tree.rs`).

**Fix path:** add two new runtime helpers — one in `src/hash.rs`
(`pub fn count(h: &DbRef, stores: &[Store]) -> u32` that walks
the bucket array via the same loop as `records()`), one in
`src/tree.rs` (recursive in-order count).  Then add
`OpLengthHash` and `OpLengthIndex` in `default/01_code.loft`
that delegate to these helpers.  Mirrors the OpLengthSorted
shape; ~30 lines total.

**Severity:** Low — `count = 0; for _ in h { count += 1 }` is
a one-line workaround that gives the same O(n) cost as a
traversal-based `len()` would.

### 191. Local-var index/hash iteration returns wrong elements

**Symptom:** sequential iteration over a local-var
`index<T[key]>` returns 0 elements; over a local-var
`hash<T[key]>` returns wrong sums.  Struct-field iteration
of the same types works correctly.

```loft
fn test() {
  ix: index<Score[name]> = [];
  ix += Score { name: "a", value: 10 };
  ix += Score { name: "b", value: 20 };
  sum = 0;
  for s in ix { sum += s.value; }
  println("sum = {sum}");   // prints "sum = 0", expected 30
}
```

```loft
fn test() {
  h: hash<Score[name]> = [];
  h += Score { name: "a", value: 10 };
  h += Score { name: "b", value: 20 };
  sum = 0;
  for s in h { sum += s.value; }
  println("sum = {sum}");   // prints "sum = 195", expected 30
}
```

The struct-field versions both return the correct sum (30):

```loft
struct Db { items: index<Score[name]> }
// or: struct Db { items: hash<Score[name]> }
db = Db { items: [Score { name: "a", value: 10 }, Score { name: "b", value: 20 }] };
for s in db.items { sum += s.value; }   // sum = 30 ✅
```

**Where:** P190 (commit `6ffbe6a`) fixed the on-demand
`database.index` / `database.hash` registration in
`src/parser/vectors.rs::get_type` so the iteration codegen no
longer panics on local-var keyed collections.  But the
**underlying iteration mechanism** still doesn't produce the
right element sequence for local-var index/hash:

- **Index** (`fill_iter` on=1, `Parts::Index`): iteration walks
  the tree.  For local-var index, returns 0 elements — likely
  the tree root pointer is unset or the local's storage layout
  differs from struct-field layout in a way that confuses
  `OpStep`'s tree-walk.
- **Hash** (`fill_iter` on=3, `Parts::Hash`): expects the
  parser's hash-special-case at `src/parser/collections.rs:990-1008`
  to have substituted the iterated expression with a
  `n_hash_sorted` scratch.  For local-var hash this still runs,
  but the resulting sum is wrong (195 vs 30) — likely a
  store-rebase or scratch-layout interaction.

**Sorted is unaffected** — P190 plus the standard
`fill_iter` on=2 path produces correct results (verified by
`tests/issues.rs::p190_local_var_sorted_iteration`).

**Fix path (investigated, found deeper than expected):**

Initial hypothesis was that `gen_set_first_keyed_null`
(P188's local-var alloc) registers the database type too
late — after the struct-layout pass.  For `database.index`,
this is a real concern: it appends bookkeeping fields
(#left/#right/#color) to the content struct, and they need
positions assigned by `finish_type`.  But pre-registering
in `typedef.rs::fill_all` (a tested attempt) causes:
- Index: SIGSEGV in the codegen path — appending fields
  after `database.field` ran for the user fields shifts
  layout and confuses downstream emission.
- Hash: still returns 195 instead of 30 — `database.hash`
  doesn't append fields, so the pre-registration changes
  nothing.  Hash's bug is elsewhere (likely in
  `n_hash_sorted` or its scratch-vector layout).

The "register early" fix is insufficient; the right fix
needs to either:
- Properly thread the bookkeeping-field-add through the
  struct-layout pass (potentially refactoring how
  `database.index` interacts with `finish_type`), OR
- Add a compile-time check that rejects local-var
  index/hash and tells users to use a struct field.

**Severity:** Low — workaround (put the keyed collection in
a struct field) is the canonical loft pattern and works.

**Blocks:** plan-06 phase 4d.B's
`par_hash_input_t4` and `par_index_input_t4` canaries —
they use the local-var pattern in their test setup.  After
P191 closes, both should pass via the same 4d.B desugar
that closed `par_sorted_input_t4`.

### 189b. Tuple-as-vector-element field access reads garbage

**Symptom:** `vector<(T1, T2, …)>` literal construction works after
P189's `tuple_def` fix, but reading the tuple back via index access
or for-loop iteration returns wrong bytes:

```loft
fn test() {
  pairs: vector<(integer, integer)> = [(1, 10), (2, 20)];
  p = pairs[0];
  println("p.0 = {p.0}");   // prints 21474836482 (garbage), expected 1
  println("p.1 = {p.1}");   // prints 12884901896 (garbage), expected 10
}
```

For-loop iteration produces a different error:

```
Error: Field access not supported on type tuple([integer(...), integer(...)])
```

**Where:** local-tuple access uses `OpTupleGet(slot, byte_offset)`
which reads `size_of::<element>()` bytes directly from the local
slot — assumes inline-on-stack representation.  Vector-element
tuples are stored as 16-byte heap records and `pairs[0]` returns a
12-byte `DbRef`.  When the parser emits `OpTupleGet` for `p.0` where
`p: (integer, integer)` was assigned from a vector index access, it
reads the DbRef bytes (`store_nr` + `rec` + `pos` packed) as if they
were tuple elements.  Same root cause for the for-loop case: the
iteration's element-binding emits a stack-tuple read that doesn't
exist for heap-tuple shape.

**Fix path:** the parser needs to track whether a tuple-typed value
is in **inline** (stack) or **boxed** (heap-via-DbRef) form, and
emit different access opcodes.  Vector-element reads (and
struct-field tuple reads) hand back a DbRef; the access path needs
to unbox via per-element `OpGetInt(dbref, field_offset)` /
`OpGetText(dbref, field_offset)` etc.  Equivalent to how struct
field access already works on a heap struct — tuple is the
anonymous-struct case.  Tracked in plan-06 phase 9b (or as a
follow-up to T1.8a tuple-return-convention).

**Severity:** Low — the workaround (use a named `struct`) is
idiomatic loft and gives correct access via the existing struct
field-resolution path.

### 189d. `vector<(T1, text)>` text element reads as zero-length

**Symptom:** after P189c closed the `(int, int)` case, vector-of-
tuple elements containing `text` come back empty:

```loft
fn label_len(p: const (integer, text)) -> integer { len(p.1) }
fn run() -> integer {
  pairs: vector<(integer, text)> = [(1, "one"), (2, "two"), (3, "three")];
  sum = 0;
  for p in pairs par(r = label_len(p), 4) { sum += r; }
  sum
}
```

Expected sum: `3 + 3 + 5 = 11`.  Got `0`.

**Root cause — text has different in-vector vs on-stack
representation.** Text in a struct field (in-database / in-vector
storage) is a **4-byte text-pointer** that interns into the
field's local store.  Text in a stack tuple slot (variables::size
in Argument context) is a **16-byte `Str` struct** (8-byte
length + 8-byte pointer-to-bytes).  For non-tuple struct fields
this is bridged transparently — `OpGetText` at field-access time
inflates the 4-byte pointer to the full `Str`.  For tuples-as-
vector-elements, the worker's tuple-element access goes through
`OpTupleGet(slot=0, offset=8)` which reads raw bytes from the
slot — but the slot was filled by P189c's wide-input dispatch
which `memcpy`s `element_size` bytes from the row record.  The
12 bytes (8 int + 4 text-pointer) of the in-vector tuple don't
fit the worker's expected 24 bytes (8 int + 16 Str).

**Specifically:**
- `tuple_def`'s synthetic `__tuple<integer,text>` struct has
  fields `_0: integer` (8B) and `_1: text` (4B in struct layout).
  Database struct size = 12B.
- Worker's `p: (integer, text)` parameter expects slot 0 to be
  24 bytes (`variables::size` for tuple = 8 + `size_of::<Str>()`
  = 8 + 16 = 24).
- `read_primitive_at_wide` reads `element_size` bytes from the
  row record.  If element_size = 12 (the database stride), the
  worker reads 12 bytes into a 24-byte slot — text bytes 13-24
  are zeros, and the worker's text-Str access reads
  length-and-pointer of zero (empty string).
- If element_size = 24 (var_size), the read overflows the 12-byte
  row record into the next row.

**Fix path:** the per-row read for tuple elements containing text
needs to (a) read the in-vector representation (4-byte text-pointer
at the field offset), (b) inflate to the 16-byte `Str` for the
worker's slot.  Either:
- A new `read_tuple_at_wide(types, row_ref, struct_def_nr)` that
  walks the tuple's elements per-type and assembles the worker
  slot bytes (inflating text fields).  More complex than
  `read_primitive_at_wide`'s plain memcpy.
- OR force tuples-as-vector-elements to use the stack
  representation throughout (24 bytes for `(int, text)`) — but
  that wastes vector storage and conflicts with the tuple_def
  struct layout.
- OR teach the worker to access tuple-of-text differently when
  the source is a vector — emit `OpGetText` against the field
  offset (struct-style) instead of `OpTupleGet`.

Interacts with P189b (tuple field access via DbRef) — solving
P189b might subsume P189d, since both are about teaching the
parser/codegen to recognize "this tuple lives in heap storage,
unbox via per-field opcodes" instead of the stack-tuple opcodes.

**Severity:** Low — workaround (use a named struct with a
text field) works today.

## Web Services

*(none)*

## Graphics / WebGL

*(none)*

## Package / Multi-file

*(none)*

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
