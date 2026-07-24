<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Nested narrow-int vector width — one fact, seven homes

**Status — FIXED, all §8 gates green.** Stage A (probes + oracle + root cause) is
recorded below as written; §6a records what implementation changed about it, and §7 is
resolved — **the layout of every previously-covered type is unchanged, so there is no
data-at-rest migration**, proven by the @PLN97 golden diff, not argued.

The design said **three** homes. Implementation found **seven**: the three below, plus
the index-read stride, the iteration stride, the slice materialiser, and the runtime
append/copy/format strides. Each extra one surfaced the same way — the probe matrix went
green for one width and red for another, because a reader and the writer had been wrong
*together*. That is the shape of a coincident derivation, and it is why the fix could not
stop at the first three (§6 already predicted the half-fix would regress; it did, twice).

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

## 6a. What implementation changed about §6 (written after the fact)

**The resolver is `Data::vector_element_type` (`src/data.rs`)** — narrow leaf →
`narrow_vector_content`; nested vector → recurse and wrap; else the leaf's own
`known_type`. It returns `Option`, and `None` means "no id yet" (forward reference,
generic type variable): each caller keeps its own recovery for that, because the options
genuinely differ — `typedef::fill_database` can fill the type on the spot, `vector_of`
must bake a sentinel for the later pass.

**Seven callers, not three.** Beyond §5's three, the same fact was re-derived at:

| home | was |
|---|---|
| `parser/fields.rs` index-read stride | `size(known)` off the level-collapsed elem, clamped `.max(4)` |
| `parser/collections.rs::vector_elem_iter_stride` | `element_stack_size(inner).max(4)` — the INNER scalar |
| `parser/expressions.rs` slice materialiser | one id for two roles (container for `OpNewRecord`, element for `OpCopyRecord`) |
| `parser/mod.rs::append_elem_tp` + `objects.rs` field bind | `vector_of(content)` — one level too deep once nesting registers honestly |
| `database/structures.rs` `record_new` / `vector_add`, `database/format.rs` `next_element` | `size(content(x)).max(4)` at three runtime sites |

Every one of those was *right* while the type table could not express
`vector<vector<u16>>`; each is now a plain read of the element type's own size. The
`.max(4)` clamps are gone — the clamp WAS the coincidence.

**The flag was not built.** Its purpose was to run the matrix old-vs-new without touching
default behaviour; a `/usr/local/bin/loft` from before the change gives the same
comparison for free, and it was used that way throughout (baseline, every regression
bisect, and the positive control proving the new guards fail on the old binary). Two code
paths deleted in the same commit would have added risk without adding evidence.

**Two extra defects fell out**, both masked by the thing being fixed and both now guarded:

- `((v[i] ?? 0) & 255) as u8` failed native compilation (E0308) — the consumer's loft#622
  follow-up shape, which §9 below wrongly recorded as already fixed. A narrow-int
  value-block cast its own tail to the element width, but a loft `integer` is `i64` in
  every Rust position except a function's return signature; return and assignment widened
  it back, the operand seam of `op_logical_and_int` did not. Confined to the function BODY
  block (`block_tail_cast`, `src/generation/mod.rs`).
- Removing that cast exposed a real interp/native divergence it had been papering over:
  native's `OpReadFile` read every narrow int as `i8`/`i16` and let a downstream cast
  re-narrow it, so an unsigned `u16` 0xBEEF arrived as −16657 the moment nothing did. The
  interpreter's @PLN47 rule (sign from the type's own range) is now mirrored in
  `codegen_runtime.rs`, so the backends agree by construction rather than by a consumer's
  cast.

## 7. RESOLVED — no migration, and no layout change to migrate

The design expected the outer slot to move 8 → 4 for a narrow-inner nested vector and
asked whether persisted stores needed a migration path. Implementation makes the question
moot, and the @PLN97 golden proves it rather than arguing it:

**The `layout_algo_hash` did not move.** Struct FIELDS already registered nested vectors
honestly — `typedef::fill_database`'s @PLAN58 branch routed them through `db_type`'s
recursive `Vector` arm — so the layout of data at rest was never the collapsed one. The
level-collapse lived in `vector_of`, which serves locals, parameters, returns and
literals: none of it is data at rest. Re-running `tests/layout_golden.rs` unchanged
passed 5/5.

The corpus could not SEE the class, though: it carried `VecNest { vv:
vector<vector<integer>> }` and no narrow inner, and a narrow inner in a struct field IS
the one field shape whose registration the fix changes (`db_type`'s scalar arm sizes a
narrow int as if nullable, so `vector<vector<u8>>` had registered as `vector<short>`).
So `VecNestNarrow { vv: vector<vector<u8>> }` joins the corpus. The re-blessed golden
diff is **exactly three added rows and no changed row**:

```
> VecNestNarrow          size=4  struct{vv@0:vector<vector<byte>>}
> vector<byte>           size=4  vector<byte>(elem_size=1)
> vector<vector<byte>>   size=4  vector<vector<byte>>(elem_size=4)
```

`LAYOUT_ALGO_HASH` moves for the added coverage alone. `LAYOUT_CONTRACT` stays 0
(@PLN102's flip-gate is inert pre-freeze).

## 8. Verification gates — ALL GREEN

1. `./run.sh --check-build` — **24 probes** (3 added for the fn-return axis), both
   backends: `pass=46 wrong=2 crash=0 vacuous=0`, the 2 being the harness control that
   must stay WRONG.
2. `184-nested-narrow-int-vector`, `446-nested-vector-format`, `553-nested-vector-slice`,
   `555-two-nested-vector-slices` — green both backends. Also `173-reassign-return`,
   `302-vector-buffer-delivery` and `628-nested-vector-struct-field-bind`, which the
   half-finished fix broke and the complete one restores.
3. The consumer's 78-cell differential: **`diverging: []` on both backends**. Their gate
   now fails only because the pin is stale — which is its designed behaviour and their
   file to update (§10).
4. `make ci` — 3464/3464, fmt + clippy clean.
5. @PLN97 golden — re-blessed with the narrow-nested corpus type, diff hand-verified
   (§7), `LAYOUT_ALGO_HASH` updated.
6. Graduated: `tests/scripts/624-nested-narrow-width.loft` (render × 5 widths, element-wise
   reference, slice, all four construction paths, depth 3, concat, reassign+implicit
   return) and the narrowing-cast half appended to
   `tests/scripts/622-ncc-text-compare-call-arg.loft`. Both verified NON-VACUOUS: the old
   binary SIGSEGVs on the first and fails E0308 on the second.
   `tests/scripts/pln110-size-vector.loft` updated — `size(vector<vector<integer>>)` is
   now 2 × 4, which is what `OpSizeVector`'s contract already said ("a heap/reference
   element counts as its 4-byte record pointer"); it had pinned 2 × 8 to match the stride
   bug.

## 9. Out of scope

- **loft#628** — nested vector bound into a struct field loses every row. The consumer's
  own analysis shows it is **not** width-tracking (it loses rows for `integer` and `text`
  too) and it is *unstable* (empty in isolation, SIGSEGV in a larger file). A separate,
  nesting-specific defect; `tests/scripts/628-nested-vector-struct-field-bind.loft` exists.
- The consumer's `loft-libs-core#24` (crypto 0.3.5 dropped four `[wasm.bridge].routes`) —
  a library repo, not core.
- ~~Their loft#622 narrowing-cast shape and the flat #624 cells: **verified fixed** on this
  tree; their handoff is stale on those two.~~ **Wrong on the first half — the consumer was
  right.** The flat #624 cells were indeed fixed, but
  `((v[i] ?? 0) & 255) as u8` still failed E0308 here, exactly as their matrix said. It
  was Stage A that checked the wrong variant: nearly every neighbouring shape compiles, so
  a probe picked without their 7-row table reports it fixed. Fixed in this plan (§6a); the
  table is now a guard in `tests/scripts/622-…`. Their handoff was stale on the flat #624
  cells only.

## 10. Note for the consumer report

Their gate pins only `nest<u8>/slice`, but this suite shows `u16` and `single` **crash** on
nested slice and `i16/i32/u16/u32` all corrupt on render. Their file explains why (a
SIGSEGV takes the whole gate down, so crashing cells were named in prose instead of
pinned) — so their pin under-reports the class by design, not by oversight. Worth telling
them once the cells merely differ, so they can promote them.

**What to tell them now that it is fixed:**

1. **Remove the `nest<u8>/slice` pin** — `known()` should return `""`. All 78 cells agree
   with element-wise access on both backends; their gate is red purely because the pin is
   stale, which is the direction they built it to catch.
2. **Promote the cells they could only name in prose.** Nested render and nested slice no
   longer crash for any width, so `nest<u16>`, `nest<i32>` and `nest<integer>` rows can be
   probed rather than described — the reason for excluding them is gone.
3. **Their loft#622 workaround can go.** `((v[i] ?? 0) & 255) as u8` compiles on
   `--native`; the "bind the `??` to a local first" form is no longer needed, and neither
   is dropping the mask. All seven rows of their variant table are now a regression test
   here.
4. **Their read of the pattern was right and is worth repeating back**: in all three cases
   the surviving shape was the one added later, in a comment. Both defects this plan fixed
   were re-assertion-site problems — seven homes for one fact, and a value normalisation
   that happened only if some consumer cast. The fix in each case was to delete the extra
   sites, not to add one more.
