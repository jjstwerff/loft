<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Recent-bug evaluation (pre-probe analysis)

Input to Stage A: the four sev:high store-lifetime bugs of this cycle + the prior
finished investigation (@PLN51), evaluated for a shared mechanism. **Marks every
claim VERIFIED (issue body / cited code line / this session's work) or HYPOTHESIZED
(Stage-B probes must confirm or kill).** No probes run yet — this is the map that
tells Stage A which shapes to probe.

## Per-bug mechanism + fix site

| Bug | Trigger (boundary) | Root mechanism | Manifests at | Fix site | Root fixed? |
|---|---|---|---|---|---|
| **#405** | heap local, conditional × **unused** × nested loop | `init_create_stack` (`state/mod.rs:1854`) points the local's slot at its hidden-buffer **dep slot but never initialises the slot's content**; the dep ref is on the volatile per-iteration stack, reused across iterations → the `x=null` pre-Set free reads stale stack → frees a bogus `store_nr` | `allocation.rs:206` `free_named` (OOB / store-table exhaustion) | `25ce899` — *defensive* OOB-refuse in `allocation.rs` | ❌ **band-aid at the manifestation point**; the uninitialised-dep-slot root stands |
| **#406** | struct with **enum fields from a variable** (not literal), appended to a vector | `copy_claims` `Parts::Enum` (`allocation.rs:1449`) reads a **-1 (uninitialised) discriminant** for the variable-sourced enum field → `values[-1 as usize]` OOB | `allocation.rs:1449` `copy_claims` | `#412` `8f286e60` — patched `copy_claims` (+32) | ⚠️ crash fixed; **two-enum-field struct readback still corrupt** — layout root not fully closed |
| **#409** | loft **wrapper** forwarding an FFI (null-store) `vector<u8>` return; caller `+=` | local borrows the **foreign store**; its `__retbuf` stays empty; in-place `+=` rebuilds the empty buffer → drops the value | silent (len 4→1) / escalates to SIGSEGV | `parser/control.rs` (return-site materialise) | ✅ root, at the return chokepoint |
| **#410** | **direct** `#native`-decl FFI vector return; caller `+=` | same foreign-store borrow, no wrapper — local's `__vdb` dep names an **empty** buffer | silent drop | `parser/expressions.rs` (assignment-site materialise) | ✅ root, at the assignment chokepoint |
| **@PLN51 cluster II** (prior) | hidden buffer across loop iterations (double-Set, many-iters, **conditional set**) | cross-iteration hidden-buffer **slot dangling** | leak / corruption / teardown SIGSEGV | fixed via `OpInitRefSentinel` + narrowed `is_hidden_buf_arg`/`is_borrowed_view` | ✅ for the probed shapes |

## Finding 1 — #405 is an uncovered sibling of @PLN51 cluster II (VERIFIED)

@PLN51 cluster II already characterised "cross-iteration hidden-buffer slot dangling"
and closed it for *double-Set*, *many-iters*, and *conditional-set* shapes
(`tests/scripts/142/145/146`), with the cure being **`OpInitRefSentinel`** — i.e.
*sentinel-initialise the slot so a stale cross-iteration read is caught*. #405 is the
**(conditional × UNUSED × nested)** corner that escaped that probe net: the slot is
left uninitialised on the path where the buffer was never assigned. So the class was
not retired — the *probe coverage* missed a combination, and the *invariant*
(`OpInitRefSentinel` on every slot crossing a reuse boundary) wasn't applied on this
path. This is the investigation's premise made concrete.

## Finding 2 — manifestation is concentrated; roots scatter (VERIFIED)

Three of the four corrupt at **`src/database/allocation.rs`** — `free_named` (#405) and
`copy_claims` (#406); the FFI pair drop silently in the vector-rebuild path. These are
**lifetime operations that TRUST a slot** (a `store_nr` dep, an enum discriminant, a
buffer's contents). The *roots* are scattered **producer** codegen paths that fail to
establish what the consumer assumes: `init_create_stack` (#405), the foreign-store
return/assignment delivery (#409/#410), the enum-field struct construction (#406). Fixes
to date are therefore either **per-producer** (control.rs, expressions.rs — correct but
local) or **defensive-at-manifestation** (#405's OOB-refuse — doesn't fix the root).
Neither retires the class: a new producer that leaves a slot uninitialised re-opens it.

## Finding 3 — candidate shared invariant (HYPOTHESIZED — Stage B decides)

> **Every slot a lifetime operation (free / copy_claims / in-place rebuild) will read
> must be initialised to a valid value (a real store/discriminant, or a recognisable
> sentinel) by EVERY construction path that produces the record.**

All five fit as *violations of this one invariant by different producers*: an
uninitialised dep slot (#405), an empty dep buffer vs a foreign value (#409/#410), an
uninitialised enum discriminant (#406), a dangling cross-iteration slot (@PLN51-II).
**Open question for Stage B:** is #406 (enum discriminant inside `copy_claims`) the same
family, or a distinct record-layout mechanism (the enum field isn't *copied* vs not
*initialised*)? The matrix decides — do not assume.

## Implication for the fix design (HYPOTHESIZED)

Two structural options the evidence already frames, for Stage C:
- **(A) Manifestation-point sentinel/assert** — make `allocation.rs` free/copy_claims
  *debug-assert* every slot they read is an initialised-store-or-sentinel, panicking with
  the violating construction context. Turns silent corruption into a loud, located
  failure across the WHOLE class (the standing instrument generalising plan-51's
  `OpInitRefSentinel` + #405's OOB-refuse). Catches future producers automatically.
- **(B) Producer chokepoint** — funnel hidden-buffer/dep-slot creation through one path
  that always sentinel-initialises. Likely impossible to fully centralise (codegen
  producers are inherently many — the H1/H3 "scattered setters" problem), which is
  *why* (A) may be the realistic retire-the-class move: you can't dedup the producers,
  but you can make the consumer reject any producer that violates the invariant.

## Stage A probe targets (derived)

Port as assertion-bearing probes, both backends: #405 (+ its (conditional×unused×nested)
neighbours — vary each factor), #406 (+ the two-enum-field readback that's *still*
corrupt, + literal-vs-variable enum source), #409/#410 (foreign-store return × +=/read-
only/concat), and a **real-consumer** extraction (crypto/imaging FFI return, the cbor
map-decode `vector<CborEntry>` readback #406 blocks). Re-run @PLN51's cluster-II probes
for live siblings. Then test Finding 3's invariant against the full matrix.
