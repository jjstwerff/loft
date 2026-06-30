<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 DN1 `_`-arm audit — findings

The DN1 default-flip phase makes `integer?` (and `text?`/`bool?`/…) first-class and
pervasive, so an `Optional(τ)` `Type` can flow anywhere a scalar type flows. Any
**non-exhaustive** `match`-on-`Type` with a scalar arm + `_` fallthrough that does NOT
peel (`tp.base()` / `tp.peel_optional()`) and has no `Type::Optional` arm will silently
mishandle `Optional(τ)` in its `_` arm. (Exhaustive matches were force-handled by Step 1.)
This audit finds those sites. Two complementary halves:

- **Empirical** — drive `τ?` through every feature gate-ON, let the compiler surface
  reachable mishandles (`optional-flow-instrument.loft`). HIGH confidence, exercised paths.
- **Static** — enumerate + classify every `_`-arm `Type::Integer` match across `src/`.
  COMPLETE coverage, reachability is a judgment.

## Method note (reproduce)

Gate with `LOFT_PLN25_OPT=1` (Optional is constructed, the `(N-Store)` teeth are OFF so
Optional propagates through codegen instead of being rejected). `LOFT_PLN25_DN3=1` would
reject un-discharged Optional at the store sites and mask the flow.

---

## Empirical findings (both backends)

### CONFIRMED NEEDS-FIX

**E1 — native: a `null` tuple-element typed `τ?` is codegen'd as `()` not the sentinel.**
`fn pair() -> (integer?, integer) { (null, 2) }` (and `(integer, integer?) { (1, null) }`)
runs correct on `--interpret` (`-1 2`) but `--native` emits invalid Rust and fails to
compile (E0308):
```
{{let db = (var___ref_1); let v = (()); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i64) as u32, v);}};
                                  ^^^^ expected i64, found ()
```
The tuple-element store path emits the `null` literal as `()` (unit) instead of the base
type's i64 null sentinel — it does not peel `Optional(Integer)` to find the sentinel. A
non-null tuple element (`(5, 1)`) and a plain tuple (`(1, 2)`) are both fine, so the trigger
is specifically `Value::Null` into an `Optional` tuple-element slot in NATIVE codegen.
Repro: `dn1-audit/tuple-null-native-bug.loft`. Site: native tuple-construction /
typed-null path in `src/generation/` (does not peel Optional for the per-element null).

### LOUD ERROR — design/ergonomics, NOT silent corruption

**E2 — both backends: direct `"{x}"` interpolation of `x: τ?` is rejected.**
`fn f(x: integer?) -> text { "{x}" }` → `error: Cannot format type integer?` on BOTH
backends. The format type-check rejects `Optional`; the user must discharge (`x ?? d`) to
format. Consistent across backends, loud (not a corruption). Decision for DN1: either keep
the loud error (discharge-to-format) or peel-and-format-the-value/null. Repro:
`dn1-audit/format-nullable-rejected.loft`.

### CONFIRMED WORKING on BOTH backends (slice (b) coverage validated)

Captured as the regression instrument `dn1-audit/optional-flow-instrument.loft`: struct
field (incl. narrow `u8?`/`i32?`), parameter, return (implicit tail / explicit / if-else),
arithmetic with AND without discharge, `== null` compare, `??` discharge, `as τ?` cast,
method (`t_`) nullable return, `vector<integer?>` element, `[for …]` map producing
optionals, nested optional struct fields, enum optional payload field, direct
field-assign-`null`, explicit `return null`. All green — the common paths peel correctly.

---

## Static findings (the `_`-arm enumeration)

> Filled from the 5 subsystem audits (native codegen / interp codegen+exec / parser-core /
> parser-collections / data+types+IR). Each site: `file:line | fn | class | reason | fix`.

### ROOT CAUSE (highest leverage) — `src/data.rs` `type_elm` lacks an Optional arm

`type_elm` (≈`data.rs:4752`) returns `u32::MAX` for `Optional(τ)` via its `_` arm, whereas
its sibling `type_def_nr` (≈4711) already peels Optional. That single upstream gap is what
turns the downstream `vector<integer?>` element-size / db-type-id sites into real layout
corruption (`collections.rs:3964`, `fields.rs:795/807`, `vectors.rs:2724`). **Fix `type_elm`
to peel (mirror `type_def_nr`) and several downstream NEEDS-FIX rows are neutralised at the
producer.** Start here (see memory: start-at-the-producer-of-the-wrong-fact). The parser-local
peels are still needed for the rows that don't route through `type_elm`.

### Group 4 — parser collections / objects (agent: 8 NEEDS-FIX, 1 SAFE-UNREACHABLE, 2 SAFE-BENIGN)

```
vectors.rs:2724     | get_type          | NEEDS-FIX | Optional→`_ => u16::MAX` not the db-type id → OOB panic / mis-strided vector store | match in_t.base() or add Optional arm
vectors.rs:2893     | cell_struct_name  | NEEDS-FIX | Optional→`_ => None`: mutated `integer?` capture gets no __cell box → mutation may not propagate | peel before matching
vectors.rs:2930     | cell_value_type   | NEEDS-FIX | `other => other.clone()` keeps Optional as cell value type → non-canonical, flows into get_type crash | peel so value field is canonical I32/I64
objects.rs:840      | ensure_io_type    | NEEDS-FIX | `_ => {}` skips byte/short db registration for `integer?` file write (companion if-let at 897 too) | peel at entry `t = t.base()`
collections.rs:115  | narrow_route_for  | NEEDS-FIX | `_ => None` routes `integer?`/narrow par return through wide u64 queue → width/stride mismatch | peel ret_type at entry
collections.rs:1219 | append_data       | NEEDS-FIX | `_` arm = the E2 "Cannot format type integer?" error; base is formattable | peel + dispatch on base
collections.rs:3862 | (FieldValue refl) | NEEDS-FIX | `_ => continue` silently OMITS nullable-scalar struct fields from reflection | peel `attr_type.base()` → FvInt/FvLong
collections.rs:3964 | element_store_size| NEEDS-FIX | `_ => 12` (DbRef) for `vector<integer?>` element not 8 → stride corruption (downstream of type_elm) | peel elm at entry
objects.rs:553      | (cell OpGet sel)  | SAFE-UNREACHABLE | value_tp is a __cell value field, never Optional — IFF cell_struct_name/cell_value_type stay peeled | none (depends on the two cell fixes)
builtins.rs:93      | (par form 2)      | SAFE-BENIGN | no scalar arm; Optional rejected identically to plain Integer | none
builtins.rs:281     | (nullable-par)    | SAFE-BENIGN | detects the OLD `__nullable<S>` enum, not the sentinel scalar; Optional needs no wrapper | none
```
fields.rs: ZERO formal rows; but `fields.rs:795`+if-let@807 mis-size `vector<integer?>`
elements via the same `type_elm` gap (cross-cutting, fixed at the root).
Companion `matches!`/`if let` sites (same `.base()` fix, not counted): `vectors.rs:3039`,
`vectors.rs:2480`, `fields.rs:807`, `objects.rs:897/922`.

### ⚠️ Reconciliation discipline (static NEEDS-FIX vs empirical reachability)

The agents were told to mark NEEDS-FIX when reachability is uncertain (conservative). Before
fixing any row, cross-check it against the empirical instrument — some flagged sites are not
reached in practice:
- **`collections.rs:3964` / `fields.rs:795` (`vector<integer?>` element size)** is flagged
  stride-corruption, yet `optional-flow-instrument.loft` runs `vector<integer?> = [1,null,3]`
  and `[for …]`-mapped optionals GREEN on both backends. The dense-vectors half (`vector<S?>`,
  already on `main`) very likely routes `vector<τ?>` around the scalar-Optional element path,
  so these may be SAFE-in-practice (or reachable only by an unusual route). **Resolve each by
  instrumenting the actual path before editing — verify the site is reached, don't patch on the
  static flag alone.** `type_elm` itself is still worth peeling (cheap, mirrors `type_def_nr`,
  removes the latent producer), but its *downstream* rows need the reachability check.
- Confirmed-reachable empirically: **E1 native tuple-null** (a real bug) and **E2 append_data
  format** (= `collections.rs:1219`, loud error both backends).

### SECOND RISK CLASS — the `matches!(Type, Type::Text|Integer|…)` predicate family

Beyond `match`-blocks, a large family of `matches!(<Type>, Type::Text(_) | Type::Integer(_) |
…)` predicates returns `false` for any `Optional`, mis-routing a nullable scalar onto the
wrong (heap/store) path. Slice (b) peeled the load-bearing signature ones (`mod.rs:645/668`).
Highest-risk remaining (native): `coroutine.rs:251` `suitable` (Optional persistent local
dropped → loses value across yields — silent), `dispatch.rs:579` + `mod.rs:3003` `is_scalar`
(Optional treated non-scalar → leak/E0425), `dispatch.rs:158` + `emit.rs:215`
(`RefVar(Optional(scalar))` misses `*mut T`), and the ~40 `matches!(returned(), Type::Text(_))`
ABI gates in `emit.rs`/`calls.rs` (Optional(Text) skips String-wrap/ptr-len → E0308). **The
uniform fix is the same `.base()` peel; this family needs its own sweep alongside the
match-block rows.** (The interp/parser agents were scoped to `match` blocks too — expect a
parallel `matches!` family in those subsystems.)

### Group 1 — native codegen (agent: 14 NEEDS-FIX, 2 SAFE-UNREACHABLE, 5 SAFE-BENIGN)

Uniform fix for every row below: match/peel on `.base()`. Most are in paths the empirical
instrument did NOT exercise (wasm/cdylib externs, coroutines, direct `#native` calls) → genuinely latent.
```
mod.rs:579   | narrow_int_cast        | NEEDS-FIX | narrow Optional (byte?/u8?) → `_ => None` → width coercion skipped at return/store/arg/tail seams → E0308/wrong-width
mod.rs:896   | default_native_value   | NEEDS-FIX | Optional(Float)→"0" not "0.0_f64", Optional(Text)→"0", Optional(Bool)→"0" not "255u8" → wrong-typed default → E0308 (tuple arm @928 same) ★ likely E1 root
mod.rs:1213  | emit_file_header (wasm param)  | NEEDS-FIX | Optional extern param ABI: wide-int→i32 truncate, Float→i32, Text→i32 not ptr,len
mod.rs:1240  | emit_file_header (wasm return) | NEEDS-FIX | Optional extern return ABI: wide-int→i32 truncate, Float→i32, Single→i32
mod.rs:1336  | emit_file_header (cdylib param)| NEEDS-FIX | same as 1213 for cdylib extern params
mod.rs:1359  | emit_file_header (cdylib ret)  | NEEDS-FIX | cdylib extern return ABI corruption (wide-int truncate, Float→i32, Text→i32 not LoftStr)
mod.rs:3548  | output_native_direct_call (arg) | NEEDS-FIX | Optional arg drops `as _`/`!= 0` coercion, Text emits 1 arg not ptr,len → ABI/segfault
mod.rs:3742  | vector_elem_rust_type  | NEEDS-FIX | Optional elem → `_ => u8` → vector<integer?> to #native gets *const u8 not *const i64 → stride corruption
coroutine.rs:316  | emit_struct_def (ForLoop __values elem) | NEEDS-FIX | Optional(Text) yield → "i64" not "String" → Vec<i64> stores text → corruption
coroutine.rs:358  | emit_factory_fn (field init)    | NEEDS-FIX | Optional(Text) param omits .to_string() → &str into String field → E0308
coroutine.rs:643  | emit_next_i64 (shadow-bind)     | NEEDS-FIX | Optional(Text) moves String out of &self → E0507
coroutine.rs:1067 | emit_for_body_factory (init)    | NEEDS-FIX | same as 358 → E0308
dispatch.rs:1025  | tuple_has_text_leaf    | NEEDS-FIX | Optional(Text) tuple elem not counted → to_string wrap missing → E0308
dispatch.rs:1043  | tuple_has_non_copy_leaf| NEEDS-FIX | Optional(Text) tuple elem treated Copy → .clone() skipped → E0507 (Optional(Integer) genuinely Copy, benign)
coroutine.rs:294  | emit_struct_def (persistent field) | SAFE-UNREACHABLE | gated out by suitable@251; peels if reached. COUPLED: fix with 251
coroutine.rs:384  | persistent_default     | SAFE-UNREACHABLE | gated by 251; `_ => "0_i64"` would mishandle Optional(Text/Bool/Float) → fix WITH 251
mod.rs:1082/1096  | live_entry_check       | SAFE-BENIGN | Optional → None → live-reload skipped (degradation, no corruption); optional peel to re-enable
mod.rs:3391       | direct_call browser stub ret | SAFE-BENIGN | `_ => Default::default()` infers the peeled return type → correct
coroutine.rs:284/959 | emit_struct_def/for_body param | SAFE-BENIGN | `other => rust_type(...)`/rebind already peels → correct
```
Already-safe reference (explicit Optional arm): `mod.rs:763` rust_type, `emit.rs:1049` write_typed_null.

### Group 2 — interp codegen + exec — _pending_
### ★ GATING FINDING — the `change_var` guard blocks nullable LOCALS (the `(N-Decl)` seam)

`variables/mod.rs:1257` (`change_var`) errors *"Variable 'x' cannot change type from integer?
to integer"* whenever a base-typed value meets an `Optional`-typed slot (or vice versa).
Empirically reconciled (gate-ON, both backends): `x: integer? = 5` (a NON-null literal!),
`y = seed; y = 9`, `&x` on `x: integer?`, and `t: (integer?,integer) = (5,6)` ALL fail with
this one error. So **nullable locals and local tuples are essentially unusable today** — only
nullable PARAMS / FIELDS / RETURNS work (slice (b) peeled those; locals were not).

Consequence for sequencing: **most of the interp-codegen NEEDS-FIX rows are latent-but-
UNREACHABLE** because you cannot construct the nullable local that would reach them
(`set_var:3710`, the 9 tuple panics, the refvar panics). `take(null)` (param) works (`-7`),
`(null,2)` tuple RETURN works on interp — both confirmed. **Fix `change_var` to treat `τ` and
`Optional(τ)` as layout-compatible FIRST (the `(N-Decl)`/`(N-Store)` coercion); only then do the
downstream interp peels become reachable and necessary.** This guard is the true gate of the
local half, parallel to the return-site `(N-Store)` already landed.

### ★ HIGH-CERTAINTY CLASS — sibling-pair misses (slice (b) fixed one twin, missed the other)

The cleanest, highest-confidence fixes — a peeled sibling proves the intended shape; the twin
was simply missed. All layout/crash/leak class:
```
data.rs: size(1648)✓ / align — variables/mod.rs:1753 ✗  → Optional(Int) align 1 not 8 → misaligned i64 slot → UB/SIGSEGV
data.rs: type_def_nr(4711)✓ / type_elm(4752) ✗          → Optional → u32::MAX → data.def(MAX) panic / field skipped
data.rs: element_align(1866)✓ / tuple_def inline-align(3971) ✗ → size 8 / align 1 mismatch → LinkedFieldGroup offset corruption (SIGSEGV class)
generation::rust_type(mod.rs:763)✓ / Data::rust_type(data.rs:4832) ✗ → Optional → panic!("Incorrect type") HARD CRASH (native bridge gen)
```
Fix each by mirroring its peeled twin. These should land first (cheap, certain, highest stakes).

### Group 2 — interp codegen + exec (agent: 13 NEEDS-FIX, 5 SAFE-UNREACHABLE, 9 SAFE-BENIGN)

All 13 NEEDS-FIX in `state/codegen.rs`; uniform fix = `match <type>.base()` (mirror the
already-peeled `generate_var:3158` / `gen_set_first_at_tos:2256`). **Reachability gated by the
`change_var` finding above** — re-verify each is reachable after that lands.
```
codegen.rs:1527 | emit_typed_null    | NEEDS-FIX(reconcile) | Optional → `_ => push 12-byte DbRef` not 8-byte i64::MIN → slot-width/sentinel. BUT take(null) works empirically → verify reach
codegen.rs:3710 | set_var (reassign) | NEEDS-FIX(gated)     | Optional → `_ => panic!` on nullable-local reassign — BLOCKED upstream by change_var today
codegen.rs:3619/3274 | set_var/generate_var RefVar | NEEDS-FIX(gated) | &nullable-scalar (RefVar(Optional)) → panic — blocked by change_var
codegen.rs:538/574/639/676/1222/1273/1312/2091/3232 | tuple element ops | NEEDS-FIX(gated) | Optional tuple element → panic — needs a nullable-element tuple LOCAL (blocked by change_var)
```
SAFE: add_const(3444), compile.rs const/Goto sites (operator operands never Optional), debug
renderers (mod.rs:2528/3199/2466, debug.rs:702/1715) = display/debugger-only, no corruption.
`fill.rs`/`state/io.rs`/`state/text.rs` = ZERO `Type::` sites.

### Group 5 — data + types + IR + misc (agent: 15 NEEDS-FIX, 9 SAFE-BENIGN, 2 SAFE-UNREACHABLE)
```
variables/mod.rs:1753 | align            | NEEDS-FIX(HIGH) | Optional(Int) → 1 not 8 → misaligned i64 slot UB/SIGSEGV (sibling pair, above)
data.rs:3971  | tuple_def align table   | NEEDS-FIX(HIGH) | size8/align1 mismatch → LinkedFieldGroup offset corruption (use element_align)
data.rs:4832  | Data::rust_type         | NEEDS-FIX(HIGH) | Optional → panic! HARD CRASH (native bridge gen, create.rs:89/206/215)
data.rs:4752  | Data::type_elm          | NEEDS-FIX(HIGH) | Optional → u32::MAX → data.def(MAX) panic / field skipped (the ROOT-CAUSE row)
slots_v2.rs:70| slot_kind               | NEEDS-FIX(HIGH) | Optional(Text/ref) → Inline not RefSlot → drop op not emitted → LEAK
data.rs:946   | to_default              | NEEDS-FIX | Optional → Value::Null not base default → wrong storage width
data.rs:1503  | depending               | NEEDS-FIX | Optional(Text/ref) deps not rebased onto frame var → borrow hole
data.rs:1527  | deps_ref                | NEEDS-FIX | Optional(Text/ref) → None drops dep list → borrow hole
data.rs:1544  | depend                  | NEEDS-FIX | Optional(Text/ref) → empty deps → lost borrows
data.rs:1840  | has_lifetime_concern    | NEEDS-FIX | Optional(Text/ref) return ownership rewrite skipped → leak/UAF
data.rs:2099  | owned_elements          | NEEDS-FIX | Optional(Text/ref) tuple elem not in scope-exit cleanup → LEAK
intervals.rs:53| compute_intervals      | NEEDS-FIX | Optional(Text/ref) → needs_early_first_def wrongly false → slot-order corruption
main.rs:2759/2822 | native bridge gen   | NEEDS-FIX | Optional param/ret → `() /* not supported */` → broken FFI shim (loud, not silent)
```
SAFE-BENIGN: data.rs display/show/argument/Display(1574/1720/1791/2176), narrow_vector_content
(3051, loses narrowing opt only), extensions.rs compute_sig family (fail-closed None), main.rs
field-offset comment. SAFE-UNREACHABLE: ir_read.rs:164 / ir_schema.rs:366 (write_type peels →
no Optional discriminant ever written; lossy-but-consistent round-trip — revisit when the
marker gains teeth at DN1/DN3). `typedef.rs`/`ir_store.rs`/`ir_node.rs`/`variables/validate.rs`
= ZERO unguarded sites (all already peel).

### Group 3 — parser type-check core (agent: 19 NEEDS-FIX, 10 SAFE-UNREACHABLE, 18 SAFE-BENIGN, 2 INTENTIONAL)

Two root-cause families. **(i) layout/codegen mishandling `Optional(Integer)`** (resolve_type_var
hands an un-peeled `concrete`): mod.rs 3089/3257/3742/3778/4354. **(ii) the `text?` return-buffer
ABI** (whole text-return work-buffer setup skipped → SIGSEGV — the text analog of the landed
scalar return-site): control.rs 897/4098/4116/5752/6057/6378, operators.rs 657/904, definitions.rs:850.
```
mod.rs:3089       | substitute_type_in_value | NEEDS-FIX | `_ => None` keeps OpConvBoolFromRef on i64 Optional loop var → SIGSEGV/E0610 | concrete.base()
mod.rs:3257       | is_primitive_vector_element_target | NEEDS-FIX(matches!) | false for Optional → parametric OpCopyRecord left on i64 elem → corruption | matches!(tp.base(),…)
mod.rs:3742       | type_element_size | NEEDS-FIX | `_ => 12` (DbRef) not 8 → wrong vector stride / inline struct size | tp.base() + forced_size guard
mod.rs:3778       | wrap_vector_get_val | NEEDS-FIX | `_ =>` emits NO OpGetInt → raw slot read as DbRef → crash | tp.base()
mod.rs:4354       | emit_set_one_element | NEEDS-FIX | `_` Level::Error rejects (integer?,text) + writes Null | elem_tp.base()
definitions.rs:2527 | parse_field | NEEDS-FIX(matches!) | narrow-int alias not captured for u8?/u16? field → wider storage than non-? twin | matches!(tp.base(),Int) + capture from base
operators.rs:657  | parse_operators | NEEDS-FIX(matches!) | text?/char? LHS of + not routed to append_text → valid `text?+x` REJECTED | eff_type_for_plus.base()
operators.rs:1789 | handle_operator | NEEDS-FIX(matches!) | float?/single? == null skips NaN-sentinel → OpEqFloat on NaN → always-false (correctness) | peel ctp+second_type in float_null guard
operators.rs:904  | parse_part | NEEDS-FIX(matches!) | chained call → Optional(Text): no work_text buffer → SIGSEGV (P227) | matches!(ret_type.base(),Text)
control.rs:897    | block_result | NEEDS-FIX | Optional(Text) return fails Text if-let → text_return ABI not set up | t.base()
control.rs:1364   | rewrite_dep_in_type | NEEDS-FIX | Optional(Text(deps)) → `_ => None` → inner deps never rewritten in work-ref unification → stale | Optional(inner)=>recurse
control.rs:2027   | parse_match | NEEDS-FIX | `match` on integer? → subject falls to `_` → "match requires enum/struct/scalar" ABORT | add Optional(scalar) or subject.base()
control.rs:4065   | for_type | NEEDS-FIX | `for x in nullable` misses Text/Integer arms → "Unknown in expression type" | peel in_type
control.rs:4098   | text_return | NEEDS-FIX | `-> text?` if-let fails → text-return dep/work-buffer skipped → dangling ABI (pair w/ definitions.rs:850) | returned.base()
control.rs:4116   | text_return | NEEDS-FIX(matches!) | Optional(Text) dep var hoisted as plain param not RefVar(Text) work-buffer | matches!(tp.base(),Text)
control.rs:5752   | parse_return | NEEDS-FIX | explicit `return <text?>` misses text_return + Vector-buffer → no owned-copy/dep delivery | t.base()
control.rs:6057   | parse_call (fn-ref) | NEEDS-FIX(matches!) | text?-returning fn-ref: no work_text buffer → empty-slot read → SIGSEGV (P227) | matches!(ret_type.base(),Text)
control.rs:6378   | try_fn_ref_call | NEEDS-FIX(matches!) | same P227: text?-returning fn-ref → zero work buffers → SIGSEGV | matches!(ret_type.base(),Text)
control.rs:6770   | seeds_vector_hint | NEEDS-FIX(matches!) | vector<u8?> literal: elem not seeded narrow stride → #432 stride corruption | matches!(elem.base(),Int)
```
SAFE: mod.rs get_val(4042)/set_field_check(4548)/null(7687) already peel; n_store_violation(1833)
+ handle_null_coalesce(1315) INTENTIONAL; the rest diagnostic/range-opt/fail-closed.
**Coupling:** (1) mod.rs:3293 MUST gain `.base()` the moment 3257 is fixed (else returns None on
a now-rewritable Optional); (2) definitions.rs:850 + control.rs:4098 must be peeled TOGETHER
(interface-stub vs concrete-impl `__work_1` arg counts must agree).

---

## SYNTHESIS — 69 NEEDS-FIX, the root-cause families, and the staged fix-sequence

Totals: native 14 · interp 13 · collections 8 · data/IR 15 · parser-core 19 = **69 NEEDS-FIX**
(`match` blocks + the `matches!`/`if-let` dispatch of the identical hazard). Plus the gating
`change_var` seam and the `matches!`-predicate second class. Uniform fix idiom everywhere:
**peel with `.base()` before the type dispatch** (or add an explicit `Type::Optional(inner) =>`
arm that recurses). Every fix is byte-identical gate-OFF (the Optional arm is dead code until an
Optional is constructed), so the layout/leak fixes can land UNGATED and additive.

**Family A — layout/size/align (HIGHEST stakes: SIGSEGV / panic / corruption).** The
sibling-pair misses + the parser layout sites: `align`(variables 1753), `tuple_def`-align(data
3971), `type_elm`(data 4752, the root), `Data::rust_type`(data 4832, panic), `to_default`(946),
`type_element_size`(mod 3742), `wrap_vector_get_val`(mod 3778), `emit_set_one_element`(mod 4354),
`substitute_type_in_value`(mod 3089), `seeds_vector_hint`(control 6770), `parse_field` narrow
alias(def 2527). Cheapest + most certain (mirror a proven twin).

**Family B — the `(N-Decl)` gate** (`change_var`, variables 1257). Unblocks nullable LOCALS;
gates the reachability of the whole interp-codegen group. Re-run the instrument after it lands.

**Family C — deps / lifetime / leak holes for `Optional(Text/ref)`** (matter once `text?`/`S?`
flow): `depending`(1503), `deps_ref`(1527), `depend`(1544), `has_lifetime_concern`(1840),
`owned_elements`(2099), `compute_intervals`(53), `slot_kind`(70). Validate with leak-check
(`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`), not value alone.

**Family D — the `text?` return-buffer ABI** (its own sub-thread, the text analog of the landed
scalar return-site): control.rs 897/4098/4116/5752/6057/6378, operators.rs 657/904, def 850.

**Family E — the `matches!`-predicate second sweep**: `is_scalar`(dispatch 579, mod 3003),
coroutine `suitable`(251), RefVar scalar-link(dispatch 158, emit 215), the ~40 `Type::Text(_)`
ABI gates in emit.rs/calls.rs. Same `.base()` peel, mechanical.

**Family F — feature type-check gaps**: `match` on `integer?`(control 2027), `for x in
nullable`(control 4065), `text?+` concat(operators 657), `float? == null` NaN(operators 1789).

**Family G — empirical confirmed reachable**: E1 native `null` tuple-element → `(())` (the
native default/typed-null path, ties to `default_native_value` 896); E2 `"{x}"` format reject
(collections 1219 / append_data) — a design call.

### Recommended fix-sequence (each ends green gate-ON, both backends)
1. **Family A ungated — the 4 sibling-pair misses DONE** (`align`/`tuple_def`-align/`type_elm`/
   `Data::rust_type`, mirror the proven twin). Validation: gate-OFF byte-identical introspect ✓,
   full suite green (only the pre-existing chrome `html_asyncify` #450 fails — env), instrument
   green gate-ON both backends ✓, wasm rlib rebuilds clean ✓. **Honest caveat:** these are the
   latent FOUNDATION layer — not independently crash-falsifiable today (their triggers are
   routed-around / silent-UB misalignment / masked by the interp+native tuple bugs that crash
   FIRST — e.g. `(g(),5)` panics at `emit_tuple_put_ops` before `tuple_def` align matters). They
   are validated by construction (twin-parallelism) + byte-identical, not by a flipped probe;
   they must land first so that once the tuple/interp bugs are fixed, correct layout is already
   underneath (else silent offset corruption replaces a clean panic). The remaining Family A
   parser layout sites (mod 3742/3778/4354/3089, control 6770, def 2527, data `to_default` 946)
   are NOT yet done.
2. **Family B** (`change_var`) — unblock locals; re-run `optional-flow-instrument.loft` extended
   with nullable LOCALS to see which interp sites (Family, below) become reachable.
3. **Reachable interp peels** — the now-reachable subset of the 13 (set_var, tuple ops).
4. **Family C** — deps/leak holes under leak-check on `text?`/`S?` probes.
5. **Family E + F** — the `matches!` sweep and the feature gaps.
6. **Family D** — the `text?` return-buffer ABI sub-thread.
7. **Family G** — E1 with the native typed-null peel; decide E2.
8. THEN the DN1 default flip (`IntegerSpec.not_null`) + the `.loft` sweep + gate default-on.

All behind `LOFT_PLN25_OPT`/`DN3` except Family A/C/E layout+leak peels (gate-OFF-inert, additive).
