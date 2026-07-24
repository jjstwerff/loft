<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Nested narrow-int vector width — one fact, three homes

**Status — DESIGNED, NOT IMPLEMENTED. Blocked on ONE decision (§7).** Stage A (probes +
oracle + root cause) is complete and reproducible from this directory; no product code
has been changed. The tree is clean.

**Step 0 for whoever picks this up: claim a `loft-lang/plans` issue and rename this
directory `<N>-nested-narrow-width`.** It is unnumbered on purpose — numbering by
scanning local dirs mints a collision with any unmerged sibling branch
([loft-plan-workflow](../README.md)).

**Lineage.** The named remainder of a family that has now shipped four times:
plan [58-nested-vector-layout](../finished/58-nested-vector-layout/) (finished),
loft#483 (closed), loft#624 (closed), loft#437, loft#457. The `b9c7fb87` commit message
says it outright: *"#483 is NOT fully closed: 184-nested-narrow-int-vector still fails —
the narrow-int nested path holds another coincident derivation."* This plan is that
remainder. Reported again from outside by the Commonstore consumer
(`../../../../zero-trust-shared-files/upstream/LOFT_HANDOFF.md`), whose `vector<u8>` is a
ciphertext carrier — for them a silently-wrong byte vector is a silently corrupted backup.

---

## 1. What is broken

A nested vector with a **narrow numeric inner element** is misread by the *format* and
*slice* paths, on **both backends, identically**:

```
v: vector<vector<u16>> = [[1001, 2002], [3003, 4004]];
println("{v}");        // [[131204073,0],[262409147,0]]     131204073 == (2002<<16)|1001
v: vector<vector<u8>>  = [[11, 22], [33, 44]];
println("{v}");        // SIGSEGV (interpret) / rustc failure (native)
```

Two inner elements are read as one wider slot. Right length, plausible contents, no error —
the silent-corruption signature of the whole family.

## 2. The instrument

`./run.sh [--check-build]` runs `probes/*.loft` on **both** backends. 21 probes, one
composition axis each. Every probe carries a **hand-computed** `//! expect:` line —
agreement between backends is not a pass, both were wrong the same way here.

The runner encodes four tooling failures that produced false readings during Stage A, all
of them mine; do not "simplify" them away:

- `$?` is taken immediately after the run, never off a pipeline (`cmd | grep`; `rc=$?`
  reads GREP's status — a SIGSEGV read as rc=0).
- `--check-build` touches a source file and reports **cargo's own** exit code; a cache hit
  reported as "Finished in 0.05s" is not a compile.
- A cell with no output is **VACUOUS**, never a pass.
- `zz_harness_control_must_fail.loft` must report WRONG. It caught a real harness bug: a
  blanket `grep -v '^\['` meant to drop log lines also ate every answer, because a
  rendered vector starts with `[`.

## 3. Baseline (`main` + the #629 follow-ups, both backends)

`pass=16 wrong=22 crash=2 vacuous=2` (2 of the "wrong" are the harness control; the 2
vacuous are `--native` refusing to compile the u8/u16 cases).

| axis | result |
|---|---|
| **element-wise oracle** (`v[i][j]`) | **ok for u8/u16/i32/integer, both backends** |
| flat `u16` / `i32` | ok (fixed earlier — a regression here would be ours) |
| render | WRONG `i16 i32 u16 u32` · CRASH `u8` · ok `integer` |
| slice | WRONG `i32 u8` · CRASH `u16` · ok `integer` |
| 3-deep `i32` | WRONG |
| construction path (literal / typed local / append / fn-return) | **byte-identical wrong output in all four** |

Two readings that redirect the diagnosis:

1. **The stored data is correct.** The element-wise oracle passes everywhere, so the
   writer is fine and only the *reader* misreads. Nothing about storage needs to change
   for correctness.
2. **It is not literal type-inference.** All four construction paths produce identical
   wrong output, which refutes the in-tree comment at `src/typedef.rs`
   (*"Narrow-int element width is lost upstream in literal type-inference"*). Fix that
   comment when the code lands.

## 4. The layout oracle — read off the cases that WORK

`LAYOUT_ORACLE` (a temporary `eprintln` in `next_element`, not committed) printed the true
element spacing:

| declared | inner handles at | stride | `ci.size` |
|---|---|---|---|
| `vector<vector<character>>` | 8, 12 | 4 | 4 |
| `vector<vector<boolean>>` | 8, 12 | 4 | 1 |
| `vector<vector<text>>` | 8, 12 | 4 | 4 |
| `vector<vector<integer>>` | 8, **16** | 8 | 8 |

So #477's rule `max(ci.size, 4)` is **correct** and the writer uses the same rule
(`src/parser/vectors.rs` ~1744 / ~2581: *"for a sub-4 inner (boolean) pass the outer
vector type so the handle strides by 4"*). The format walk is not the culprit.

The decisive cell: the u16 trace is **character-for-character identical to integer's** —
`outer=vector<vector<integer>> c=vector<integer>(sz=4) ci=integer(sz=8) step=8`. A
declared `vector<vector<u16>>` **registers as `vector<vector<integer>>`**, so the renderer
cannot know the width. Inner elements written 2 bytes wide are read 8 wide.

## 5. Root cause — one fact, three homes

The fact is *"the schema type id for element type T"*. Three independent derivations:

| home | behaviour |
|---|---|
| `Data::narrow_vector_content` (`src/data.rs`) | **correct** — `1→byte`, `2 nullable→short`, `2→short_raw`, `4→int` |
| `Stores::db_type` (`src/database/types.rs`) | **wrong** — `1→byte`, `2→short`, `_→integer`; no `4→int`, no `short_raw`, and it sizes assuming nullable so `u16` bumps to 4 and falls through to 8-byte `integer` |
| `Parser::vector_of` (`src/parser/mod.rs:1974`) | tries `narrow_vector_content` first (right for flat leaves), else falls back to `def(type_elm).known_type()`, which level-collapses for a nested element |

They agree only when the element is 8 bytes wide — exactly the observed boundary
(`integer` ok, everything narrower broken; `character`/`boolean`/`text` ok because they
have their own type variants and never enter the integer arm).

## 6. Fix design

**One resolver, three callers.** A single function mapping a loft element `Type` → schema
type id: narrow leaf via `narrow_vector_content` if applicable, recurse for
`Type::Vector`, else `known_type`. `db_type`, `vector_of`'s fallback and the flat path all
call it. Writer and reader then move together and the coincident derivation is gone —
this is the "fold the fact into the data structure, one home per fact" move, and it is
what stops the fifth instance.

**Consequence:** the outer slot for a narrow-inner nested vector goes **8 → 4** bytes.
That is a storage-layout change (see §7).

**Do not ship half of it.** Narrowing `db_type` alone was tried and is a **regression**:
element access and `len` become correct while render/slice CRASH, and it breaks
`tests/scripts/184-nested-narrow-int-vector.loft`, which passes on `main`. The writer
keeps the old stride, so reader and writer disagree.

**Method (as directed): duplicate, gate, switch when correct.** Build the new resolver
alongside the existing ones behind `LOFT_NESTED_WIDTH_V2`, so the whole matrix can be run
old-vs-new without touching default behaviour. Flip the default and delete the flag only
when every gate in §8 is green, in one commit with the layout-hash bump.

## 7. THE OPEN DECISION — the only thing blocking implementation

The layout change is not optional (§6) and the `layout_algo_hash` bump is its honest,
visible consequence — @PLN97's golden conformance test exists to make exactly this
deliberate. What needs a human call is **persisted stores**: a store on disk holding a
nested narrow-int vector would be read with the new stride.

Exposure is narrow — any such store was written by a program that never rendered or sliced
those vectors, since both crash or corrupt today — and loft is pre-1, so a layout bump is
permitted ([COMPATIBILITY.md](../../COMPATIBILITY.md)).

**Decide: plain hash bump, or hash bump + a migration path for data at rest.**

## 8. Verification gates (all must be green before the flag is removed)

1. `./run.sh --check-build` — 21 probes, both backends, `wrong=crash=vacuous=0` except the
   harness control, which must stay WRONG.
2. `tests/scripts/184-nested-narrow-int-vector.loft` (the named remainder), plus
   `446-nested-vector-format`, `553-nested-vector-slice`, `555-two-nested-vector-slices`.
3. The consumer's 78-cell differential on both backends —
   `../../../../zero-trust-shared-files/upstream/collection-width-differential.loft`. It
   currently PASSES with one pin (`nest<u8>/slice`); the fix must let that pin be
   **removed**, and their gate is red in both directions so a stale pin fails too.
4. `make ci`.
5. @PLN97 golden layout test + `layout_algo_hash` updated deliberately, per §7.
6. Graduate the probes to `tests/scripts/` so the class stays covered.

## 9. Out of scope

- **loft#628** — nested vector bound into a struct field loses every row. The consumer's
  own analysis shows it is **not** width-tracking (it loses rows for `integer` and `text`
  too) and it is *unstable* (empty in isolation, SIGSEGV in a larger file). A separate,
  nesting-specific defect; `tests/scripts/628-nested-vector-struct-field-bind.loft` exists.
- The consumer's `loft-libs-core#24` (crypto 0.3.5 dropped four `[wasm.bridge].routes`) —
  a library repo, not core.
- Their loft#622 narrowing-cast shape and the flat #624 cells: **verified fixed** on this
  tree; their handoff is stale on those two.

## 10. Note for the consumer report

Their gate pins only `nest<u8>/slice`, but this suite shows `u16` and `single` **crash** on
nested slice and `i16/i32/u16/u32` all corrupt on render. Their file explains why (a
SIGSEGV takes the whole gate down, so crashing cells were named in prose instead of
pinned) — so their pin under-reports the class by design, not by oversight. Worth telling
them once the cells merely differ, so they can promote them.
