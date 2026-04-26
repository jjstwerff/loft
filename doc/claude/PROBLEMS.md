
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
| 190 | `for x in <local sorted/hash/index>` panics at `src/state/codegen.rs:1689` with `Too few parameters on OpIterate (got 2, need 6)`.  P188 enabled local-var keyed collections but the iteration path's `get_type(Type::Sorted/Hash/Index)` at `src/parser/vectors.rs:1626` looks up the database type name (e.g. `sorted<Score[value]>`) which is only registered via `fill_database` for struct fields — local-var keyed collections don't trigger the field-typedef pass.  `get_type` returns `u16::MAX`, `fill_iter` exits early, `OpIterate` gets only 2 args (the trailing zeros) instead of the 6 it needs.  Independent of par; sequential iteration over a local-var sorted hits the same panic.  **Blocks plan-06 phase 4d.B (par over keyed collections)** because the canaries (`par_sorted_input_t4` etc.) use local-var sorted as input. | Low | **Workaround:** put the keyed collection in a struct field (`struct Db { items: sorted<...> }; for x in db.items`) — the field path triggers fill_database registration and OpIterate gets the right args. |
| 189b | `vector<(T1, T2, …)>` element field access via DbRef returns garbage.  `pairs[0]` returns a 16-byte `DbRef` to the heap record holding the tuple bytes; `.0` / `.1` parses to `OpTupleGet` which reads 8 bytes from the local slot directly — but the slot holds the DbRef, not the inline tuple.  Result: reading `pairs[0].0` returns `(store_nr \| (rec << 32))` masquerading as `i64` (saw `21474836482` instead of `1` for `(1, 10)`).  Iterating with `for p in pairs { … }` reports `Field access not supported on type tuple([…])` instead of unboxing.  P189 (literal construction + `len()`) is fixed — this is the access-side follow-up. | Low | **Workaround:** wrap the tuple in a `struct` (`struct Pair { a: integer, b: integer }`) and use `vector<Pair>`.  Struct field access via DbRef is correct. |
| 189d | `vector<(T1, text)>` element write returns 0 length for the text element.  P189c closed the per-attribute write path for primitive tuple elements (`(int, int)` works), but text elements within a vector-of-tuple read back as empty / zero-length.  Surfaced via `par_tuple_input_int_text` — workers see `len(p.1) == 0` instead of the expected `3, 3, 5`.  Likely root cause: `set_field`'s `Type::Text` arm in `src/parser/mod.rs:1716` writes via `OpSetText` which interns the string into the field's local store, but a vector-element write needs a different routing because the "field" is a tuple element inside a vector record (different store / different position computation). | Low | **Workaround:** wrap the tuple in a `struct` containing a text field — works correctly via the standard struct path. |

## Interpreter Robustness

### 190. Local-var keyed collection iteration panics in OpIterate

**Symptom:** `for x in <local sorted/hash/index/spacial>` panics:

```loft
fn test() {
  sorted_items: sorted<Score[value]> = [];
  sorted_items += Score { value: 30 };
  sorted_items += Score { value: 10 };
  sum = 0;
  for s in sorted_items { sum += s.value; }   // ← panics here
}
```

```
thread 'main' panicked at src/state/codegen.rs:1689:9:
Too few parameters on OpIterate (got 2, need 6)
```

**Where:** `src/parser/vectors.rs::get_type` at line 1669-1688
looks up the database type name for `Type::Sorted/Hash/Index` by
constructing a name string (`sorted<Score[value]>`) and calling
`self.database.name(name)`.  This name is only registered via
`fill_database` when the keyed collection appears as a struct
field — local-var keyed collections (enabled by P188) don't
trigger this registration path, so the lookup returns `u16::MAX`.
`fill_iter` then exits early at vectors.rs:680, leaving `ls` with
only the 2 trailing zero ints that the caller pushed afterward —
when `OpIterate` (which has 6 attributes) is built from `ls`,
the parameter-count assert fires.

**Independent of par.**  Sequential iteration over a local-var
sorted hits the same panic.  **But it blocks plan-06 phase 4d.B**
(par over keyed collections) because the canaries
(`par_sorted_input_t4`, `par_hash_input_t4`, `par_index_input_t4`)
all use local-var keyed collections as input.

**Fix path:** P188's `gen_set_first_keyed_null` (the codegen
helper that allocates the backing store for a local-var keyed
collection) needs to also call the database to register the type
name — or `get_type` needs a fallback that registers the type
on demand if the lookup misses.  The struct-field path's
fill_database registration produces the right type registration;
mirror that for the local-var path.

**Severity:** Low — workaround (put the keyed collection in a
struct field) is the canonical loft pattern and works today.

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
