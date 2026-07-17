<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN06 — Simple typed `par`: everything is a store

## Status

**DONE 2026-05-09.**  Reference for the post-@PLAN06 surface
(par(...) syntax, par_fold, dispatcher inventory) lives in
[`../../../THREADING.md`](../../../THREADING.md).  The live
closure record is [ARC.md](ARC.md).  User-facing summary:
[`../../CHANGELOG_TECHNICAL.md`](../../../CHANGELOG_TECHNICAL.md)
under "Plan-06 (typed-par redesign) closed 2026-05-09".

This file is the closure record for the plan as a whole; the
phase files in this directory remain as historical archaeology.

### Final tally

- A1–A7 + A5b + A8.b + A11 shipped.
- A8 (Queue trait collapse in `src/parallel.rs`) deferred with
  audit — divergence is structural, not boilerplate.
- A9 superseded by A4 (light path retired entirely).
- A10 (browser parallel) out-of-scope; ships as its own arc.
- Ignored par canaries: 8 → 1.  Remaining is heterogeneous-
  vec-of-fn (D11a row 8), outside @PLAN06 scope.

### Closure commits

- `f974770` — closeout docs + A8 deferral marker + A9 superseded
- `15a7aab` — @P235 par half via wrapper synthesis (closes
  `par_tuple_destructure_in_for`)
- `bcac52f` — A8.b stitch_id consolidation in `src/native.rs`
  (~150 LOC saved)

## Goal (achieved)

Replace today's branching `par` runtime with one uniform path:
every parallel worker takes input as a Store, writes output
into its own output Store, and main-thread stitching
concatenates the per-worker output stores into a single result
Store.  No special cases for text, references, or primitives —
they all live in stores.

The reference content for the post-@PLAN06 architecture
("everything is a store", dispatcher set, par_fold sugar)
lives in THREADING.md.  The full pre-shipping rationale +
phase-by-phase design is in this directory's phase files
(00–10) and DESIGN.md, retained as historical archaeology.

## Realised value — bug discovery

Plan-06's headline metric is "~1100 LOC retired."  That undersells
the work: getting there surfaced **18+ P-issues** in the type-
system × native-codegen × parallel-runtime intersection that
would have hit users in their own threading code with much harder
reproducers than a curated canary.  The plan functioned as a
structured fuzz/bug-hunt of that intersection.

| P-issue | Surface | Title |
|---|---|---|
| P188 | keyed collections | `field += elem` on hash/index/sorted (compound-assign codegen) |
| P189 | par dispatch | typed wide-input dispatch + tuple-as-vector-element |
| P189c | par dispatch | vector<tuple> element write + light wide path |
| P190 | type registration | local-var sorted/hash/index types weren't registered on demand |
| P191 | keyed bookkeeping | `index<T[key]>` 4-byte int<0,false> (range-bookkeeping mismatch) |
| P192 | stdlib | `len()` overload missing for hash/index |
| P194 | tuple structs | tuple-struct field reassignment |
| P195 | lexer | chained tuple-index lex (`n.v.0.0`) |
| P196 | native codegen | tuple-of-fn-ref native codegen ((u32, DbRef) as i32) |
| P197 | dep tracking | dep-tracked text reads (bundled with P194) |
| @P198 | scan_set | unwrap Span in scan_set + native deep-copy emission |
| @P199 | native ABI | `&mut Stores` → `&UnsafeCell<Stores>` (3-commit fix series) |
| @P200 | native codegen | int compare emission (closed in @PLAN09; surfaced here) |
| @P201 | misc | branch-local regression (filed 2026-04-29) |
| @P234 | lexer/runtime | tuple-of-struct member access |
| @P235 | par half | tuple-destructure-in-for |
| @P236 | native codegen | heap-owned reference returns from if/else native data corruption |

Several (P191, P195, P196, @P198, @P199) are bugs that ordinary
doc-tests don't surface — they require specific type-shape
interactions that only the par() canaries forced.

## Phase index (historical)

The phase files in this directory are kept as the design and
implementation log.  ARC.md A-step → phase-file mapping is in
ARC.md itself.

| File | Topic |
|---|---|
| [00-baseline-and-bench.md](00-baseline-and-bench.md) | Characterisation suite + perf bench + D11 type-coverage tracker |
| [01-output-store.md](01-output-store.md) | Workers write to per-worker output Stores |
| [01.5-rayon-pool.md](01.5-rayon-pool.md) | Shared rayon pool via `parallel_workers` template |
| [02-stitch-not-copy.md](02-stitch-not-copy.md) | Main-thread stitch via store-pointer rebase |
| [03-one-native-fn.md](03-one-native-fn.md) | Collapse 3 native fns + stitch trait |
| [04-typed-input-output.md](04-typed-input-output.md) | Typed `parallel_for(input: vector<T>, fn, threads) -> vector<U>` |
| [04d-fn-ref-closure-storage.md](04d-fn-ref-closure-storage.md) | Fn-ref struct fields store both d_nr + closure DbRef (16 B) |
| [04d-followups.md](04d-followups.md) | 4d follow-up bugs and refinements |
| [05-auto-light.md](05-auto-light.md) | Auto-light heuristic via D12 caller-graph fixed-point |
| [06-cleanup-and-doc.md](06-cleanup-and-doc.md) | Delete now-unreachable runtime variants |
| [07-fused-for-par.md](07-fused-for-par.md) | Fused `for x in ls par(r=foo(x), 4) { … }` + `par_fold` sugar |
| [08-browser-workers.md](08-browser-workers.md) | Browser par via `wasm-bindgen-rayon` (deferred to its own arc) |
| [09-tuple-support.md](09-tuple-support.md) | Tuple inputs / returns for par |
| [10-no-output-vector.md](10-no-output-vector.md) | Strategic shift — drop materialised result vector; stream-only |

**Deferred phase-5 tail (now its own plan):** the auto-light *heuristic* shipped
([05-auto-light.md](05-auto-light.md)), but the parent-store **sharing** it was meant
to unlock (the `Arc<Store>` / read-only-borrow that would stop workers byte-copying a
large captured structure per job) did not — the live dispatch still `clone_for_worker()`s.
That work is tracked as [@PLN108](https://github.com/loft-lang/plans/issues/108)
([`108-share-read-only-stores/`](../../108-share-read-only-stores/README.md)).

Cross-cutting design docs:

- [ARC.md](ARC.md) — live execution sequence (A1–A11) with
  scope-locked acceptance tests.  The authoritative closure
  record.
- [DESIGN.md](DESIGN.md) — cross-cutting decisions D1–D13
  (Stitch policy enum, worker-store relationship, fn return
  accessor, type spectrum, caller-graph infrastructure, SAB
  transfer + DbRef rebase across worker boundary).
- [PRIORITY.md](PRIORITY.md) — historical priority spec
  (superseded by ARC.md).

## See also

- [`../../../THREADING.md`](../../../THREADING.md) — reference
  for the post-@PLAN06 surface (par(...) syntax, par_fold,
  dispatcher inventory, @PLAN06 phase 0 baseline retained as
  reference benchmark)
- [`../../CHANGELOG_TECHNICAL.md`](../../../CHANGELOG_TECHNICAL.md) —
  per-A-step shipped manifest under "Plan-06 (typed-par
  redesign) closed 2026-05-09"
- [`../../PROBLEMS.md`](../../../PROBLEMS.md) — P188–@P236 family
  bug entries (most closed; see Realised value table above)
- [`../../ROADMAP.md`](../../../ROADMAP.md) — A14 / A15 (1.1+
  parallel work cooperators) + browser parallel arc (was A10)
- `src/parallel.rs` / `src/native.rs` / `src/codegen_runtime.rs` —
  shipped runtime surface
