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

## RESUME HERE (read first after a fresh start)

All work is on branch **`tuxedo-pln94-ownership-dataflow`** (off `origin/main`), a **pure observer**
in `src/ownership_cfg.rs` (nothing in the compile path consumes it; SI-1 holds). Done: Phase 0
(falsify ✓), Phase 1 (CFG + reaching-defs fixpoint ✓), Phase 2 → 3.1/3.2/3.3 ✓, 3.4a characterised.

**How to run the oracle** (env `LOFT_OWN_ORACLE`, dumps to stderr; always set `LOFT_NO_CACHE=1` so
`scopes::check` re-runs on the user file):
```bash
cargo build --bin loft
# CFG structure (Phase 1.1):   LOFT_OWN_ORACLE=cfg
# reaching-defs fixpoint (1.2): LOFT_OWN_ORACLE=rd
# ownership fact + shadow-diff (Phase 2): LOFT_OWN_ORACLE=own
LOFT_NO_CACHE=1 LOFT_OWN_ORACLE=own ./target/debug/loft --interpret <file.loft> >/dev/null 2>dump.txt
# per-function line: "OWN <fn>  blocks=N passes=P  agree=A precision=Pr disagree=D", then DISAGREE/PRECISION lines
cargo test --release --lib ownership_cfg   # the 6 unit tests
```
Probes live in `probes/` (00 blindspot · 01 cfg · 02 loops · 03 ownership · 04 precision · 05 interproc
· 06 capture). SI-1 check: `cargo test --release --test wrap loft_suite` green with the module in.

**The immediate next action (3.4a adjudication)** — the one open decision blocking the op-tail:
does a closure record (`OpDatabase` store that holds a captured DbRef, and IS freed) have ownership
`Owned` (my dataflow — about its store) or `Borrowed` (B — about its dependency)? Resolve against
**how `scopes::get_free_vars` actually frees the closure record** (read `src/scopes.rs:3157-3420`
+ the `__closure_*` cascade at `src/database/allocation.rs:497-539`). If the record's free keys off
"owned", B's `Borrowed` is the imprecise one → **first real cross-check catch** (record it, keep my
`Owned`). If "borrowed" is load-bearing, teach my transfer to mark a var whose store holds a captured
DbRef as `Borrowed`. Repro: `probes/06-capture.loft`.

**Then, in order, to a fully functional oracle:** drain the rest of the op-tail (3.4: after captures,
`#rust`-return metadata via `returns_borrowed_view`, then coroutines / `par`) one family per commit
until DISAGREE→0 on the corpus + the `program_ownership` fuzzer; 3.5 (soundness sweep + confirm the
fact flags `LOFT_NO_A1B` and agrees on the fixed default — the A1b payoff); then **Phase 4** — turn
the fact into the consistency oracle over emitted IR ("every store freed once; no free of a live/
borrowed store"), default-on in test/CI, red on `LOFT_NO_A1B` + injected faults; then **Phase 5** —
land it beside behind `LOFT_OWN_ORACLE` with the fuzzer hook + a `tests/ownership_oracle.rs` binary.
That Phase-5 state IS the "fully functional oracle" (the H end-state). Codegen cutover stays out (P4/VH).

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

- **1.1 — CFG construction only ✅ (2026-07-07).** `src/ownership_cfg.rs` builds a structured CFG
  over the Value-IR: statement-level and control-carrying `If` split into then/else/join; `Loop`
  gets a header (back-edge) + exit; `Break(n)`/`Continue(n)`/`Return` add the right edge via a loop
  stack (`n=0` = innermost). Reached only via `LOFT_OWN_ORACLE=cfg` (SI-1 held: `loft_suite` + 158
  parse tests green with the module compiled in, oracle unset). **Gate PASSED:** the dump matches
  hand-drawn edges for straight-line, if-join (diamond), single loop (top exit-test + back-edge),
  nested loop (each `break(0)` targets its own loop; inner-exit → outer header), and
  early-return-in-loop (`return` → *function* exit, not loop exit). Corpus:
  [`probes/01-cfg-corpus.loft`](probes/01-cfg-corpus.loft). Key IR finding: `for` lowers to a
  `Loop` whose exit test is an `If(cond, Break(0), Null)` **buried in the range `Set` RHS**, so the
  builder recurses into `Set` values to find control transfers.
- **1.2 — the worklist engine on a trivial lattice ✅ (2026-07-07).** Reaching definitions (forward
  may-analysis, `OUT = gen ∪ (IN \ kill)`, union meet), round-robin fixpoint. Reached via
  `LOFT_OWN_ORACLE=rd`. **Gate PASSED:** hand-verified on straight-line (`out={v1@b0,v2@b0}`) and the
  one-branch `ifjoin` — the join `b4` has `in={v1@b2,v1@b3}` (both arms) and the initial `r=0`
  (`v1@b0`) correctly does NOT reach (killed on both arms). SI-3 holds: passes = 2 (straight) / 3
  (branch), both `≤ n+2`. SI-1 held (normal run `6 1 3 3` unchanged, clippy clean).
- **1.3 — loops / break / early-return in the engine ✅ (2026-07-07).** **Gate PASSED:** reaching-defs
  correct AND SI-3 (bounded convergence, `≤ n+2` passes) on single loop (5), early-return-in-loop (4),
  break-out (5 — a user `break` plus the for-exit test, both targeting the loop) and nested loop (7 —
  each `break(0)` targets its own loop). No non-reducible edge seen (structured control flow → the
  builder never needed the basic-block fallback). Locked in by **four parser-free Rust unit tests**
  (`src/ownership_cfg.rs` mod tests): the branch-join unions both arms and kills the initial def; the
  loop header sees the loop-carried def from both init and body across the back-edge; an early return
  edges to the *function* exit; nested loops converge with two distinct headers. Probe:
  [`probes/02-loops-rd.loft`](probes/02-loops-rd.loft).

**Phase 1 COMPLETE (1.1 + 1.2 + 1.3).** loft now has a structured CFG over its Value-IR and a monotone
worklist dataflow fixpoint over it, validated on the classic reaching-defs lattice across every
control-flow shape — the substrate the ownership fact (Phase 2) rides on. Pure observer throughout
(SI-1 held: `loft_suite` + parse suite green, oracle unset). Next: **Phase 2** — the forward ownership
lattice (`Owned`/`Borrowed(base)`/`Moved` + arm-meet `Join`) on this engine.

---

## Phase 2 — Backward liveness, shadow-diffed vs the position-proxy (first value, MH)

**DEFERRED — the forward ownership fact (Phase 3) was prioritised over liveness**, because
ownership is what drives the A1b-class detection the oracle exists for; liveness (a backward fact
for *free-placement*) is a sibling brought in when the Phase-4 free-consistency check needs it.
The engine (Phase 1) is direction-agnostic, so backward liveness reuses it unchanged. Steps below
stand as written.

- **2.1 — loop-free liveness + shadow harness.** Compute backward liveness; diff vs analysis A's
  proxy on the loop-free corpus. **Gate:** **zero** disagreement (proxy is valid there) — fix the
  new pass until zero. SI-1/SI-2 hold.
- **2.2 — liveness through loops.** **Gate:** ≥1 documented in-loop divergence, hand-verified that
  the new liveness is correct and the proxy is not (the precision the proxy admits it lacks).
- **2.3 — scale on the fuzzer.** Run the shadow-diff over `program_ownership`. **Gate:** zero
  outside-loop disagreement across the fuzz run (subsumption holds at scale, not just the corpus).

---

## Phase 3 — Forward ownership fact (the effort heart — subdivide hardest)

- **3.1 — lattice + core transfer functions ✅ (2026-07-07).** `OFact = Bottom | Owned |
  Borrowed(base) | Join(base)` with a lattice `meet` (unit-tested: `ofact_meet_is_a_join_semilattice`).
  Forward fixpoint on the CFG (`LOFT_OWN_ORACLE=own`); the per-def transfer REUSES the shipped
  `ownership_of` for a structural RHS and resolves a bare `Var` RHS flow-sensitively to the source's
  current state. **Gate PASSED:** shadow-diff `diff=0` (agrees with B) on every function of the
  ownership corpus — `s=mk()`→Owned, `r=s.v`→Borrowed, and threaded through branches. The
  flow-sensitive precision is already visible per-block: in `branch_reassign`, `r`=Owned in the
  then-arm, Borrowed(s) in the else-arm, meeting to `Join(s)` at the join — where the flow-*insensitive*
  classifier says `Join` everywhere. SI-3 held (`≤ n+2` passes); SI-1 held. Probe:
  [`probes/03-ownership.loft`](probes/03-ownership.loft).
- **3.2 — arm-meet `Join` + precision win ✅ (2026-07-07).** The per-path meet at statement-`if`
  arm-joins is my dataflow's `meet` (`branch_reassign`: then-arm `Owned` ⊔ else-arm `Borrowed(s)` =
  `Join(s)`). The shadow-diff is now split three ways — AGREE / **PRECISION** (mine `⊏` B's `Join`,
  a win) / **DISAGREE** (mine does not refine B — coarser or unsound; must be 0), via `OFact::refines`
  (unit-tested, incl. the soundness direction: claiming `Owned` where B says `Borrowed` does NOT
  refine → flagged DISAGREE). **Gate PASSED:** `reassign_win` reports `precision=1 disagree=0` (mine
  a definite `Borrowed` where B is `Join`); and at **scale** — `own` over `505-collection-capture.loft`,
  **712 functions — DISAGREE=0**: the flow-sensitive fact never unsoundly disagrees with B (pre-validates
  3.5's soundness sweep on this corpus). Probe: [`probes/04-precision.loft`](probes/04-precision.loft).
- **3.3 — interprocedural summaries ✅ (2026-07-07).** The transfer now consumes a callee's
  `return_ownership` SUMMARY directly (`call_own` + `caller_arg_base`, mirroring
  `use_analysis::call_ownership`) for a non-native user-function call — so calls are computed
  INDEPENDENTLY of the shipped classifier, not delegated. **Gate PASSED:** the two-fn borrow-return
  classifies the caller binding correctly — `a = id(x)` → `Borrowed`, `b = fresh()` → `Owned`,
  `disagree=0`. **The at-scale cross-check is the real deliverable:** running `own` over
  `505-collection-capture` (712 fns) with independent calls surfaced **22 DISAGREE**, all the
  dangerous `mine=Owned / B=Borrowed` direction — and since 505 runs correct + leak-clean, B is
  right, so these are *my* gaps: **capture-induced borrowing** (e.g. `test_vector_lookup`'s `xs` is
  an owned literal that B marks `Borrowed` because it is captured into a closure — @PLN93) and
  **`#rust`-bodied stdlib** whose return ownership is carried by codegen metadata, not the loft body
  `return_ownership` reads. This 22-site divergence map **is 3.4's work-list**, produced by the
  cross-check rather than guessed — the plan's premise (an independent implementation surfaces what
  a single one can't) demonstrating itself on the friendliest target first: my own incompleteness.
  Observer only — no shipped impact (SI-1 held). Probe: [`probes/05-interproc.loft`](probes/05-interproc.loft).
- **3.4 — the op-tail, ONE op family per commit** (this is the bulk; iterate). Order:
  closures/capture → coroutines → `par` → native ops. Each commit adds that family's transfer
  functions + its corpus shape. **Gate (per family):** shadow-diff agrees-or-more-precise vs B on
  that family; the two-closures-one-hash (@PLN93) cell classifies both handles `Borrowed(outer)`,
  outer sole `Owned`, value + no-double-free hand-checked. `log()` any op left unmodeled (no silent gap).

  **3.4a — capture family, CHARACTERISED (2026-07-07); adjudication pending.** The minimal repro
  (`probes/06-capture.loft`) pins the divergence: for `xs: vector = […]; c0(fn(){ xs[1] ?? -1 })`
  the disagreeing var is the **closure record `___clos_1`** — built by `OpDatabase` (its OWN fresh
  store, and it IS freed: `OpFreeRef(___clos_1)`) but holding `OpSetDbRef(___clos_1, 0, xs)`, a
  *borrow* of `xs`. My dataflow says `Owned` (correct about its **store** — owned, freed); B says
  `Borrowed(xs)` (tracking that it **depends on** `xs`). This is NOT a simple my-gap: it is a genuine
  semantic fork — *store-ownership* vs *dependency* — and which reading is correct depends on how the
  fact drives free-placement (the closure record is freed, so "owned" is defensible; B's "borrowed"
  may be conflating dependency with store-borrow). **This is the first divergence that may indict B
  rather than me** — exactly the cross-check's intended payoff — so it earns proper adjudication
  against ground-truth free-placement, NOT a rushed transfer tweak. Blocked-pending-adjudication;
  the remaining ~20 sites (incl. `#rust`-return metadata) queue behind it. Repro:
  [`probes/06-capture.loft`](probes/06-capture.loft).
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
