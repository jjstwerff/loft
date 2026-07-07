<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN94 — verifiable implementation steps (oracle end-state, the H tier)

Scope: build the flow-sensitive CFG+dataflow fixpoint and run it **beside** the shipped analysis
as an independent completeness **oracle**. No codegen cutover (P4/VH, out of scope here). Each
**sub-step is independently committable and states its own executable gate** — a sub-step that
cannot name its gate is too big or not real. Every gate runs on **both backends** and leaves
shipped output byte-identical. Discipline mirrors the @PLN93 steps: probe in `/tmp` first,
hand-compute every expected value, graduate to `tests/scripts/` + an oracle test binary at the end.

## Standing invariants — re-check after EVERY sub-step

- **SI-1 — shipped codegen byte-identical.** `loft introspect <corpus>` identical with the oracle
  compiled-in-but-observing vs. `LOFT_OWN_ORACLE=off`. Empty diff = proof it only observes.
- **SI-2 — backend fact-identity.** `LOFT_OWN_ORACLE=dump` under `--interpret` vs `--native` is
  byte-identical (O-NoDiverge, checked mechanically).
- **SI-3 — termination.** Fixpoint converges in `≤ 2·n_blocks` iterations on every corpus function.

**Corpus** (one `.loft` per shape, grown as steps need it): straight-line · if/match-join · single
loop · nested loop · break-out-of-loop · early-return-in-loop · loop-carried borrow · A1b
temp-subject-borrow-return · `v[i] ?? d` Join · two-closures-mutate-one-hash (@PLN93).

---

## Phase 0 — Falsify the oracle's reason to exist (BEFORE any framework)

**Premise to kill:** *an independent static fact detects a wrong free/own plan that the shipped
observable gates (exit, stdout, interp-vs-native oracle, poison, leak) can pass.* Cheap; do it first.

- **0.1 — a real wrong plan on demand.** Run a small program under `LOFT_NO_A1B` (opts out of the
  @PLN90 A1b fix). **Gate:** it produces a demonstrably wrong free/own plan (differs from default
  under `LOFT_POISON`/`LOFT_NATIVE_LEAK_CHECK`).
- **0.2 — a backend-CONSISTENT wrong case (the blind spot).** Shape a `LOFT_NO_A1B` program whose
  stdout + exit are identical on both backends and that emits no leak-warn without poison. **Gate:**
  the `tests/oracle/` differential oracle **passes** it — proving the observable gates have a blind spot.
- **0.3 — an independent count catches it.** Hand-count alloc-vs-free per store per path on 0.2.
  **Gate:** the count is **red** on 0.2 and **green** on its A1b-fixed (default) sibling.

**Red gate ⇒ STOP:** if every wrong plan you can build already diverges the backends, or the count
can't separate wrong from right, the oracle is redundant — reconsider before building anything.

---

## Phase 1 — CFG + fixpoint engine

- **1.1 — CFG construction only** (`src/ownership_cfg.rs`, new): blocks + succ/pred edges for
  `If`/match/`Loop`/`break`/`return`, no fixpoint yet. **Gate:** `LOFT_OWN_ORACLE=cfg` dump matches
  hand-drawn edges for if-join, single loop, nested loop, early-return-in-loop.
- **1.2 — the worklist engine on a trivial lattice** (reaching-defs), straight-line + one branch.
  **Gate:** reaching-defs hand-verified on 2 shapes; SI-3 holds.
- **1.3 — loops / break / early-return in the engine.** **Gate:** reaching-defs correct AND
  SI-3 (bounded convergence) on single loop, nested loop, break-out, early-return-in-loop — the
  cases the position-proxy cannot express. A non-reducible edge here forces the basic-block
  fallback; record it.

---

## Phase 2 — Backward liveness, shadow-diffed vs the position-proxy (first value, MH)

- **2.1 — loop-free liveness + shadow harness.** Compute backward liveness; diff vs analysis A's
  proxy on the loop-free corpus. **Gate:** **zero** disagreement (proxy is valid there) — fix the
  new pass until zero. SI-1/SI-2 hold.
- **2.2 — liveness through loops.** **Gate:** ≥1 documented in-loop divergence, hand-verified that
  the new liveness is correct and the proxy is not (the precision the proxy admits it lacks).
- **2.3 — scale on the fuzzer.** Run the shadow-diff over `program_ownership`. **Gate:** zero
  outside-loop disagreement across the fuzz run (subsumption holds at scale, not just the corpus).

---

## Phase 3 — Forward ownership fact (the effort heart — subdivide hardest)

- **3.1 — lattice + core transfer functions, straight-line.** `OpDatabase`/`OpNewRecord`/struct-lit
  → `Owned`; projection (`OpGetField`/`OpGetVector`/`OpGetDbRef`) → `Borrowed(base)`. Shadow vs
  `ownership_of` on straight-line. **Gate:** agrees with B on the straight-line corpus.
- **3.2 — arm-meet `Join`.** The per-path meet at `if`/match arm-joins (replacing B's join-of-all-defs).
  **Gate:** `v[i] ?? d` yields `Join` at the meet; ≥1 case where B over-reports `Join` and the new
  fact is a definite `Owned`/`Borrowed` (documented precision win).
- **3.3 — interprocedural summaries.** Consume callee `return_ownership` at call sites. **Gate:** a
  two-fn borrow-return (`fn id(v)->vector{v}` caller) classifies the caller binding correctly.
- **3.4 — the op-tail, ONE op family per commit** (this is the bulk; iterate). Order:
  closures/capture → coroutines → `par` → native ops. Each commit adds that family's transfer
  functions + its corpus shape. **Gate (per family):** shadow-diff agrees-or-more-precise vs B on
  that family; the two-closures-one-hash (@PLN93) cell classifies both handles `Borrowed(outer)`,
  outer sole `Owned`, value + no-double-free hand-checked. `log()` any op left unmodeled (no silent gap).
- **3.5 — the soundness-direction sweep + the Step-0 payoff.** **Gate:** across corpus + fuzzer, no
  case where the new fact says `Owned` while B says `Borrowed`/`Join` in a way that under-frees
  (hand-verify each such disagreement); the new fact **flags** `LOFT_NO_A1B` and **agrees** on the
  fixed default.

---

## Phase 4 — The consistency oracle over emitted IR

- **4.1 — the checker, unit-tested.** Walk the shipped IR; assert per path: every store freed
  **exactly once**, no free of a still-live store, no free of a borrowed (non-owning) alias.
  **Gate:** on a hand-built IR, catches a hand-injected fault (unit test), passes the correct one.
- **4.2 — drive false-positives to zero.** Run `LOFT_OWN_ORACLE=check` over issues/leak/wrap/native/
  `loft_suite`. **Gate:** green everywhere (shipped code IS complete there — no crying wolf). SI-1 holds.
- **4.3 — true-positive gate.** **Gate:** **red** on `LOFT_NO_A1B` and on an injected fault (delete
  one `OpFreeRef`, or flip one `Owned`→`Borrowed`), reporting the exact store + site.

---

## Phase 5 — Land the oracle beside (the H end-state)

- **5.1 — flag plumbing + SI-1 as a test.** `LOFT_OWN_ORACLE=off|cfg|dump|check`, default check in
  test/CI, off in the fast path. **Gate:** an introspect before/after test asserts SI-1.
- **5.2 — fuzzer hook.** Every `program_ownership` case runs the oracle. **Gate:** fuzz run green
  with the oracle on, both backends.
- **5.3 — graduate the corpus.** `tests/scripts/94-*.loft` + `tests/ownership_oracle.rs` (per shape:
  SI-2 fact-identity, oracle green on correct, red on injected fault). **Gate:** the binary is green.
- **5.4 — formalise.** Add the "independent oracle" note to `formal/ownership.md`: completeness is
  now cross-checked by a flow-sensitive fixpoint, not only by interp-vs-native agreement.

**Explicitly NOT here:** routing any shipped consumer (`scopes::get_free_vars`, `state/codegen.rs`,
`generation/dispatch.rs`) to the new fact, and retiring the position-proxy / flow-insensitive-join.
That is **P4 (VH)** — the self-referential analysis-and-rewrite cutover — taken only if the oracle's
evidence + the perf fork (README Open q4) justify it.

---

## Done = the H milestone

The fixpoint runs beside the shipped compiler on every test + fuzz case, both backends, and:
(1) never false-positives on the existing suite, (2) flags every injected fault + the `LOFT_NO_A1B`
reintroduction, (3) never touches shipped output (SI-1), (4) reads identically on both backends
(SI-2). loft then has an **independent, machine-checkable completeness check** on the ownership
fact — the safety net the rustc evaluation asked for — with zero bytes of emitted code changed.
