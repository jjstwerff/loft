<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 25 — Nullable sequences (`vector<T>` participates in the null model)

> **Identity pending.** The `loft-lang/plans` issue (`@PLN25`) is NOT yet filed —
> creating it needs explicit authorization (external-repo write). `25` is provisional
> (tracker latest = 24); rename the dir if the issue lands on a different number.

## Status

Open — design validated, no implementation yet. Promoted from the design discussion
on loft#384 (negative-slice silent-empty / wrap-around garbage) and loft#387 (text
fn-ref buffer family). The invariant was validated against a boundary matrix and three
subsystem maps (struct-or-null blueprint, vector-consumer inventory, H6 sentinel
model); the representation decision is pinned. **This plan intersects the H6 stability
hotspot** — it is the vector-axis reconciliation of the scattered null-sentinel matrix,
not adjacent to it.

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
| **P3** — Producers | Slice from-end + clamp (**loft#384**) — LOCALIZED, not yet implemented (see finding 8); `default_native_value` Vector arm (nullable field → `u16::MAX`; `not null` → empty) — note: changes init semantics for ALL vector fields, blast radius (see finding 9). | In progress |
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

## P3 findings (localized; implementation pending careful both-backend verify)

8. **Slice bug is the iteration BOUNDS, not the element fetch — loft#384.** A vector
   slice `v[a..b]` lowers (in `parse_in_range_body`, `src/parser/objects.rs:1605`) to an
   iterator `ivar = a, a+1, … until till(=b) <= ivar`, fetching `v[ivar]` via
   `OpGetVectorNullable`. The per-element fetch already resolves a negative index
   from-end and returns null on OOB — but the iteration **endpoints** (`expr`=start,
   `till`=end) are the raw user values, never resolved/clamped. Result on `[10,20,30,40,50]`:
   `[2..100]` reads garbage past the end (`i64::MIN` sentinels), `[-2..]` wraps
   (`[40,50,10,20,30,40,50]`), `[2..-1]` silently empties.
   **Fix (localized):** in `parse_in_range_body`, when `data != Null` (a vector slice, NOT
   a pure `0..10` range), wrap BOTH `expr` and `till` through a from-end+clamp helper:
   `r = if b<0 { b+len } else { b }; clamp(r, 0, len)` where `len = OpLengthVector(data)`.
   Gives `[2..-1]→[30,40]`, `[-2..]→[40,50]`, `[2..100]→[30,40,50]`, pure ranges
   untouched. **Caveat:** `OpLengthVector` returns `i64`; bounds may be `i32` — the clamp
   IR must coerce types (use `conv_op` with matching `in_type`/`I32`/`I64`), and bind
   `b`/`len` to temps to avoid re-evaluating a side-effecting bound. Must be verified on
   BOTH backends against the full slice matrix (`a..b`, `a..=b`, `a..`, `..b`, neg a/b,
   reversed, over-range) — a subtle coercion error reintroduces garbage. Deferred from
   this session for that careful verification rather than shipped half-checked.
   Q3 resolved: **clamp** (a partial slice like `[2..100]` must yield the valid tail
   `[30,40,50]`, not null); a *fully* out-of-range slice clamps to empty `[]`. Returning a
   null vector for a whole slice is awkward (slices materialize element-by-element) and is
   NOT adopted.
9. **`default_native_value` Vector arm has blast radius.** Making a nullable vector field
   default to null (`u16::MAX`) instead of empty changes init semantics for EVERY struct
   vector field — existing code assuming "unset = empty `[]`" could break. Needs its own
   matrix + `not null` opt-in to the empty fast path before shipping; do as a separate,
   independently-verified change, not bundled with the slice fix.

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
