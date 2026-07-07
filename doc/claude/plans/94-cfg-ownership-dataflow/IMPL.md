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

## Phase 0 — Falsify the oracle's reason to exist (BEFORE any framework) — ✅ PASSED (2026-07-07)

**Premise to kill:** *an independent static fact detects a wrong free/own plan that the shipped
observable gates (exit, stdout, interp-vs-native oracle, poison, leak) can pass.* Cheap; done first.
**Result: premise HOLDS — the oracle is not redundant. Proceed to Phase 1.** Probe:
[`probes/00-a1b-silent-blindspot.loft`](probes/00-a1b-silent-blindspot.loft).

- **0.1 — a real wrong plan on demand. ✅** `LOFT_NO_A1B` on `tests/scripts/85-temp-subject-borrow-return-uaf.loft`:
  interp fails the `len==3` assert (UAF → wrong len), native+`LOFT_POISON` panics `len: 0`. Default
  is correct on both. So the toggle is a real known-wrong-plan source.
- **0.2 — a backend-CONSISTENT wrong case (the blind spot). ✅** The assert-stripped probe under
  `LOFT_NO_A1B` prints `len=0` on **both** backends, exit 0, **no leak-warn** (`LOFT_STORES=warn` /
  `LOFT_NATIVE_LEAK_CHECK`). Correct answer is 3. So stdout+exit+leak — every observable gate,
  including the interp-vs-native differential oracle — **passes a definitively wrong plan**. Only the
  original hand-written `assert` (a human oracle of correctness) caught it.
- **0.3 — an independent static fact catches it. ✅** `loft introspect` diff of `h`, default vs
  `LOFT_NO_A1B`: the correct plan **copies** g's result into an owned `__retbuf`
  (`OpAppendVector(__retbuf, n_g(...))`) *before* `OpFreeRef`-ing the temps, then returns `__retbuf`;
  the wrong plan collapses the return onto `__ref_1(0)` sharing `__vdb_1`, then
  `OpFreeRefIfDistinct(__vdb_1, __ref_1(0))` frees a store the return borrows, then returns it. The
  fact *"no store backing the return value is freed before the return"* is **red on wrong, green on
  correct** — detection power the observable gates lack.

**Gate: PASSED** — an independent flow-sensitive completeness check catches a real class (a
backend-consistent wrong free/own plan) that interp-vs-native + leak gates structurally miss.

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
