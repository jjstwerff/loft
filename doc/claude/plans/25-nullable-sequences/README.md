<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 25 — Nullable sequences (`vector<T>` participates in the null model)

> **Tracker:** `@PLN25` ([loft-lang/plans#25](https://github.com/loft-lang/plans/issues/25),
> `status:active`). **Live status + concrete next steps: [RESUME.md](RESUME.md).**
> **Branch:** `lima-default-borrow-elision` (scalars + TIGHTEN, in flight); the vectors half
> is merged to `main` (`#412`/`#467`/`#468`). The **dense-default** approach below is LIVE and
> supersedes the earlier `LOFT_E2_SYNTH` enum-synthesis line (branch `2026-07-mac`, abandoned).

## Decision (2026-06-17) — finish to FULL coherence; no escape hatch; one final PR

**Resolved — this supersedes the "keep-vs-drop" ultimatum recorded below.**  E2 ships **only**
when it is fully coherent (every seam green on BOTH backends), and the gate (`LOFT_E2_SYNTH`)
flips exactly once, at the very end.  There is **no "gated, documented known-limitation" / "just
use an enum" carve-out** — that escape hatch IS the inconsistency this plan exists to remove.  A
third-party programmer or AI must never meet "null works for `vector<int>` but not `vector<Item>`".

- **Driver = third-party coherence, not the author's need or delivery speed.**  The author uses
  loft directly (and would reach for an enum); the point is that *other* users / AI never bump
  into the asymmetry.  **Effort and time are explicitly NOT the constraint — a multi-session,
  even multi-year, grind is acceptable.**  Weigh each seam by the coherence it delivers, never by
  effort saved; do not propose de-scoping.
- **Not "just consistency over enums."**  Enums cover *hand-constructed* data; they do NOT cover
  *deserialised* data — a JSON `null` in a typed array has no faithful enum target (which
  variant?).  That is the load-bearing use case (JSON / CSV / nullable DB rows).
- **Working model.**  One dedicated stream owns this.  Every change stays **gate-inert** (fires
  only for `__nullable<>` synth types, which exist only gate-on) so the monthly release branch
  stays green for the parallel feature/fix agents.  **No PR until E2 is done** — work accumulates
  on `2026-07-mac` and reaches `main` via a single final PR.

**Canonical incoherence probe** — the case the plan is measured against.  **Now COHERENT gate-on**
(seam **A4** built, 2026-06-17): `len 3`, `items[1] == null` is true, the present objects keep their
fields, both backends.  Still incoherent on shipped `main` (gate-off) until the gate is removed
(Step 6) — that final flip is what makes it the default:

```loft
struct Item { name: text, value: integer }
items = `[ {{"name":"a","value":1}}, null, {{"name":"c","value":3}} ]` as vector<Item>;
//  len(items)        == 3
//  items[1] == null  -> false       (claims it is NOT null)
//  items[1].value    -> null(oob)   (field read returns the OOB sentinel)
```

The `null` becomes a slot counted in `len` that reports `!= null` yet returns garbage on field
access — a silent corruption: `for x in items { if x == null {…} else { x.value } }` skips the
guard, then detonates.  **E2 is "done" when this yields a real, null-safe absent element on both
backends with the gate removed.**  (Scalars already pass: `[1, null, 3] as vector<integer>`
imports the null correctly — that exact asymmetry is what E2 closes.)

> **Stopgap (deferred — not doing it now).**  A loud error on `null` → non-nullable `vector<S>`
> would stop the silent corruption on `main`, but it is a gate-off behaviour change and we are
> taking **no interim PR**, so it folds into the final landing (and is moot once A4 preserves the
> null).  Revisit only if the silent corruption shipping in monthly releases becomes a concern
> before E2 is done.

## Distance to consistency (seam ledger, 2026-06-17)

"Consistent" = the gate flip (Step 6: drop `LOFT_E2_SYNTH`, default-on).  The **conceptual core
is done** — gate-on, a `vector<__nullable<S>>` already constructs, reads, does field/method/format,
null/OOB, plain-`for`, and basic `par`.  What remains is **not conceptual**: each subsystem has to
be made `Some`-aware on BOTH backends.  As established (§ Step-1 verdict), there is **no single
chokepoint** — this is a known, bounded, seam-by-seam tail.

This table is the scannable index; the mechanics for each row live in § RESUME HERE.

**Done + gate-inert (the bulk):** free-on-null leak · generics · inferred comprehension · A1
(anon-literal resolution) · A2 (dense-var append) · loop-var field access · par worker read ·
method dispatch · transparent format · the 06-structs alias-copy root · OOB disc guard · basic par.

**Open — the actual distance, in finishing order:**

| Seam | Size | Status |
|---|---|---|
| ~~**A4(b)** — JSON walker builds `Some` for a present object~~ | — | **FIXED** 2026-06-17 — gate-off-nullable was cache contamination (no leak); 1 walker arm; both backends + regression |
| ~~**A3** — `hash<__nullable<S>>` (key-resolution + lookup unwrap + anon-literal + for-loop + native)~~ | — | **DONE** — 12-collections passes gate-on on BOTH backends (rungs 1–5 + for-loop + native); regressions both backends |
| **gap 2** — `__nullable<S>` ↔ dense `S` at value boundaries | M (representation refactor) | **interim copy DONE** (`OpNullableToDense`, all value paths green both backends); **superseded by the single-payload refactor — see RESUME HERE banner** (deletes the copy op; the correct long-term form) |
| **gap 4** — `e = null` on a nullable LOCAL | XS | open |
| **Step 2** — par with USER EXTRA args (dispatcher arity) | S, self-contained | open |
| **Step 3** — native codegen of `__nullable<>` (name-mangle family) | M, whole backend | gated behind Step 1 |
| residual **wasm** + p143 / p379 (unconfirmed parser shapes) | unknown | not probed |
| **Step 5** — re-verify graphics/engine consumers, triage residue | the one genuinely-unknown count | not run gate-on since recent fixes |
| **Step 6** — flip gate, graduate probes, fold P3 field-default arm, close | XS | last, irreversible |

**How far, honestly:** ~5 known seams + one native/wasm backend pass + an unknown consumer-residue
triage, then the flip.  Step 5's residue is the only part that cannot be sized yet — that is *why*
Step 5 exists (re-run the consumers gate-on; triage only what does not go green from Steps 1–3).
**A4(b) is landed** (2026-06-17); **A3 rung 1** (key-resolution chokepoint, `key_owner`) is landed
(2026-06-17, gate-inert) — but A3 proved to be ≥4 substrate rungs (see Step-1 A3).  Next concrete
move: **A3 done**; **Step 3 + Step 5 sweep done** — the gate-on native tail is now MAPPED into
clusters F/C/K/N (see Step 5 § Gate-on native sweep).  **Cluster F done** + **Cluster K done**
(2026-06-19, both backends): F = keyed-SET routing + `for_type` Hash arm → `Enum(.., true)`; K =
keyed construction over nullable (parse_vector content-normalize + transparent-construction
`Reference(syn)` shape + native `bare_field_name` key_owner).  Together they cleared the keyed
cluster (119/120/122/126/127/128/291/32 + 131/134) both backends; the gate-on interpret divergence
set dropped 21 → ~11.  **gap 2 unwrap primitive DONE** (`OpNullableToDense`, commit `bfc60f9d`, both
backends — fixes 150-interp; the value-boundary `__nullable<S>`→dense `S` field-copy that the layout
proof showed was required).  **gap-2 ROUTING — value boundaries DONE both backends:** dense-local
assign C3 (`c05ff61c`); return-boundary via-a-local + direct-`??` (`ed20f61d`, `b399059c` — the
ref-return dispatch materialises a Var/Block/If-sourced `OpNullableToDense` tail into the return
buffer; a plain `v[i]` index tail stays the default direct-return).  151 + coalret + qret + ret all
green.  **Remaining gap-2 routing tail:** 150's pick_mid MID-BODY `return t[i] ?? d` (RetSite::
MidReturn — a materialise attempt mis-read fields, reverted), `&S`-arg 100 (copy-IN/OUT), return-WRAP
55 (dense `vector<S>` from native `stack_trace()` → nullable), single-element `+=` store_persist
(RHS-as-Some), by-value-arg leak, 149 (deep nested-lookup consumer).  Then **interface delegation**
(86), **par/forward-ref** (22/22c/40/371), **gap 4**, then the Step-6 flip.  Interpret divergences
hold at 9 (the return fixes were native-only for already-interp-passing 150/151).

## Status

**Core SHIPPED + default-on.  E2 (embedded-record null) is GATED.  The CONCEPTUAL core works
gate-on — construct/read/field/method/format/null/OOB/plain-for/par(no-extra-arg), and the deep
06-structs synthesis desync is fixed (commits 996f566d, 7c03a1c0).  Default-on still has a broad
tail (~24 raw failures), now reorganized into ~3 mechanism clusters + singletons + downstream
consumers, with a falsification-first 6-step execution plan (item 4): Step 1 probes whether the
four parse-path suites share ONE root in `nullable_enum_for`'s struct→enum keying — if so, one
chokepoint fix collapses most of the tail.  See item 4 for the full plan.**

*Shipped, default-on, verified both backends:* a `vector<T>` can be null (absent),
distinct from empty `[]` — runtime null-safe (P1); `v == null` / `!= null` / `return null`
(P2); slice resolves negative bounds from-end and clamps so it never runs off an edge,
with reverse-slice fixed (P3, loft#384); **simple-typed element null** (int/bool/float/text
— typed inner null, length-based iteration past a null, `float == null` via is-nan);
**E1** nullable enum VARIABLE null.  Plus a pre-existing `main` crash fixed ungated: the
`OpCopyRecord` null-source guard (`vec[i] = <runtime-null>`), with a `tests/scripts`
regression.

*E2 — embedded-record null (struct stored INLINE in a vector element / struct field, via a
synthesised `__nullable<S>` enum), gated `LOFT_E2_SYNTH`, off by default → suite
byte-identical.*  **All 24 gap-matrix probes pass on BOTH backends.**  Done: struct fields +
the inline `== null` discriminant test; `vec[i] = null` (discriminant-0 store) and
`vec[i] = <expression source>` (present/null convert); `??` coalesce on an inline-null
element; `v += [null]`; `match v[i] { null => … }`; the **`not null` dense ELEMENT opt-out
(E3)** — the cost escape hatch; the rewrite **unified to ONE chokepoint** (vector-type
resolution, so local / param / return / field / nested all agree); and **all four
construction-propagation forms** — nested `vector<vector<S>>`, inferred `v = [S{…}]`, return
bodies, comprehensions.

> **Branch:** `2026-07-mac` (rebased onto `main` + #393; all commits pushed).

## RESUME HERE (next action)

> ### ⇒ NEXT: the single-payload representation refactor (DECIDED 2026-06-19)
>
> **Current tree state:** clean at HEAD `63af849a`, ALL suites green (`plan25_e2_gap2`,
> `_hash`, `_json`, `_layout` + 150/151 interp).  `OpNullableToDense` is the **interim**
> copy-op and stays ONLY until the refactor below lands, then it is deleted.
>
> **The decision (user: "what is best for the long term loft code, I don't care about speed"):**
> change the `__nullable<S>` representation so the `Some` variant carries a **single
> `payload: S` field** (a real struct-enum `Some { payload: S }`) instead of S's fields copied
> INDIVIDUALLY.  Why: the individual-field form is GAP-FILLED by the packer (reorders the payload
> away from dense `S`), which is the ENTIRE reason `OpNullableToDense` (a field-copy) exists and
> why a sub-ref reinterpret reads garbage.  A single struct-typed field keeps S's dense layout
> (VERIFIED: `db.field(t67,"payload",t65)`, payload@8 = dense S), so **a payload sub-ref IS a valid
> dense `S` reference** → args/returns/`??`/`&mut`/field-reads all flow on the EXISTING reference +
> #306 view-return machinery with NO copy and NO new op.  It DELETES machinery instead of adding it,
> and has NO layout special-case (unlike the rejected `no_gap_fill` packer hack, which also broke
> the byte-identity test).
>
> **This is an ALL-OR-NOTHING big-bang** (~30 load-bearing sites; the representation change breaks
> them all at once — no green intermediate).  Land ONLY when F/K/A4 + gap-2 + the suite are green on
> BOTH backends; else revert to `63af849a`.  Migration map (full version with file:lines in the
> `pln25-nullable-coherence` memory):
> 1. `nullable_enum_for` (data.rs:3720) → add ONE `payload: S` field, not S's fields.  (The
>    method-filter comment at 3712 becomes moot — the payload is just dense S, methods and all.)
> 2. Field access — `find_poly_enum_field` (fields.rs:543) + callers (97, 281) resolve `e.field`
>    THROUGH `payload` (offset = payload-base + S-field-offset).
> 3. **THE CRUX — key-position contract.**  `key_owner` (types.rs:440) / `key_bearing_def`
>    (typedef.rs:760) return the `Some` variant whose DIRECT fields are the keys today; single-
>    payload sinks keys into `payload`, so `determine_keys` (types.rs:455), `get_keys` /
>    `field_content` (search.rs:292/65), and keyed codegen (generation/mod.rs:2028) must add the
>    payload base to EVERY key offset.  Must be right on BOTH backends or keyed collections corrupt
>    silently — verify F (keyed-set/iter) + K (keyed construction) hard.
> 4. Construction builds `payload` not individual fields — objects.rs:168/2322, collections.rs:736,
>    vectors.rs:1956/2099, builtins.rs:339 (the dense→Some per-field copy loops collapse to one
>    payload write).
> 5. `format.rs:810` writes the payload struct (not the `Some` field list).
> 6. `convert` (mod.rs:1336) → sub-ref at the payload offset (dense `S`), replacing the
>    `OpNullableToDense` emission.
> 7. DELETE `OpNullableToDense` — default/01_code.loft:1030, structures.rs:1033 `nullable_to_dense`,
>    fill.rs:239/1920, control.rs:678-691 + `tail_is_nullable_unwrap` (3605), operators.rs:296
>    `is_struct_returning_call` arm, pre_eval.rs:418.  (`materialize_view_return`/`_return_into`
>    SURVIVE — still used by genuine #306 views at control.rs:744/4290.)
> 8. Update `tests/plan25_e2_layout.rs` — the byte-identity assertion changes from "== a hand
>    `HSome { id, tag }`" to "the `Some.payload` field is a dense `S`" (the stronger, correct
>    invariant; the old one optimised for matching a *struct-variant* that is itself gap-filled and
>    never delivered free transparency).
>
> Pure name-identity `starts_with("__nullable<")` gates (most of category 3 in the map) survive
> untouched — they only need the enum to remain recognizable.

**The gate (`LOFT_E2_SYNTH`) is the marker that E2 is not yet finished** — "finished" means
default-on (gate removed), per the project standard.  The remaining work, in finishing order:

1. **The free-on-null leak (Sev 5) — FIXED.**  Root cause was GENERAL, not E2-specific:
   `remove_claims` had no `Parts::Enum` arm (it fell through to the no-op `_`), so freeing an
   inline enum NEVER freed its live variant's heap payload — every inline struct-enum with a
   text / nested-vector field leaked it on overwrite, default-on, since before E2 (baseline
   confirmed: a `vector<Maybe{Has{text},Empty}>` grew 52→2052 over 2000 churns; fixed holds at
   2).  Fix = the symmetric `Parts::Enum` arm (allocation.rs, twin of `copy_claims`' arm) +
   an `OpClearKeyed`-free emitted before the two E2 overwrite paths in `towards_set`
   (null-literal, present↔null convert).  Verified leak-flat + value-correct on BOTH backends.
   Regressions: `tests/leak.rs::pln25_inline_struct_enum_payload_free` (gate-off record-count)
   + `tests/scripts/389-inline-enum-payload-free.loft` (cross-backend correctness).
2. **Generics** — `vector<T>` (generic element) — FIXED.  Design chosen: the generic
   parameter stays DENSE and carries whatever element type the caller's vector holds —
   nullability is decided at INSTANTIATION (monomorphization substitutes `T`'s bound type
   directly; the caller's `vector<Row>` is already `vector<__nullable<Row>>`, so `T` binds to
   the nullable element and the generic is a transparent passthrough).  Two fixes: (a)
   `e2_nullable_elem` skips the active type-var stub (`Parser::cur_type_var`) so `vector<T>`
   is NOT rewritten — else `T` is buried in `__nullable<T>` and the first-param/return checks
   fail; (b) the monomorph-name mangle (mod.rs) flattens `<>,` → `_` (length-preserving) so a
   `t_15__nullable<Row>_count` def becomes a valid Rust identifier — native codegen emits the
   name verbatim, and rustc was parsing `<Row>` as a chained comparison.  Validated across 8
   shapes (return-T / non-T / nested / local `vector<T>` / multi-instantiation / `not null` /
   bounded scalar / null-through-generic) × both backends.  Regression:
   `tests/plan25_e2_generics.rs`.  (Surfaced two PRE-EXISTING, out-of-scope main bugs, both
   baseline-confirmed and filed: @P394 bare no-payload enum-variant value leaks a store;
   @P395 a generic over a tuple element reads garbage / fails native.)
3. **Inferred comprehension** `v = [for … { S{…} }]` (no annotation) — FIXED.
   `parse_vector_for` peeks the comprehension body for a leading struct-literal `{ S{…} }`
   and sets the block's expected type to `__nullable<S>`, mirroring the inferred-literal PEEK;
   the element then matches the declared form.  Both backends gate-on.

4. **GATE REMOVAL — the inline-element auto-unwrap glue (default-on trigger), IN PROGRESS.**
   A boundary matrix (`scripts/probe-matrix`, scalar `struct P{v}`, gate-on vs gate-off, hand-
   computed `@EXPECT` + failing control) collapsed the apparent "~107 scattered failures" to a
   **narrow class: ~4 unwrap gaps sharing ONE root** — a `__nullable<S>` value consumed where the
   dense `S` is expected, with the unwrap not applied.  What ALREADY works gate-on: direct
   `v[i].field`, text fields, `e = v[i]; e.field` (local), whole-vector arg/return, ALL null
   semantics (`= null` / `== null` / `+= [null]`), ALL mutation/append (incl. `+= [nullable_elem]`),
   nested-vector elements, `len()`.  The gaps:
   - **(1) field access on a loop variable** `for x in v { x.field }` — **FIXED**: `for_type`
     kept the `__nullable<S>` element in `Enum` form (not `Reference`) so field access hits the
     `find_poly_enum_field` unwrap (control.rs).
   - **(2) fn-call coercion** `f(v[i])` / `f(local)` / `f(loopvar)` → `fn f(r: S)` — "expected
     P, got `__nullable<P>`".  Fix = `__nullable<S>` and the `Some` payload region are
     byte-identical, so coerce via an **offset-ref reinterpret** (point at the payload, the same
     shift `find_poly_enum_field` uses) at `can_convert` + the call-arg lowering.  *No re-pack.*
   - **(3) par worker element read** — reads offset 0 (the `Some` discriminant) → e.g. 3×2=6
     instead of 60.  **FIXED**: synthesize a `__par_nullable_w(e: __nullable<S>){ w(<payload>) }`
     wrapper worker (builtins.rs), mirroring the destructure-wrapper pattern; the body reuses the
     gap-2 offset-ref.  Both backends.
   - **(4) `e = null` on a nullable LOCAL** — "cannot change type from `__nullable<P>` to null".
     Still open (rare; `v[i] = null` element form works).

   **DEFAULT-ON DESIGN (decided): E2 applies ABOVE the native stdlib.**  The flip surfaced that
   native `#rust` functions write the DENSE struct ABI (`fields()` → `vector<JsonField>`), which
   an E2 wrap desyncs (empty reads).  So **`STD_SOURCE` keeps dense `vector<S>`**; the rewrite
   applies to user files + libraries.  This cut the tree-wide fallout from ~50 (JSON / `code!`
   clusters all parse_str = STD_SOURCE).

   **Trajectory (full-tree flip, gaps applied):** 107 → 81 (gaps 1-3) → 30 (stdlib dense) → **25**
   (after the `!nullable` is-null operator + the field-vector-in-arg first-pass re-type fix).
   Two more access seams then **FIXED** (gate-inert, committed 32393141):
   - **method dispatch on a nullable receiver** (`v[i].method()`) — `field()` now unwraps the
     `__nullable<S>` receiver to dense `S` (gap-2 offset-ref) when the access is a method call
     (trailing `(`) or the name isn't a `Some` data field.
   - **transparent format** — `{v[i]}` rendered `Some {…}`; the runtime formatter now renders a
     synthetic `__nullable<S>` as the dense `S` (present) or `null` (absent).

   **THE 06-structs ROOT — SOLVED 2026-06-16 (commit 996f566d).**  The earlier
   "stale `known_type` / db-index" theory was WRONG (the layout was self-consistent; `Area` is
   genuinely `u16+u8+u8+u8` = 8 B, so stride 8 is correct).  The real root: `nullable_enum_for`
   copied only each field's **type** into the `Some` variant, dropping **`alias_d_nr`**.  Codegen
   distinguishes a struct field from a bare narrow-vector element by `alias_d_nr != MAX`; the
   alias-less synth field looked like a narrow-vector element, so it stored RAW (`OpSetShortRaw`,
   no `+1`) while the DB typed it nullable — reads/format applied the `-1` decode to a raw value
   (`u16 1234 → 1233`; `0 → null sentinel`).  **Fix:** carry `alias_d_nr`/`nullable`/`mutable`/
   `init` so the `Some` payload is byte-identical to a dense `S` and uses the same `OpSet*`/`OpGet*`
   encoding.  `tests/scripts/06-structs.loft` passes gate-on.
   - **(also fixed, 996f566d) OOB disc read** — `OpGetEnum` lacked the `rec == 0` null-record
     guard that `OpGetInt`/`OpGetCharacter` have, so an out-of-bounds `vector<__nullable<S>>`
     element (`get_vector` returns `rec:0`) read a garbage disc byte and tested as PRESENT, not
     absent — `!v[oob]` was false.  Fixed in the `#rust` annotation + regenerated `fill.rs`.
   - **(fixed, 7c03a1c0) par() over a nullable vector** — the worker did not resolve in pass 1
     (`parse_parallel_worker_method` built the method name from `__nullable<S>` instead of dense
     `S`), so `fn_d_nr=MAX` → `build_parallel_for_ir` emitted `Null` not `Value::Parallel`, and the
     block parser's `;`-exemption (keys on `Parallel`) fired a spurious "Expect token ;" on the
     statement AFTER the loop.  Fix: resolve on the dense `S` in BOTH passes; the gap-3 wrapper is
     now a shared `synth_nullable_par_wrapper` used by both worker forms that mirrors the worker's
     params 1.. (the `ref_return` hidden out-param), so method / primitive-return / struct-return
     par all work.  **Still open: par with USER EXTRA args** (`worker(a, extra)`) — the dispatcher's
     extra-arg marshalling collides with the wrapper's mirrored params (stack underflow `8<12`);
     blocks `threading` + `script_threading`.

   **REMAINING TAIL → EXECUTION PLAN (default-on, ~24 raw failures).**  The earlier
   "heterogeneous, no shared root" read was over-pessimistic: four of the failing suites all run
   through the SAME element-type-resolution chokepoint (`e2_nullable_elem` → `nullable_enum_for`,
   expressions.rs:1940 / data.rs:3651), and the big graphics/engine consumers almost certainly
   fail DOWNSTREAM of them.  So the ~24 raw failures collapse to ~3 mechanism clusters + a few
   singletons + downstream-verification targets.  Each step is matrix-first (gate-on probe in
   external `/tmp` scratch, hand-computed `@EXPECT` + a failing control cell, fix at the
   chokepoint, verify BOTH backends, graduate the probe into
   `tests/scripts/25-nullable-sequences.loft`).

   - **Step 1 — Cluster A: element-type resolution (highest leverage + the falsification).**  ONE
     boundary matrix gate-on over the CONSTRUCTION-PATH axis: `{named-struct literal,
     anonymous-struct literal, += append, return body, cross-lib same-name struct}` ×
     `{construct, read-back}`.  This directly tests whether 11-vectors (struct append),
     12-collections (`[{t:"One",v:1}]` → mis-resolves to `i_parse_errors`), p143 (nested-vector
     struct return) and p379 (cross-lib same-name struct) share ONE root in `nullable_enum_for`'s
     struct→enum keying.  The `i_parse_errors` mis-resolution points at anon-struct synthetic-name
     keying (collision / unstable synth name).  If they share the root, one chokepoint fix closes
     all four suites + likely the graphics consumers; if not, the matrix shows the real boundary
     and the cluster splits — but KNOWN, not guessed.

     **STEP 1 RESULT (2026-06-16) — the shared-root hypothesis is REFUTED; Cluster A is a STACK of
     independent seams, not one chokepoint.**  A 7-cell construction-path matrix (named-literal-local
     / anon-literal-field / append-var / append-literal / return / nested-return / wrong-assert
     control), hand-computed `@EXPECT` + a verified-red control, on `--interpret` with the gate
     hard-flipped (the `LOFT_E2_SYNTH` ENV path is inert — only a source flip enables the rewrite;
     irrelevant to shipping, which is correctly off-by-default).  Findings:
     - **A1 — anon-struct-literal element resolution — FIXED** (commit pending).  `unique_elm_var`
       (vectors.rs) only honoured a `Reference` `assign_tp`; for a `__nullable<S>` ENUM element it
       fell back to `Reference(type_def_nr(parent_tp.content))`, which mis-resolved to an arbitrary
       def → "Unknown field i_parse_errors.a".  Named `S{ … }` literals dodge this via the
       name-driven transparent path (objects.rs:151); an anon `{ … }` has no name, so it relies on
       the element var's type to hit `parse_block`'s record-scan.  Fix: type the element var as the
       `Some` variant (`Reference(variant_of(syn,"Some"))`) so the scan builds the present payload.
     - **A2 — append/store of a DENSE struct VARIABLE into `vector<__nullable<S>>` — FIXED** (commit
       pending; both backends).  `v += [p]` emitted a raw `OpCopyRecord(dense-S → Some-slot)`: no
       discriminant set, and the `Some` payload is laid out INDEPENDENTLY from dense `S` (the packer
       reorders fields around the disc — e.g. dense `P` = `a@0,b@8`, `Some` = `disc@0,b@4,a@8`), so
       the copy writes every field to the wrong offset → garbage reads (`v[1].a == 4`).  The literal
       append (c3b) works because it builds `Some` field-by-field.  Fix (parse_item, vectors.rs):
       for a dense `Reference(S)` source into a `__nullable<S>` element, build `Some` field-by-field
       via get_field/set_field (type/alias aware) + set the disc present — exactly as a literal does;
       non-Var sources stashed once.
     - **A3 — `hash<S[k]>` over a nullable element — RE-MATRIXED 2026-06-17 (cache-clean, gate-on);
       it is a STACK of ≥4 substrate rungs, of which rung 1 is now FIXED.**  `struct Counting {
       entries: vector<Count>, lookup: hash<Count[t]> }` — gate-on BOTH `entries` and `lookup` are
       rewritten to the synth enum (`vector<__nullable<Count>>` / `hash<__nullable<Count>[t]>`; the
       `hash` arm at definitions.rs already mirrors the `vector` arm via `e2_nullable_elem`).  The
       earlier "reads null" symptom was superseded — the matrix (gate-on, `LOFT_NO_CACHE=1`) shows
       four independent rungs:
       - **Rung 1 — key fields LOST → empty key spec — FIXED (commit pending; gate-inert).**  Three
         resolution sites (`Stores::hash` build, `create_key` for sorted/index, `determine_keys`, and
         the runtime `field_content`) all guard on `Parts::Struct | EnumValue` for the element type and
         silently SKIP `Parts::Enum`, so the synth-enum hash resolved to `hash<__nullable<Count>[]>`
         (NO keys → garbage bucket).  Fix = a `Stores::key_owner(content)` chokepoint: for a synth
         `__nullable<S>` element it returns the `Some` variant's `EnumValue` (resolved by db name
         `__nullable<S>::Some`, since the enum's variant LIST still holds a `u16::MAX` placeholder at
         build time), which the existing guards already accept — so the key field numbers + byte
         positions agree across build (`hash`/`create_key`) and run (`determine_keys`/`field_content`).
         Plus a build-ORDER fix (typedef.rs `Type::Hash` arm): force-build the `Some` variant
         (`key_bearing_def` → `fill_database`) BEFORE `database.hash` resolves keys, else `Some` is
         registered after the hash type and the key spec still comes out empty.  Result: the hash type
         is now `hash<__nullable<Count>[t]>` (key present).  Gate-off byte-green (2397/2398; only the
         pre-existing non-E2 `kernel_port`).
       - **Rung 2 — FIXED 2026-06-17 (interpret; gate-inert).**  The earlier "construct-then-corrupt"
         framing was WRONG — a per-op frame trace showed the constructed var stays VALID; the garbage
         was a MISALIGNED STACK READ.  `get_keys` (search.rs:292) is a FIFTH `key_owner` site that
         rung 1 missed: its `Parts::Hash`/`Sorted`/`Index` arms guarded on `Parts::Struct | EnumValue`
         and skipped the synth `Parts::Enum`, returning an EMPTY key-type list.  `read_key` then popped
         ZERO key values, leaving the lookup key (`"Five"`) on the stack, so `get_record`'s
         `data = get_stack::<DbRef>()` read the text bytes as a container ref (`store_nr` = a text-byte
         → `hash::find` OOB).  Fix: `key_owner` in both `get_keys` arms.  Lone-hash + lookup hit/miss
         now correct on the interpreter.
       - **Rung 3 — FIXED 2026-06-17 (interpret; gate-inert).**  `c.lookup[k].field` parse-errored
         (`Unknown field __nullable<Count>.v`) because `index_type` (fields.rs:689) converts a keyed
         collection's `Type::Enum(_, true)` element to `Type::Reference(d_nr)` — correct for a normal
         struct-enum (points at the variant) but wrong for a synth `__nullable<S>` (here `d_nr` IS the
         enum, so `Reference` has no payload fields).  Fix: keep the `Enum` type for a synth nullable
         (exactly as a `vector<__nullable<S>>` element is typed), so the existing field-access unwrap
         (fields.rs:95) resolves S's fields through `Some`.  `lookup[k].v` + `lookup[k] == null` now
         work on the interpreter.  Regression: `tests/plan25_e2_hash.rs` (interpret).
       - **Rung 5 — anon-struct-literal vector into a hash FIELD — FIXED 2026-06-17 (gate-inert).**
         `hd.h = [ {name:"one"}, {name:"two"} ]` (12-collections:104) parse-errored gate-on
         (`unexpected ','` / `Unknown field __nullable<Row>.name`).  Root: `unique_elm_var`
         (vectors.rs) typed the anon element from `assign_tp`, but a keyed-collection field's
         `content()` is `Reference(__nullable<S>)` (not `Enum(..)`), and the `Type::Reference` arm used
         it verbatim → the element resolved against the enum.  Fix: a `Reference(__nullable<S>)`
         assign-tp resolves the anon element against the `Some` variant, same as the `Enum(syn,true)`
         (vector) arm.
       - **For-loop over a struct-field nullable vector — FIXED 2026-06-17 (gate-inert).**  `for item
         in c.entries { item.v }` summed garbage.  Root (collections.rs:313): a struct-field vector that
         SHARES records with a sibling hash is a LINKED array of 4-byte ref slots, but the for-loop
         element read upgraded to the deref `OpVectorRefNullable` only for a `Type::Reference` element —
         a `__nullable<S>` element is `Enum(syn,true)`, so it kept the inline `OpGetVectorNullable(4)`
         and read the rec-id slot AS the record (every field offset junk).  Fix: take the deref path for
         a synth-nullable `Enum` element too when the collection `is_linked`.  (An INLINE
         `vector<__nullable<S>>` local keeps the inline read.)
       - **Rung 4 — native codegen of a `hash<__nullable<S>>` — FIXED 2026-06-18 (both backends).**
         Mechanism (confirmed by reading the emitted Rust + `src/generation/mod.rs`): native labels
         every type `t{compile_tid}` and REQUIRES the generated `db.*` calls to reproduce those tids in
         creation order (interning is first-call-wins).  A keyed-collection STRUCT FIELD (@P296,
         `field_keyed`) is created INLINE during the struct's field emission and excluded from the
         tid-ordered `bare_io` flush — correct only when the hash's compile-tid is reachable then.
         Lazily building `Some`/`Null` during the hash typedef gave them tids AFTER the struct, so
         native interned hash↔Some swapped and the baked `OpGetRecord(hash_tid)` resolved to `Some` →
         `find called on non-collection type`.  **Fix (two parts):**
         - (typedef.rs `fill_all`) build each synth `__nullable<S>` enum's `Null` + `Some` variant
           STRUCTURES right AFTER their wrapped struct `S` in the layout loop — so `S`'s field types are
           created first (no native forward-ref) yet the variants precede any later struct that holds a
           `hash<__nullable<S>>` field.  Both backends now agree on tids (interpret is tid-agnostic;
           native replays them).
         - (mod.rs `convert`) a `__nullable<S>` value used as a BOOLEAN (`if hash[k]` / `assert(hash[k])`)
           coerces to its present-check (disc != 0 via `OpGetEnum` @ 0, which has the rec==0 guard) —
           else the raw nullable DbRef reached the bool context and native emitted `(DbRef) as u8`
           (E0605).
         Validated: `12-collections` passes gate-on on BOTH backends (exit 0); all `plan25_e2_*` gate-on
         suites pass; gate-off byte-green (2400/2401, only `kernel_port`).  Regression extended to
         `--native` in `tests/plan25_e2_hash.rs`.
         **`06-structs` native gate-on — FIXED 2026-06-18 (a method-attribute leak, the first Step-3
         win).**  It failed E0425 `cannot find value t<N>` (identically with AND without the
         eager-variant build, so NOT the hash type-id swap; 06-structs native gate-on had never been
         validated).  Root: a method `fn m(self: S)` is registered as a `Type::Routine` ATTRIBUTE on
         `S` (gate-independent; the dense layout omits it and `--native` skips it).  `nullable_enum_for`
         copied ALL of `S`'s attributes — INCLUDING the method — into the `Some` variant; building
         `Some`'s db structure then resolved that `Routine`'s `known_type`, which made the DENSE `S`'s
         method attribute look like a concrete field, so `--native` emitted `db.field(S, "m", t<N>)`
         for a type it never declares.  Fix: `nullable_enum_for` copies only DATA fields —
         `.filter(|a| !matches!(a.typedef, Type::Routine(_)))` (a declared fn-ref data field is
         `Type::Function`, kept).  Minimal repro `[P{x}].val()` + 06-structs now pass gate-on on BOTH
         backends; regression `tests/plan25_e2_hash.rs::method_on_nullable_vector_element_native`.
         Gate-inert (`nullable_enum_for` runs only gate-on).
       **A3 STATUS: DONE — `tests/scripts/12-collections.loft` passes gate-on end-to-end on BOTH
         backends (interp + native, exit 0).**  Rungs 1–5 + the for-loop-deref FIXED + gate-inert
         (gate-off suite 2400/2401, only the pre-existing non-E2 `kernel_port`); regressions in
         `tests/plan25_e2_hash.rs` (both backends).  E2-internal (a keyed collection over an ENUM only
         arises from the synth rewrite), so nothing is filed.  (Out-of-A3 follow-up: the separate
         pre-existing `06-structs` native forward-ref noted under rung 4.)
     - **A4 — `"…" as vector<S>` / `vector<S>.parse(json)` into nullable structs — LOCALIZED
       2026-06-17 (boundary matrix; the earlier mechanism here was WRONG on two counts).**  A 7-cell
       gate-on `--interpret` matrix + `introspect` (scalar control, present-only, null-mid/lead/all,
       struct-field-vector, nested) showed:
       1. **Wrong walker named.**  The `as vector<S>` cast lowers to **`OpCastVectorFromText`** →
          `state/io.rs db_from_text` → `database/format.rs parse` →
          **`database/structures.rs::walk_parsed_into`** — NOT `parse_string` /
          `n_struct_from_jsonvalue` (that walker serves `Struct.parse(JsonValue)`).
       2. **False premise "the vector element is rewritten to `__nullable<S>`".**  Gate-on, the cast
          target is dense `vector<ref(S)>` (introspect: `vector<ref(Item)>`, db_tp 66), byte-identical
          to gate-off — the rewrite fires ONLY for INFERRED literals (`vectors.rs:1387/1667`,
          `is_unknown()`), not the cast target nor an annotated local.  A dense ref-vector cannot even
          originate a null element (`v[0]=null` is a silent no-op), and `walk_parsed_into`'s
          `Parsed::Null` arm (structures.rs:522) just `set_default_value`s the dense struct → the
          `null(oob)` corruption (counted in `len`, `==null` false, fields read OOB).
       So A4 is **two gate-inert parts, not one**: **(a)** route the cast / `.parse` target element
       through `e2_nullable_elem` so gate-on it becomes `vector<__nullable<S>>` (matching inferred
       literals — matrix-verified to interoperate: a `__nullable<S>` vector PRESERVES its null across
       a `vector<S>` param and an annotated assignment); **(b)** teach `walk_parsed_into` to build the
       `Some` variant for a `Parsed::Object` and the null variant (disc 0) for `Parsed::Null` when the
       element type is a synth `__nullable<S>` enum (mirror the literal Some-build in
       `vectors.rs::parse_item`).  Everything downstream of a `__nullable<S>` vector already works, so
       the blast radius is the cast-type site + the one walker arm.
       **Part (b) is fully designed** (set Some-disc + reuse the existing enum-variant fill: map a plain
       JSON object to the `Some` variant; `Parsed::Null` already lands disc-0 via `set_default_value`).
       **Part (a) is ALREADY DONE — RESOLVED 2026-06-17, after a STALE-BINARY false trail.**  The
       earlier "cast stays dense `vector<ref(S)>`" reading was an artifact of a STALE `target/release/loft`
       (built from an earlier point in the 56-commit branch).  Rebuilt from current source + instrumented
       (`LOFT_TRACE_E2`, since reverted), `introspect` shows `as vector<S>` resolves to
       `v:vector<ref(__nullable<S>)>` (db_tp nullable, `GetVectorNullable` stride 16) and emits
       `OpCastVectorFromText(text, <nullable-kt>)`.  **Lesson logged** (engineering-rigor § calibration):
       ALWAYS rebuild the lens before trusting a matrix cell — every cell from the stale binary was void.
       JSON `null` already deserialises correctly (disc 0 via `set_default_value`): `[null,null]` → len 2,
       both `==null`.
       **Part (b) — FIXED 2026-06-17 (commit pending; both backends).**  A present JSON object reaching
       `walk_parsed_into` with element type `__nullable<S>` (an Enum) hit the `Parts::Enum` arm, whose
       tag extraction rejected a plain multi-field object → `Err(mismatch)` → the `?` in the array loop
       ABORTED the parse at that element.  Fix (structures.rs `Parts::Enum` arm): detect a synth
       `__nullable<S>` by `self.types[tp].name.starts_with("__nullable<")` and route a bare
       `Object`/`Constructor` to the `Some` variant — `("Some", Some(parsed))` reuses the existing
       variant-fill machinery (find `Some`, set its disc, recurse the payload `EnumValue`).  `null` was
       already correct (`Parsed::Null` → `set_default_value` → absent disc 0).  Verified on the
       null-POSITION axis × both backends gate-on: `[{a},null,{c}]`→len 3 (`a/1`, absent, `c/3`),
       `[null,{b}]`→len 2, `[null,null,null]`→len 3 all-absent, `[{a},{b}]`→len 2 all-present.
       Regression: `tests/plan25_e2_json.rs` (4 tests, interpret + `--native`).
       **verify (i) RESOLVED — there is NO gate leak and NO shipped regression; it was CACHE
       CONTAMINATION.**  The earlier "cast resolves to `__nullable<S>` even gate-off" reading (and the
       "backtick form differs from plain") were artifacts of the warm program cache (`~/.cache/loft/
       program-<hash>.store`, @PLN11 G2/M6): its key hashes SOURCE CONTENT, not `LOFT_E2_SYNTH`, so a
       gate-off run and a gate-on run of the same file share one cache slot and stomp each other
       (last-writer-wins) — and a warm hit skips parsing entirely (`e2_nullable_elem` never called).  On
       a cache-clean binary (`LOFT_NO_CACHE=1`) the gate is correct and form-independent: gate-off →
       dense `vector<ref(Item)>` (db_tp 66, stride 12) for BOTH the plain-string and backtick forms;
       gate-on → `vector<ref(__nullable<Item>)>` (db_tp 69, stride 16) for both.  `11-vectors` passes
       gate-off because it is dense (and has no nulls).  Since `__nullable<>` is synth machinery that
       exists ONLY gate-on, `main` cannot produce it — branch-internal, nothing to file.
       **CALIBRATION LESSON (logged, engineering-rigor § calibration — twin of the stale-binary trap):**
       when probing E2 gate states, ALWAYS pass `LOFT_NO_CACHE=1`.  The warm cache key omits the gate
       env, so without it gate-off/gate-on runs of one file silently serve each other's stale bundle —
       every cross-gate matrix cell is void.  (Harmless for shipping: the gate will not exist once
       Step 6 removes it.)
       **Sibling found, OUT of A4:**
       nested `vector<vector<S>>` via JSON deserialises to an EMPTY outer vector (present OR null) — a
       distinct unimplemented seam, routed separately.
     - p143 (nested-vector struct return) and p379 (cross-lib same-name) did NOT reproduce from the
       named-literal cells (c4/c5 pass) — they need their real shapes; likely further distinct seams.

     **Scoping VERDICT (the Step-1 deliverable):** "fix the one chokepoint" does NOT apply — there is
     no shared root, and each suite is a LADDER (A1→A3 in 12-collections, A2→A4 in 11-vectors).  The
     first two rungs (A1, A2) were contained PARSER fixes and are landed + gate-inert.  The next two
     rungs (A3, A4) are SUBSYSTEM seams — the hash/sorted/index key-extraction and the native JSON
     walker each have to be made `Some`-aware — i.e. real, independently-verified, both-backend
     subsystem changes, not one-site patches.  And those are only the rungs visible SO FAR in two
     suites; the broader tail (native codegen, wasm, cross-lib, graphics consumers) sits behind its
     own subsystems.  **Conclusion: E2 default-on has NO chokepoint and is genuinely multi-session,
     subsystem-by-subsystem.**  **DECISION (2026-06-17, see § Decision at top): the keep-vs-drop
     ultimatum is RESOLVED → keep + finish; de-scoping to a "gated, documented known-limitation" is
     REJECTED (that carve-out is itself the inconsistency E2 removes).**  A1+A2 are kept regardless
     (verified gate-off byte-green, both backends).  Next: grind A3 (hash key-extraction) and A4
     (JSON walker) — and the rungs behind them — each to FULL coherence on both backends, no
     documented carve-out; flip the gate only when the canonical probe (§ Decision) passes.
   - **Step 2 — Cluster B: extra-arg par dispatcher** (threading, script_threading).  Single
     mechanism: `synth_nullable_par_wrapper` bails on user extra args (the `extra_vals.is_empty()`
     guard, builtins.rs:277) because the dispatcher's extra-arg marshalling assumes the ORIGINAL
     worker arity, not the wrapper's mirrored-param layout (stack underflow `8<12`).  Fix: extend
     the wrapper to accept+forward the extra args as real params and align the dispatcher arity
     (`build_parallel_for_ir`).  Self-contained.
   - **Step 3 — Cluster C: native codegen of `__nullable<>` types** (native_dir/scripts/
     library_suite, p171, p310).  Hypothesis: same family as the already-fixed generics mangle
     (`<>,`→`_`) — a synth-enum name leaking into a generated Rust identifier/type.  Probe: build
     one native repro, read the emitted Rust, find where `__nullable<S>` survives un-mangled; fix
     at the codegen name chokepoint.  Gated behind Step 1 (parse must be correct first).
   - **Step 4 — Singletons.**  gap (4) `e = null` on a nullable LOCAL (the var-type-change check
     rejects `__nullable<P>`→null — small, localized); leak p150 (free path); residual wasm.
     Each independent and small.
   - **Step 5 — Re-verify consumers, triage residue.**  Re-run the graphics/engine/wasm consumers
     (kernel_port, moros_glb, moros_editor_html, wasm_library_suite) + the wrap suites (dir, last,
     parser_debug, loft_suite, libraries, library_suite) gate-on.  Expect most to go green from
     Steps 1-3; triage ONLY the residue — that is the honest count of truly-independent seams.

     **GATE-ON NATIVE SWEEP (2026-06-18) — the tail map.**  Ran every `tests/scripts/*.loft` gate-on
     on `--native`.  The failures cluster (raw-sweep `aborting due to N errors` on the EXPECT-error
     suites — 35/36/72/74/101/102/… — are likely false positives the test harness handles; the real
     E2 seams are the `__nullable`-mentioning + runtime-panic ones):
     - **Cluster F — keyed-SET insert + field access on a loop var iterating a HASH.**  DONE
       2026-06-19, both backends (regression `tests/plan25_e2_hash.rs::keyed_set_into_nullable_hash_
       inserts_iterates_and_looks_up`).  The earlier two-part framing (parser-only change exposes a
       hash-iteration RUNTIME read crash) was WRONG — instrumenting the SIGSEGV with a no-iteration
       probe (`hash[k]=S{…}` then a bare lookup) showed the lookup *itself* returned null and the
       records were never inserted.  Real root, TWO gate-inert seams:
       (1) **keyed-SET routing** (`towards_set`, collections.rs:567): the @P305 insert-or-replace
       routing fires only for `matches!(f_type, Type::Reference(_,_))`, but gate-on a keyed
       collection over a nullable element has `f_type = Enum(__nullable<S>, true)` (the `index_type`
       rung-3 form), so the set fell through to the UPDATE-only `OpCopyRecord(value, OpGetRecord(…))`
       — which no-ops on the insert-miss (empty hash) so nothing is ever inserted; every lookup then
       reads null and *iterating the empty index* is what segfaulted.  Fixed by accepting the
       synth-nullable enum element too → routes to `OpSetKeyed` (find-or-insert; `set_keyed` reads
       the key via the SAME `key_owner`-resolved descriptors the lookup uses, so insert and lookup
       agree).  (2) **for-loop element typing** (`for_type` Hash/Sorted/Index arm, control.rs:3259):
       now keeps a synth `__nullable<S>` element as `Enum(.., true)` (mirroring the Vector arm) so
       `e.field` unwraps through `Some`.  With (1) fixed, (2) is no longer a half-fix — the SIGSEGV
       it previously "exposed" was the empty index, not a runtime-read crash.
       *Lesson:* a SIGSEGV in iteration was a symptom of an INSERT bug upstream; the no-iteration
       probe localized it in one run — the `i_parse_errors` disc-value "tell" was a disassembler
       cosmetic (the working vector path emits the identical `SetEnum(…, val: i_parse_errors)`).
     - **Cluster K — keyed construction over nullable.**  DONE 2026-06-19, both backends
       (regression `typed_local_keyed_append_builds_some_in_place`).  FOUR gate-inert fixes cleared
       the whole keyed cluster (119/120/122/126/127/128/291/32): (1) `parse_vector` normalizes a
       keyed collection's `content()` (`Reference(__nullable<S>)`) to the inline `Enum(.., true)`
       construction form; (2) the transparent Some-construction (objects.rs) now fires for the
       `Reference(syn)` parent shape (a typed-LOCAL keyed slot's element ref is `Reference(Some)`),
       not only `Enum(syn,true)` — without it the field path built `Some` in place while the
       typed-local path built a dense `S` that a raw `OpCopyRecord` mis-laid into the `Some` record;
       (3) NATIVE `bare_field_name` routes through `key_owner` so a typed-local keyed collection
       emits `db.hash(t, &["ck"])` not `&["?"]` (a "?" key hashes every record to one bucket →
       dedup-to-1 + missed lookups); (4) `key_owner` is now `pub(crate)` for (3).  Cluster N's
       119/120/122 native panics were the SAME bug (the `"?"` key), fixed here too.
     - **Cluster C / gap 2 — `__nullable<S>` ↔ dense `S` coercion at VALUE boundaries.**  The
       UNWRAP PRIMITIVE is DONE (2026-06-19, commit `bfc60f9d`, both backends); ROUTING for the
       remaining boundaries is open.
       - **The primitive — `OpNullableToDense`** (`Stores::nullable_to_dense`, declared in
         `default/01_code.loft` with a `#rust` template so one source feeds interp + native).  The
         old convert arm was SILENTLY WRONG: it sub-ref-reinterpreted the `Some` payload as dense
         `S`, but the packer reorders fields around the discriminant (dense `a@0,c@8,b@16,d@20` vs
         Some `disc@0,b@4,a@8,c@16,d@24`), so `by_val(v[0])` read `a=1 b=null c=7 d=`.  The op copies
         FIELD BY FIELD (scalar bytes Some-offset→dense-offset, then `copy_claims` deep-copies heap
         from the source).  Now correct on both backends; **fixes 150**.  Regression
         `tests/plan25_e2_gap2.rs`.  Earlier parser-level prototypes hit store/lifetime walls (null
         -ref FreeRef, `set_str` free-list corruption) — the database-layer op sidesteps them.
       - **Routing DONE (commit `c05ff61c`, both backends):** dense-local assign `d: S = v[i]` (C3)
         and `x ?? dense_default` (151) — `parse_assign_op` now runs `convert`→`OpNullableToDense`
         before `change_var` and retypes the RHS dense (ungated: the type error fires on pass 1 too).
         Native needed the vector-read source HOISTED (`needs_pre_eval`/`op_uses_stores` now flag
         `OpGetVector*`/`s.database.`-vocab templates) so `OpNullableToDense(v[i])` does not
         double-borrow `stores`.  151 passes both backends; regression
         `nullable_element_to_dense_local_assign`.
       - **Open routing:** (a) **return-BOUNDARY** unwrap on NATIVE (150 `a = pick_local(...)`, repro
         `/tmp/gap2/coalret.loft`: `chosen = t[i] ?? none(); chosen` returned, M has a `text` field).
         ROOT CAUSE FOUND (native codegen): when the unwrap is the return-coercion of a LOCAL-VAR
         return in a ref-return fn, the emitter drops it — `stores.nullable_to_dense(&var_chosen, …)`
         is emitted as a DISCARDED statement followed by `return DbRef{store_nr: u16::MAX,…}` (a NULL
         return) → caller derefs store 65535 → panic.  REFINED (the IR, not native, is wrong):
         ret.loft emits `Return(OpNullableToDense(...))` (returned directly); coalret emits
         `OpNullableToDense(chosen)` as a bare STATEMENT then `Return(null)`.  `ref_return`
         (control.rs) demoted the unwrap tail to the buffer-promotion convention (a statement fills
         the hidden `__retbuf`, then `return null`) — but `OpNullableToDense` ALLOCATES A FRESH store,
         it does not fill the pre-allocated buffer, so native discards the statement and returns the
         null sentinel.  FIX (its own change — `ref_return` is subtle 300-line buffer logic, do NOT
         rush): detect a fresh-allocation (`OpNullableToDense`) body-tail and return it DIRECTLY
         (`Return(<call>)`) as the single-tail path already does, rather than the buffer convention.
         151 passed (scalar `M`, working path); interp correct for both (its ref-return takes the
         last stack value).  (b) `&S` by-ref arg (100) needs copy-IN/OUT around the call
         (the unwrap is a copy, so a `&mut` mutation wouldn't propagate) — errors before convert.
         (c) return-WRAP (55, dense `vector<S>` → `vector<__nullable<S>>`) is the opposite direction.
         (d) single-element `h += S{…}` (store_persist) — routing the singleton compiles but the
         dense literal builds dense-offset fields copied into `Some` at the wrong offsets (`v=null`);
         needs the RHS literal to construct `Some` (the transparent-construction path, which only
         fires when the element type reaches the literal parse — it doesn't on the `+=` RHS).
         (e) a by-value arg still LEAKS one dense temp store gate-on (the arg path skips
         `copy_ref`'s free-source bit).  Each is its own routing seam on the now-working primitive.
     - **Interface delegation (86):** `__nullable<S>` must satisfy an interface its underlying `S`
       implements (`missing to_label`) — a method/interface call through the wrapper unwraps to `S`.
       Sibling of gap 2 (the method-attribute Step-3 work is the related fix).  Size S–M.
     - **store_persist (single-element `h += S{…}`):** the no-bracket keyed `+=` still types the RHS
       as dense `S` and rejects the type change; needs the same content()-normalization as the
       bracketed form, on the single-element path.  Size XS.
     - **Cluster N residue / par (Step 2):** 22/22c/40 (par over nullable), 371 (forward-ref
       vector<struct>) — remaining native/par seams after the `"?"`-key fix cleared 119/120/122.
     Method-attribute leak (the earlier Step-3 win) is FIXED; these are the next seams.
   - **Step 6 — Flip the gate + close.**  Drop `&& std::env::var("LOFT_E2_SYNTH").is_ok()` in
     `e2_rewrite_enabled` (expressions.rs:1937 — KEEP the `STD_SOURCE` dense-stdlib exclusion:
     native `#rust` writes the dense struct ABI).  Full `make ci` both backends.  Graduate all
     gated probes into `tests/scripts/25-nullable-sequences.loft`.  Then fold in the deferred P3
     `default_native_value` Vector arm (nullable field → null default, own matrix + `not null`
     opt-in — the last non-E2 open item).  Set plan status SHIPPED, close item 5.

   **Order rationale:** Step 1 first = highest leverage (one fix → 4 suites + consumers) AND it
   falsifies the heterogeneity assumption cheaply, in the first probe; Step 2 is independent and
   cheap; Step 3 depends on parse being correct; the gate flip is strictly last (the one
   irreversible ship action).  **Load-bearing risk:** the whole plan hinges on Step 1's matrix
   confirming the shared root — if the four parse seams are genuinely distinct, effort roughly
   doubles and Step 5's residue grows, but that surfaces in the FIRST probe.
   *(`imaging_fixture_png_roundtrip` is #397's, not E2 — owned by another stream.)*

   **Re-gated** (`LOFT_E2_SYNTH` restored on the 3 rewrite sites) so the monthly release branch
   stays green while this tail closes — all fixes above are **gate-inert** (fire only for
   `__nullable<>` types, which exist only gate-on; full `wrap`/`issues`/`leak` suites green gated).
   To finish default-on: close the tail seam-by-seam, then lift `LOFT_E2_SYNTH` on the 3 sites
   (keep the `STD_SOURCE` exclusion) and graduate the gated probes to
   `tests/scripts/25-nullable-sequences.loft`.  Honest scope note: the tail is broad and
   heterogeneous (native + wasm + dispatcher + cross-lib), i.e. multi-session, not a single root.

   **Not E2** (excluded by the matrix): par over a struct with a TEXT field is garbage on BOTH
   backends gate-off — a pre-existing text+par heap bug.

   **Hardened this session (gate-inert, keepable):** the `__nullable<>` variant-name collision
   fix (two such enums → `Double structure type Null`; now enum-qualified structure keys), the
   append `Enum`==`Enum` acceptance arm, the inferred-comprehension peek, generics, and the
   embedded-record leak.  Embedded NON-vector struct-field nullability (`item: Row`) is split
   onto its own opt-in `LOFT_E2_FIELDS` (more immature than the sequence path — its field-read
   auto-unwrap glue is also unbuilt).

Full gate-on behaviour matrix, the closed-vs-open catalogue, and per-site mechanics:
[embedded-record-null.md](embedded-record-null.md) (§ E2 — Known gaps; § RESUME POINT).

## Goal

Make `vector<T>` nullable like every other loft value: a vector can hold **null**
(absent), distinct from an **empty** `[]`, using the canonical reference sentinel
`store_nr = u16::MAX` — so a slice or lookup that has no answer returns a real `null`
instead of a silent-empty or corrupted vector.

## Effort + design

- **Effort:** H (multi-backend: interpreter + native + wasm; ~15 re-assertion sites).
- **Design:** ~ (invariant validated + phases defined; per-phase cell expectations TBD).
- **Last touched:** 2026-06-17

## The invariant (the one rule)

A vector is null **iff** its backing reference is the canonical null sentinel
`store_nr = u16::MAX` — the SAME sentinel struct references already use. Empty `[]`
stays a VALID store with length 0 (`rec==0` / a real record). Null ≠ empty is
guaranteed because `u16::MAX` is distinct from every real `store_nr`. Every site that
reads a vector's backing store routes through **one** chokepoint that reports "null"
only for `u16::MAX`, so the guard is asserted once, not sprayed.

**Why not the existing `rec==0`:** vector code today treats `rec==0` as *both*
unallocated and empty — so null and empty are currently the same state and cannot be
distinguished. Reusing it would make a null sub-sequence indistinguishable from an
empty one (the half-baked outcome). Reusing the reference sentinel instead nets H6
toward a single heap-null encoding.

## Composition matrix — Stage A (write as `/tmp` probes on `--interpret` FIRST)

The feature is done when **every cell is green on both backends**, not when a demo
runs. Axes: {null vector, empty `[]`, populated} × {operation} × {backend}.

| operation | null vector (`u16::MAX`) | empty `[]` | populated | notes |
|---|---|---|---|---|
| `v == null` / `!= null` | true / false | false / true | false / true | new operator overload |
| `len(v)` | `0` (decision Q2) | `0` | n | must not deref |
| `for x in v` | 0 iterations | 0 iterations | n | must not deref/OOB |
| `v[i]` (raising) | `null(oob)` raise | `null(oob)` raise | elem/raise | loud, recoverable |
| `v[a..b]` slice | null vector | clamp/empty (Q3) | sub-seq | OOB → null (the #384 payoff) |
| `v += x` | error or auto-init (Q4) | append | append | decide |
| `sort`/`reverse`/`remove`/`clear` | no-op (null-safe) | no-op | mutate | chokepoint guard |
| return `null` for `vector<T>` | compiles | — | — | unify like Reference |
| nullable field unset | null | — | — | `default_native_value` Vector arm |

Probes graduate to `tests/scripts/25-nullable-sequences.loft` as the regression suite.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **P1** — Foundation | ✅ Chokepoint `DbRef::is_null()` + `DbRef::NULL` (`store_nr==u16::MAX`) in `src/keys.rs`; vector store-accessors (`length`/`get`/`append`/`remove`/`insert`/`clear`; `sort`+`reverse` transitively via `length`) guarded through it. A null vector flows through `len`/`for`/`index`/`append`/`remove` with no `stores[u16::MAX]` OOB; null stays distinct from empty `[]`. Verified by `plan25_null_vector_tests` (4 tests) + full suite (2381 pass, 0 fail). | Shipped |
| **P2** — Surface | ✅ `OpVectorIsNull(v)=store_nr==u16::MAX` (one `#rust` template → both backends via `make fill`); `==`/`!=` dispatch lowers `Vector ⊗ Null` directly to it (NOT `eq_ref`); `convert(Null→Vector)` via `OpNullRefSentinel` (return null). `v == null`/`!= null`, `fn f() -> vector<T> { null }`, and iterate-null all green on both backends; **empty `[]` correctly ≠ null**. Regression: `tests/scripts/25-nullable-sequences.loft`. | Shipped |
| **P3** — Producers | ✅ Slice from-end + clamp (**loft#384**) SHIPPED — `parse_in_range_body` clamps both bounds into `[0,len]` via a once-computed `len` temp (finding 8); reverse-slice ordering fixed in the same session (finding 11). Both backends green; regression cells in `tests/scripts/25-nullable-sequences.loft`. **Remaining:** `default_native_value` Vector arm (nullable field → `u16::MAX`; `not null` → empty) — changes init semantics for ALL vector fields, blast radius (see finding 9). | Slice done; field-default open |
| **P4** — Hardening | consumer audit (every `t_*vector*` native fn), both backends + wasm, full suite, regression tests, docs (LOFT.md null model + STABILITY_HOTSPOTS H6). | Open |

## Phase ordering

1. **P1 first** — the chokepoint is the load-bearing risk-reducer: it collapses ~9
   silent OOB-risk guards to one. Until a null vector survives every read op, the
   surface work has nothing safe to point at.
2. **P2** — once the runtime is null-safe, expose null at the type/operator layer.
3. **P3** — only after `null` is a first-class vector value do producers emit it.
4. **P4** — audit + multi-backend + docs last; each prior phase already ran the matrix.

## Open design questions

1. **Null encoding** — RESOLVED: `store_nr = u16::MAX` (reuse reference sentinel; nets
   H6 toward one encoding). Alternative (new vector-only code) rejected: adds a 6th
   sentinel, worsens H6.
2. **`len(null)`** — RESOLVED in P1: `0` (absent reads as zero-length, matches the
   existing `rec==0`→0 path). Revisit only if the `!v` truthiness rule needs `len`/null
   to differ.
3. **Slice out-of-range** — return a null vector (consistent with this plan) vs
   clamp-and-warn vs empty-and-warn. Leaning **null vector** now that null is a real
   value — that is the whole point of the feature.
4. **`v += x` on a null vector** — error (loud), or auto-initialize to `[x]`? Leaning
   error: appending to an absent vector is a logic bug, make it loud.
5. **`not null vector<T>`** — does the field/param modifier suppress nullability and
   unlock the empty-only fast path (skip the `u16::MAX` guard)? Mirror `Reference`.

## P1 findings (recorded from the build — these constrain P2/P3)

1. **Container-null ≠ element-null — the P2 footgun.** The runtime already uses
   `rec == 0` as the "universal null-DbRef indicator" for vector *elements* and
   out-of-range results (`State::vec_get_or_raise` comment, `src/state/mod.rs`). That is
   a DIFFERENT convention from this plan's *container* null (`store_nr == u16::MAX`). The
   P2 `== null` / `!= null` operator for a vector MUST test `is_null()`
   (`store_nr==u16::MAX`), **never `rec==0`** — an empty `[]` is a valid store with
   `rec==0`, so a `rec==0` test would make `[] == null` wrongly true. This is the exact
   over-unification the design protocol flagged: two states that look alike under the
   wrong predicate.
2. **Loudness of a missed guard is debug-only.** `keys::store`/`mut_store` panic on
   `store_nr==u16::MAX` via `debug_assert!` — so a future accessor that forgets the
   `is_null()` guard crashes loudly in debug/tests but is silent OOB/UB in `--release`.
   The "make omission loud" cure (design-protocol step 2) therefore holds only under
   debug. **P4 decision:** either make the deref loud in release too (cheap branch in
   `keys::store`) or fold all vector store-access behind one `vec_record()` deref
   chokepoint so there is a single site to guard. Until then: N per-accessor guards,
   one-home *test* (`is_null`) but sprayed *guards*.
3. **Raising index path already covered (good news).** `vec_get_or_raise` reads length
   via the now-guarded `length_vector`, so a null vector → length 0 → raises a
   recoverable `IndexOutOfBounds` and returns the null sentinel — safe, no OOB. Refine
   the error *kind* to a null-specific fault later (cosmetic; not blocking).

## P2 findings (recorded while building Surface)

4. **`OpEqRef` cannot be reused for vector `== null`.** Its `#rust` body (and `eq_ref`
   in `fill.rs`) tests `rec == 0` as null — so references test null via `rec==0`, and an
   empty vector (`rec==0`) routed through it would compare `== null` TRUE. The vector
   null test MUST check `store_nr == u16::MAX` (`is_null()`). → P2 adds a dedicated
   `OpVectorIsNull(v) -> boolean` = `v.store_nr == u16::MAX`, lowered directly in the
   `==`/`!=` operator dispatch when one operand is `Vector` and the other `Null` (NOT via
   the generic `OpConv`/`call_op` matcher — there is no untyped `vector` base that
   `is_equal`-matches every `vector<T>`, the way `reference` does for structs).
5. **The null-vector *producer* already exists.** `OpNullRefSentinel` emits
   `{store_nr: u16::MAX, rec:0, pos:0}` = `DbRef::NULL`. So `return null` for a vector =
   `convert(Null→Vector)` reusing `OpNullRefSentinel` (P3's slice-OOB null uses it too).
6. **Decision (Q-new): `== null` is a store_nr identity test, not element equality.**
   `v == null`/`v != null` only ever compare against the sentinel; this plan does NOT add
   general `vector == vector` element equality (out of scope; would be a separate op).
7. **Literal `v: vector<T> = null` stays rejected — by design, matching references.**
   The var-type-change check rejects a `Type::Null` literal assigned to a typed var;
   `m: SomeStruct = null` fails identically. So this is a general null-literal-assignment
   limitation, NOT vector-specific — making vectors accept it would make them
   *inconsistent* with references. A null vector enters a variable from a nullable source
   (`v = maybe(false)`, a nullable field, a slice miss). Vectors now mirror references
   exactly: `== null` ✓, `return null` ✓, literal `= null` ✗ (shared).

## P3 findings (slice fix + reverse fix shipped; field-default arm pending)

8. **[SHIPPED] Slice bug is the iteration BOUNDS, not the element fetch — loft#384.**
   Implemented in `parse_in_range_body` (`src/parser/objects.rs`): a `slice_clamp_bound`
   helper + a once-per-loop `len`/`lo`/`hi` prelude emitted via `Value::Insert` (a flat
   sequence, NOT a scoped `v_block` — a nested block scope reclaims the temps' slots on
   exit, so the loop body read stale memory; that was the crash during the build). A vector
   slice `v[a..b]` lowers (in `parse_in_range_body`, `src/parser/objects.rs`) to an
   iterator `ivar = a, a+1, … until till(=b) <= ivar`, fetching `v[ivar]` via
   `OpGetVectorNullable`. The per-element fetch already resolves a negative index
   from-end and returns null on OOB — but the iteration **endpoints** (`expr`=start,
   `till`=end) are the raw user values, never resolved/clamped.
   **Matrix-verified scope (2026-06-15, interpreter)** — the actual failing cells on
   `[10,20,30,40,50]`:
   - **Negative end** (`[2..-1]`, `[..-1]`, `[2..=-1]`): the raw negative `till` makes
     `till <= start` true on entry → loop breaks immediately → silently **empty**.
   - **Negative start** (`[-2..]`): the per-element fetch resolves it from-end, but the
     loop keeps running up to `len` → **wraps** (`[40,50,10,20,30,40,50]`).
   - **Over-range positive end** (`[2..100]`): the loop runs to `100`, the per-element
     fetch returns null past `len`. **Materialization into `vector<T>` silently DROPS
     these nulls** (so `v: vector<T> = v[2..100]` looks correct = `[30,40,50]`), but
     **raw iteration `for x in v[2..7]` leaks the trailing nulls** — this is finding 8's
     original "garbage" symptom, and on `--native` the OOB read may surface as `i64::MIN`
     rather than null. So the fix must clamp the over-range end too, not only resolve
     negatives.
   **Fix (localized):** in `parse_in_range_body`, when `data != Null` (a vector slice, NOT
   a pure `0..10` range), compute `len = OpLengthVector(data)` once into a temp, then wrap
   BOTH `expr` and `till` through `clamp(if b<0 { b+len } else { b }, 0, len)` (inclusive
   `..=` → convert to exclusive `+1` first, then clamp to `len`). Gives `[2..-1]→[30,40]`,
   `[-2..]→[40,50]`, `[2..100]→[30,40,50]`, raw iteration with no trailing nulls, pure
   ranges untouched. **No coercion needed (correction to the earlier draft):** the
   introspect dump shows `OpLengthVector(r) -> integer` is `I32` and is compared directly
   via `OpLeInt(integer, integer)` with no conversion node — all loop math is `OpAddInt`/
   `OpLeInt` (i32), so the clamp stays in i32. Bind `len` to a temp to compute it once
   (the open-end path currently re-evaluates `OpLengthVector` every iteration inside the
   test). Verify on BOTH backends against the full slice matrix (`a..b`, `a..=b`, `a..`,
   `..b`, neg a/b, over-range, null/empty base).
   Q3 resolved: **clamp** (a partial slice like `[2..100]` must yield the valid tail
   `[30,40,50]`, not null); a *fully* out-of-range slice clamps to empty `[]`. Returning a
   null vector for a whole slice is awkward (slices materialize element-by-element) and is
   NOT adopted.
9. **`default_native_value` Vector arm has blast radius.** Making a nullable vector field
   default to null (`u16::MAX`) instead of empty changes init semantics for EVERY struct
   vector field — existing code assuming "unset = empty `[]`" could break. Needs its own
   matrix + `not null` opt-in to the empty fast path before shipping; do as a separate,
   independently-verified change, not bundled with the slice fix.
10. **Slicing a null/empty base is already safe (2026-06-15 probing).** `nul[0..3]`,
    `nul[..]`, `nul[-2..]`, `emp[0..3]`, `emp[..]` all yield empty with no OOB/crash on the
    interpreter — P1's `len(null)==0` guard (`OpLengthVector` routed through `is_null()`)
    already covers the slice base. So the bounds fix (finding 8) inherits null-base safety
    for free; no extra slice work is needed on this plan's own null axis. Re-confirm on
    `--native` at verify time.
11. **[FIXED] `rev()` on a slice did not reverse — separate flag-propagation mechanism.**
    `rev(0..5)` → `4,3,2,1,0` ✓ and `rev(v)` → `50,40,30,20,10` ✓, but `rev(v[2..5])`
    yielded **forward** `30,40,50` — the inner subscript parse never sees the `rev` token,
    so `parse_in_range_body` got `reverse = false` while `self.reverse_iterator` was set by
    the enclosing `rev(...)`. **Fix:** in `parse_in_range_body`, drive the loop direction
    from `want_reverse = reverse || self.reverse_iterator` and consume the flag; the `)` for
    the `rev(slice)` form is consumed by the enclosing `parse_in_range`, so the trailing
    `token(")")` stays gated on the `reverse` param only. Now `rev(v[2..5])` → `50,40,30`,
    `rev(v[2..100])` → `50,40,30` (clamp + reverse compose), and `rev(0..5)`/`rev(v)`
    unchanged. Both backends green; regression cell added.
12. **[FIXED] Element-level null in simple-typed vectors — the real "nullable
    sequences" core.** A `vector<integer|boolean|float|text>` element can be the inner
    typed null (e.g. `i64::MIN`, `NaN`), but two things were broken (matrix over element
    type × {iterate, ==null} × backend):
    - **Iteration broke at the first null element (silent data loss).** A vector for-loop
      terminated on `!element` (convert the element to boolean, then `OpNot`), using the
      OOB null sentinel as a proxy for "past the end". A *null element* shares that
      sentinel, so `[10, null, 30]` iterated **once**. The same proxy had already been
      patched per-type (fn-ref → `d_nr>0`, coroutine/tuple → exhausted). **Fix
      (`parse_for`, `collections.rs`):** length-based termination — break when the index
      is outside `[0, len)` (`len <= i` forward end, `i < 0` reverse `i32::MIN` end),
      independent of the element value. The length is re-read EACH iteration (so in-loop
      `x#remove`, which shrinks the vector and decrements the index, still drains) and
      taken from the collection the *fetch* reads (extracted from `iter_next`), so a
      side-effecting `for x in make()` is not re-evaluated.
    - **`float == null` was always false.** Float `==` is `!a.is_nan() && !b.is_nan() &&
      |a-b|<ε`, and `null` converts to `NaN`, so any NaN operand made it false — float
      null could never be detected, scalars included. **Fix (`operators.rs`):** a
      `float_null` dispatch (parallel to `vec_null`) lowers `float/single == null` to the
      validity check `OpNot(convert(f, boolean))` (= `is_nan`), `!= null` to its negation.
    Verified across int/bool/float/text on BOTH backends; regression cells in
    `tests/scripts/25-nullable-sequences.loft`. Still open (pre-existing, NOT simple-typed):
    **reference / embedded-struct** vector elements — `vr[i] = null` no-ops on the
    interpreter and was a native codegen crash (`OpCopyRecord` gets `()`). That is a
    representation gap shared with **nullable enums** (which crashed identically — both are
    inline value records). The fix gives a nullable inline record the **nullable-enum
    layout** (discriminant at offset 0, `0`=null) with a `vector<Row not null>` opt-out —
    designed in [embedded-record-null.md](embedded-record-null.md); one representation fixes
    nullable enums, embedded structs, vector elements, and finding 9. **E1 (nullable enum
    VARIABLE) shipped** — `enum == null` now tests the `store_nr` sentinel (`OpRefIsNull`),
    and a present enum returned from a nullable fn keeps its `store_nr` on native (a deeper
    return-ABI bug E1 surfaced: the ref-retbuf tail-capture now matches a variant against its
    enum). Both backends green. The embedded-field / vector-element cases (E2) remain. Also pre-existing: a
    reused `_` loop var across different element types is type-locked to its first type
    (native E0308) — separate from this fix.

## Cross-arc dependencies

- **H6 — null-sentinel matrix** (`STABILITY_HOTSPOTS.md`): this plan IS the vector-axis
  of H6. Coordinate so the `u16::MAX` unification here is the same one H6 adopts
  tree-wide; do not introduce a parallel encoding.
- **loft#384** (negative-slice) — closed by P3 (slice OOB → null + from-end bounds).
- **loft#387** (text fn-ref ABI) — sibling buffer/null family; not blocking, but the
  `convert`/return-unification work in P2 is adjacent.

## See also

- [embedded-record-null.md](embedded-record-null.md) — null inline-struct elements/fields
  via the nullable-enum representation (finding 12's open case + finding 9 + nullable enums).
- `doc/claude/LOFT.md` § Null representation (the "nullable unless `not null`" model).
- `doc/claude/STABILITY_HOTSPOTS.md` § H6 (null-sentinel matrix) — the shared hotspot.
- `doc/claude/DATABASE.md` (Store / DbRef / vector record layout).
- `src/vector.rs` (store-accessors), `src/parser/operators.rs` (`==` dispatch),
  `src/parser/control.rs` (`block_result`/`convert`), `src/generation/ops/ref_ops.rs`
  (null-aware codegen), `src/database/structures.rs` (`set_default_value`).
- loft#384, loft#387 (source design discussion); `@PLN25` (tracker — pending).
