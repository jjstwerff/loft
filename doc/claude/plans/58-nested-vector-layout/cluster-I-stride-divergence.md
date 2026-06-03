<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster I — Stride divergence (16 / 8 / 4 / 4)

**Severity (split by failure mode):**
- **Corruption / panic / hang:** silent-corruption *risk* — a nested-vector
  element handle is 4 bytes, but parser-computed strides claim 16 (construction
  prealloc) or 8 (index read).  Today most shapes survive via two-wrongs-cancel;
  the risk is a shape where they don't (over-reserved/under-read memory).
- **Leak:** none observed.

**Affected probes:** 01 (control), 02, 06.
**Backend asymmetry:** both — strides are IR operands baked at parse time.

## Mechanism (verified)

A vector element that is itself a vector is stored as a 4-byte `u32` rec-id
handle.  Three resolvers disagree on its size:

| Site | Resolver chain | Reports | Evidence |
|---|---|---|---|
| Construction prealloc | `type_def_nr(elem)` → `FieldValue` alias `known_type`, `size`=16 | **16** | ✅ `LOFT_LOG=static` on probe 02: `OpPreAllocVector(vv, 3, 16i32)` |
| Index read (`vv[i]`) | `type_elm(vec)` → bare `vector` builtin, `size`=8 | **8** | 🟢 prior-session read of `fields.rs:677`; not yet isolated in a crashing probe |
| Runtime deep-copy / `OpCopyRecord` | `stores.vector(...)` chain (#250 fix) | **4** | ✅ #250 regression `tests/scripts/182` passes |
| True handle | `u32` rec-id | **4** | ✅ by definition |

## What we know vs. don't

| Claim | Status |
|---|---|
| Construction prealloc emits stride 16 for `vector<vector<integer>>` | ✅ Verified — `LOFT_LOG=static` probe 02 |
| Forcing construction stride to 4 keeps 2-deep + 3-deep integer correct | ✅ Verified — probes 02, 06 PASS under `--vec4` |
| The outer `vv[0]` read already uses 4 (linked-record `OpVectorRef`, not the `elm_size` operand) | ✅ Verified — `--vec4` diff on probe 02 touched only the prealloc line, not a read |
| The read-side stride 8 bites *some* form (which?) | 🤔 Hypothesized — needs a probe that routes the outer read through `OpGetVector` (non-linked / nullable path) |
| Comprehension/map element sizing (`element_store_size`) diverges for nested vectors | 🤔 Hypothesized — lever wired there but no probe yet exercises it |

## Investigation tasks

1. Build the shape matrix (depth × element × context) and find every form where
   the read-side `8` or prealloc `16` is *not* cancelled by a sibling path.
2. For each surviving-by-luck shape, document WHICH two wrongs cancel.
3. Decide the fix: make `--vec4` permanent (clamp), or fix the resolvers
   (`type_def_nr` / `type_elm`) to return a size-4 type-id for `Type::Vector`
   elements directly, then retire the lever.

## Fix surface (preliminary)

- **Option A — permanent clamp** (the lever, de-`--vec4`'d): minimal, but leaves
  the resolvers reporting wrong sizes (other callers may still mis-resolve).
- **Option B — fix the resolvers**: `type_def_nr(Type::Vector(..))` and
  `type_elm` return a type-id whose `size` is 4 for a vector-handle element.
  Larger blast radius (every `database.size` caller) but removes the root cause.
  Preferred if the matrix shows the divergence reaches sites the lever misses.

## Why this is mapped, not yet fixed

The fix choice (A vs B) depends on whether the read-side `8` and the
`element_store_size` path break any real shape, or are always cancelled.  The
Stage-A matrix answers that; until then a fix would be guessing at scope.
