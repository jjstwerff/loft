<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN87 — Reference-default `&`-binding ownership semantics

**Tracking issue: [loft-lang/plans#87](https://github.com/loft-lang/plans/issues/87)** — the
detailed plan (phases, gates, the matrix) and the eight load-bearing concerns live there.
Implemented by the loft2 agent.

Implement loft's binding-ownership model: **heap is reference-by-default** (a binding or
parameter aliases the source; field/element mutation writes through), and **`&` makes a
whole-binding reassignment write back** to the source. One uniform meaning for `&`; fixes the
recurring "`&Object` needed to mutate" confusion; realizes the OWNERSHIP_MODEL beacon for
binding semantics.

Design + rationale: [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) +
[DESIGN_DECISIONS.md C77](../DESIGN_DECISIONS.md). Builds on @PLN85 (the store-lifetime
cluster); the W4 lint joins the @PLN46 warning family.

## Status & handoff (2026-06-22) — READ FIRST after a context reset

**P0 ✅ DONE + committed.** Migration sweep across the whole ecosystem (stdlib + all 10 registry
libs + the entire zero-trust-shared-files app) found **zero** real propagation-reliance sites →
**P2 is safe**. (Verdict in § Phases below. The gated `LOFT_SWEEP_P0` sweep instrument is
throwaway WIP in `scopes::check` — strip before any PR, task D2.)

**P1 mechanism ✅ DONE + verified — but BLOCKED on the substrate base.** `&<lvalue>` parses and
binds a single-indirect **VIEW** (NOT a `RefVar` var — that slot is double-indirect,
`codegen.rs:2139`), flagged in `Parser::amp_bindings` for P2's write-back. Verified on BOTH
backends for the reference-default (vector-index) case: `cells = &vv[0]; cells[0] = 99` →
`vv[0][0] == 99` (writes through); the no-`&` control views too. Impl: `operators.rs` base case
(sets `amp_pending`, returns the inner type), `expressions.rs::parse_assign_op` (records
`amp_bindings`, resets the flag), `mod.rs` (the two fields + init + cross-pass reset). Full P1.2
diagnosis on the loft2 agent's task #2.

**Rebased onto main `67710d7d` (2026-06-22): the substrate work MERGED** — #426 + #429 resolved,
the reference-default + `&`-to-reassign design documented (C77, OWNERSHIP_MODEL). Clean rebase,
build green; P1 **vector** case works on both backends (`cells = &vv[0]; cells[0] = 99` →
`vv[0][0] == 99`).

**The struct-field case gates on the #415 REVERSAL — not on P1, and not on the merge (which
happened).** `cells = &chunk.cells; cells[0] = 99` does NOT write through on main (the struct field
stays `10`/`20`, both backends) because **#415 makes a struct vector-field read COPY on bind**
(OWNERSHIP_MODEL.md:152) — a documented store-lifetime STOPGAP, narrowed to struct fields (vector
INDEX reads keep their view), that CONTRADICTS reference-default. The design's end state reverses it
(struct-field reads become VIEWS via dep-driven ownership — row 102), which is OWNERSHIP_MODEL /
substrate-stream work, NOT P1. Per the revised design (C77) **`&` means "reassigning writes back"
(P2) and is REDUNDANT for a field/element mutation** (the W4 lint) — so the struct write-through is
REFERENCE-DEFAULT, not `&`. The P1.2 verify (`&chunk.cells; cells[0]=99`) therefore exercises
reference-default struct-field aliasing, which can only pass AFTER the #415 reversal.

**P1's `&`-mechanism is DONE; P2 is the next actionable phase — it does NOT depend on #415.** P1
made `&` ACCEPTED + ALIASING on a local binding and flagged it in `amp_bindings` for P2 — complete
and correct (the vector case proves bind + alias). The struct cell of the P1.4 matrix gates on the
#415 reversal, so it cannot close on this stream. **P2** (non-`&` whole-binding reassignment → local
rebind; `&` → write back; P0 proved the migration empty → safe) changes REASSIGNMENT, not the
field-read copy, so it is the next actionable @PLN87 work on `main`. WIP committed on `tuxedo-work2`:
`operators.rs`/`expressions.rs`/`mod.rs` = P1; `main.rs`/`scopes.rs` = the throwaway P0 sweep (strip
before PR, D2).

## Phases
- **P0 — ✅ DONE (2026-06-22): the migration is EMPTY → P2 is SAFE.** Swept the whole
  ecosystem for heap-typed PARAMETERS reassigned wholesale (an IR `Set` on an argument
  slot, which propagates to the caller today) via a gated `scopes::check` instrument
  (`LOFT_SWEEP_P0=1 loft … / --tests …`). Result: **0** in the stdlib (`default/`),
  **0** across all 10 registry libs (cbor, crypto, web, server, time, random, regex,
  arguments, game_protocol, input), and **0** across the entire `zero-trust-shared-files`
  application (20+ packages incl. `server/core`). The only raw hits anywhere are
  crawler's `cube`/`plane`/`sphere` (3) — NRVO return-buffers (`fn cube() -> Mesh`
  builds + RETURNS a Mesh), safe by construction: the caller receives the value via the
  RETURN, not param propagation, so P2's local-rebind change cannot break them. **No code
  relies on non-`&` reassignment propagating — the load-bearing P2 risk is retired.**
  (The sweep instrument is throwaway; remove before the PR.)
- **P1** — `&` on local bindings (additive, non-breaking).
- **P2** — reassignment-locality (**breaking**: non-`&` reassignment → local rebind; `&` writes back).
- **P3** — W4 redundant-`&` lint.

## Concerns (detail in the issue)
P2 breaking-change risk (P0 first) · the `&vector` realloc edge (LOFT.md:1529 may be stale) ·
both-backends parity · borrow-checker integration (read the carried `deps` fact, don't
re-derive) · scalar-vs-heap `&` distinction · store-lifetime safety (the @PLN85 fixes stay
green) · out of scope: partial-move / copy-on-write · dogfood against a real heavy-mutation
consumer.

## Implementation steps (verifiable)

Discipline for EVERY step: small + reviewable; lands GREEN on **both** backends (interp ==
native, concern 3); READS the carried `deps` borrow fact rather than re-deriving per site
(concern 4); keeps `tests/scripts/85-store-lifetime-*` green (concern 6). Each step names a
runnable gate — graduate the passing probe to `tests/scripts/87-*.loft`. Land order: P1 → P3.0
(realloc edge) → P2 → P3.

### P1 — `&` on local bindings (additive, non-breaking)

`&` is load-bearing only for scalar params today; on heap it is a no-op (struct/vector are
already reference-by-default). P1 makes `&` ACCEPTED + ALIASING on a binding RHS; it does NOT
change reassignment (that is P2), so P1 is behaviour-preserving for existing code.

- **P1.1 — Parser accepts `&<lvalue>` on a binding RHS.** Extend the assignment-RHS parse
  (`parser/expressions.rs`) to accept `x = &<binding|field|element>`, recording `x` as a borrow
  of the source — the existing `&`-param notation, now at a local.
  *Verify:* `chunk = Chunk{cells:[1,2,3]}; cells = &chunk.cells` parses with NO error
  (parse-success test); a fresh-temp `&mk()` is handled by P1.3, a scalar `&5` stays as the
  scalar by-ref form (concern 5).
- **P1.2 — Codegen aliases the `&`-binding by reusing the `&`-param lowering.** `x`'s type carries
  the borrow dep `{source}`; codegen views the source store (no copy), reading the carried `deps`
  fact — NO new lowering, reuse the `&vector`/`&struct` param aliasing at the local site.
  *Verify (both backends):* `cells = &chunk.cells; cells[0] = 99; assert(chunk.cells[0] == 99)`.
- **P1.3 — Borrow check rejects `&` to a non-outliving source.** Reuse the source-outlives-binding
  inference the `&`-param already relies on (no lifetime annotation — C38 declined reference
  *types*; this is a binding *notation*, concern 4).
  *Verify:* `o = &mk()` (fresh temp) → load-time error naming the dangling source; `o = &existing`
  (outlives) → admitted.
- **P1.4 — P1 gate matrix → graduate.** `x = &<src>; <mutate x>; assert <src sees it>` ×
  {struct field, vector, nested-field} × {interp, native}, plus a `cells = chunk.cells` (no-`&`)
  control (unchanged). → `tests/scripts/87-p1-amp-local-binding.loft`.
  *Verify:* matrix green both backends, no leak (`LOFT_STORES=warn`).

### P3.0 — Pin the `&vector` realloc edge (do BEFORE P2/P3; concern 2)

`LOFT.md:1529` claims `&vector<T>` is needed for `+=` to propagate, but a grow seems to propagate
without `&`. Settle it with a realloc-FORCING grow on both backends.
- *Verify:* `v = &src.vec; for … { v += [..] }` past the inline capacity; `assert(len(src.vec)`
  grew on both backends. If `&` is NOT needed → `LOFT.md:1529` is stale, fix it. If a realloc cell
  DOES need `&` → record the cell so **P3.1 never flags it**.

### P2 — reassignment-locality (BREAKING — de-risked: P0 migration is empty)

Changes EXACTLY one cell of the issue matrix: "heap reassign, no `&`" propagate → local rebind.
Field/element writes are UNCHANGED (still write through).

- **P2.1 — Non-`&` whole-binding reassignment → LOCAL REBIND.** At the heap-binding assignment
  codegen site, a non-`&` `o = X` allocates a FRESH store for `o`, leaving the source untouched
  (today it overwrites the source in place).
  *Verify (both backends):* `fn f(o: Obj){ o = Obj{x:9} }  a = Obj{x:1}; f(a); assert(a.x == 1)`
  (caller UNCHANGED, was 9 pre-P2); control `fn g(o){ o.x = 9 }` still propagates (`a.x == 9`).
- **P2.2 — `&` whole-binding reassignment → WRITE BACK.** A `&` binding/param reassignment writes
  the new value into the source store — `&` now means "reassigning this binding writes back",
  uniform across scalar and heap.
  *Verify (both backends):* `fn f(o: &Obj){ o = Obj{x:9} }  a = Obj{x:1}; f(&a); assert(a.x == 9)`.
- **P2.3 — Store-lifetime safety (concern 6).** The P2.1 fresh-store rebind drops the source view;
  the old store's ownership/free must stay sound.
  *Verify:* `tests/scripts/85-store-lifetime-*` green after P2.1/P2.2 both backends; P2 matrix
  leak-free (`LOFT_STORES=warn`).
- **P2.4 — P2 gate matrix → graduate.** `fn f(o){o=X}` no-`&` → caller unchanged; `&` → caller
  sees X. × {param, local} × {struct, vector, scalar} × {interp, native}. →
  `tests/scripts/87-p2-reassign-locality.loft`. *Verify:* green both backends, no leak.

### P3 — W4 redundant-`&` lint (the @PLN46 warning family)

- **P3.1 — Detection: redundant `&` = never reassigned, HEAP only.** A `&` binding/param is
  redundant iff its body field/element-mutates but never REASSIGNS it (reuse the reassignment
  detector from the P0 sweep). Scalar `&` is always needed → never flagged (concern 5). Exclude
  any realloc cell P3.0 found load-bearing.
  *Verify:* `fn f(o: &Obj){ o.x = 1 }` → flagged; `fn f(o: &Obj){ o = X }` → NOT flagged;
  `fn f(o: &integer){ o = 5 }` → NOT flagged.
- **P3.2 — W4 warning emission.** `Level::Warning` with actionable text ("`&` here has no effect;
  `&` only matters when you reassign the binding"); silenceable via the warning-suppression flag.
  *Verify:* warning + message for the redundant case; a `tests/runtime_warnings.rs` W4 case.

### Close-out

- **D1 — dogfood (concern 8).** Validate `&`-binding ergonomics against crawler's nested-mutation
  world — does `&` read naturally at depth, or awkward? Surface the signal, don't paper over.
- **D2 — strip the throwaway P0 sweep** from `scopes::check` (the `LOFT_SWEEP_P0` instrument)
  before the PR.
