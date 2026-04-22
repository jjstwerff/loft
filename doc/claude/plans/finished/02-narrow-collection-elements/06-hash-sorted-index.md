<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 6 — Extend to Hash / Sorted / Index

**Status:** closed 2026-04-22 — no code change required; struct-key
narrowing already works via the Phase 2 + Phase 5 struct-field path.
Regression guard landed (`tests/issues.rs::p184_hash_sorted_narrow_key_field`).

---

## Audit outcome

Primitive-content collection forms are **parse errors** in loft.
The grammar requires `hash<T[key]>` / `sorted<T[key]>` / `index<T[key]>`
— a `[key]` suffix on the element struct.  The parser rejects the
bare-content forms:

```
$ target/release/loft --interpret /tmp/probe.loft
Error: Expect token [ at /tmp/probe.loft:1:31
  |
   1 | struct TestHash { h: hash<i32> }
     |                               ^
Error: Expect token [ at /tmp/probe.loft:3:33
  |
   3 | struct TestIndex { i: index<i32> }
Error: Expect token ] at /tmp/probe.loft:2:35
  |
   2 | struct TestSorted { s: sorted<i32> }
```

So "primitive narrow content in hash/sorted/index" is not a
reachable state — there's no code path to extend and no user
program that would benefit.

## What DOES work (and needs a guard)

Collections whose content is a struct with a narrow-typed key field
— e.g. `hash<Row[rid]>` where `rid: u32 not null`.  The `rid` field
is a struct field, so Phase 2's `fill_database` narrow-detection
already chooses `Parts::Int` for 4-byte storage via the existing
`forced_size(alias)` path.  No Phase 6 code change needed.

Regression test: `tests/issues.rs::p184_hash_sorted_narrow_key_field`
exercises:

- `hash<Row[rid]>` lookup by `u32` key (values 42 and 7).
- `sorted<Row[rid]>` insertion out-of-order and iteration by key.

Both work end-to-end with narrow 4-byte key storage.

`index<Row[rid]>` was also checked for parse acceptance but isn't
exercised in the happy-path test; a follow-up guard when index has
broader test coverage is fine.

## What this closes and what it doesn't

**Closed:** the "extend narrowing to hash/sorted/index" work
anticipated in the original plan.  Not applicable — the primitive
case is a parse error, and the struct-key case is already handled.

**Not closed by Phase 6:**
- `vector<u16>` / `vector<i16>` narrow storage — still blocked on
  Phase 4b (see [04b-short-encoding.md](04b-short-encoding.md)).
- Spacial indexes — the Spacial variant is a diagnostic stub per
  C7/P22, not implemented.  When Spacial ships, this audit
  should be re-run.

## Initiative closeout

With Phase 6 closed and Phase 4b blocked (but not required for any
user-visible bug), the P184 initiative's active work is paused.
The README's phase table reflects:

- Phases 0 / 1 / 2 / 3 / 4a / 5 / 6: ✅ done.
- Phase 4b: 🔴 blocked, bisect required; reactivate when a session
  has budget for the `native_dir` hang investigation.

The initiative stays in `doc/claude/plans/02-narrow-collection-elements/`
rather than moving to `finished/` because Phase 4b remains open.
Move to `finished/` once 4b lands OR the decision to indefinitely
defer 4b is made (e.g. "u16 narrow storage is an optimisation we
don't need; close the gap permanently in the plan and remove the
placeholder `vector_narrow_width` / `narrow_vector_content` arms").

## Acceptance

- [x] Primitive-content `hash<i32>` / `sorted<i32>` / `index<i32>`
      confirmed to be parse errors via a one-off probe.
- [x] `p184_hash_sorted_narrow_key_field` regression test landed
      and passes.
- [x] Plan doc updated with audit outcome.
- [x] README phase table flipped from pending to done.
