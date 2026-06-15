<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 25 — Nullable sequences (`vector<T>` participates in the null model)

> **Tracker:** `@PLN25` ([loft-lang/plans#25](https://github.com/loft-lang/plans/issues/25),
> `status:active`). **Branch:** `2026-07-mac` (all phases below committed + pushed there).

## Status

**P1 (Foundation) + P2 (Surface) + the P3 slice fix (loft#384) SHIPPED and verified on
both backends; the P3 field-default arm is the next concrete work.** A `vector<T>` can
now be null (absent), distinct from empty `[]`: the runtime is null-safe (P1), `v ==
null` / `v != null` / `return null` work (P2), and a slice resolves negative bounds
from the end and clamps into range so it never runs off an edge (P3, loft#384 — finding
8). The reverse-slice ordering bug surfaced during that work was fixed in the same
session (finding 11). What remains: the field-default arm (P3, blast-radius flagged —
finding 9), then hardening (P4).

Promoted from the design discussion on loft#384 (negative-slice silent-empty / wrap-around
garbage) and loft#387 (text fn-ref buffer family). The invariant was validated against a
boundary matrix and three subsystem maps; the representation decision is pinned. **This
plan intersects the H6 stability hotspot** — it is the vector-axis reconciliation of the
scattered null-sentinel matrix, not adjacent to it.

## RESUME HERE (next action)

The slice fix (loft#384) and the reverse-slice fix are done (findings 8, 11 — both
backends green, regression cells in `tests/scripts/25-nullable-sequences.loft`). The
fixing commit must carry `Fixes #384` so the merge to `main` closes it (loft#384 is an
existing issue; until merged it stays open with `fixed-pending-merge`).

**Next: the `default_native_value` Vector arm (finding 9).** Make a nullable vector
field default to the null sentinel (`u16::MAX`) instead of empty `[]`, with a `not null`
opt-in to the empty fast path. This changes init semantics for EVERY struct vector
field, so it needs its own boundary matrix (existing code that assumes "unset = empty
`[]`") before shipping — do it as a separate, independently-verified change, not bundled
with anything. Then P4 hardening (consumer audit + wasm + docs).

## Goal

Make `vector<T>` nullable like every other loft value: a vector can hold **null**
(absent), distinct from an **empty** `[]`, using the canonical reference sentinel
`store_nr = u16::MAX` — so a slice or lookup that has no answer returns a real `null`
instead of a silent-empty or corrupted vector.

## Effort + design

- **Effort:** H (multi-backend: interpreter + native + wasm; ~15 re-assertion sites).
- **Design:** ~ (invariant validated + phases defined; per-phase cell expectations TBD).
- **Last touched:** 2026-06-15

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

## Cross-arc dependencies

- **H6 — null-sentinel matrix** (`STABILITY_HOTSPOTS.md`): this plan IS the vector-axis
  of H6. Coordinate so the `u16::MAX` unification here is the same one H6 adopts
  tree-wide; do not introduce a parallel encoding.
- **loft#384** (negative-slice) — closed by P3 (slice OOB → null + from-end bounds).
- **loft#387** (text fn-ref ABI) — sibling buffer/null family; not blocking, but the
  `convert`/return-unification work in P2 is adjacent.

## See also

- `doc/claude/LOFT.md` § Null representation (the "nullable unless `not null`" model).
- `doc/claude/STABILITY_HOTSPOTS.md` § H6 (null-sentinel matrix) — the shared hotspot.
- `doc/claude/DATABASE.md` (Store / DbRef / vector record layout).
- `src/vector.rs` (store-accessors), `src/parser/operators.rs` (`==` dispatch),
  `src/parser/control.rs` (`block_result`/`convert`), `src/generation/ops/ref_ops.rs`
  (null-aware codegen), `src/database/structures.rs` (`set_default_value`).
- loft#384, loft#387 (source design discussion); `@PLN25` (tracker — pending).
