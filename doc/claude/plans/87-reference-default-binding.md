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

## Status & handoff (2026-06-22; P2 start 2026-06-23) — READ FIRST after a context reset

**P0 ✅ DONE + committed.** Migration sweep across the whole ecosystem (stdlib + all 10 registry
libs + the entire zero-trust-shared-files app) found **zero** real propagation-reliance sites →
**P2 is safe**. (Verdict in § Phases below. The gated `LOFT_SWEEP_P0` sweep instrument has been
removed — D2 done.)

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

### P2 — STRUCT + SCALAR cells DONE on both backends (2026-06-23); vector cell remains

**Struct reassignment-locality is implemented, sound, and verified interp == native.** Baseline was
`9 9` (both overwrote — `&` on heap was a no-op); now `1 9` (non-`&` rebinds locally, `&` writes
back). `tests/scripts/87-p2-reassign-locality.loft` is the gate (param/local × struct/scalar,
conditional, repeated rebind, field-then-rebind) — green on both backends and leak-free under
`loft_suite`'s program-exit leak gate (interp) + `--native` (native). The `&`-writeback cell is
EXCLUDED from the gate: its behaviour is correct but the path leaks (see remaining work #1).

**The mechanism (witness + distinct-guarded free — sound across conditional/repeated rebind).**
The naive "null the param slot so `OpDatabase` allocs fresh" is UNSOUND: codegen's owned pre-Set free
then frees the CALLER's store, and the fresh store leaks (store-table exhaustion in a loop). The fix
treats a wholesale-reassigned heap param as an ownership TRANSITION (borrow→owned), guarded at runtime:
- **Detect** (`parser/objects.rs::parse_object`): a struct literal assigned to a user-visible heap
  param — `is_argument && !is_compiler_generated && !is_hidden_param` (the last excludes NRVO
  return-buffers like `result`, which are compiler-promoted but user-named, so keep their in-place
  write). A `&`/`RefVar` param never reaches this branch (`type_matches` is false) → it keeps the
  existing write-back path (P2.2; behaviour correct, but see leak below).
- **Witness** (`Function::rebind_orig` map + `ensure_rebind_witness`): a skip-free + inline_ref
  `__ref_N` work-ref holding the param's caller-supplied `DbRef`.  inline_ref is LOAD-BEARING — it
  makes the witness's entry null-init lower to the non-allocating `OpInitRefSentinel`; a plain owned
  ref would `OpInitRef`→`null()` a kt=65535 store that the stash then orphans (the leak `loft_suite`
  caught — the standalone binary's leak check did NOT, only the harness's cloned-DB run did).
- **Entry stash** (`parser/expressions.rs` preamble): `Set(__orig, Null)` (the inline_ref sentinel
  first-def, for slot assignment) then `OpPutRef(__orig, param)` — a RAW DbRef copy (same store_nr),
  NOT `Set(__orig, param)` which DEEP-COPIES into a fresh store and defeats the distinctness check.
  Snapshots param's ENTRY store; later rebinds move param's slot but not the witness.
- **Rebind** (parser emits, both backends share the IR): `OpFreeRefIfDistinct(o, __orig)` (frees a
  PRIOR rebind store; no-op on the first, `o == __orig`) → `OpInitRefSentinel(o)` (null slot, no free)
  → `OpDatabase(o)` (fresh store).
- **Exit free** (`scopes::check::get_free_vars`, `to_scope == 1` = the function-exit hook every
  `return`/tail routes through; args are otherwise excluded from the free sweep): `OpFreeRefIfDistinct(o,
  __orig)` — frees the rebound store iff distinct from the caller's original. Sound for conditional
  (untaken path: `o == __orig`, no-op) and repeated rebind.
- **Native twins** (`generation/ops/ref_ops.rs` + registered in `ops/mod.rs`): `OpInitRefSentinel`
  (`var = DbRef::NULL`) and `OpPutRef` (`var = value`) were interp-only opcodes; added emitters so the
  shared IR compiles on `--native`. `OpFreeRefIfDistinct` + `OpDatabase` already had emitters.

Whole change is **gated on the rebind detection** (off for all existing code — P0 proved heap-param
reassignment is ~nonexistent), so regression risk to the store-lifetime machinery is contained.
Verified: `issues.rs` 723 pass, `loft_suite` green (incl. the new gate), both backends.

### Method note — when iterating through these multi-pass areas, ALWAYS diff against `main`

The parser is two-pass and the store-lifetime codegen has many interacting sites, so the same area gets
re-read many times and a symptom is easy to mis-attribute to the in-flight change. **When a problem
pops up, compare against `main` first to separate NEW breakage from PRE-EXISTING** — it repeatedly
short-circuited dead ends here (the struct `&` write-back "leak" looked like a P2.1 regression until a
main diff showed the `SetStackRef` path is untouched and the leak is pre-existing; the standalone
binary vs `loft_suite` divergence was another). Concretely:

- **Behaviour / leak:** build a `main`-tip `loft` and run the same probe both ways. `scripts/probe-matrix
  --baseline <main-binary>` auto-classifies each cell REGRESSION vs PRE-EXISTING (CLAUDE.md § matrix-first).
- **IR / bytecode:** `LOFT_LOG=static loft --dump prog.loft` on each binary and diff; or read the source
  diff directly — `git diff main -- <file>`, `git show origin/main:<file>` — to confirm a path is in
  fact unchanged (NO working-tree switch — commit WIP first; CLAUDE.md § Git safety).

If `main` shows the same symptom, it is pre-existing: file it (a `main` bug) or work it as its own item,
don't fold it into the active fix.

**P2 CONSISTENCY MATRIX — ✅ COMPLETE (2026-06-23), both backends, leak-free.** Every cell of
`{struct, vector, scalar} × {non-&, &} × {reassign, field/element}` now behaves uniformly and is pinned
green in `tests/scripts/87-p2-reassign-locality.loft` (interp + `--native` + `--native-wasm`, under the
program-exit leak gate) with the three former gaps' lock-ins un-ignored:

1. **P2.2 `&` write-back leak — ✅ DONE.** `fn f(o: &Obj){ o = Obj{..} }` writes back (`g.x == 9`) AND
   frees the displaced caller store. The RefVar-set lowering frees the OLD `*o` store before installing
   the new value, read LIVE through `o` (interp `OpGetStackRef`+`OpFreeRef` at `codegen.rs`; native
   `OpFreeRef(cell, *var_o)` at `dispatch.rs`) — PATH-SENSITIVE, so conditional `&` reassignment is sound
   with no witness; the transferred construction temp is `skip_free` (the caller owns + frees it). Gated:
   arg + RefVar heap inner + the RHS's `result_var` is the skip_free transfer temp. Lock-in
   `pln87_struct_amp_writeback_is_leak_free`.
2. **Vector non-`&` rebind — ✅ DONE.** `fn f(v: vector<T>){ v = [..] }` REBINDS locally. The `=`-vs-`+=`
   distinction is unavailable at the vector materialiser, so captured EARLIER: in `parse_assign_op`,
   BEFORE the RHS parse, a `=`-to-a-visible-vector-param with next token `[` (peek) marks the param a
   rebind; the RHS parse's `vector_db` then hands it a FRESH `__vdb` backing (skip_free; freed at exit by
   the param's `OpFreeRefIfDistinct(v, witness)`). `+=` / self-concat / `v = other` keep the caller
   backing. Lock-in `pln87_vector_param_reassign_is_local`.
3. **Vector `&` write-back — ✅ DONE.** `fn f(v: &vector<T>){ v = [..] }` WRITES BACK. A `&`-vector ref
   shares the caller's backing and cannot repoint, so the write-back is a CLEAR + refill IN PLACE:
   `parse_assign_op` prepends `OpClearVector(v)` before the literal (which appends to the caller's store)
   → replace, not grow. `+=` keeps the grow. Lock-in `pln87_vector_param_amp_writes_back`.

**REMAINING (not P2-core):**
- **P3 — ✅ IMPLEMENTED, OPT-IN (`LOFT_WARN_REDUNDANT_AMP=1`).** The W4 lint flags a `&` on a heap
  STRUCT param that the body never reassigns (field mutation propagates regardless; `&` only matters for
  write-back). Excludes `self` (the method-receiver convention), scalar `&` (always load-bearing), hidden
  return-buffers, and never-read params (covered by `test_used`). `parser/operators.rs::warn_redundant_amp`,
  called per-fn from `definitions.rs`; tests in `tests/runtime_warnings.rs` (`w4_*`). **Kept OPT-IN**
  because reference-default (P2) only just made the pattern redundant, so ~20 `&` ref-param regression
  tests + ~9 scripts still use it intentionally — enabling by default flags them all at once. **Follow-up:
  an ecosystem cleanup pass (modernise / acknowledge those usages), then flip W4 on-by-default + silenceable
  per the original P3.2 spec.**  Vector/Enum inners and the `&vector` realloc edge (P3.0) are out of this
  first cut.
- **D2 — ✅ DONE** — the throwaway `LOFT_SWEEP_P0` instrument is removed from `scopes::check` + `main.rs`.

**Post-merge probing — `&` write-back RHS shapes (2026-06-23).** The P2 matrix tested the LITERAL
forms only; probing the others found two gaps and a latent concern:
- **#1 (was a LEAK) — ✅ FIXED via clean rejection.** `&`-struct write-back from a CALL (`o = mk()`)
  or VARIABLE (`o = src`) leaked the displaced caller store (the P2.2 displaced-free fires only when
  the RHS `result_var` is the skip_free construction temp, i.e. only `o = Obj{..}`). A `Block`/`Insert`
  temp-wrap to reuse that path collided with fragile RefVar-assignment transforms (the `Set(o,…)`
  write-back got dropped), so full ownership-transfer support is DEFERRED. Until then these shapes are
  **rejected at parse time** with a clear message (no silent leak): `parser/expressions.rs`,
  gated on `RefVar(Reference|Vector)` + non-skip_free RHS. Lock-ins: `parse_errors`
  `pln87_amp_writeback_*_rejected`, `leak::pln87_struct_amp_literal_writeback_no_leak`, and the
  ignored `issues::pln87_amp_writeback_from_call_writes_back` (flips to PASS when full support lands).
- **#2 (was a WRONG MESSAGE) — ✅ FIXED.** `&vector = otherVector` reported the misleading
  "`&` but is never modified"; now the same clear write-back-not-supported error. The duplicate
  "never modified" is suppressed via `Parser::writeback_rejected` (cleared per function).
- **#3 — leak-detection divergence (OPEN, investigating).** The standalone binary's exit leak-check
  under-reports leaks the cloned-DB harness (`loft_suite`/`leaks_for`) catches; the harness is the
  authority used throughout P2. Root-causing why the binary misses them.

Runnable probe (the behavioral pair — now prints `1 9` on both backends):

    struct Obj { x: integer }
    fn f(o: Obj)  { o = Obj { x: 9 } }    // non-& param: REBINDS local → a.x == 1
    fn f2(o: &Obj){ o = Obj { x: 9 } }    // &-param: write-back → a2.x == 9
    fn main() { a = Obj{x:1}; f(a); a2 = Obj{x:1}; f2(&a2); print("{a.x} {a2.x}\n"); }

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
  (The sweep instrument has been removed — D2 done.)
- **P1** — `&` on local bindings (additive, non-breaking).
- **P2 — ✅ COMPLETE both backends, leak-free (2026-06-23).** reassignment-locality (**breaking**:
  non-`&` reassignment → local rebind; `&` writes back), uniform across struct / vector / scalar; the
  full consistency matrix is green in `tests/scripts/87-p2-reassign-locality.loft` (see § P2 above).
- **P3 — ✅ IMPLEMENTED (opt-in `LOFT_WARN_REDUNDANT_AMP=1`; on-by-default pending an ecosystem
  cleanup pass).** W4 redundant-`&` lint.

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
- **D2 — ✅ DONE — stripped the throwaway P0 sweep** (the `LOFT_SWEEP_P0` instrument) from
  `scopes::check` and `main.rs`.
