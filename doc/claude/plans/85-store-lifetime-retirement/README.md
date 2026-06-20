<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 85 — Retire the store-lifetime bug class

Plan id: [@PLN85](https://github.com/loft-lang/plans/issues/85) · investigation-style
(reading order: Status → Probes → Cluster docs → Roadmap).

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue (extract from the real recent bugs + a real consumer) | ✅ probes 01–04 (evaluation in [recent-bugs.md](recent-bugs.md)); of 3 predicted siblings: #1 latent, #2 closed, #3 LIVE |
| B — Mechanism investigation (shared-root vs N-independent) | ✅ **CLOSED via debugger** — real mechanism: `ki` is a borrowed alias of the NRVO return buffer `__ref_1`; under conditional×unused, scope analysis emits a per-iteration `OpFreeRef(ki)` that double-frees the return buffer via a stack-ref ([cluster-II](cluster-II-slot-init-dominance.md) § CLOSED OUT). NOT the slot-init story. |
| C — Fix design | 🟡 direction set: NRVO-return-bound local must be **borrowed/skip-free** (the buffer owns the single free) — fix at the ownership-marking chokepoint |
| D — Implementation | ⏸️ pending C — validate against matrix A–F + leak gates, both backends |

**What triggered this:** the *known* store-lifetime mechanisms are already hardened and
shipped — H1 (fn-arity freeze), **H2 (typed `Deps` newtype + value-tagged spaces — see
[DEPS_INVENTORY.md](../../DEPS_INVENTORY.md))**, H3 (ownership carried, not re-derived), and the
finished @PLN51 hidden-buffer-aliasing investigation. **Yet the class still produced four
sev:high bugs this cycle** (#405/#406/#409/#410), in mechanisms the prior work did not cover.
So the open question is not "apply the known fix" (done) but: *is there a deeper shared root the
hardening missed, or are these genuinely independent mechanisms?*

## Goal

Ship one of two provable outcomes: **(a)** a structural change that retires the class (a single
chokepoint/invariant the residual clusters all violate), **or (b)** the verified finding that the
clusters are independent, with each one's irreducible invariant named + enforced at its own
chokepoint AND a standing instrument (corpus / sanitizer / fuzz) that keeps the class closed so
the next cycle stops re-discovering siblings. Either way: a probe suite that proves the class
closed on both backends.

## Central hypothesis (HYPOTHESIZED — Stage B must verify or kill)

Refined by the recent-bug evaluation ([recent-bugs.md](recent-bugs.md)) from "value-vs-dep
desync" to the broader **slot-initialisation-before-lifetime-op** invariant:

> **Every slot a lifetime operation (free / `copy_claims` / in-place vector rebuild) reads must
> be initialised — a real store/discriminant or a recognisable sentinel — by EVERY construction
> path that produces the record.**

All recent bugs fit as *different producers violating this one invariant*: uninitialised dep
slot (#405), empty dep buffer vs a foreign value (#409/#410), uninitialised enum discriminant
(#406), dangling cross-iteration slot (@PLN51-II — and **#405 is an uncovered sibling of that
already-"closed" cluster**). Key structural fact the evaluation verified: **corruption manifests
concentrated** in `src/database/allocation.rs` (`free_named`, `copy_claims`), while **roots
scatter** across producer codegen paths — so per-producer fixes (control.rs, expressions.rs) and
defensive-at-manifestation patches (#405's OOB-refuse) each leave the class open.

**Stage-B update:** this slot-init framing was **FALSIFIED for the live cluster II** — its real
mechanism is an **NRVO return-buffer double-free** (`ki` borrows `__ref_1`; debugger-verified), not
a producer slot-init gap. The clusters are turning out *independent*, not one shared root:
#1 latent FFI gap, #2 copy_claims (closed by #412), #3=II NRVO-alias ownership. So the plan is
trending toward outcome (b) — per-mechanism invariants — over a single chokepoint.

## In-plan vs spinoff policy (default: in-plan)

Findings during this investigation are fixed in-plan and recorded in the cluster catalogue, not
double-filed (the probes + cluster docs are the record). On closure the rule inverts: any still-open
residual gets a forward home (PROBLEMS.md / QUALITY.md `## Open work`) citing its cluster doc.
Full policy: [`_INVESTIGATION_TEMPLATE.md`](../_INVESTIGATION_TEMPLATE.md).

## Cluster catalogue (seeded — mechanisms verified in [recent-bugs.md](recent-bugs.md); to be confirmed/split by probes)

| ID | Cluster | Severity | Backends | Status of the instance | Doc |
|---|---|---|---|---|---|
| I | FFI foreign-store `vector` return delivered to a local; in-place `+=` drops it (local borrows foreign store, dep buffer empty) | high | both | #409 (wrapper) + #410 (direct) FIXED at producer chokepoints — mechanism cluster open | cluster-I (todo) |
| II | **NRVO return-buffer double-free** (was mis-described as a dep-slot init bug): `ki` aliases the NRVO buffer `__ref_1`; under conditional × **unused**, scope analysis emits a per-iteration `OpFreeRef(ki)` that whole-store-frees the stack-alias (#405/#306) while `__ref_1` also frees it at fn exit | high | **interp SIGSEGV / native completes** | 🔴 **LIVE on main** (probe 04); mechanism VERIFIED via debugger. THE live cluster. | [cluster-II](cluster-II-slot-init-dominance.md) § CLOSED OUT |
| III | enum-discriminant corruption: struct-with-enum-fields-from-a-variable appended to a vector; `copy_claims` reads a -1 discriminant | high | both | ✅ **CLOSED on main** (probes 02/03): #412 fixed the enum shape; `copy_claims` breadth (vector/sub-struct fields, nested) verified clean | — |
| IV | @PLN51 hidden-buffer-aliasing residuals (siblings not re-probed since closure) | tbd | both | re-extract from `finished/51` probes | cluster-IV (todo) |

## Probe suite

`probes/` (to be created in Stage A). Per the investigation method: **write probes before reading
source**, liberally; extract at least one from a REAL consumer (crypto/imaging FFI returns, moros),
not only the synthetic repros; run every probe on `--interpret` AND `--native` and record the
matrix. A probe graduates to `tests/scripts/85-*.loft` only when it passes assertions + clean exit +
no leak (`LOFT_STORES=warn`) + bounded runtime.

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-native-struct-return.loft` | direct `#native` struct (non-vector heap) return + read/mutate | I (sibling) | read-only RED but **NOT a confirmed sibling** — isolated to an FFI-layout gap (no `alloc_struct` helper; `alloc_record` doesn't lay out a loft-readable struct ref). Sibling #1 is **latent/unreachable**, gated on a future struct-return helper — see [recent-bugs.md](recent-bugs.md) |
| `02-enum-fields-in-vector.loft` | struct w/ 1 & 2 variable-sourced enum fields, appended to a vector | III (#406) | ✅ **PASS on current main** — #412 fully closed the #406 shape (the "still corrupt" in its thread was the pre-merge WIP). Control (direct read) proves non-vacuous. |
| `03-copy-claims-breadth.loft` | struct w/ vector field, sub-struct field; nested `vector<vector>` appended | II/III breadth | ✅ **PASS** — `copy_claims` deep-copies these field kinds correctly; sibling #2 (copy_claims family) closed for reachable shapes |
| `04-slot-init-405.loft` | heap local conditional × unused × nested loop (#405 shape) | II (#405) | 🔴 **LIVE — interpret SIGSEGV on current main.** VERIFIED (debugger): `OpFreeRef(ki)` double-frees the NRVO return-buffer alias `__ref_1` via a stack-ref (store_nr=8·(rec−1) > len → #405/#306). native completes. Can't graduate while it crashes. (filename predates the corrected mechanism.) |

## Roadmap (next session)

1. **Stage A** — port the four repros (#405/#406/#409/#410) into `probes/` as assertion-bearing
   `.loft` files; add a real-consumer FFI-return extraction; run both backends; record the matrix.
2. Pull the @PLN51 probe set forward and re-run for live siblings (cluster IV).
3. **Stage B** — for each cluster, a `cluster-<id>.md` with a verified-vs-hypothesised table; test
   the central hypothesis (shared desync root) by checking whether one chokepoint invariant covers
   I/II and whether III is the same or independent. Instrument (one env-flag trace) rather than
   theorise when a mechanism resists two reads.
4. **Stage C/D** — either the chokepoint fix (if shared) or per-mechanism invariants + a standing
   instrument; graduate probes per cluster as each fix lands.

## Anchors

- Design: [DEPS_INVENTORY.md](../../DEPS_INVENTORY.md) (typed `Deps`, done) · [LIFETIME.md](../../LIFETIME.md) (dep model) · [STABILITY_HOTSPOTS.md](../../STABILITY_HOTSPOTS.md) (H-register) · [STABILITY_METHOD.md](../../STABILITY_METHOD.md)
- Evidence: loft#405, #406, #409, #410 · finished @PLN51 (`plans/finished/51-hidden-buffer-aliasing/`)
- Method: `CLAUDE.md` § matrix-first · the `design-protocol` skill (one invariant, or N?)
