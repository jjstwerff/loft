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

**Resolved.**  The null model ships **only** when it is fully coherent (every seam green on BOTH
backends) — there is **no "gated, documented known-limitation" / "just use an enum" carve-out**;
that escape hatch IS the inconsistency this plan exists to remove.  A third-party programmer or AI
must never meet "null works for `vector<int>` but not `vector<Item>`".  This is realised by the
**dense-default** model ([RESUME.md](RESUME.md)), not the abandoned `LOFT_E2_SYNTH` enum synthesis.

- **Driver = third-party coherence, not the author's need or delivery speed.**  The author uses
  loft directly (and would reach for an enum); the point is that *other* users / AI never bump
  into the asymmetry.  **Effort and time are explicitly NOT the constraint — a multi-session,
  even multi-year, grind is acceptable.**  Weigh each seam by the coherence it delivers, never by
  effort saved; do not propose de-scoping.
- **Not "just consistency over enums."**  Enums cover *hand-constructed* data; they do NOT cover
  *deserialised* data — a JSON `null` in a typed array has no faithful enum target (which
  variant?).  That is the load-bearing use case (JSON / CSV / nullable DB rows).
- **Working model.**  One dedicated stream owns this; it merges to `main` only at **green phase
  boundaries** (never mid-phase), so the release branch stays green for the parallel feature/fix
  agents.  The vectors half landed this way (`#412`/`#467`/`#468`); the scalars + TIGHTEN half is
  on `lima-default-borrow-elision` and reaches `main` via the finishing PR (`Closes @PLN25`).

**Canonical incoherence probe** — the case the plan is measured against.  **Now coherent on `main`**
via the dense-default opt-in: written `as vector<Item?>` it gives `len 3`, `items[1] == null` true,
and the present objects keep their fields, on both backends.  The broken form below is the *old
default* (`as vector<Item>` importing a `null`) that motivated the plan:

```loft
struct Item { name: text, value: integer }
items = `[ {{"name":"a","value":1}}, null, {{"name":"c","value":3}} ]` as vector<Item>;
//  len(items)        == 3
//  items[1] == null  -> false       (claims it is NOT null)
//  items[1].value    -> null(oob)   (field read returns the OOB sentinel)
```

The `null` becomes a slot counted in `len` that reports `!= null` yet returns garbage on field
access — a silent corruption: `for x in items { if x == null {…} else { x.value } }` skips the
guard, then detonates.  **This is now resolved on `main`:** `vector<Item?>` yields a real,
null-safe absent element on both backends.  (Scalars already passed even before — `[1, null, 3] as
vector<integer>` imports the null correctly — that exact asymmetry is what the vectors half closed.)

## Superseded approach — the `__nullable<S>` enum-synthesis line (abandoned)

> The seam ledger, status, and "RESUME HERE" single-payload refactor that stood here
> tracked the **enum-synthesis** implementation: nullable vectors realised by synthesising
> a `__nullable<S>` enum behind the `LOFT_E2_SYNTH` gate (branch `2026-07-mac`). **That
> approach is abandoned.** The **dense-default** value model replaced it — `vector<S>` is
> dense and `vector<S?>` is the nullable opt-in, live on `main` for vectors
> (`#412`/`#467`/`#468`). Live status + the concrete remaining steps: **[RESUME.md](RESUME.md)**.
> The dense-default design: [storage-vs-access-nullability.md](storage-vs-access-nullability.md)
> + [full-design.md](full-design.md). The `Goal` / invariant / P1–P3 findings below are the
> original nullable-vector foundation (the `u16::MAX` container sentinel) and remain the
> shipped record for that arc.

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
    enum). Both backends green. **E2a — the embedded FIELD case shipped (loft#896).** A field
    written `item: Row?` is the synthetic `__nullable<Row>` enum, so `??`, `== null` and
    `f = null` all agree with the declaration; a field with no `?` stays dense and pays for no
    discriminant. The representation and its read/construct glue were already built — what was
    missing is that the field-side rewrite matched a bare `Reference`, while `Row?` reaches the
    type table as `Optional(Reference(Row))`. It therefore fired on exactly the COMPLEMENT of
    its intended set (every non-nullable field, since the legacy `nullable` attribute flag is
    true by default), which is why it had been held behind an opt-in and why it never once
    fired for the `S?` it was written for. Cells in `tests/issue_896_nullable_field.rs`, both
    backends. The vector-ELEMENT half of E2 was already default-on; still open is a
    `vector<T>?` FIELD (an `Optional(Vector)`, a different payload shape from
    `Optional(Reference)` — the synthetic enum wraps a struct). Also pre-existing: a
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
