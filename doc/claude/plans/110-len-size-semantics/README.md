<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 110 — `len` = logical count, `size` = occupied bytes (allocation-local)

Tracker: [@PLN110](https://github.com/loft-lang/plans/issues/110) · `subject:loft` · `status:next`

## Status

**Phase 2a FLIP LANDED** (2026-07-17) — `len(text)` now = character count, `size(text)` = byte
count (swapped `OpLengthText`/`OpSizeText`). The red set after the flip matched the Phase-0 inventory
**exactly** (17 value canaries + golden Section A), validating the inventory. All converted to green
(2b–2e): stdlib byte sites (`starts_with_at`, path helpers) + `lib/lexer` + `tests/fixtures/libs`
(incl. the critical `glb.loft` binary chunk length) moved `len→size`; canaries updated by intent
(value flip vs `len→size` where the test verifies a byte count); doc-prose + `doc/examples.js` fixed.
Script suites, `strings`, `host_input`, `doc_hygiene` all green both backends. `len(character)` kept
as UTF-8 byte width (owner call), so the `c#next == c#index + len(c)` identity still holds.
**3a DONE** — the text strict-index lint is default-on (`for i in 0..len(s){s[i]}` warns; opt-out
`LOFT_NO_STRICT_INDEX_TEXT`); it broke 0 existing tests (no code used that units-error shape).
**Remaining:** Phase-4 validation/dogfood, then the `CONTRACT_VERSION 0 → 1` flip. (Pre-existing, not from this work: `index_hygiene` flags 14 closed-issue
`@P*` refs in untouched files.)

**Phase 1 COMPLETE** (2026-07-17) — sub-arc **1a `size(vector<T>)` ✅ landed**, both backends; the
uniform mechanism + per-type formulas + build order are in [phase1-size-impl.md](phase1-size-impl.md).
Two Phase-1 findings: (1) `size` correctly reflects loft's **two vector representations** — inline
`Vector<T>` (`size(T) × len`, e.g. a standalone `vector<Point>` = 16 × len) vs by-reference
`Array<T>` (`4 × len`, when `T` is shared with a keyed collection and promoted at `finish_type`).
This is exactly the plan's rule #1 (inline sub-records fully / array members by 4-byte ref) — both
representations are handled and tested. (2) The 0f `character` recommendation was **corrected** —
`size(character)`=4 (fixed slot), keep `len(character)`=UTF-8 width; 1g deferred on an owner call.

**Phase 0 complete** (2026-07-17) — full inventory + baseline done; see
[phase0-inventory.md](phase0-inventory.md). Semantics decided (design 2026-07-17). This resolves
@PLN102 arc-E lib-audit **H2** (the `len(text)`=bytes / `size(text)`=chars inversion) and is the
**last open stdlib freeze blocker** for `CONTRACT_VERSION 0 → 1`. The risk is the migration +
`size`'s new implementations, not the semantics.

**Phase 0 findings (headline):**
- The whole migration is a `len→size` byte-conversion — **byte-intent sites dominate** (16 stdlib + 4
  `lib/lexer` + 29 consumer + a cluster in `tests/fixtures/libs/`) with **0 COUNT sites anywhere**.
  Consumers never used `len` for visible-width layout (crawler measures pixels; markdown pure-byte-
  slices), so the flip *fixes no consumer bug* — its risk is purely regressing byte-addressing if a
  site is missed.
- **The definitive red set** the Phase-2a flip must reproduce = **17 value-assert canaries** (11
  `len` + 6 `size`, listed in phase0-inventory.md § 0c) + the golden fixture Section A.
- **Two top-risk sites — both write a byte length into a binary format:** `markdown.loft:783`
  (`i += char_str.len()` inline byte cursor) and `graphics/src/glb.loft:150`/`:456` (GLB binary
  JSON-chunk byte length → corrupt `.glb` on a non-ASCII glTF name). Migrate first, with non-ASCII
  regression tests.
- **New sub-arc 1g:** `len(character)` is a real half-feature (returns UTF-8 byte width today, would
  be inconsistent with `len(text)`=chars) — reconcile to `len(character)`=1 / `size(character)`=UTF-8
  bytes. ~0 callers, low-risk.
- ⚠ **Caveats for later phases:** `moros` has **no loft code** (drop it from the consumer list);
  `lib/markdown` is **not checked out on this box** (Phase 0d used a mirror — re-inventory the real
  `loft-libs-docs` repo at Phase-2e convert time).

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

## Whole-surface consistency — no half-features

Text has **full** support, so "secondary priority" does **not** license warts: the redesign
must leave the **entire** text surface coherent, not just `len`/`size`. There are two consistent
worlds, and every operation must sit clearly in one:

- **Byte world (O(1), byte offsets):** `size`, `s[p]`, slices `s[a..b]`, `find`/`rfind` (byte
  offsets), `byte_at`, the `#index` / `#next` loop attributes.
- **Character world:** `len` (char count), `for c in s` iteration, the character classifiers.

Every method's return **unit** must be consistent and documented (e.g. `find` returns a **byte
offset** → it pairs with `s[p]` and slicing, never with `len`). The Phase-0 inventory audits the
**whole** surface for unit consistency — not only `len`/`size` — and Phase 4 validates it, so we
never fix `len`/`size` and leave a slice or a method's return-unit mismatched.

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
| 0a | Inventory `len`/`size`-on-text in stdlib (`default/*.loft`), classify count vs byte | read-only | ✅ Done — 16 BYTE (`len→size`), 9 safe, 0 `size(text)` |
| 0b | Inventory in `lib/*` + registered libraries | read-only | ✅ Done — 4 BYTE + 1 safe (all `lib/lexer.loft`) |
| 0c | Inventory in `tests/` (scripts, docs, `code!` cases) + examples + STDLIB.md | read-only | ✅ Done — 17 value canaries + ~9 doc-prose + `tests/fixtures/libs` BYTE (incl. critical `glb.loft`) |
| 0d | Inventory in the consumer programs (games / crawler / `lib/markdown`) | read-only | ✅ Done — 29 BYTE, **0 COUNT**; caveats ⚠ below |
| 0e | Land the golden-behavior corpus for the text-using consumers (the visibility baseline) | additive · green | ✅ Done — `tests/scripts/pln110-text-surface-golden.loft`, green both backends |
| 0f | Audit the **whole** text surface (slices, `#index`/`#next`, `find`/`rfind`, `byte_at`, classifiers) for byte-vs-char unit consistency; reconcile any mismatch so the surface is coherent — **no half-features** | read-only + reconcile | ✅ Done — surface coherent; **1** half-feature found → new 1g |
| 1a | `size(vector<T>)` = N × in-buffer stride + tests | additive · green | ✅ Done — `OpSizeVector`, both backends; `tests/scripts/pln110-size-vector.loft` |
| 1b | `size(struct)` = packed record size (inline sub-records fully; `text`/heap fields as 4B) + tests | additive · green | ✅ Done — `OpSizeStruct`, both backends; `tests/scripts/pln110-size-struct.loft`. Enums deferred to their own step |
| 1c | `size(hash)` = full bucket table (holes included) + tests | additive · green | ✅ Done — `OpSizeHash`/`hash::table_bytes` (stdlib overload), both backends; `tests/scripts/pln110-size-hash.loft` |
| 1d | `size(sorted)` = buffer (len×stride); `size(index/spatial)` = a **single node record** (no aggregate structure — tree bookkeeping is per-record) + tests | additive · green | ✅ Done — both backends; `tests/scripts/pln110-size-sorted-index-spatial.loft` |
| 1e | `size(<scalar>)` = storage width (narrow-aware) + tests | additive · green | ✅ Done — `OpSizeScalar`, both backends; `tests/scripts/pln110-size-scalar.loft`. Settles `size(character)`=4 (size-half of 1g) |
| 1h | **(discovered)** `size(enum)`: simple = 1 (scalar-like); data enum-typed = max-variant record; bare variant = its own record. Fixes a 1b silent-empty on non-struct references. + tests | additive · green | ✅ Done — both backends; `tests/scripts/pln110-size-enum.loft` |
| 1g | **(found in 0f, corrected in 1e)** `size(character)` = 4 (the code-point slot, ✅ done via 1e); **keep `len(character)` = UTF-8 byte width** (the byte-world quantity `#index`/`#next` needs — do NOT redefine to 1). Only the `len(character)` decision remains, deferred on an owner contract call. | contract call | ⏸ Deferred (size half done) |
| 1f | `s[p]` unchanged — the original (pre-Claude) implementation, already has its own tests (verified: mid-char snaps to the char's start; out-of-byte-range → null; negatives from the end). Not a build; just **don't regress**. | no-op | Open |
| 2a | Flip `len(text)`=chars / `size(text)`=content bytes (swap `OpLengthText`/`OpSizeText` `#rust` bodies + doc comments) | flip | ✅ Done — red set == the Phase-0 inventory (validated) |
| 2b | Convert stdlib text sites → re-green | convert | ✅ Done — `03_text.loft` (starts_with_at), `02_files.loft` (path helpers); emptiness kept as `len` |
| 2c | Convert libraries → re-green | convert | ✅ Done — `lib/lexer.loft` (4 byte sites) |
| 2d | Convert tests / docs / examples → re-green | convert | ✅ Done — canaries updated (value or `len→size` by intent), doc-prose (STDLIB.md, doc tests), `doc/examples.js` regenerated |
| 2e | Convert consumer programs → re-green | convert | ✅ Done — `tests/fixtures/libs` (incl. `glb.loft` binary chunk length); ⚠ `lib/markdown` not checked out (re-run against `loft-libs-docs`) |
| 3a | Strict-index lint **default-on** for the text case | lint | ✅ Done — `text_index_units_lint_enabled` (opt-out `LOFT_NO_STRICT_INDEX_TEXT`); warns on `for i in 0..len(s){s[i]}`; oracle `tests/strict_index_text_lint.rs`, both backends; broke 0 existing tests |
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

- [phase0-inventory.md](phase0-inventory.md) — the full Phase-0 inventory, canary red set, and
  whole-surface audit (the worklist Phase 2 converts against).
- [lib-audit.md](../102-stability-contract/lib-audit.md) § H2 — the trigger.
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) #9 (`txt[i]`→character) — bears on the `s[p]` question.
- `default/01_code.loft` (`len`/`size`), `03_text.loft` — the stdlib surface being redefined.
- Tracker: [@PLN110](https://github.com/loft-lang/plans/issues/110).
