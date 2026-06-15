<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Embedded-record null — the nullable-enum representation

> **Part of** [@PLN25](README.md) (nullable sequences). **Status:** design (not yet
> built). **Resolves:** finding 12's open `reference`/struct element case, finding 9
> (nullable struct field defaults), the `field = nullable_source()` crash, **and the
> same crash for nullable enums** (a pre-existing bug — see § The unification). **Method:**
> written as a hypothesis under `design-protocol`; § Probes lists the falsification tests
> to run before code. An earlier draft proposed an out-of-band validity bitmap; § Rejected
> alternative records why the in-band discriminant wins.

## The problem (grounded)

A struct stored **inline** — a `vector<Row>` element (`8 + i*size`) or an embedded field
(`Box { item: Row }`) — has no encoding for "absent". Every byte pattern is a valid value
(`Row{id:0, tag:null}` ≠ "no Row"), so a null `Row` value (which a *variable* can hold via
the `u16::MAX` DbRef sentinel) cannot be *stored* into an inline slot: `OpCopyRecord` derefs
the null source's store and OOB-crashes (`allocation.rs:560`, index `65535`; native
`OpCopyRecord(cell, ())`).

Storing the struct **by-reference** (a `rec==0`-nullable handle + a separate record per
element) adds an indirection per embedded record — a pointer-chase and an allocation on
loft's hottest data path. **That breaks the flat/inline memory model, so it is rejected.**

## The unification (what probing revealed)

The crash is **not** struct-specific. Probed both scalar cases:

| value | storage | `x = mb(false)` (null source) |
|---|---|---|
| struct **variable** `m: Row` | a `DbRef` slot | ✅ holds null (`m == null` true) |
| **enum variable** `x: Shape` | **inline value record** | 💥 `allocation.rs:560` OOB (index 65535) |
| struct **field / vector element** | **inline value record** | 💥 same crash |

So the real family is **inline value records** — enums *today*, plus embedded structs and
vector elements. A struct *variable* escapes only because it is a reference (`DbRef`) slot.
Nullable enums (`Type::Enum(_, /*nullable*/ true, _)`) are an *intended* feature that is
**currently broken by this exact crash.** One representation + one fix covers all three.

And the encoding already exists: an enum's discriminant at offset 0 numbers variants from
**1** (`v = f_nr + 1` in `set_default_value`), leaving **`0` = no variant = null**. A
nullable inline record is null iff its discriminant is 0 — the layout loft already ships.

## The invariant (the one rule)

> A nullable inline value record is null **iff** its discriminant (offset 0) is `0`. A
> nullable struct field/element carries that discriminant and is therefore **byte-identical
> to a nullable enum** (`Null | Some(Row)`); `not null` carries no discriminant and is
> pure inline (today's bytes).

This is not a new encoding — it is loft's existing enum-value representation (discriminant
at offset 0, fields after it, `match` dispatch, `0`-default). A nullable `Row` ≈
`Option<Row>` ≈ a 2-variant enum, and reuses that machinery rather than inventing a bitmap.

## Why in-band discriminant beats an out-of-band bitmap (the slice/copy axis)

The discriminant lives **in** each element's bytes, so every data-movement op carries it for
free; a bitmap is out-of-band and must be moved/aligned separately:

| op | out-of-band bitmap | in-band discriminant |
|---|---|---|
| **slice `v[a..b]`** (loft materializes element-by-element) | extract bits `a..b`, **re-pack** to a new bit-aligned buffer | per-element copy carries the discriminant — nothing extra |
| **copy `v2 = v1`** | two memcpys + two allocations, kept in sync | one memcpy, discriminants included |
| **append / remove-shift** | grow/shift **two** regions in lockstep (silent-desync risk) | grow/shift one region |
| **space** | 1 bit / elem | +1–2 bytes / elem (the discriminant) |
| **machinery** | all-new (bitmap, bit-slice, two-buffer free) | **reuses enum** (layout, match, default, field offsets) |

The bitmap's only win is space (a bit vs a byte). Weighted by loft's element-by-element
slices and whole-vector copies — and by the machinery it would NOT have to build — the
in-band discriminant wins decisively. The space cost is opted out by `not null`.

## The two orthogonal `not null` axes

`not null` is the efficiency control on both axes; only the **element** axis is new.

| spelling | non-null thing | encoding |
|---|---|---|
| `v: vector<Row> not null` | the **container** (no absent-vector) | P1/P2 `u16::MAX` store sentinel |
| `vector<Row not null>` | the **elements** | **no discriminant** — pure inline |
| `Box { item: Row not null }` | the embedded **field** | no discriminant on `item` |
| `vector<Row not null> not null` | both | fully dense — **byte-identical to today** |

`vector<T not null>` is established: `vector<integer not null>` parses and reclaims the
scalar sentinel code today; the struct case is the structural analog (drop the
discriminant). The parser currently rejects `not null` after a *named* element type in a
generic (`Expect token >`) — wiring that is part of the work.

**Default = nullable** (matches loft's "nullable unless `not null`" model): a plain
`vector<Row>` / `item: Row` carries the discriminant (and so becomes enum-shaped). **`not
null` = zero overhead:** no discriminant, inline bytes only, `[i] = null` / `= null` is a
*compile error*. Dense, perf-critical data declares `not null` and is byte-identical to
today; only declared-nullable data pays the discriminant.

## The fix surface (small, because the layout is reused)

- **Represent** a nullable struct field/element with the discriminant-at-0 (enum) layout;
  fields sit after the discriminant, exactly as enum-value fields already do.
- **Assign null** (`slot = null`, or a runtime-null source `slot = mb(false)`): set the
  discriminant to `0` and free the slot's heap deps — **do not** call `OpCopyRecord` on the
  null source. This is what retires the `allocation.rs:560` / native crash, for enums and
  structs alike.
- **Detect null** (`== null`): read the discriminant.
- **Default value**: discriminant `0` (null) for a nullable slot; a zero-initialized inline
  record for `not null` (finding 9's resolution).
- **Slice / copy / append / remove**: unchanged — the discriminant is inline, so the
  existing byte-movement carries it.

## Probes — falsify before building (design-protocol step 3)

Each is the cheapest test that could prove a load-bearing claim FALSE. Expect to falsify.

1. **Byte-identity with a hand-written enum (the core claim).** *Claim:* a nullable
   embedded `Row` is byte-identical to an explicit `enum { Null, Some(Row) }`. *Probe:*
   `introspect` both — record layout, discriminant size/offset, field offsets, and the
   generated field-read code — and diff. A mismatch means "reuse the enum representation"
   is aspirational, not real (the protocol's over-unification guard, step 4).
2. **`not null` reproduces today's bytes.** *Claim:* `vector<Row not null>` /
   `item: Row not null` carry no discriminant and are byte-identical to today. *Probe:*
   dump the record bytes before/after; any diff means the discriminant leaked into the fast
   path.
3. **Null-assign no longer derefs the sentinel.** *Claim:* `slot = mb(false)` writes
   discriminant `0` instead of copying from store 65535. *Probe:* the exact crashing
   programs (`vr[i] = mb(false)`, `bx.item = mb(false)`, **and `enum_var = mb(false)`**) run
   clean on both backends; `== null` is true after.
4. **Slice/copy carry the discriminant.** *Claim:* `v[a..b]` and `v2 = v1` preserve which
   elements are null with no bitmap logic. *Probe:* a vector with a null in the middle,
   sliced and copied; assert each surviving element's null-ness rode along.
5. **Discriminant size / field-offset shift is handled on both backends.** *Claim:* fields
   after the discriminant read at the right offset in interp AND native. *Probe:*
   `vr[i].id` on a present element + `--native-emit` the field read; confirm the offset
   includes the discriminant.

## Implementation plan — verifiable steps

Each step states **Do** (the concrete change + site) and **Verify** (the gate that passes
only if the step is correct). A step does not start until the previous one's gate is green;
each phase ends with a full-suite gate on **both backends**. Every step begins by reading
its named site — the sites below are the entry points, not a claim that nothing else moves.

### Phase E1 — nullable enum VARIABLE [DONE — both backends green]
*Probing corrected the plan: an enum **variable** is a `DbRef` slot (like a struct
reference), so its null is the `store_nr==u16::MAX` SENTINEL — NOT a discriminant-0 record.
The discriminant-0 encoding is for INLINE enums (fields/elements), which is E2. E1 turned
out to be two distinct bugs, neither the assign:*

- **E1.0 Characterize.** The assign (`x = mb(false)`) does NOT crash — `x` holds the
  sentinel fine. The crash is in **`x == null`**: enum `==null` lowered to
  `OpEqInt(OpConvIntFromEnum(OpGetEnum(x,0)), …)`, and `OpGetEnum` derefs the absent
  record (`store_nr=65535`) → `allocation.rs:560` OOB.
- **E1.1 `== null` via the sentinel.** Added `OpRefIsNull(r) = store_nr==u16::MAX`
  (`default/01_code.loft`, regenerated into both backends) and an `enum_null` branch in the
  `==`/`!=` dispatch (`operators.rs`) that lowers `enum ⊗ null` to it. NOT `OpEqRef`: its
  `rec==0` test misreads a *present* enum on native (enums are inline-represented there, so
  `rec==0`), the same finding-1/4 distinction.
- **E1.2 Native return-ABI bug (deeper, pre-existing — surfaced by E1's native gate).** A
  nullable-returning fn's *present* enum value came back as the sentinel on native, so even
  `match` crashed. Root: the ref-retbuf **tail-capture** (`pre_eval.rs heap_shape_matches`)
  only matched `(Reference, Reference)` / `(Enum,true × Enum,true)`. The tail
  `if c { Circle{} } else { null }` infers to the **variant** type `Reference(Circle)`,
  while the target is `Enum(Shape,true)` — no match → capture missed → the present value was
  dropped and a sentinel returned. Fix: match a variant-ref/enum tail against its enum target
  via `def(variant).parent() == enum`. Nullable structs were unaffected (inline retbuf).
- **E1 gate met:** `enum == null` (null→true, present→false), present payload survives the
  nullable return, on BOTH backends; regression `enum_null()` in
  `tests/scripts/25-nullable-sequences.loft`; full suite green (2381).

(Original plan text, for E2/E3, kept below.)

- **E1 gate:** enum-var null round-trips on both backends; `find_problems` full suite green;
  the repro graduates to `tests/scripts/`.

### Phase E2 — nullable struct fields + vector elements

**E2.1 gate result (probed 2026-06-15): byte-identity is NOT free — chose approach (a).**
A plain struct and the hand-written enum differ: `Row{id,tag}` lays out `id@0, tag@8`
(size 12, no discriminant); the enum `RSome{id,tag}` reorders for packing — `disc@0,
tag@4, id@8`. So the only way to byte-match is to **represent the nullable struct
field/element AS a synthesized nullable enum** (approach (a)) and run it through the
existing enum layout/construct/access/copy machinery — not to bolt a discriminant onto the
struct layout. Staged below; each stage has its own gate and the entry point found in the
machinery survey.

- **E2a.1 — Synthesis helper [foundation]. [DONE]** `Data::nullable_enum_for(lexer,
  struct_d) -> enum_d` (`src/data.rs`, after `tuple_def`) builds, once per struct, a
  synthetic enum `{ Null, Some<fields-of struct_d> }` by mirroring `parse_enum_values`'
  first-pass calls EXACTLY (parent per-variant `constant` attrs with discriminants 1/2; the
  `Some` variant's `enum` discriminant at offset 0 + `struct_d`'s payload copied in order;
  `Null` is unit), so layout-identity is by construction.  Memoized on `__nullable<{name}>`,
  registered at `STD_SOURCE`, `mark_synthetic`.  *Verified:* `tests/plan25_nullable_enum.rs`
  (4 tests — def structure, unit-Null/payload-Some, verbatim field types, idempotent).
- **E2a.2 — Wire nullable struct FIELDS to it. [DONE — gated]** A new pass
  `synth_nullable_struct_fields` at the **start of `fill_all`** (`src/typedef.rs`, before the
  unit-variant discriminant pass + the layout loop) rewrites each nullable inline
  struct-reference field's typedef to `Type::Enum(syn, true, _)` and registers the synthetic
  enum via the extracted `register_enum_db` helper (shared with `actual_types_deferred`'s
  `Enum` arm, so synthetic + hand-written enums register identically).  **Scaffolding:** gated
  behind env `LOFT_E2_SYNTH` and limited to non-stdlib sources — default-off keeps every
  existing program byte-identical (a plain `Row{…}` / `b.item.id` still lowers as a struct
  until the E2a.3/4 glue exists).  *Verified (Probe 1):* `tests/plan25_e2_layout.rs` — with
  the gate on, `Box { item: Row }` rewrites `item` to `__nullable<Row>`, and the synthetic
  `Some` variant is **byte-identical** to a hand-written `enum { HNull, HSome{…} }`
  (discriminant @0 on both, matching `id`/`tag` offsets, matching variant size).  *Gotcha
  recorded:* `parse_str` parses into source 0 (= STD_SOURCE) so the scaffolding skips it — the
  layout test parses a real FILE (`Parser::parse`, source = MAIN_SOURCE) like the binary does.
- **E2a.3 — Construction coercion.** *Do:* `Box { item: Row{…} }` builds the `Some` variant
  (struct-literal → `Some(...)`; discriminant = present). *Verify:* construct + read
  `bx.item.id` round-trips on both backends.
- **E2a.4 — Access / null / default.** *Do:* `bx.item.id` unwraps `Some` (fields at the
  post-discriminant offsets); `bx.item == null` → inline discriminant test (distinct from
  E1's sentinel — this value is inline); `bx.item = null` → `Null` variant; default → `Null`
  (finding 9). *Verify:* null/present round-trip, neighbour fields intact, both backends.
- **E2a.5 — Vector elements.** *Do:* `vector<Row>` elements take the synthetic enum; wire
  `vr[i]=null` / read / `== null` / iterate. *Verify:* Probe 3 (`vr[i]=null` clean both
  backends), Probe 4 (slice/copy preserve null-ness), Probe 5 (`vr[i].id` at the
  post-discriminant offset on native).
- **E2a.6 — `not null` opt-out.** *Do:* `Row not null` field/element skips synthesis (plain
  inline). *Verify:* Probe 2 — byte-identical to today.
- **E2a.7 — native parity + full suite** on both backends; regression cells.

*Risk note:* E2a.3/E2a.4 (struct-literal↔`Some` coercion, `.field` unwrap) are the load-
bearing unknowns — the syntax says `Row` but the type is the enum, so construct/access need
glue. If the existing enum machinery does NOT make these fall out cheaply, that is the alarm
to re-scope (the "reuse, don't build" premise would be weaker than the design assumed).

### Phase E3 — `not null` opt-out + dense fast path
- **E3.1 Parser.** *Do:* accept `not null` after a named element type in a generic
  (`vector<Row not null>`) and on embedded fields (`item: Row not null`). *Verify:*
  `vector<Row not null>` parses (today it errors `Expect token >`); `vr[i] = null` on it is a
  *compile error*.
- **E3.2 Skip the discriminant.** *Do:* a `not null` field/element carries no discriminant.
  *Verify:* **Probe 2** — byte-dump a `not null` vector/field before vs after the whole change:
  **byte-identical to today** (no discriminant leaked into the fast path).
- **E3 gate:** dense `not null` path is zero-overhead and byte-identical to today; full suite
  green.

**Interim crash-safety (until E1/E2 land):** a `not null`-only stopgap — reject `= null` on
inline struct fields/elements at compile time and raise a clean recoverable error on a
runtime-null source — keeps the `allocation.rs:560` OOB / native codegen crash from shipping
while the representation work proceeds.

## RESUME POINT — start E2a here (standalone; read this section cold)

**Shipped + green on `2026-07-mac`** (all pushed): slice from-end+clamp (loft#384) +
reverse-slice; simple-typed vector ELEMENT null (int/bool/float/text — iteration no longer
breaks at a null element + `float == null`); **E1** = nullable enum VARIABLE null on both
backends. **E2 (this doc) = nullable inline STRUCT fields + vector elements — NOT started in
code.** Full suite was 2381-green at the last push.

### Step 0 RESULT (probed 2026-06-15, BOTH backends) — YES, (a) is sound
`/tmp/p_step0/step0.loft` + `step0_present.loft` (hand-written `enum NRow { RNil, RSome
{ id, tag } }` as `struct Box { item: NRow }`):
- **Present value works on both backends:** construct `Box { item: RSome{…} }`, payload via
  `match b.item` (id=5, tag="a"), present `== null` → false / `!= null` → true, and an
  explicit `RNil` variant — all green on interpret AND native. So enum-typed fields are NOT
  themselves broken → **synthesis (a) is the sound path** (gate's YES branch).
- **Null source into the field crashes — the PREDICTED target, not a re-scope.**
  `Box { item: maybe(false) }` (a nullable-returning fn) hits exactly `allocation.rs:560`
  (index 65535) — the `OpCopyRecord`-on-null-source bug — on BOTH backends at the SAME
  runtime site (cleaner than the anticipated separate native-codegen crash). This is E2's
  fix-surface target: on null-assign, set discriminant 0 + free heap deps, do NOT
  `OpCopyRecord` the null source.
- **Two things Step 0 did NOT settle (both blocked behind that crash → E2a.4, neither
  changes the GO):** (1) whether present `== null`→false goes through the *discriminant*
  test vs. accidentally reading inline bytes as a `store_nr` (the crucial distinction below
  — inline MUST use the discriminant, not `OpRefIsNull`); (2) whether inline `== null`
  returns *true* for a null (unobservable until the construct/assign crash is fixed).

So the remaining E2 work is the **transparency glue** (next §) + the null-assign fix, NOT a
question of whether enums-as-fields work. Proceed to E2a.1 synthesis + the vertical slice.

### E2a.1 + E2a.2 DONE (gated) — NEXT: E2a.3 construction coercion
**Built + verified (gate off by default; suite byte-identical):**
- **E2a.1** `Data::nullable_enum_for` (`src/data.rs`) — synthesises `__nullable<T>`; 4 tests
  in `tests/plan25_nullable_enum.rs`.
- **E2a.2** `synth_nullable_struct_fields` pass + `register_enum_db` helper (`src/typedef.rs`),
  gated behind `LOFT_E2_SYNTH`, non-stdlib-only — rewrites a nullable struct field to the
  synthetic enum. **Probe 1 PASSES** (`tests/plan25_e2_layout.rs`): byte-identical to a hand
  enum.

**NEXT — E2a.3 (construction coercion), the load-bearing glue.** Scoped this session (no code
yet — the hook needs threading work, see below):

- **Failure mode (gate ON, characterized):** `Box { item: Row{id:5, tag:"hi"} }` does NOT
  crash — it reads back `id=4 tag=null` (WRONG). Construction wrote `Row`'s bytes at `Row`'s
  offsets (`id@0, tag@8`), but the field is now the enum, whose `Some` variant reorders to
  `disc@0, tag@4, id@8` — so access reads the wrong slots. Coercion is simply missing.
- **Storage model (confirmed):** a nullable struct field is a **big enum → 4-byte record
  pointer** (`parse_object_field`'s `Type::Enum(_,true,_)` arm, `objects.rs:1902` — "enum-big
  header is a 4-byte u32 record pointer"), NOT inline bytes. Probe 1's byte-identity is the
  variant RECORD's internal layout; the field holds a pointer to it. Construction must
  allocate a `Some` record, write disc + fields, store the rec-id — exactly the path a
  hand-written `RSome{…}` already takes (Step 0).
- **DESIGN RESOLVED — null = discriminant 0** (read from `set_default_value`,
  `database/structures.rs`: `Parts::Enum` defaults the offset-0 byte to `0`). So:
  - default / unset / `= null` → **discriminant 0** (the absent state; `== null` true).
  - `Row{…}` → the **`Some` variant** (discriminant 2).
  - the synthesized **`Null` variant (discriminant 1) is VESTIGIAL** in transparent mode —
    never produced; null is disc-0, not the Null variant. (Could simplify E2a.1 to a
    single-variant `{Some}` later; not worth churning the verified 2-variant helper now.)
  - **E2a.4 `== null` tests `discriminant == 0`** — NOT the Null variant, and NOT E1's
    `OpRefIsNull` (that is the VARIABLE store-sentinel; an inline field has no DbRef slot).
- **The hook (chose (a), DONE for the literal form):** the existing machinery already
  propagates the field's expected enum into `parent_tp` (`parse_single`, `vectors.rs:408-413`,
  via `enum_context`), so the redirect lives in `parse_var` (`objects.rs`, just before
  `parse_constant_value`): when `parent_tp == Enum(__nullable<S>, true)` and the literal is
  `S{…}`, call `parse_object(variant_of(syn,"Some"), code)` — building the `Some` variant into
  the primed field target. **Verified (gate on, interpreter):** `Box { item: Row{id:9,
  tag:"x"} }` round-trips — `b.item.id`/`b.item.tag` read correctly AND access fell out for
  free (the enum-field `.id` resolves to the `Some` payload), and present `b.item == null` is
  `false`. The redirect is INERT with the gate off (no `__nullable<>` enums exist → the name
  check never matches), so the default-off suite stays byte-identical.

**E2a.4 DONE (`==null` + default-init, BOTH backends).** Two fixes, both gated/inert off:
- **`== null` via the discriminant** (`operators.rs` `enum_null` branch): for a synthetic
  `__nullable<S>` enum, lower `== null` to `OpEqInt(OpConvIntFromEnum(OpGetEnum(e,0)), 0)` — read
  discriminant 0 directly, NO deref. (A user enum VARIABLE keeps `OpRefIsNull`, E1 — the branch
  forks on the `__nullable<` name.)
- **Default-init skip** (`object_init`, `objects.rs`): an omitted nullable-struct field is left
  at its `OpDatabase`/`set_default_value` zero-init (discriminant 0 = null). The generic
  `Reference`-recursion / `to_default` paths corrupted the inline enum bytes → the
  `allocation.rs:560` crash on `Box{}` AND a slot mismatch on `x = d.item == null`; skipping
  fixes both.
- *Verified (gate on, BOTH backends):* `present → isnull=false`, `default → isnull=true`,
  `b.item.id`/`.tag` read correctly, assignment form clean. The native `E0308` from the
  earlier present-literal run is also resolved by these fixes.

**STILL OPEN:**
1. **Runtime null SOURCE rejected** at `convert` (`mod.rs:1177` → `handle_field` diagnostic,
   `objects.rs:2368`): `Box { item: maybe(false) }` → "Cannot assign ref(Row) to ref(
   __nullable<Row>)". Needs a `Reference(S)` → `Enum(__nullable<S>)` convert: present source →
   copy into `Some`; **null source → discriminant 0 + free deps, NOT `OpCopyRecord`** (THE
   crash retirement, the original E2 motivation). Verify on both backends.
2. **Vector elements (E2a.5)** — `vector<Row>` element null/`==null`/iterate via the same
   synthetic enum.
3. **Gate removal + .loft regressions** — graduate the gated probes to
   `tests/scripts/25-nullable-sequences.loft` (which runs both backends without the env flag)
   in the final green-without-flag commit; delete `LOFT_E2_SYNTH` + the non-stdlib restriction.

Once 1–2 land, the gate comes off and the suite stays green WITHOUT the env flag (the vertical
slice).

### The corrected first move (this session changed the order)
The staged list above puts synthesis (E2a.1) first, but the **load-bearing unknown is
construct/access glue** (E2a.3/E2a.4), and the protocol says probe that BEFORE building. So
**Step 0 is a probe, not code:**

> **Step 0 — de-risk with a HAND-WRITTEN enum field.** Write `struct Box { item: NRow }`
> where `NRow` is a real `enum { RNil, RSome { id: integer, tag: text } }`, then try
> `b = Box { item: RSome { id:5, tag:"a" } }`, `b.item == null`, and reading the payload
> (probably via `match b.item { … }`). This answers the whole gamble: *does a struct field
> that holds an enum already construct/access/`==null` correctly on both backends?*
> - If YES → synthesis (a) is sound; the remaining work is the **transparency glue** that
>   makes the user's `item: Row` / `Row{…}` / `b.item.id` map onto the synthesized enum.
> - If NO (enum-typed fields are themselves broken) → STOP and re-scope; (a) is not "reuse",
>   it's "build", and the bitmap or a different design may win after all.

### The transparency question (the real design crux for E2)
The design wants `item: Row` to *transparently* be nullable (stored as the enum) — so the
user keeps writing `Row{…}` and `b.item.id`, never seeing `Some`. That requires coercion
glue at three sites, and whether it's "reuse" or "new code" is the open risk:
1. **Construct:** a `Row{…}` literal assigned to an enum-typed field must build the `Some`
   variant (set discriminant=present, then the fields).
2. **Access:** `b.item.id` on an enum-typed field must read the field at its
   post-discriminant offset (the `Some` payload), not at the struct's offset.
3. **null/default:** `= null`→`Null` variant, `== null`→discriminant test, default→`Null`.
Decide explicitly whether transparency is worth the glue, or whether nullable struct fields
should be *visibly* enum-shaped (less magic, less glue). Resolve this BEFORE E2a.2.

### E2a.1 synthesis recipe (concrete — pure execution once Step 0 passes)
`nullable_enum_for(&mut self, lexer, struct_d) -> u32` in `data.rs`, modeled on `tuple_def`
(`data.rs:3530`) + `parse_enum_values` (`definitions.rs:199`):
1. Memoize on `format!("__nullable<{}>", name)`; register at `STD_SOURCE` like `tuple_def`.
2. `e = add_def(name, pos, DefType::Enum)`; `set_returned(e, Enum(e, true, none))`.
3. `Null` variant: `nv = add_def(_, pos, DefType::EnumValue)`, `definitions[nv].parent = e`;
   on `e` add a `constant` attribute `"Null"` typed `Enum(e,true)` with value `Enum(1,MAX)`;
   on `nv` add the discriminant attribute `"enum"` typed `Enum(def_nr("enumerate"),false)`
   with value `Enum(1,MAX)`; `set_returned(nv, Enum(e,true))`. (No payload.)
4. `Some` variant: same shape with value `Enum(2,MAX)`, PLUS copy every attribute of
   `struct_d` into `sv` via `add_attribute(lexer, sv, attr.name, attr.typedef)`.
5. Layout is computed by the existing typedef pass via `calculate_positions(fields,
   sub=true)` (`calc.rs:18`) — `sub=true` reserves the discriminant at offset 0 and gap-packs
   the rest (→ `disc@0, tag@4, id@8`, byte-identical to a hand enum). So synthesize during
   the **2nd parse pass** (where the lexer + resolved `struct_d` exist), like `tuple_def`, and
   let the pass lay it out — do NOT hand-roll layout.

### Why E2a.1 can't be a tiny isolated commit (so plan the vertical slice)
The synthesis helper is dormant until wired, and **wiring breaks construct/access for every
existing nullable-struct-field program** until the glue (E2a.3/4) lands — the suite goes red
in between. So the **first GREEN commit is a vertical slice**: synth + wire + construct +
access + null, for ONE simple field, gated end-to-end on both backends. Don't try to land
E2a.1 alone.

### Key entry points (verified this session)
- Synthesis: `data.rs` `tuple_def:3530` (template), `add_def:3035`, `add_attribute:2979`
  (only touches the lexer on error — safe to pass the parser's), `set_returned:3166`,
  `variant_of:4136`, `def(d).parent():2410`.
- Enum construction recipe: `parser/definitions.rs` `parse_enum:367`, `parse_enum_values:199`.
- Layout: `calc.rs` `calculate_positions:18` (`sub=true` = the discriminant-reserving path).
- Field nullability hook (for E2a.2 wiring): `typedef.rs` (`attr_nullable && !not_null`,
  ~`:378-420` field-type/offset region).
- Construct/access/null sites to glue (E2a.3/4): the struct-literal `Object` build and
  `OpGetField` lowering in `parser/objects.rs` + `parser/fields.rs`; `== null` dispatch in
  `parser/operators.rs` (the `enum_null` branch E1 added is the model, but inline fields use
  the **discriminant**, not the `store_nr` sentinel — see below).

### Crucial distinction carried from E1 (do NOT conflate)
A nullable enum/struct **VARIABLE** is a `DbRef` slot → null = `store_nr==u16::MAX` sentinel
(E1 fixed this with `OpRefIsNull`). A nullable **INLINE** field/element has no DbRef slot →
null = **discriminant 0** (E2). These are two different null encodings on the two storage
forms; E2's `== null` for an inline field must read the discriminant, NOT call `OpRefIsNull`.

## Open questions

- **Discriminant size** — enums use a byte (small) or short at offset 0; confirm via Probe
  1 which a nullable struct should take, and whether a 1-byte discriminant suffices for the
  2-state (null / present) case while staying compatible with multi-variant enums.
- **Default cost vs. consistency** — default-nullable adds a discriminant to existing
  `vector<Struct>` / embedded fields tree-wide. Consistent with the model and `not null`
  recovers the fast path, but it IS a layout change; confirm via Probe 2 + the corpus that
  cost is paid only where nullability is declared, and decide on a migration note.
- **`u16::MAX` (container null) vs. discriminant-0 (element/inline null)** stay distinct —
  both are this plan's H6 surface; keep them as the two agreed axes, not a third sentinel.

## Rejected alternative — out-of-band validity bitmap

A prior draft put a validity bit per slot in the container (a side record beside the vector
data, or a hidden bitmap word in the parent struct). Rejected because (a) loft's slices
materialize element-by-element and its vectors copy wholesale, so an out-of-band bitmap pays
bit-slice + two-buffer-sync cost on every slice/copy/append/remove — the in-band
discriminant pays none; and (b) it would be all-new machinery, where the discriminant
**reuses the enum representation loft already ships** and fixes nullable enums in the same
stroke. The bitmap's lone advantage (1 bit vs ~1 byte) does not pay for that.

## See also
- [README.md](README.md) findings 8–12 (the matrix that surfaced this), § The invariant.
- `src/database/structures.rs` `set_default_value` (enum discriminant at offset 0, `v =
  f_nr + 1` ⇒ `0` = null; the Vector + Struct arms gain the discriminant).
- `src/state/io.rs` `do_copy_record` / `copy_ref_or_null` (the null-source path E1 retires).
- `src/vector.rs` (the 32 `8 + i*size` element-base sites — the discriminant rides *inside*
  the element, so these stay untouched).
- `src/data.rs` `Type::Enum(_, true, _)` (the nullable-enum type this representation unifies
  with); `IntegerSpec::byte_width(nullable)` (the scalar-sentinel precedent).
