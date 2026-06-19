<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# E2 single-payload representation refactor — design

> **Status:** COMPLETE (2026-06-19, branch `2026-07-mac`) — every E2 test green on BOTH
> backends (full suite 2415/2416; the lone fail is the pre-existing environmental
> `kernel_port`, NOT E2), clippy + fmt clean.  `OpNullableToDense` and its copy machinery are
> DELETED (WIP 6, `2022f0b5`) — the value-boundary + return unwrap is a payload sub-ref view.
> The refactor's own 7 steps are all done.  What remains is the BROADER @PLN25 tail (NOT this
> refactor): the Step-5 consumer sweep + the gate flip.  Context: [[pln25-nullable-coherence]]
> memory + README § RESUME HERE.

## The one invariant (CONFIRMED by construction, not assumed)

> **`__nullable<S>`'s `Some` variant carries a single inline `payload: S` field whose type
> IS the same struct definition as a standalone `S`.  Therefore a DbRef at
> `some_record + payload_offset` shares S's exact field-offset table and IS a valid dense
> `S` reference — every operation defined on dense `S` works on it verbatim, with no copy
> and no per-field translation.**

This replaces the old form where `nullable_enum_for` copied S's fields *individually* into
`Some` (a 5-field packed struct: `disc, a, b, c, d`).  The packer gap-fills the 1-byte
discriminant's alignment slack and reorders the payload (dense `a@0,c@8,b@16,d@20` vs Some
`disc@0,b@4,a@8,c@16,d@24`), so a sub-ref read garbage and a field-by-field copy
(`OpNullableToDense`) was forced.  A single struct-typed field keeps S's own packing, so
the copy is unnecessary.

### Constructive evidence (the protocol's "plot the answer" step)

Hand-written `enum Maybe { Null, Some { payload: S } }` over
`struct S { a: integer, b: text, c: integer, d: boolean }`, via `loft introspect`:

```
t65 = structure("S")           : a@0(int), b(text), c(int), d(bool)
t67 = structure("Some", 2)     : enum=disc@0 ,  payload: t65        ← payload's TYPE is t65
match Some { payload }          → OpGetField(e, 8, 65)              ; payload@8, type 65 (=S)
payload.a                       → OpGetInt(payload, 0)              ; offset 0 in the SAME S def
```

`payload` is **inline-embedded** at offset 8 (after the disc byte, padded to S's 8-byte
alignment) and its type is literally `t65` — there is no separate "Some payload" layout, so
field offsets are identical *by shared definition*, not by coincidence.  The payload base
(8 here) is **alignment-dependent** (an S whose max align is 4 → payload@4); it must be
*computed* from the built `Some` structure, never hardcoded.

## Re-assertion sites and the N→1 collapse (protocol step 2)

The whole key system already funnels through ONE redirect, `Stores::key_owner(content)`
(`src/database/types.rs:440`).  Today it returns the `Some` *variant*, whose **direct**
fields are S's (individually copied), so a key field's `position` is already absolute
within the record.  Single-payload makes `Some`'s direct fields `{enum, payload}` and sinks
the keys into `payload`, so two things change at the redirect:

1. **`key_owner(__nullable<S>)` returns the inner `S` struct** (the payload's content type),
   not the `Some` variant — so every *name→index* and *index→field* resolution indexes S's
   own field list (`"a"→0`), exactly as a non-nullable `hash<S[k]>` does.
2. **Key byte-positions gain a base offset.**  A key field's `position` is now S-relative,
   but the stored record is the `Some` record, so the absolute offset is
   `Some.payload.position + S_field.position`.

Site inventory (all currently route through `key_owner`):

| Site | File | Role | Needs base? |
|---|---|---|---|
| `key_owner` | types.rs:440 | the redirect | — (change target to inner S) |
| `hash` / `create_key` / `field_name` | types.rs:1049/1294/1097 | build: name→index | no (indices only) |
| `determine_keys` | types.rs:455 | build: index→`keys[].position` | **YES** |
| `field_content` | search.rs:65 | runtime: read key bytes at `rec.pos+position` | **YES** |
| `get_keys` | search.rs:292 | runtime: key content TYPES (for `read_key`) | no (types only) |
| `bare_field_name` | generation/mod.rs:2035 | native codegen: key NAME | no (names only) |

**The brittleness = the two `YES` sites** (`determine_keys` build vs `field_content`
runtime) computing the same absolute position *independently*: if base is added at one and
not the other, build-time and run-time offsets drift → **silent keyed-collection
corruption** (no crash, wrong bucket / wrong compare).  **Collapse: one chokepoint**
`key_field(content, k) -> (content_type, abs_position)` that does `key_owner` + `key_base` +
the field lookup; `determine_keys`, `field_content`, and `get_keys` all route through it, so
the base-math is asserted **once**.  `key_base(content)` returns `Some.payload`'s position
for a `__nullable<S>`, else 0.

## Failure paths (write them down — this is where the invariant earns its keep)

1. **Payload base hardcoded to 8** → corrupts any S whose max alignment < 8 (payload@4).
   *Guard:* `key_base` computes it from the built `Some` structure.
2. **Base added at build but not runtime read (or vice-versa)** → keyed lookup reads the
   wrong bytes; insert and find disagree silently.  *Guard:* the single `key_field`
   chokepoint (above).
3. **`key_owner` still returns the `Some` variant** → `fields[k]` indexes `{enum,payload}`,
   so `k=0` hits the discriminant; every keyed read is junk.  *Guard:* change the redirect
   target + a `hash<S[k]>`-over-nullable regression on BOTH backends (Cluster F/K suites).
4. **Native tid ordering** (the rung-4 hazard): `Some`/`Null` structures must be interned in
   the same creation order on both backends, or native bakes a swapped type-id.  *Guard:*
   keep the existing `fill_all` eager-variant build; re-run the native hash suite.
5. **`payload: S` field built as a separate-record reference, not inline** → a sub-ref no
   longer lands inside the `Some` record.  *Guard:* the introspect layout check above
   (`OpGetField(e, 8, 65)` = inline sub-ref) re-run on a *synth* `__nullable<S>` gate-on.

## Implementation order (dependency-first; line numbers approximate post-rebase)

1. **Layout** — `nullable_enum_for` (data.rs:3808): replace the per-field copy loop
   (3877-3900) with ONE `payload` attribute typed `Reference(struct_d)`.  Verify a *synth*
   `__nullable<S>` introspects to `Some{enum@0, payload@base:S}` gate-on.
2. **Key contract** — `key_owner` → inner S; add `key_base`; add the `key_field` chokepoint;
   route `determine_keys` / `field_content` / `get_keys` through it.  (The build sites
   `hash`/`create_key` keep using `key_owner`; they only need indices.)
3. **Field access** — `find_poly_enum_field` (fields.rs:543) + callers (97/281): resolve
   `e.field` through `payload` (payload base + S-field offset).
4. **Construction** — build `payload` not individual fields: objects.rs, collections.rs,
   vectors.rs, builtins.rs (the dense→Some per-field copy loops collapse to one payload
   write / a sub-ref + dense build).
5. **Format** — format.rs: render the payload struct, not the `Some` field list.
6. **convert / unwrap deletion** — `convert` (generation/mod.rs) emits a payload sub-ref
   (dense S) instead of `OpNullableToDense`; then DELETE `OpNullableToDense`
   (default/01_code.loft, structures.rs `nullable_to_dense`, fill.rs, control.rs +
   `tail_is_nullable_unwrap`, operators.rs `is_struct_returning_call` arm, pre_eval.rs).
   `materialize_view_return` SURVIVES (genuine #306 views).
7. **Test** — update `tests/plan25_e2_layout.rs`: byte-identity assertion becomes "`Some.payload`
   is a dense `S`" (the stronger, correct invariant).

## Verify (both backends, gate-on)

- The gate-on suites: `plan25_e2_gap2`, `_hash`, `_json`, `_layout`, `_generics` + 150/151.
- Clusters F (keyed set/iter) + K (keyed construction) — the silent-corruption surface.
- The canonical incoherence probe (`[{…}, null, {…}] as vector<Item>` → len 3, real absent).
- Full `make ci` both backends before considering the gate flip (separate, later step).

## Progress (RESUME HERE)

**Done + verified (Steps 1-3 + part of 7):**
- **Step 1 — layout.**  `nullable_enum_for` (data.rs) emits ONE `payload: Reference(struct_d)`
  attribute instead of the per-field copy.  VERIFIED two ways: (a) introspect of a *synth*
  `__nullable<S>` gate-on → `Some{enum@0, payload: t65}` where t65 is dense S (inline,
  byte-identical to the hand-written probe); (b) `tests/plan25_nullable_enum.rs` def-level
  contract (`Some` = `{enum, payload:Reference(Row)}`) — updated + green.
- **Step 2 — key contract.**  `key_owner(__nullable<S>)` now returns the inner S struct;
  added `nullable_some_variant`, `key_base`, and the `key_field(content,k)->(type,abs_pos)`
  chokepoint (types.rs).  `determine_keys` (build) + `field_content` (runtime read) +
  `get_keys` (types) all route through `key_field`, so the payload base is added single-site.
  GATE-OFF SAFE: for non-nullable elements `key_owner`=identity, `key_base`=0, so `key_field`
  is behaviourally identical — confirmed by the full suite (no non-nullable keyed-collection
  regression; 2398 pass, the 17 reds are all gate-on E2 + the pre-existing `kernel_port`).
- **Step 3 — field access.**  The `__nullable<S>` unwrap (fields.rs path-1) now forms the
  dense-S sub-ref at the `payload` offset (`position(Some,"payload")`), not S's first field.
  `for_type` already keeps the element `Enum(..,true)`, so loop-var access hits this path.
- **Step 7 (partial).**  `tests/plan25_nullable_enum.rs` updated to the new contract (green).

**WIP 2 progress (full suite: 2410 pass / 6 fail, from 18; NO non-E2 regression):**
- **Step 4 CONSTRUCTION — mostly DONE.**  `plan25_e2_generics`, `_json`, `_hash` (6/6),
  `_nullable_enum` all GREEN gate-on both backends.  Done: (a) the three dense-value sites
  (objects.rs handle_field, collections.rs `[i]=expr`, vectors.rs append) now use a shared
  `Parser::build_some_present(some_d, ref, src)` (mod.rs) — set disc + ONE `set_field` copy
  of the dense S into `payload`, replacing the per-field loops; (b) named/anon `S{…}` literal
  via a `parse_object` guard → `parse_some_payload_object` (objects.rs): alloc `Some`, set
  disc, parse the body as dense S into the inline `payload` sub-ref; (c) the JSON walker
  (`walk_parsed_into`, structures.rs) recurses a present object into the `payload` sub-record.
- **Step 2/3 also extended:** `key_bearing_def` (typedef.rs) now returns the inner S (was
  missing — caused an OOB in `set_mutable`); convert (mod.rs) emits a payload sub-ref via
  `get_val` (inline struct → sub-ref, NOT a raw `OpGetField` which derefs).

**REMAINING (5 E2 reds — the checklist):**
- **Step 4 tail — inline-vector-literal + HEAP field corruption — FIXED (WIP 3, `3241b962`).**
  ROOT (NOT the earlier `remove_claims` theory): `set_default_value` (structures.rs)
  treated content type 6 (a 4-byte u32-raw field — e.g. a `character` codepoint) like type 0,
  writing an 8-byte `i64::MIN` via `set_int`.  The extra 4 bytes spilled into the next slot;
  for a TRAILING character in a tightly-sized single-payload `Some` payload the spill overran
  the record and clobbered the adjacent free block's size header → `fl_size` negate-overflow
  (debug) / `grow_words` overflow (release).  Fix: write content-6 defaults 4 bytes wide.
  Pre-existing latent bug; the 4-byte write is correct for every content-6 field.  Full suite
  2412/4 (from 6); construction + `local_assign` + `by_value_arg` gap2 now green.
- **Step 6 return-boundary view-return — DONE (WIP 4, `3fe8ef40`, both backends).**  The 2 gap2
  return reds returned null because the ref-return routing detected an `OpNullableToDense` tail;
  the unwrap is now a sub-ref `OpGetField`.  Fix: `tail_is_nullable_unwrap` now recognizes an
  `OpGetField` whose SOURCE is a `__nullable<S>` value (`unwrap_source_is_nullable`: a `Var`
  local, or a materialised `Block`/`If` tail like the `??` ncc block) — `materialize_view_return`
  then copies the viewed `S` into the return buffer (owned, not a dangling view).  The
  source-is-nullable check distinguishes it from an ordinary struct-field read; a direct `v[i]`
  index source is excluded.  gap2 4/4 both backends.
- **Step 7 layout byte-identity test — DONE (WIP 5, `3241b962`'s sibling).**  `plan25_e2_layout.rs`
  now asserts the single-payload contract (`Some` carries one inline `payload: Row`, dense Row
  layout intact, payload fits in `Some`) instead of byte-identity with a hand individual-field enum.

- **`OpNullableToDense` deletion — DONE (WIP 6, `2022f0b5`).**  Removed the op decl
  (default/01_code.loft), `Stores::nullable_to_dense` (structures.rs), the generated `fill.rs`
  dispatch (via `make fill`), and the dead handlers (pre_eval.rs hoisting arm, operators.rs
  `is_struct_returning_call` arm); stale comments updated.  `materialize_view_return` SURVIVES.
  Behavior-preserving (zero emitters): full suite 2415/2416, clippy + fmt clean.

**THE REFACTOR IS COMPLETE.**  All E2 tests green both backends.  What remains is the BROADER
@PLN25 plan (NOT this refactor):
- **Step 5 consumer sweep — STARTED (gate-on, `LOFT_E2_SYNTH=1 LOFT_NO_CACHE=1`).**  The wrap
  suite is the broadest consumer (runs the whole script corpus + libs against golden output);
  gate-on it should be byte-identical to gate-off if E2 is transparent.  Most individual script
  tests pass; the aggregates surface a heterogeneous, multi-session residue.  Gate-OFF stays
  green (2415/2416), so NONE of this affects shipped releases — it is exactly what the gate flip
  would surface.  **FIXED so far:** transparent format (WIP 7).  **RESIDUE (distinct seams):**
  1. **`&S` by-ref arg unwrap** — `expected &P160Item, got __nullable<P160Item>` on `set_p160`
     (tests/scripts/100-enhancements.loft:140).  TWO convert-arm attempts FAILED (both reverted):
     (a) `RefVar(Reference(S))` → payload sub-ref with `Deps::none()` → interp `px=0` (the by-ref
     arg path treats a no-deps `Reference` as OWNED and COPIES it; gate-off `__ref_1` is typed
     `ref(P)["items"]` — a deps-carrying VIEW — which is why it mutates back) + native E0308.
     (b) Same but carrying the source's deps (to mark it a view) → interp allocation OOB
     (allocation.rs:650, index 1000, a free-list/coroutine-store reference) + native still E0308.
     CONCLUSION: this is NOT a `convert` arm — the mutable-by-ref-into-a-nullable-element needs
     work at the by-ref MACHINERY level (the arg-binding view/copy decision + native `&S` codegen),
     a focused multi-backend matrix-first session.  Likely cleaner: copy-IN/OUT around the call
     (copy payload→tmp dense S, `f(&tmp)`, copy tmp→payload back via `build_some_present`) so no
     view aliasing is needed.  `convert` stays by-value-only (NOTE in mod.rs).
  2. **dense↔nullable `vector` `+=` — FIXED (seam 2, `02b78b74`).**  A CROSS-LIB `::`-qualified
     inferred literal (`[testlib::Point{…}]`) stayed dense because the inferred-literal peek
     (vectors.rs) read only `testlib`, missed the `{` (next token was `::`), and skipped the
     rewrite — while DECLARED `lib::S` sites are nullable, so `v += [lib::S{…}]` mismatched.  The
     peek now skips past `::` to the struct name (last segment).  17-libraries (the `libraries`
     wrap aggregate) passes gate-on both backends; gate-off unaffected.
  3. **null-store OOB at structures.rs:43 (record_new) — PARTIALLY FIXED (seam 3a, `51e00466`).**
     `record_new`/`record_finish` now redirect a `__nullable<S>` field-parent through the payload
     (`nullable_field_parent` = key_owner/key_base; E2-validated, gate-off-inert), eliminating the
     structures.rs:43 OOB class.  BUT 16-parser then hits a DEEPER upstream bug: `record_new(parent
     =__nullable<Definition>, field=0)` is a nested-construction call whose `field=0` matches
     NEITHER the enum's field 0 (disc) NOR a collection in `Definition` (Definition.field[0] is a
     `boolean` → `Cannot add to none-structure 'boolean'` at structures.rs:88).  So the CALLER'S
     field index is wrong for this nested cross-lib construction — an upstream codegen field-index
     bug (Definition is a cross-lib struct).  RE-CONFIRMED post-3a (instrumented): `record_new
     (parent=Definition [3a already redirected the enum→payload], field=0) -> tp=boolean` — so even
     with the parent correctly resolved to dense `Definition`, `field=0` is a SCALAR; the caller's
     index is wrong (not the nested collection's real position).  Also breaks the `audience_crystal`
     lib (`Cannot add to none-structure 'integer'`).  NEXT: find the construction-codegen
     (OpNewRecord) site that emits `field=0` for a nested collection inside a nullable cross-lib
     element; the index must be the collection field's real position in S.  (NOT shared with seam 2,
     which was the inferred-literal peek; this is the nested-construction field-index in codegen.)
  4. **wrong value in 15-lexer** — `assertion failed: Incorrect plus` (tests/docs/15-lexer.loft),
     a value divergence (not a crash) gate-on; isolate the differing computation.
  5. **par over nullable** — PARTLY FIXED (seam 5a, `bbb918d4`): `synth_nullable_par_wrapper`
     computed the dense-S coercion offset as `position(Some, S's-first-field)` (the old
     individual-field pattern) → wrong under single-payload; now `position(Some, "payload")`.
     `22e-par-many-materialise` clean gate-on both backends.  REMAINING: `22c-par-sources`
     (`hash par: 12` — par over a HASH yields a wrong sum, a deeper par-over-keyed seam) and the
     USER EXTRA-args bail (`script_threading`, builtins.rs:277 `extra_vals.is_empty()` — extend the
     wrapper to accept+forward extras + align the dispatcher arity; broader-plan Step 2).
  Then re-run the engine/wasm consumers (moros_glb, moros_editor_html, wasm_library_suite) gate-on.
- **The gate flip** — drop `LOFT_E2_SYNTH` in `e2_rewrite_enabled` (KEEP the `STD_SOURCE`
  dense-stdlib exclusion); fold the deferred P3 `default_native_value` Vector arm; graduate the
  gated probes into `tests/scripts/25-nullable-sequences.loft`; full `make ci` both backends;
  set the plan SHIPPED.  This is the single final PR to `main`.
- **Step 5 — format** (format.rs): render the `payload` struct, not the `Some` field list.
- **Step 6 — DELETE `OpNullableToDense`** (convert already emits the sub-ref): remove the op
  + its gap2 return-routing built around copy/alloc semantics — control.rs
  `tail_is_nullable_unwrap` + return materialization, operators.rs `is_struct_returning_call`
  arm, pre_eval.rs, fill.rs, structures.rs `nullable_to_dense`, default/01_code.loft.  With a
  sub-ref the routing is unneeded (flows on existing ref + #306 view-return).
- **Step 7** — `tests/plan25_e2_layout.rs` byte-identity → "`Some.payload` is a dense S" (1 red).
- **Then** both-backend gate-on verify + gate flip.

## Rollback

Pre-refactor clean state: branch tip `9a4e97ce` (before the first refactor commit).  Big-bang
has no green intermediate; if F/K/A4/gap-2/suite are not all green on both backends, revert.
