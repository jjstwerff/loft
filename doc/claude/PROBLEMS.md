
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
| 188 | `out += Tag {...}` on `sorted<T[key]>` / `hash<T[key]>` / `index<T[key]>` rejected by the type-checker with `Variable 'out' cannot change type from sorted<Tag,[…]> to Tag; use a new variable name or cast with 'as'`.  The `+=` element-insertion path that works for `vector<T>` doesn't recognise keyed collections as the same kind of "append target" — it tries to reassign the variable as the RHS element type instead of inserting. | Low | **Workaround:** build keyed collections one element at a time via the explicit add path, or assemble in a `vector<T>` first and then convert.  Surfaced via `tests/threading_chars.rs::par_struct_to_keyed_collection_t4` (currently `#[ignore]`d). |

## Interpreter Robustness

### 188. `out += Element` on `sorted<T[key]>` / `hash<T[key]>` / `index<T[key]>` rejected by type-checker

**Symptom:** building a keyed collection inside a function via the
`+=` operator produces a type-mismatch error:

```loft
fn build_tags(s: const Score) -> sorted<Tag[id]> {
  out: sorted<Tag[id]> = [];
  out += Tag { id: s.value, label: "v{s.value}" };  // ← Error
  out
}
```

```
Error: Variable 'out' cannot change type from
       sorted<Tag,[("id", true)]> to Tag;
       use a new variable name or cast with 'as'
```

Same shape on `hash<T[key]>` and `index<T[key]>`.  The `vector<T>`
case works because the `+=` codegen for vectors recognises the
LHS as an "append target" of the element type; the keyed-collection
codegen doesn't pattern-match the RHS as a singleton element.

**Where:** parser-side `+=` handling routes `vector<T> += T` and
`vector<T> += vector<T>` through a vector-append path
(`src/parser/expressions.rs` around the `OpAppendVector` /
`OpNewRecord` emission), but the same pattern for
`sorted/hash/index` falls through to the generic compound-assign
lowering — which checks `lhs_type == rhs_type` and refuses.

**Fix path:** mirror the vector `+=` element-append codegen for
keyed collections.  Each keyed type already has a single-element
add path the user can call directly (e.g. via the collection's
`.add(...)` method or operator-form); the `+=` lowering should
route to that instead of the generic compound-assign.

**Surfaced via:** `tests/threading_chars.rs::par_struct_to_keyed_collection_t4`
— par-specific symptom is "type-checker rejects the worker fn at
parse time, before par-safety analysis runs", but the bug is
independent of par.  Closing this would unblock that canary as a
side effect.

**Severity:** Low — there's a workaround (collect into a vector
then convert, or use the explicit add path), and most stdlib /
example code today builds vectors then converts when a keyed
collection is needed.

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
