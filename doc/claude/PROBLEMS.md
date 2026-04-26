
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
| 189b | `vector<(T1, T2, …)>` element field access via DbRef returns garbage.  `pairs[0]` returns a 16-byte `DbRef` to the heap record holding the tuple bytes; `.0` / `.1` parses to `OpTupleGet` which reads 8 bytes from the local slot directly — but the slot holds the DbRef, not the inline tuple.  Result: reading `pairs[0].0` returns `(store_nr \| (rec << 32))` masquerading as `i64` (saw `21474836482` instead of `1` for `(1, 10)`).  Iterating with `for p in pairs { … }` reports `Field access not supported on type tuple([…])` instead of unboxing.  P189 (literal construction + `len()`) is fixed — this is the access-side follow-up. | Low | **Workaround:** wrap the tuple in a `struct` (`struct Pair { a: integer, b: integer }`) and use `vector<Pair>`.  Struct field access via DbRef is correct. |
| 189d | `vector<(T1, text)>` element write returns 0 length for the text element.  P189c closed the per-attribute write path for primitive tuple elements (`(int, int)` works), but text elements within a vector-of-tuple read back as empty / zero-length.  Surfaced via `par_tuple_input_int_text` — workers see `len(p.1) == 0` instead of the expected `3, 3, 5`.  Likely root cause: `set_field`'s `Type::Text` arm in `src/parser/mod.rs:1716` writes via `OpSetText` which interns the string into the field's local store, but a vector-element write needs a different routing because the "field" is a tuple element inside a vector record (different store / different position computation). | Low | **Workaround:** wrap the tuple in a `struct` containing a text field — works correctly via the standard struct path. |

## Interpreter Robustness

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

**Where:** P189c's `Value::Tuple` arm in
`src/parser/vectors.rs::new_record` emits per-attribute
`set_field(tuple_def_d_nr, i, 0, elm, val)` calls.  For
text values, `set_field`'s `Type::Text` arm at
`src/parser/mod.rs:1716` emits `OpSetText` which writes a
4-byte text-pointer into the field at the right offset.  The
write itself probably executes, but the resulting text
either isn't readable through the worker's tuple-element
access path or the pointer is interpreted in the wrong store.

Probably interacts with P189b (tuple field access via DbRef)
since reading `p.1` where `p` is a tuple-via-DbRef needs
DbRef-aware unboxing.  Worth investigating P189b first.

**Fix path:** trace what `set_field` emits for the text case
(probably `OpSetText` at offset 8 of the synthetic
`__tuple<integer,text>` struct), then verify whether the
worker's `OpTupleGet(slot=0, offset=8)` reads it as a text
pointer or as raw bytes.  Likely the worker needs to use
`OpGetText` against the DbRef rather than `OpTupleGet`
against the slot.

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
