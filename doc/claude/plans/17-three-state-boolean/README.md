<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 17 — Three-state boolean (true / false / null)

## Status

**SHIPPED on the `booleans` branch (2026-06-10) — design A, both backends.**  Tri-state
boolean (false=0 / true=1 / null=255) is implemented and verified: the interpreter and
`--native` produce byte-identical results across the `{false,true,null}` × `{==false,
==true, ==null, if, !, &&, ||, fmt, hash→bool, tuple, vector, param, return}` matrix, and
the full suite is **2156/2157** (the lone failure, `markdown_renderer`, is a pre-existing
sandbox `cc`-link env limitation — the interpreted viewer can't build its cdylibs here;
markdown renders correctly on `--native` and `starts_with` works on `--interpret`).
History (Stage A characterization, the decision-A reversal, the B/C interpreter spike)
is in [SPIKE.md](SPIKE.md).  The spike
([SPIKE.md](SPIKE.md)) confirmed the invariant holds under construction (the
`hash → boolean` consumer is unblocked) with a bounded 7-op change, and surfaced the
gating risk: **native lowers boolean to Rust `bool`, so tri-state is a native
*representation* change (sub-arc E, resized to M), not a marshal tweak.**  Spike code
reverted to keep the branch green; diff preserved as [`spike.diff`](spike.diff).
**Semantics resolved — decision (B), 2026-06-09:** `==`/`!=` coerce (`null → false`);
`b == null` is the sole null test.  Native design ✓ — the `u8`(storage)/`bool`(expr)
two-form split mirrors the existing `text` String/Str `Context` split (see § Native
backend design).
`boolean` is today the **only** common-value scalar whose zero-value collides with
its null sentinel (the null sentinel for `boolean` *is* `false`).  Every other
scalar — `integer`, `float`, `text`, plain `enum` — distinguishes "zero value" from
"absent".  Probes confirmed boolean is 2-state **at the type level**, not merely in
storage: a deliberate **#256 guard cluster** *rejects* `null` on boolean (`null`
literal, `return null`, `??`, `== null` all error).  This plan replaces those
rejections with a real representation: three-state in data (false / true / null)
unless a field is `not null`, with `null` collapsing to `false` at the single
boolean-logic chokepoint.  Tracked as [@PLN17](https://github.com/loft-lang/plans/issues/17).

## Goal

A nullable `boolean` distinguishes `null` from `false` everywhere it is stored,
copied, or compared, and collapses to `false` only when consumed as a truth value —
so a `hash`/`index` map to `boolean` can express absent vs false vs true.

## Effort + design

- **Effort:** M — touches the null model; the risk is *coverage of the coercion
  sites* and the native-marshalling boundary, not representation room.
- **Design:** ~ (partial) — the invariant is clear and three claims are already
  confirmed by code-read; the remaining load-bearing claims need falsification
  probes (Stage A) before any code.
- **Last touched:** 2026-06-09

## The invariant (Design Protocol 1, step 1)

> A `boolean` value that is not `not null` has **three runtime states** —
> `false` = byte `0`, `true` = byte `1`, `null` = byte `255` — reusing the
> byte-storage sentinel scheme plain enums and narrow ints **already** use.  The
> third state is **preserved** by storage, assignment, copy, and field/param/return
> passing; the **sole way to observe it is `== null`**.  It **collapses to `false`**
> the instant it enters boolean *logic* — `if` / `while` / `assert` / `!` / `&&` /
> `||` **and** `==` / `!=` (decision B): all coerce `null → false`, so
> `null == false` is `true`, `null == true` is `false`, and only `b == null`
> tests absence (exactly how `null` is tested on integer / float / text).

Why this is a *consistency fix*, not a new feature: a `boolean` is stored in one
byte (`data.rs:1055` — `Type::Boolean | Type::Enum(_, false, _) => 1`), the same
storage class as a plain enum, and byte `255` is *already* the universal null
sentinel for that family (`store.rs:1756`, `fill.rs:1221`).  The third state
physically exists and is reserved; boolean is the lone type whose read/compare path
flattens the byte to a 2-state Rust `bool`.

## Re-assertion sites — the prospective tell (Design Protocol 1, step 2)

The design is correct **iff** `null → false` is enforced at *one* chokepoint, not
re-stated per context.  Every "forced context" must route a value through the same
truthiness coercion:

`if` · `while` · `assert` · `&&` · `||` · `!` · for-`if` filter · match guard ·
ternary-style `if` expression.

If each compiles its own "is-this-true" test, that is **N silent re-assertion
sites = the brittleness, known now**.  The cure is to confirm (or build) a single
coercion every site emits.  Early read: `if` lowers to `OpGotoFalseWord`
(`codegen.rs:737`) whose impl `goto_false` reads the byte as `bool` (`fill.rs:302`,
`!= 0`) — so **255 currently reads as `true`**.  The coercion belongs at this op and
its peers; Stage A must enumerate every consumer of a boolean-as-truth-value and
prove they reduce to one site (or a small, named set), **no narrower, no wider**.

## Composition matrix — Stage A (REQUIRED, before any code)

Write these as `/tmp` probes on `--interpret` first; the feature is done only when
every cell is green on **both** backends and the probes graduate to
`tests/scripts/`.  Axes: **value** `{false, true, null}` × **context**.

| Context | false | true | null | Expected after fix |
|---|---|---|---|---|
| `if x` / `while x` / `assert(x)` | skip | run | **skip** | null coerces to false |
| `!x` | true | false | **true** | `!null` = `!false` = true |
| `x && y`, `x \|\| y` | logic | logic | **false-coerce** | coerce-at-context, not Kleene |
| `x == false` | true | false | **true** | **B**: `==` coerces null→false |
| `x == true` | false | true | **false** | **B**: coerce |
| `x == null` | false | false | **true** | **B**: the sole null test (`==255`) |
| `x == y` (bool == bool) | eq | eq | coerce | **B**: coerce both → bool, then compare (null==null → true) |
| stored field read (nullable) | false | true | **null** | round-trips 255 |
| stored field read (`not null`) | false | true | n/a | unchanged — 2-state |
| nullable field **default-init (omitted)** | false | true | **false** | UNCHANGED — omitted fields default to the zero value for *every* type (int→0, text→"", bool→false); not null.  Verified consistent across types |
| **explicit** null in a field (`S{b:null}` / `s.b=null`) | false | true | **false (gap)** | should be null — see Open follow-up below |
| `hash`/`index`/`sorted` map → bool | false | true | **null** | the real-consumer trigger |
| `{x}` format / `{x:…}` | "false" | "true" | **?** | decide null rendering (F) |
| native: var / field / param / return | u8 | u8 | **u8 (255)** | **E**: storage form = `u8`; expr form = `bool` |
| native: `if` / `!` / `&&` / `==` operand | coerce | coerce | coerce | **E**: var read coerces `u8→bool` (`==1`) in logical/compare ctx |
| `vector<boolean>` element | false | true | **null** | byte-packed element round-trip |
| closure capture of nullable bool | false | true | **null** | capture preserves third state |

Extract the **real-consumer probe** verbatim from the `hash → boolean` shape the
agent's confusion pass hit (issue body) — real extraction catches classes the
synthetic cells miss.

## Stage A findings — current behaviour (measured 2026-06-09, `--interpret`)

Probes in `/tmp/claude/bprobes/`.  The headline: boolean is already 2-state **at the
type level**, enforced by a deliberate guard cluster — so the plan mostly *adds*
capability rather than changing behaviour of currently-valid programs (small blast
radius).

**Calibration — `integer` is the reference model (everything boolean lacks):**
`fn -> integer { null }` compiles; `n == null` → `true`; `n == 0` → `false`
(distinguishable); `if n`/`!n` treat null as falsy.  Boolean is *uniquely*
restricted.

**The #256 guard cluster — boolean rejects null at parse/type time (backend-independent):**

| Form | Result today | Site |
|---|---|---|
| `fn f() -> boolean { null }` / `return null` | **error** "Cannot use null with boolean — boolean has no null representation" | `parser/mod.rs:6127` |
| `b ?? x` (boolean LHS) | **error** "Cannot use null coalescing '??' on boolean …" | `parser/operators.rs:1237` |
| `b == null` | **error** "No matching operator '==' on 'boolean' and 'null'" | operator resolution |

These are the single home to flip: #256 chose to *reject* null-on-boolean (make the
collision loud); this plan supersedes that by giving boolean a real null so all three
forms *work*.

**Runtime cells — where null silently becomes false:**

- Unset **nullable** field → reads `false` (`if`→else, `!nf`→true, `==false`→true,
  `{nf}`→`"false"`).  Indistinguishable from explicit `false`.
- **`not null`** unset field → also `false`.  → **No observable difference between
  `boolean` and `boolean not null` today** — the plan introduces it.
- `&&` / `||` with an unset(=false) operand behave exactly as `false`.
- **Null record-ref projection** (`fc.on` where `fc = h["absent"]`) does **not**
  halt — it silently returns `false` and continues.
- The **real consumer is uncompilable today**: a `fn get(h, k) -> boolean { …; if !f
  { return null } f.on }` accessor fails on the #256 `return null` guard — exactly
  the `hash → boolean` blockage that motivated this plan.

**Implications for the design:**

1. The truthiness chokepoint fix is real and needed: `goto_false` reads the byte as
   `!= 0` (`fill.rs:302`), so a `255` sentinel would read **true** — must coerce.
2. `== null` / `??` / `null`-literal on boolean are **net-new surface** to add (not
   changes to existing valid code), since they don't compile today.
3. Backward-compat scan (sub-arc G) is *narrow*: programs relying on `bool == null`
   or `return null`-bool don't exist (they never compiled).  The only behavioural
   flip is unset-nullable-field default `false → null` and `== false` no longer
   matching a now-null field.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — Stage A matrix | probes for every cell above; record current vs expected | **Done** 2026-06-09 (see findings §) |
| **B** — representation | nullable bool round-trips `255`; `OpConvBoolFromNull` producer | **DONE (interp)** 2026-06-09 — design A, full matrix green; [SPIKE.md](SPIKE.md) |
| **C** — truthiness chokepoint | `null → false` via `@v != 1`; generator reads bool operands as `u8` (UB fix) | **DONE (interp)** 2026-06-09 |
| **G256** — retire the #256 guard cluster | replace the three null-on-boolean *rejections* (`mod.rs:6127`, `operators.rs:1237`, `==`-resolution) with real support | `null`-literal flipped in spike; `??` + `== null` open |
| **D** — `== null` + `??` + coerce-`==` | add boolean `== null` (`==255`) + `??`; flip `eq_bool`/`ne_bool` to coerce-compare (decision B) on both backends | Open |
| **E** — native u8/bool split | rust_type two-form split + `narrow_int_cast` + operand-wrap + predicate-coerce + if-arm `bool_unify` + FFI/runtime-helper (`n_assert`/`n_set_store_lock`/`n_json_bool`/extern-decl/direct-call) + tuple-element coercions | **DONE** 2026-06-10 — suite 2156/2157 |
| **F** — format rendering | decide + implement `{nullable_bool}` output (today renders `"false"`) | Open |
| **G** — backward-compat scan | NARROW (per findings): unset-nullable default `false→null` + `== false` on now-null fields; scan + document | Open |
| **H** — docs + graduate | LOFT.md null table; graduate probes to `tests/scripts/`; record `&&`/`!` + #256-supersession in `DESIGN_DECISIONS.md` | Open — last |

## Phase ordering

1. **A — done.**  Matrix measured (findings §); decision B set the expected column.
2. **E with B/C** — native is the gating change and CANNOT lag the templates (shared
   templates assume the operand type).  Land the `u8`/`bool` two-form split (E) in the
   same change as the interpreter representation (B) + truthiness coercion (C); the
   spike proved the interpreter half, E makes native match.
3. **D + G256** — flip `eq_bool`/`ne_bool` to coerce-compare (both backends); add
   boolean `== null` + `??`; retire the remaining #256 rejections.
4. **F + G** — null rendering + the (narrow) compat scan; G gates the release call.
5. **H** — docs, regression graduation, decision record (B + #256 supersession).

## Open design questions

1. **`&&` / `||` / `!` — coerce vs Kleene.**  **RESOLVED — coerce-at-context**
   (`null → false`): simpler, matches "forced context", keeps `if x`/`if !x`
   backward-compatible.  Kleene (null propagates as "unknown") declined.
2. **`== false` distinguishing null — RESOLVED: decision (B), 2026-06-09.**  `==` /
   `!=` **coerce** (`null → false`), so `null == false` is `true` and `null == null`
   is `true`; the **sole** null test is `b == null` (`== 255`).  Chosen over (A)
   raw-byte-distinguish because it is consistent with how `null` is tested on every
   other type (`n == null`, never `n == 0`), keeps native's "expressions are `bool`"
   rule carve-out-free, and matches "null collapses to false the instant it enters
   boolean logic."  Consequence: the spike's raw-byte `eq_bool` flips to
   coerce-compare on **both** backends (parity), and a boolean `== null` op is added
   (G256 / D).  Graduate to `DESIGN_DECISIONS.md` + `LOFT.md` at sub-arc H.
3. **Default-init flip under feature-freeze.**  Nullable bool field default flips
   `false → null`.  `not null` fields are unaffected (the escape hatch).  G's scan
   sizes the blast radius; the truthy idiom (`if field`) is preserved regardless.
4. **`{nullable_bool}` rendering.**  Likely `"null"` (mirroring other nullable
   scalars) — confirm against existing `{nullable_int}` behaviour in A.
5. **Stack width.**  A local assigned from a nullable source (`b = h[k].flag`) must
   carry the third state — confirm the stack slot round-trips `255` (C6).

## Native backend design — sub-arc E (decision B, design ✓ 2026-06-09)

The spike proved the interpreter side and surfaced E as the gating risk: the
`#rust` templates are shared, but native lowers boolean to a Rust `bool` (no room
for `255`).  The fix is **not** "everything u8" (the spike's over-broad approach —
56 errors); it is the **two-form** model `text` already uses, with `null` only ever
living in the storage form:

- **Storage form = `u8`** (0/1/255): locals, struct fields, vector elements,
  **function params, function returns** — everything that persists or crosses a call
  boundary and can therefore be `null`.
- **Expression form = `bool`** (2-state, never `null`): the transient result of any
  operation (`==`, `!`, `&&`, comparisons, literals).  Left untouched — this is why
  the 56 expression sites stay valid.

**Implementation seam** — `rust_type` *already* splits a type by `Context`
(`data.rs`: `Type::Text(_) if context == Context::Variable => "String"` else
`"Str"`).  Boolean rides the same fork:

```rust
Type::Boolean if context == &Context::Variable => "u8",  // storage form
Type::Boolean => "bool",                                 // expression form
```

**Coercion points** (insert at the use site, like the text String↔Str wraps in
`src/generation/emit.rs`):

| Site | Coercion | Code |
|---|---|---|
| var read → logical/compare/`if` ctx | `u8 → bool` (lossy, intended) | `(var == 1)` |
| `b == null` (the null test) | u8, raw | `(var == 255)` |
| bool expr → store / param / return / var-copy | `bool → u8` | `(expr as u8)` |
| `if` / `while` predicate (`output_test_predicate`, `emit.rs:1010`) | `u8 → bool` | `cond == 1` |
| param decl (`mod.rs:802`) / return type (`mod.rs:817`) | type = `u8` | — |
| `OpConvBoolFromNull` | producer | `255u8` |

**Parity note (decision B):** `==` / `!=` coerce on *both* backends — so the
interpreter's spike `eq_bool` (raw-byte compare) flips to coerce-compare
(`(v1 == 1) == (v2 == 1)`), and native compares the two coerced `bool`s.  `null ==
false` → `true`; `b == null` is the only distinguishing test.  The differential
sweep (Goal D) must assert interp ≡ native on the full `{false,true,null}` × op
matrix — the backends agreeing on the boolean's *value semantics* despite differing
on its *Rust type* is the property E must hold.

## Over-unification guard (Design Protocol 1, step 4)

The cleanest claim — *"boolean becomes exactly a 2-variant plain enum, so it's all
free"* — is the one to attack.  Enums have **no** `&&` / `||` / `!`, no native
`bool` marshalling, no `{b}` truthy formatting, and are not the canonical `if`
subject.  Each of those is a site the enum analogy does **not** cover; the matrix
(B/C/E/F rows) is exactly the set of operations boolean has that enums don't, and
the build is what proves the chokepoint actually covers them with one mechanism.

## Cross-arc dependencies

- **`plans/1-integer-width-discipline/`** — sibling null-model / S-tier plan; same
  "make a scalar's null discipline consistent" flavor.  No code dependency; shared
  reviewer context.
- **`DESIGN_DECISIONS.md` § C69** (`!x` is a null test on non-booleans) — adjacent,
  **no conflict**: this plan touches boolean `!`; C69 governs non-boolean `!`.
  H must record the boolean-`!` semantics so the two read as one coherent story.
- The S-tier **collection-validation** plan (`plans/future/20`) overlaps the
  `hash/index/sorted → bool` matrix rows — coordinate the keyed-collection cells.

## Open follow-up — null *stored in* a boolean field (not the unset default)

Investigated 2026-06-10.  Two things were conflated under "unset nullable field
default"; the investigation separated them:

- **Unset/omitted fields default to the zero value for EVERY type** (int→0, text→"",
  float→0.0, bool→false) — verified on both backends.  Boolean is already consistent;
  there is nothing to fix here.
- **A boolean field cannot *hold* null** even when set explicitly: `S { b: null }` and
  `s.b = null` collapse to `false`, whereas an integer field holds null (`i: null` →
  `i == null` is true).  This IS a real inconsistency.

**Cause (localized):** the boolean field-access codegen forces 0/1 — write emits
`OpSetByte(rec, fld, if val { 1 } else { 0 })`, read emits `OpEqInt(OpGetByte(…), 1)`
(introspect on `S{b:null}`).  The field byte is full-width (each boolean field owns its
own byte, `BOOL_MASK=1`, `sizeof` counts 1/field), so there is physical room for the
`255` sentinel; the codegen wrapper is what collapses it.

**Why it's a scoped sub-arc, not a quick fix:** making boolean field bytes `0/1/255`
changes the *stored* representation, which entangles serialization — JSON
(`to_json`/`parse` of a null-bool field), binary I/O (`f += bool`), and snapshots — and
the field-value codegen is a structured `FieldValue`/`FvBool` path on both backends.  It
belongs with the **binary-I/O validation matrix** ([plans/future/43](../future/43-binary-io-validation/README.md),
which already lists boolean).  The motivating `hash → boolean` consumer does **not**
need it (its null comes from the absent-case `return null`, a local/return value that
already works).  Route here; do not force under the current change.

## See also

- `doc/claude/LOFT.md` § Null representation — the sentinel table this plan changes.
- `doc/claude/DESIGN_PROTOCOL.md` — the design discipline this README applies.
- `doc/claude/plans/README.md` § The composition axes — what the Stage A matrix varies.
- [@PLN17](https://github.com/loft-lang/plans/issues/17) — the tracker issue (identity).
