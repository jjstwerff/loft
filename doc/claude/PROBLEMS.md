
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
| 188 | Local-var keyed collections (`sorted<T[key]>` / `hash<T[key]>` / `index<T[key]>` / `spacial<T[key]>`) not functional as locals.  Today the language only supports keyed collections as struct fields; `out: sorted<Tag[id]> = []; out += Tag {...}; out` fails because (a) the `+=` element-insertion path doesn't recognise keyed-collection LHS as an "append target", and (b) the slot allocator/codegen never wires up an initialization path for keyed-collection locals.  The visible symptom is the type-checker rejecting `out += Tag {...}` with `Variable 'out' cannot change type from sorted<Tag,[…]> to Tag; use a new variable name or cast with 'as'`. | Low | **Workaround:** assemble in a `vector<T>` local first and return that, or build the keyed collection as a struct field rather than a local.  Surfaced via `tests/threading_chars.rs::par_struct_to_keyed_collection_t4` (currently `#[ignore]`d). |

## Interpreter Robustness

### 188. Local-var keyed collections not functional (`sorted/hash/index/spacial<T[key]>`)

**Symptom (1, parser):** building a keyed collection inside a
function via the `+=` operator produces a type-mismatch error:

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

Same shape on `hash<T[key]>` / `index<T[key]>` / `spacial<T[key]>`.

**Symptom (2, slot allocator):** even the bare initialisation
`out: sorted<Tag[id]> = []` is unreachable today: parsing accepts
it after the symptom-1 patch, but the slot allocator never assigns
a stack position to `out` because the language never had a working
keyed-collection initialisation path for locals.  Codegen panics at
`src/state/codegen.rs:2028` with `Incorrect var out[65535] versus 4`.

**Why both layers fail:** keyed collections were designed as
struct fields, never as local variables.  Every working example
in the codebase places `sorted/hash/index` inside a struct
(`tests/scripts/12-collections.loft`, `tests/docs/10-sorted.loft`,
…); local-var initialisation, `+=` insertion, and read/load were
never wired up.

**Where the gaps are:**
- `src/parser/expressions.rs` — `+=` element-append for keyed
  collections (WIP: parser arm in place; routes through
  `OpNewRecord` + `OpFinishRecord`).
- `src/parser/vectors.rs::new_record` — must pass the *collection*
  type id (not `vector_of(T)`) to `OpNewRecord` so `record_new`
  dispatches via `Parts::Sorted/Hash/Index/Spacial` (WIP: `lhs_known`
  override in place).
- `src/state/codegen.rs::generate_var` — `OpVarVector` arm for
  keyed-collection locals (WIP: arm in place).
- `src/parser/operators.rs::create_vector` — needs an analogous
  `create_keyed_collection` (or extension) that emits an
  `OpClearKeyed` / `OpInitKeyed` opcode on first assignment so
  the slot allocator sees a real `Set(out, init)` and assigns a
  position.  **This is the missing piece.**
- Slot allocator already classifies these as `RefSlot`
  (`src/variables/slots_v2.rs::slot_kind`), so once the IR has a
  proper `Set(out, init_kind)` node a slot will be allocated.

**Workaround:** assemble in a `vector<T>` local first and convert
to the keyed collection at the assignment site, or place the
keyed collection in a struct field rather than as a local.  All
stdlib and example code uses one of these patterns today.

**Surfaced via:** `tests/threading_chars.rs::par_struct_to_keyed_collection_t4`
— par-specific symptom is "type-checker rejects the worker fn at
parse time, before par-safety analysis runs", but the bug is
independent of par.  Closing this would unblock that canary as a
side effect.

**Severity:** Low — solid workaround exists, idiomatic pattern
(keyed collections in struct fields) is unaffected.

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
