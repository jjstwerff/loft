<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_REDFLAGS.md — the re-derived facts a stable future must compute once

> **This is a forward-stability map, not a fix-now list.** It names the
> implementations that will keep manufacturing bugs, grouped by the **missing
> fact** each one re-derives — so the path to a stable future is "land the fact
> once; N forests collapse together," not "patch N sites." Companion to
> [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (the H-register) and
> `OWNERSHIP_MODEL.md` (the ownership holes table); this doc is the cross-cutting
> *red-flag* view those two are read through. This map is scoped to
> runtime/memory/codegen; the FRONT-END rough spots it structurally misses — the
> typing/conversion relation (where #432/#433 live) and grammar precedence — are in
> [FORMALIZATION.md](FORMALIZATION.md) (the formal-definition-as-lens companion).
>
> **Two referenced docs — `OWNERSHIP_MODEL.md` and `CODEGEN_METHOD.md` — are the
> ownership-migration docs from the active store-lifetime plan (`@PLN85`) and are
> not yet merged into this tree; they are referenced by name (no local link) until
> they land here.**

## The one thesis

Per `CODEGEN_METHOD.md`: **codegen re-deriving a non-local fact
(ownership, transfer/borrow, null-encoding, container layout) per call-site is a
diagnostic of a missing type-system fact — fix the fact, not the generator.** A
red flag here is therefore not "ugly code"; it is a *fact computed in the wrong
place / N times*. The test for every entry below is the method's own:

> **Would this code collapse to a simple read if the type carried one more fact?**
> If yes → the fact belongs on the type, and every re-derivation site is a sibling
> of one bug class.

The rigor failure modes ([engineering-rigor skill](../../.claude/skills/engineering-rigor/SKILL.md)):
**under-reach** (N mechanisms for one family — a spray), **over-reach** (one
mechanism forced over N families — a false invariant), **wrong-signal** (a decision
keyed on a type where the authority is a runtime delta), and
**defensive-at-manifestation** (guard the symptom, leave the root).

**Survey provenance:** four parallel read-only rigor-audits, 2026-06-21, against the
`../loft` `plan-85-store-lifetime-retirement` tree; line numbers below are grounded
in **this** tree (loft2, `tuxedo-work2`) where confirmed, else cited by function.
The *patterns* are durable; re-confirm exact lines before acting.

---

## The map — five clusters, ~3 missing facts

| Cluster | Missing fact | Re-derivation sites | Failure mode | Tracking |
|---|---|---|---|---|
| **A** Return/bind ownership | return-dep: owned/transferred vs borrows-{source} | ~10 | under-reach + wrong-signal + divergence | OWNERSHIP_MODEL 99/102/103/104 (mostly OPEN) |
| **B** Stack-balance signal | net stack delta (not the last-expr type) | 2 (+1 fixed) | wrong-signal | NEW (gen_if `null_else` already fixed; siblings open) |
| **C** Container taxonomy | a `for_each_owned_child` traversal keystone + per-kind descriptor | ~27 + ~20 | under-reach (per-kind spray) | **NEW** (H7-sibling on the heap-cascade side) |
| **D** Null-sentinel codec | the `sentinel(tp)` encode/decode table (H6) | ~3 sites × 3 | under-reach (one fact, drifting copies) | H6 (consumers unconverged) |
| **E** Manifestation guards | — (downstream of A) | 4 | defensive-at-manifestation | #405 / @P290 / @P317 / @P377 |

Clusters **B/E are mostly *consequences*** of A and C: a missing ownership fact is
why the stack and the free-path get guarded at the symptom. Landing **A** (return
ownership) and **C** (the traversal keystone) is what retires the bulk — D and E
largely dissolve behind them.

---

## Cluster A — the return/bind transfer-vs-borrow fact (cluster-II root)

**The fact:** every heap value's binding should read one carried answer — *does this
value OWN its store (move it), or BORROW another's (copy on bind/escape)?* Today
that answer is re-derived from accreting heuristics (`has_ref_params`, `is_argument`,
`vector_bound`, `dep.is_empty()`, runtime store-nr witnesses) at **~10 independent
sites**, which is why the same class produced #405/#406/#409/#410 this cycle and why
the `a = x.v` field-read aliasing was still open after the `return v` fix.

Re-derivation sites (loft2 tree):

| Site | loft2 location | Smell | Mode |
|---|---|---|---|
| BlockTail/MidReturn/native-forwarder return thicket | `src/parser/control.rs` (`block_result`, `ref_return`) | per-callee-shape adopt/copy/borrow arms + a `RetSite` fork | under-reach |
| `returned_var` single-`u16` walk | `src/scopes.rs:2285` | differing match/if arms collapse to "no return var" → arm buffers freed | **over-reach** (false invariant: ≤1 return source) |
| return-save-to-temp forest (B5-L3) | `src/scopes.rs` (`free_vars`) | ~5 type-keyed save-temp shapes each deciding "hoist before frees?" | under-reach |
| `has_ref_params` adopt-vs-copy | `src/` — **11 sites** (`grep has_ref_params`), incl. `state/codegen.rs` ×2 + `scopes.rs` | a heuristic standing in for "transfer or borrow?"; admits "cannot resolve statically" → runtime `OpFreeRefIfDistinct` | wrong-signal |
| `is_borrowed_view` — computed **twice, divergently** | `src/state/codegen.rs:1716,2441` (interp) **vs** `src/generation/dispatch.rs:178` (native) | the same `0x8000` source-free bit, two different structural derivations → drift on the OOB/hidden-only edge | **divergence** (H4) |
| bind-site owned-vs-borrow | `src/parser/expressions.rs` (`assign`) | per-RHS-shape copy decision (bare Var / borrowing Var / field-read / non-Var) | under-reach |
| reassign free-strategy forest | `src/state/codegen.rs` (`gen_set_first_at_tos` region) | `is_hidden_buf_arg && owned_ref && rhs_reads_v && …` — the canonical `has_ref_params && …` forest | under-reach |
| scan_set paired-witness | `src/scopes.rs` (`scan_set`) | emits a **runtime** store-nr comparison because the static fact is absent | re-derived → runtime |

**`dep.is_empty()` also HIDES the class from one backend** (measured, loft#882). Empty
deps mean OWNED to `--native`'s assignment lowering, so a read whose dep is *missing*
(not absent-because-owned) gets a defensive `OpDatabase` + `OpCopyRecord` and the program
comes out right; the interpreter aliases and reads freed bytes. The keyed-element borrow
scored `--interpret` 6/17 boundary cells against `--native` 14/17 **on the same IR**. Two
consequences for anyone working this cluster:

- a store-lifetime matrix that is lopsided between backends is evidence of a missing dep,
  not of a codegen bug — read the emitted Rust and look for an `OpCopyRecord` beside a
  plain element read;
- a `--native` PASS is not evidence the dep is present, so every ownership probe has to
  run `--interpret` under `LOFT_POISON=1` as well. The defensive copy is also why this
  class survives a green suite: it is a silent perf cost on one backend and a
  use-after-free on the other.

**Twins drift exactly on the input nothing tests.** `State::append_copy` and
`codegen_runtime::OpAppendCopy` disagreed on a NEGATIVE count (`--native` clamped, the
interpreter cast `as u32` and walked off the store until glibc aborted) and on whether to
re-read the backing record after a resize. A twin gets hardened where its bug was
*observed*, and an observation happens on one backend. When touching one twin, diff it
against the other line by line, and land every new boundary row in the shared `.loft`
guard so both backends run it — a "keep the two in step" comment is not a gate.

**Would-one-fact-collapse-it?** Yes — all of the above are the OWNERSHIP_MODEL
remedy verbatim: *return-dep empty ⇒ adopt; `{Attr(src)}` ⇒ copy* (row 102), plus
the *return-source SET over arms* (row 99) and *one funnelled return path* (row
104). Landing the carried return-ownership dep collapses these ~10 sites and
deletes the runtime witnesses. **This is the single highest-leverage migration in
the tree** (most-reused decision; the documented cluster-II root).

---

## Cluster B — stack-balance keyed on type, not runtime delta

**The fact:** "did this branch leave a value on the eval stack, and how many bytes?"
is a **runtime stack-position delta**, never the last-expression *type*
(`generate_block` reports the last expr's type, not the net push — a value-typed
but stack-neutral tail op like `OpAppendVector` pushes nothing).

- ✅ **Fixed precedent:** `gen_if`'s `null_else` gate now reads `true_stack !=
  stack_pos` (the #405 fix). It is the template for the rest.
- 🔴 **`gen_if` B5 rebalance** — `src/state/codegen.rs:841`: uses
  `size(def(self).returned())` (the *function's* return type) as the bytes-to-
  preserve, not `size(tp)` (the if-**expression's** result type). Wrong whenever a
  non-tail value-`if`/`match` with eval-stack-divergent arms has result type ≠ the
  function return. **Masked** today because its only natural trigger is recursive
  functions where `tp == returned()`; latent, not dead.
- 🟡 **`size_code` If/divergent arm** — `src/stack.rs` (`size_code`): an `if`'s
  drop-size is read from the then-arm's static type; a divergent then-arm falls to
  `0` while the else-arm pushed a value. Same wrong-signal family; uncommon shape.

**Cleared (right signal, not red flags):** fn-ref `step(20)`/`step(16)` and callee
`size(op.returned())` advances — there the type *selects the opcode that pushes
exactly those bytes*, so the type IS the authority.

---

## Cluster C — the per-Type-kind container taxonomy (the heap-cascade keystone)

**The fact:** "to traverse / construct / free a collection's owned nested heap —
which element type, at which stride, with which container walk" is ONE per-`Parts`
descriptor. Today it is hand-re-encoded across the family below — the **densest bug
cluster in the tree** (the @P29x/@P3xx history). <!--noindex-->

| Family | loft2 location | Count | Named bugs from the drift |
|---|---|---|---|
| `copy_claims` / `remove_claims` / `validate_claims` triad | `src/database/allocation.rs:1374 / 1682 / 984` (+ `copy_claims_{seq_vector,array,hash,index}_body` at `:1119/1163/1233/1303`) | 3 dispatchers × ~9 `Parts::` arms ≈ **27**, already drifting (copy has 4 helpers; remove/validate don't) | @P290 (SIGSEGV, `room*2` vs `(room-1)*2`), @P306/@P318 (hash slot-drift), @P309 (missing length header) |
| `record_new` / `record_finish` / `insert_record` construction | `src/database/structures.rs` | 3 fns × ~8 arms; **3 independent encodings** of "element-record word count" | @P309 class |
| `gen_set_first_*_null` codegen family | `src/state/codegen.rs:1091/1143/1303/1378/1390` + the multi-arm `gen_set_first_at_tos` ladder | 5 fns + ladder + 3× copy-pasted `sentinel \| owned-init \| borrowed-view` tri-state | #260/#330 class (wrong null-init → leak / use-before-init) |
| keyed `Type::{Sorted,Hash,Index,Spacial} → database.{kind}` re-dispatch | `src/state/codegen.rs` + `src/parser/vectors.rs` + `src/generation/dispatch.rs` (+more) | same 4-arm block in **≥4 files**, interp/native already shaped differently | interp/native drift (H4) |
| `Stores::{vector,hash,sorted,index,spacial,child_rec}` constructors | `src/database/types.rs` | 6–7 near-identical intern-or-push bodies + **3× key-resolution drift** (`hash` resolves vs `key_owner`, `sorted`/`spacial` vs raw content) | latent @PLN25 nullable-key bug |

**Would-one-fact-collapse-it?** Yes — a single `for_each_owned_child(tp, rec) ->
Iterator<(child, child_tp)>` keystone (the per-`Parts` walk as a carried fact), with
copy/free/validate/construct as thin visitors over it. This is the `for_each_child`
keystone **H7** already names for *codecs* — here on the heap-*cascade* side, and
**not yet a tracked H-row** (H3's pass-2 explicitly scoped itself to `scopes.rs`
free-*placement*, away from this traversal cascade). **Highest-leverage NEW finding.**

---

## Cluster D — the null-sentinel codec (H6 consumers, unconverged)

**The fact:** the per-width null encoding (`u8`→255, `u16`→65535, `*Raw`→`i32::MIN`,
…) is **one** `sentinel(tp)` table. It is still hand-inlined at read AND write sites
beyond the two H6 already unified:

- `is_null_field` — `src/database/types.rs` (3 inline narrow arms).
- `set_default_value` — `src/database/structures.rs` (the write twin).
- `walk_parsed_into` (JSON path) — `src/database/structures.rs` (a third copy).

This is H6's thesis exactly ("one width-fact, N drifting copies" — already cost the
`389-h6` nullable-narrow bug). Tracked under **H6**; these specific consumers are the
named-but-unconverted ones. Lands behind the staged `NullEnc`/`sentinel(tp)` table.

**Landed — the heap-ref + character facet (Cluster D D.1/D.2).** The four named
typed-null encoders (`write_typed_null` native, `emit_typed_null` interp,
`STRING_NULL`, `init_ref_sentinel`) are converged:

- **Heap-ref null = one source, `DbRef::NULL` (`keys.rs`).** Every heap-ref null
  encoder (native `write_typed_null` / `default_native_value` / `dispatch` / `calls`
  / `coroutine`, the `codegen_runtime`/`parallel`/`structures` runtime writers, and
  the interp `init_ref_sentinel` / `null_ref_sentinel`) read the single
  `DbRef::NULL` const instead of re-spelling `DbRef { store_nr: u16::MAX, … }`. The
  drift was a mixed `pos: 0` (interp/canonical) vs `pos: 8` (native) literal —
  semantically inert (`is_null()` keys off `store_nr`, ignores `pos`) but real byte
  drift, now gone. Round-trip matrix byte-identical on both backends; regression
  `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- **Character null H4 cell, found + closed.** `Parser::null` folded `character`
  into `OpConvIntFromNull` (the i64 integer sentinel), so `-> character { return
  null; }` emitted `return i64::MIN` into an `i32` return slot — native rustc E0308,
  interpreter tolerated. Now routes to `OpConvCharacterFromNull` (char-domain null
  `'\0'`), correct on both backends.

**Deliberately LEFT (distinct representations, not the same fact).** `STRING_NULL`
(a text `Str` sentinel `"\0"`, already a single `const` read by all text-null sites)
and the interp `Reference` path's `database.null()` (which allocates a real null
*store*, a different runtime mechanism than the `DbRef::NULL` sentinel) — forcing
either onto `DbRef::NULL` would be a false merge. **Still open (the narrow-width
facet):** `is_null_field` / `set_default_value` / `walk_parsed_into` per-width arms
above — a separate `IntegerSpec::range_to_width` sub-thread, not the heap-ref one.

---

## Cluster E — defensive-at-manifestation guards (downstream of A)

These guard the *symptom* of the missing ownership fact rather than preventing the
bad value. They are correct stopgaps; the stable future **retires** them when A lands
(and a `debug_assert` that documents a contract is the GOOD form — keep those).

| Guard | loft2 location | What it hides | Tracking |
|---|---|---|---|
| `free_named` OOB-refuse | `src/database/allocation.rs:191` | a wrong/stale free with an out-of-range `store_nr` — refused (release) instead of not-produced | #405 (cluster II) |
| `free_protected` call-bracket | `src/database/allocation.rs:680` + `lock_store` | "safety net for `is_borrowed_view` mis-detection" (its own @P290 words) — runtime locking to stop the wrong free landing | @P290 (retire when dep inference is complete) |
| `["??"]` one-buffer return marker | `src/generation/mod.rs:714`, `src/generation/dispatch.rs:173` | a placeholder dep standing in for the unresolved return-ownership fact | OWNERSHIP_MODEL `"??"` row |
| `n_protect_store_frees` `rec != 0` guard | `src/native.rs` (per @P377) | a half-state `free_protected` leak across `unlock`/`init` | @P377 |

---

## Landing order (leverage-first — the stable-future roadmap)

> **Site-level steps + per-step verification gates for each cluster below:**
> [STABILITY_REDFLAG_REMEDIATION.md](STABILITY_REDFLAG_REMEDIATION.md) — the
> actionable *how* to this map's *what*.

1. **Cluster A — carried return/bind ownership dep** (OWNERSHIP_MODEL 99/102/104).
   Collapses ~10 forests, deletes the runtime witnesses, and **dissolves Cluster E**
   and the `is_borrowed_view` divergence. The most-reused decision; do first.
   *Prereq already laid:* typed `Deps` (H2 / DEPS_INVENTORY.md).
2. **Cluster C — the `for_each_owned_child` traversal keystone.** The densest bug
   history and entirely NEW; one keystone fixes copy/remove/validate +
   construction + the null-init family together. Independent of A — can proceed in
   parallel.
3. **Cluster B — apply the `true_stack`-delta template to `gen_if` B5 + `size_code`.**
   Small, bounded; the #405 fix is the worked precedent.
4. **Cluster D — converge the remaining H6 `sentinel(tp)` consumers.** S-sized;
   any gap.

Each is a *single fact computed once*, validated on **both backends** per
CODEGEN_METHOD — not a patch. The win is structural: a new collection kind / return
shape / narrow width then arrives *with its fact*, not as the next special case.

## What is NEW vs already-tracked

- **NEW (file a forward home / H-row when picked up):** the Cluster-C
  `for_each_owned_child` keystone (the copy/remove/validate cascade + construction
  triad + keyed re-dispatch), and the Cluster-B `gen_if:841` / `size_code`
  wrong-signal siblings.
- **Already tracked (this doc is the cross-cut, not a new filing):** Cluster A →
  OWNERSHIP_MODEL holes 99/102/103/104 + H3; the `is_borrowed_view` divergence → H4
  + DEPS_INVENTORY H2; Cluster D → H6; Cluster E → #405/@P290/@P317/@P377.

No bugs filed: open items map to existing OWNERSHIP_MODEL/H rows; the genuinely-new
ones are structural-debt forward risks (this doc), not `main`-reproducing defects.
