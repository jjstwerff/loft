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
| C — Fix design | ✅ **DESIGNED** — the structural fix is a **type-system change**: make heap-return ownership a computed type fact (the return type's `Deps`: owned/transferred vs borrows-attr), so codegen (caller adopt-vs-copy; callee free-or-not) collapses to mechanical reads — per [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md), the `has_ref_params`/`returned_var` complexity is the symptom of this missing fact. Type design: **[type-ownership-design.md](type-ownership-design.md)**; target bytecode + rungs: [stage-c-move-convention-design.md](stage-c-move-convention-design.md). |
| D — Implementation | ✅ **CLUSTER II SHIPPED** — the cluster-II owned/borrow vector-`match`-return is correct AND leak-free on BOTH backends; probe 05 graduated to `tests/scripts/85-store-lifetime-vector-match-return.loft`; full suite clean. Two landed changes: (1) **scopes.rs** `collect_return_sources` + narrowed skip_free (the SET that fixes `returned_var`'s match-collapse); (2) **control.rs** `materialize_vector_arms_into` — per-arm NRVO delivery into `__retbuf` (each arm copies its elements into the caller's buffer, frees the dead local backing, yields `__retbuf`), closing the interp work-ref leak (root-caused to `init_ref`'s eager `null()` store) while staying native-compilable. Plus a real native-generator bracing-bug fix (emit.rs). Full mechanism: [type-ownership-design.md § 6h](type-ownership-design.md). **Also fixed + guarded:** #405 (probe 04, `gen_if` stack-delta gate) and the `return v` borrow-return aliasing (param→buffer copy). **→ the cluster-II CRASH/CORRUPTION class is retired on both backends** (5 graduated regressions in `tests/scripts/85-*`). **Two ownership-COMPLETENESS residuals** (distinct ownership holes, NOT crashes; multi-cycle migration rungs per OWNERSHIP_MODEL) are forward-homed as issues with verified `wa:clean` workarounds: **loft#415** field-read adopt-vs-copy (`a = x.v` aliases; sev:medium) and **loft#416** implicit-tail `{ match }` interp leak (sev:low). **Plan ready to close (outcome b).** |

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
| `04-slot-init-405.loft` | heap local conditional × unused × nested loop (#405 shape) | II (#405) | ✅ **FIXED — BOTH backends, graduated** to `tests/scripts/85-store-lifetime-405-conditional-loop.loft`. ROOT (different from the earlier caller-side-NRVO theory — that was WRONG): `gen_if`'s `null_else_value` gate keyed on the true-branch RETURN TYPE (`tp != Void`), but a block ending in `OpAppendVector` reports a non-Void type while pushing NOTHING — so the else path emitted a spurious `ConstInt(i64::MIN)`, leaking 8 stack bytes per false-branch iteration → stale `DbRef` → SIGSEGV at `OpFreeRef`. Fix (`src/state/codegen.rs`): gate on the actual stack delta `true_stack != stack_pos`. Found by a loft-codegen-**skilled** eval agent that proved working-vs-broken bytecode on both backends first (the method this plan exists to enforce); a no-skill baseline on the same bug did not fix it. |
| `05-enum-arg-vector-return-aliasing.loft` | **REAL-CONSUMER (cbor):** `fn(enum) -> vector` called N times, results held live | II | ✅ **FIXED — BOTH backends, graduated.** Correct independent values + no leak on `--interpret` and `--native`; graduated to `tests/scripts/85-store-lifetime-vector-match-return.loft`. Root cause of the cbor map-encode corruption. Fix: per-arm NRVO delivery into `__retbuf` ([type-ownership-design.md § 6h](type-ownership-design.md)). |
| `06-field-read-adopt-vs-copy.loft` | binding a vector FIELD: `a = x.v` (no fn) and `a = getv(x)` (fn returns `b.v`) | adopt-vs-copy ([OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) holes 102/103) | 🟠 **FILED → [loft#415](https://github.com/loft-lang/loft/issues/415)** (sev:medium, `wa:clean` — explicit `af=[]; af+=x.v` copies, verified both backends). LIVE on main: `af`/`ag` len == 4 (alias) vs control `a = x` (len 3). Silent value-semantics violation (no crash/leak); the bare `a = x.v` aliases with NO function — distinct from the CLOSED `return <param>` hole (row 101). A multi-cycle ownership-completeness rung (the adopt-vs-copy fact extended to field reads), not a cluster-II crash. Not graduated (LIVE until the fact is completed). |

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
