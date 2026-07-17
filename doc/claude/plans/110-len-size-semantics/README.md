<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 110 — `len` = logical count, `size` = occupied bytes (allocation-local)

Tracker: [@PLN110](https://github.com/loft-lang/plans/issues/110) · `subject:loft` · `status:next`

## Status

Open — semantics decided (design discussion 2026-07-17), no implementation. This is the
resolution of @PLN102 arc-E lib-audit **H2** (the `len(text)`=bytes / `size(text)`=chars
inversion) and is the **last open stdlib freeze blocker** for `CONTRACT_VERSION 0 → 1`.
The risk is the migration + `size`'s new implementations, not the semantics.

## Goal

`len(x)` is the **logical count** (characters / elements / entries) and `size(x)` is the
**occupied byte footprint of the value's own allocation**, uniformly across every
structure — killing text's `len`/`size` inversion and making `size` a real memory
primitive.

## Effort + design

- **Effort:** MH (semantics small; `size` per-type implementations + a stack-wide migration
  with per-site validation are the bulk).
- **Design:** ✓ (semantics settled below; one open sub-question — `s[p]` return type).
- **Last touched:** 2026-07-17.

## The settled semantics

### `len(x)` — logical count
Characters for `text`, elements for `vector`, entries for `hash` (and the other keyed
collections). Matches every mainstream language; `len(text)` flips from bytes to chars.

### `size(x)` — occupied bytes of *this* allocation, by two rules
1. **Allocation-local (across allocations, a reference counts as its width).** `size` is the
   footprint of *this* allocation; a field/element that points to a **separate** allocation
   counts as its reference width (≈4 bytes), never the target's content. So:
   - a record with **inline sub-records** (same allocation) counts them fully;
   - an **array** counts each member as its 4-byte reference (`N × 4`), not the member's content;
   - a record with **`text` fields** counts the text *handle*, not the text's bytes.

   **We do not iterate into referenced structures.** A programmer who needs the total
   reachable size composes `size` calls / walks the structure themselves where needed.
2. **Design-vs-reserve (within the allocation).** Count space that is part of the value's
   *design*; exclude unused *reserve*. An over-allocated `text` and a `vector`'s spare capacity
   report **content only**; a `hash` counts its **full table, holes included** (open addressing
   *is* the format — and the table is one allocation).

Deterministic, defined off the **@PLN97** layout contract (both backends agree; a
resize-policy change becomes a conscious layout-contract change).

### `text` indexing stays byte-positioned
`text[p]` is byte-positioned (O(1)); **no** O(n) character random-access. Characters come from
iteration (`for c in s`). `size` is the byte-indexing bound; `len` is the human count — the two
are different units and **do not compose**. `for i in 0..size(s) { s[i] }` is the correct byte
walk; `for i in 0..len(s) { s[i] }` is a units error, caught by the strict-index lint made
**default-on for the text case** (today's opt-in `LOFT_LINT_STRICT_INDEX`).

## Several new `size` implementations

Today `size` is essentially text-only and returns a *character* count; the byte-footprint
`size` is mostly new code — one per structure type, each computing **one allocation's**
footprint (no recursion):

| type | `size` = |
|---|---|
| `text` | content bytes (over-allocation excluded) — *redefine* |
| `vector<T>` | the buffer: `N ×` element in-buffer width (inline value's size, or a 4-byte reference) |
| `hash<…>` | **full table, holes included** |
| `sorted` / `index` / `spatial` | their table / tree bytes |
| struct | flat allocation: inline fields + inline sub-records; `text`/heap/ref fields = reference width |
| scalar | the scalar width |

Each reads the value's own layout (@PLN97) and sums by the allocation-local + design-vs-reserve
rules — no owned-subtree walk, no deps traversal.

## Resolved — `s[p]` stays exactly as today

`s[p]` keeps today's behavior: byte-positioned (`p` is a byte offset, O(1)) and **always returns
the character that offset hits** — a mid-character offset is snapped to the **start of the
character it falls within**, so an in-range offset never yields null. Negative `p` counts from
the end. It returns `null` **only** when `p` is outside the text's byte range. No change to text
indexing. Owner call (2026-07-17): this is *not error-prone* (every in-range offset yields a
real character), so the least-churn choice is also the safe one; it may read as strange, but
programmers catch on quickly and **iteration (`for c in s`) is the canonical way to walk
characters.** There is no character-*index* random access (that would be O(n)). The
`len`-vs-byte-index mismatch (a char count driving a byte offset) is defended by the default-on
strict-index lint. This is the **original (pre-Claude) implementation with its own tests** —
nothing to build for `s[p]`; the plan only must not regress it. **No open design questions remain.**

## Migration safety — how a both-meaning flip stays de-risked

The danger is that flipping `len(text)`/`size(text)` moves every call site's behavior
**silently**. Two mechanisms remove that:

- **A golden-behavior corpus first (Phase 0e).** Capture the current output of every
  text-using consumer, so *any* behavior move after the flip is a visible diff, never silent —
  the same discipline as @PLN109.
- **The additive half lands green (Phase 1).** `size` on `vector`/`struct`/`hash`/… does not
  exist today, so implementing it breaks nothing — each type lands independently, green, tested.
  Only the *text* `len`/`size` redefinition is a flip.
- **Source-by-source conversion (Phase 2).** Flip the two `text` definitions in **one clear
  commit**; the corpus + suite then go red at *exactly* the moved sites (= the Phase-0
  inventory). Convert one source at a time (stdlib → libs → tests/docs → consumers), each
  re-greening before the next. Small, verified, reversible; the branch's red set is the
  worklist, and nothing moves unseen.

If a red interval on the branch is unacceptable, the **green-throughout alternative** is
disambiguation-first: add explicit `char_count(text)`/`byte_len(text)` (additive, green),
migrate every site to the explicit name matching its intent (green, behavior identical), then
flip `len`/`size` as a **no-op** (no site uses them). Costs temporary/extra names; keeps the
tree green at every step. (Decide which at Phase 2 start.)

## Sub-arcs — the granular steps (small, safe, validation-heavy)

| # | Step | Kind | Status |
|---|---|---|---|
| 0a | Inventory `len`/`size`-on-text in stdlib (`default/*.loft`), classify count vs byte | read-only | Open |
| 0b | Inventory in `lib/*` + registered libraries | read-only | Open |
| 0c | Inventory in `tests/` (scripts, docs, `code!` cases) + examples + STDLIB.md | read-only | Open |
| 0d | Inventory in the consumer programs (games / crawler / `lib/markdown`) | read-only | Open |
| 0e | Land the golden-behavior corpus for the text-using consumers (the visibility baseline) | additive · green | Open |
| 1a | `size(vector<T>)` (buffer bytes; members as inline size / 4-byte ref) + tests | additive · green | Open |
| 1b | `size(struct)` (flat allocation; inline sub-records; `text`/heap fields as ref width) + tests | additive · green | Open |
| 1c | `size(hash)` (full table, holes included) + tests | additive · green | Open |
| 1d | `size(sorted / index / spatial)` (table/tree bytes) + tests | additive · green | Open |
| 1e | `size(<scalar>)` (width) + tests | additive · green | Open |
| 1f | `s[p]` unchanged — the original (pre-Claude) implementation, already has its own tests (verified: mid-char snaps to the char's start; out-of-byte-range → null; negatives from the end). Not a build; just **don't regress**. | no-op | Open |
| 2a | Flip `len(text)`=chars / `size(text)`=content bytes — one commit (corpus/suite red = the worklist) | flip | Open |
| 2b | Convert stdlib text sites → re-green | convert | Open |
| 2c | Convert libraries → re-green | convert | Open |
| 2d | Convert tests / docs / examples → re-green | convert | Open |
| 2e | Convert consumer programs → re-green | convert | Open |
| 3a | Strict-index lint **default-on** for the text case | lint | Open |
| 4a | Full suite, **both backends** | validate | Open |
| 4b | Run the known consumer programs (dogfood) | validate | Open |
| 4c | Republish / validate affected libraries | validate | Open |
| 4d | Clear @PLN102 **H2** → the `CONTRACT_VERSION 0 → 1` flip is unblocked on the stdlib side | close-out | Open |

## Cross-arc dependencies

- **@PLN102 arc-E H2** — this resolves it; H2 is the last stdlib freeze blocker.
- **@PLN97** (layout contract) — the source of truth for every structure's byte layout that
  `size` reads.
- The `LOFT_LINT_STRICT_INDEX` lint (@PLN102 case-D) — flipped default-on for text in Phase 3.

## See also

- [lib-audit.md](../102-stability-contract/lib-audit.md) § H2 — the trigger.
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) #9 (`txt[i]`→character) — bears on the `s[p]` question.
- `default/01_code.loft` (`len`/`size`), `03_text.loft` — the stdlib surface being redefined.
- Tracker: [@PLN110](https://github.com/loft-lang/plans/issues/110).
