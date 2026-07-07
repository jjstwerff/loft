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

All work is on branch **`tuxedo-pln94-ownership-dataflow`** (off `origin/main`, unmerged), a **pure
observer** in `src/ownership_cfg.rs` (nothing in the compile path consumes it; SI-1 holds).
**H-tier END-STATE REACHED — Phases 0–5 all done.** Phase 0 (falsify ✓), Phase 1 (CFG + reaching-defs
✓), Phase 2 COMPLETE (3.1–3.5 ✓ — two oracle unsoundnesses found + fixed by the inward cross-check:
structural-op mis-route, `= null` default), Phase 4 ✓ (`LOFT_OWN_ORACLE=check`: Check A A1b catch +
the free-based Check B/C consistency layer), Phase 5 ✓ (`tests/ownership_oracle.rs` runs it on every
`cargo test` + the proof skeleton in `formal/ownership.md`). **Check C under-free BUILT + PROMOTED**:
the definite-leak scan runs on the default `check` path (927 → 0 behind its own `check-leak` ratchet,
injected-free positive control fires). `check` now catches BOTH directions.

**Still open** (refinements, not the end-state): (1) the one formal lemma — local transfer soundness;
(2) the leak-scan's next `check-leak` ratchet targets — conditional/`Join` leaks, adopted-owned
non-`OpDatabase` stores, closures; (3) promote the `check-dev` over-free Check B + resolve its
`n_choose` residual; (4) the self-contained A1b catch (waits on base resolution — A1b is caught by
Check A meanwhile). Out of scope (declared): P4/VH codegen cutover + the perf fork (README Open q4).

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

**3.4a RESOLVED (2026-07-07) — it was MY unsoundness, not a B indictment.** The adjudication
inverted the prior session's read on two counts. (1) The disagreeing var was mis-identified: not the
closure record `___clos_1` but **`xs`**, a vector local `xs = OpGetField(__vdb_1, 0, 22)` — a VIEW
into store `__vdb_1`. The emitted IR frees `__vdb_1` and `___clos_1` but never `xs`, so
`xs = Borrowed(__vdb_1)` is the sound free-placement fact: **B was right, my oracle said `Owned`**
(the over-free direction the `refines` gate flags). (2) Root cause: the Phase-3.3 call-arm guard
(`DefType::Function && native().is_empty()`) also swallowed the primitive STRUCTURAL ops
`OpGetField`/`OpNewRecord` (Function-typed, empty native) → `call_own` → `Owned`, bypassing
`ownership_of`'s projection handling (→ `Borrowed(base)`). **Fix:** exclude structural ops via the
new `use_analysis::classifies_structurally` predicate (= the exact set `classify` special-cases:
`OpDatabase`/`OpNewRecord` + projection ops) from the call-arm, so they fall to `ownership_of`. **All
22 "3.3 disagreements" on 505-collection-capture were this one bug** — corpus is now DISAGREE=0 (712
fns); precision win (probe 04) and interproc independence (probe 05) preserved; SI-1 green; 6 unit +
631 lib tests green. The cross-check delivered its payoff aimed INWARD — an independent impl caught
my own unsoundness. Note the design consequence: primitives are now delegated to `ownership_of`, so
the oracle's independence surface (where it could still catch a B bug) is **flow-sensitivity +
interprocedural summaries**, not primitive classification. Repro: `probes/06-capture.loft`.

**Phase 2 is COMPLETE and the A1b payoff is DEMONSTRATED (2026-07-07, 3.5 below):** on the canonical
`85-…-uaf.loft` the oracle is clean under the correct default (`n_h agree=7 disagree=0`) and FLAGS
the wrong plan under `LOFT_NO_A1B` (`disagree=1: __ref_1 mine=Join / B=Owned`); the fuzzer soundness
sweep is DISAGREE=0 across all 54 cells on both backends. The framework works end-to-end; what
remains is a self-contained RED check + landing it.

**Phase 4 is built (4.1/4.3 ✅, 4.2 nearly — see below and [`PHASE4_DESIGN.md`](PHASE4_DESIGN.md)):**
`LOFT_OWN_ORACLE=check` runs two gating checks BESIDE the shipped analysis and goes RED on
`LOFT_NO_A1B` (Check A) — the coexistence model the design doc pins.

**Immediate next action — close 4.2's last residual, then Phase 5.** (1) Clear the one `n_choose`
false positive: record `OpDatabase(v)` re-mints (a `Call`, not a `Set`) as owns entries — the same
class as the 3.4a structural-op fix — so a retbuf re-minted in an arm reads `Owned`, not the stale
`Borrowed`; if `Join`-vs-`Owned` remains after that, model the retbuf materialisation B collapses to
`Owned`. Then extend the 0-false-positive sweep to issues/leak/wrap/native. (2) **Phase 5** — land it
beside with the fuzzer hook (wire the 54-cell sweep + the `check` gate into `cargo test`) + a
`tests/ownership_oracle.rs` binary (per shape: SI-2 fact-identity, green on correct, RED on an
injected fault — the 4.3 true-positive test). That Phase-5 state IS the "fully functional oracle"
(the H end-state). Codegen cutover stays out (P4/VH).

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

  **3.4a — capture case, RESOLVED (2026-07-07): a real unsoundness in MY transfer, not a B fork.**
  The minimal repro (`probes/06-capture.loft`) was adjudicated against the emitted IR and inverted
  the prior read on two counts. (1) **Var mis-identified.** The disagreeing var was not the closure
  record `___clos_1` but **`xs`** — a vector local `xs = OpGetField(__vdb_1, 0, 22)`, a VIEW into
  store `__vdb_1`. The IR frees `__vdb_1` and `___clos_1` but never `xs`, so the sound fact is
  `xs = Borrowed(__vdb_1)`: **B was right; my oracle said `Owned`** — the over-free direction the
  `refines` gate flags. (2) **Root cause in my transfer.** The Phase-3.3 call-arm guard
  (`DefType::Function && native().is_empty()`) also captured the primitive STRUCTURAL ops
  `OpGetField`/`OpNewRecord` (Function-typed, empty native body) and routed them to `call_own`
  (→ `Owned`), bypassing `ownership_of`'s projection handling (→ `Borrowed(base)`). **Fix:** a new
  `use_analysis::classifies_structurally(data, d)` predicate — the exact set `classify` special-cases
  (`OpDatabase`/`OpNewRecord` + `projection_ops`) — added to the call-arm guard so structural ops
  fall through to `ownership_of`. **Gate PASSED:** capture probe `xs = Borrowed(__vdb_1)`,
  `disagree=0`; **all 22 "3.3 disagreements" on 505-collection-capture were this single bug** — the
  corpus is now DISAGREE=0 across 712 fns (the 3.3 characterisation of them as capture-induced
  borrowing / `#rust`-return metadata was wrong); the flow-sensitive precision win (probe 04,
  `precision=1`) and the interprocedural independence (probe 05, `a=Borrowed / b=Owned`) both
  survive; SI-1 green (`loft_suite`); 6 unit + 631 lib tests green; clippy clean. **Design note:**
  primitives are now delegated to `ownership_of`, so the oracle's independence surface — where it
  can still catch a *B* bug — is flow-sensitivity + interprocedural summaries, not primitive
  classification (which both analyses share by construction). The op-tail's genuine unmodeled
  families (coroutines, `par`) remain, but the cross-check surfaces none on this corpus. Repro:
  [`probes/06-capture.loft`](probes/06-capture.loft).
- **3.5 — the soundness-direction sweep + the Step-0 payoff. ✅ COMPLETE (2026-07-07).** Two parts:
  the fuzzer soundness sweep (below) and the A1b payoff (further below).

  **Fuzzer sweep — DISAGREE=0 across the @PLN85 ownership corpus (54 cells, both backends).** Ran
  `own` over `grammar_gen.py`'s 9 shapes × 2 values × 3 churn. First pass flagged 3 cells
  (`local_source__struct__*`, the `#462` conditional-local-view shape) — a SECOND real unsoundness in
  the oracle, distinct from 3.4a's: the `??` null-coalesce temps `__ncc_N` (Reference-typed, dep on
  the base, and `skip_free` — never freed) read `mine=Owned` where B says `Borrowed(base)`. Root
  cause: the temp's only STATEMENT-level owns entry my CFG records is its `= null` declaration (the
  real def `OpGetVectorNullable(base,…)` is nested inside the guard `if`), and the transfer classified
  `= null` as `Owned` via the catch-all. B's `collect_defs` SKIPS `= null` sentinels. **Fix:** skip
  `Null` owns entries in the transfer (one guard) — a declaration-only var falls through to
  `ownership_of` (B's fact) on read instead of defaulting `Owned`. **Gate PASSED:** all 54 cells
  DISAGREE=0; SI-2 verified (native == interp oracle on a cell); 505 still 0; all 7 probes 0;
  precision win + A1b payoff preserved; SI-1 + fuzz gate + 631 lib + 6 unit green. The fuzzer earned
  its keep — it exposed a gap the 505 corpus did not exercise. (Not yet wired as a standing
  `cargo test`; that is Phase 5.2's fuzzer hook.)

  **A1b payoff — DEMONSTRATED on the canonical case (2026-07-07).** On `tests/scripts/85-temp-subject-borrow-return-uaf.loft`
  (`h` returns `g(Filled{items: inner})`; `g`'s return is a join — `items` Borrowed on the Filled arm,
  `[]` Owned on the Empty arm):
  ```
  LOFT_NO_CACHE=1 LOFT_OWN_ORACLE=own loft --interpret 85-…-uaf.loft 2>&1 >/dev/null   | grep 'OWN n_h'
  # DEFAULT (correct plan): OWN n_h … agree=7 disagree=0        ← oracle agrees, no false alarm
  LOFT_NO_CACHE=1 LOFT_NO_A1B=1 LOFT_OWN_ORACLE=own loft --interpret 85-…-uaf.loft 2>&1 >/dev/null
  # LOFT_NO_A1B (wrong plan): OWN n_h … disagree=1  →  DISAGREE v6(__ref_1): mine=Join(?) B=Owned
  ```
  The discriminator is the FLOW-SENSITIVE fact: the return work-ref `__ref_1` reads `Owned` under the
  correct plan (h materialises g's result into an owned retbuf before the frees) but `Join` under the
  wrong plan (the un-materialised borrowing return is returned directly). B (flow-insensitive) says
  `Owned` in BOTH plans and so cannot tell them apart; the shadow-diff surfaces mine-vs-B as
  **agree (correct) vs disagree (wrong)** — the oracle flags the known-wrong plan and stays clean on
  the fix. This is Step-0's premise delivered by the built oracle. The soundness sweep (above) then
  confirmed no residual `Owned`-where-B-says-`Borrowed`/`Join` under-free across the fuzz corpus, so
  the flag does not come at the cost of false alarms elsewhere.

---

## Phase 4 — The consistency oracle over emitted IR (design: [`PHASE4_DESIGN.md`](PHASE4_DESIGN.md))

Design-doc-first (Design Protocol 1): a candidate self-contained A1b invariant was PROBED and
FALSIFIED before code (both the unsafe A1b return and the safe `id()` borrow-of-param have an
unresolved base `65535`, so base resolution can't tell them apart). Pivoted to **two gating checks
under `LOFT_OWN_ORACLE=check`, run BESIDE the shipped analysis** (coexistence — neither replaces the
other; both flag; overhead ~1.6%, gated-off zero): **Check A** the shadow-diff as a gate (the A1b
catch); **Check B** free-legitimacy (an unconditional `OpFreeRef` of a `Borrowed` store).

- **4.1 ✅ (2026-07-07)** — `check` mode + `run_check` + 2 unit tests (`free_of_borrowed` flags only
  the `Borrowed` free; `collect_free_targets` finds only free-op arg0). Pure observer (SI-1).
- **4.2 — NEARLY DONE (2026-07-07): 377 `tests/scripts` swept; Check B fully clean; Check A 11 → 1.**
  The build was the last probe — three false-positive classes found + fixed: Check B on
  `OpFreeText`/`OpFreeRefIfDistinct` (narrowed to unconditional `OpFreeRef`); Check A `Join(a)` vs
  `Join(b)` (compare by KIND, not base); Check A self-borrow `Borrowed(self)` (@P302 marker →
  normalise to `Owned`). **Residual 1:** `n_choose` — a retbuf re-mint via `OpDatabase(r)` (a `Call`,
  not a `Set`) the transfer misses; a leak-direction (SAFE) imprecision, deferred. SI-1 holds; 505 +
  fuzzer + probes still `disagree=0`.
- **4.3 ✅ (2026-07-07)** — RED on `LOFT_NO_A1B` (`n_h`, via Check A). The injected-fault
  true-positive (delete an `OpFreeRef` / flip a fact) is wired as a test in Phase 5.

---

## Phase 5 — Land the oracle beside (the H end-state) — ✅ CORE DONE (2026-07-07)

`tests/ownership_oracle.rs` makes the oracle a STANDING check run on every `cargo test` (default-on
in CI via the test binary, gated-off in the fast path — SI-1). 4 fast gates + 1 release-gate, all
green:

- **5.1 ✅ — flag plumbing + SI-1 as a test.** `LOFT_OWN_ORACLE=cfg|rd|own|check`;
  `oracle_is_a_pure_observer_si1` asserts `introspect` stdout byte-identical with the oracle on vs off.
- **5.2 ✅ — fuzzer hook.** `oracle_clean_on_generated_fuzz_corpus` regenerates `grammar_gen.py`'s 54
  cells and runs `check` on each — all 0 RED.
- **5.3 ✅ — the oracle binary.** `oracle_clean_on_correct_corpus` (7 probes + 505, 0 RED — no crying
  wolf), `oracle_flags_the_a1b_wrong_plan` (RED on `LOFT_NO_A1B`, clean on the default — the
  true-positive gate), `oracle_fact_is_backend_identical_si2` (SI-2 fact-identity interp vs native,
  release-gated). Corpus referenced in-place (the probes ARE the graduated shapes).
- **5.4 ✅ — formalise.** `formal/ownership.md` § "Machine-checkable soundness" — the proof skeleton
  for the flow-sensitive oracle's abstract-interpretation soundness (obligation ledger; one open
  lemma = local transfer soundness). Intro note updated: the substrate replacement is BUILT.

**Remaining tail (not blocking the H end-state):** the `n_choose` 4.2 residual (excluded from the
clean corpus); extending the 0-false-positive sweep to issues/leak/wrap/native; and the open formal
lemma (4). The injected-fault true-positive beyond `LOFT_NO_A1B` (delete an `OpFreeRef`) is a
regression guard — Check B is unit-proven but no toggle emits an unconditional free-of-borrowed.

## Check C — under-free / leak detection ✅ BUILT + PROMOTED ([`CHECK_C_UNDERFREE_DESIGN.md`](CHECK_C_UNDERFREE_DESIGN.md))

**OUTCOME (2026-07-07):** the definite-leak scan (`run_leak_scan`) now runs on the DEFAULT `check`
path, so the oracle catches both directions. Recognizer: only a MINTED (`OpDatabase`) var leaks (a
type-dep-only phantom like `__retbuf` has no store); transferred = returns (closed transitively through
the dep) ∪ consumes/captures (NOT closed) ∪ the shipped `skip_free`/`caller_hidden_buf` flags. Drove
its own `check-leak` ratchet **927 → 0** (the `__retbuf` phantom was ~889); an injected-free positive
control (`LOFT_OWN_INJECT_DROP_FREE`, `oracle_leak_scan_flags_an_injected_leak`) proves it fires — not
vacuous. KNOWN GAPS (ratchet targets, not FPs): conditional/`Join` leaks (runtime leak-check's class),
adopted-owned non-`OpDatabase` stores, closures. The journey below (the dev-tier/ratchet workflow, the
C.0 blocker, the promotion attempt) is the historical record; the workflow is now a nudge in the
engineering-rigor skill.

---

Extends the oracle from the over-free class (Check A/B) to under-free. A gated prototype was built +
MEASURED, then reverted; the numbers fixed both the scope and a BLOCKER (design doc has the detail):

- **Scope:** the honest target is the **DEFINITE leak** (an `Owned` heap local freed on NO path, not
  transferred) — catches a *deleted* free (the 4.3 injected-fault true-positive A/B miss); NOT
  conditional/`Join` leaks (`LOFT_NO_JOIN_OWN` — free op statically present, skipped at runtime → the
  RUNTIME leak-check's class; coexistence). False positives: heap filter 70→9-35/file, then the
  transfer-out set (returned ∪ consumed-into-container) → **0–4/file**.
- **C.0 ✅ BUILT (dev tier) + the REAL blocker found.** The oracle observed PRE-codegen IR (`oracle`
  is line 3 of `scopes::check`; `get_free_vars` inserts frees during codegen), so user-function free
  sets were empty. Fixed with `oracle_free_checks` — a POST-codegen pass at the end of `scopes::check`,
  gated on `LOFT_OWN_ORACLE=check-dev` (Check A stays pre-codegen; SI-1 held). With frees now visible,
  the 377-script sweep exposed the deeper blocker: **the ownership fact is not materialisation-aware**
  — a struct copy `r1 = a` reads `Borrowed(a)` but is freed as an owned copy (the `n_choose` gap,
  pervasive) → **153 free-based findings**, flooding both B and C. The fix is a fact-precision upgrade
  (classify copied `x = borrowed-source` as `Owned`), not more check tuning.
- **The WORKFLOW (dev tier + ratchet):** the dev checks live behind `check-dev`, never on the default
  `check`; `tests/ownership_oracle.rs::oracle_dev_free_check_ratchet` (`#[ignore]`) asserts findings
  `≤ DEV_FP_BASELINE` — a one-way ratchet lowered by each improvement; `0` promotes them into `check`.
- **RATCHET 153 → 0 (2026-07-07).** The "materialisation-aware fact" = use the POST-codegen **type
  dep** as the ownership signal (a copied `ac_copy = f(a, __ref_2)` has an empty dep = owns; the
  usage-dependent borrow-elision is baked in there). Check C: `depend().is_empty()`; Check B: freed
  var with non-empty dep. + `transferred_out += OpSetDbRef` capture + narrow exclusions (`__ncc_*`,
  self-dep work-refs, freed params, closures). `DEV_FP_BASELINE=0`. **Honest tradeoff:** this makes
  B/C CONSISTENCY checks (free-placement vs dep — catches `get_free_vars`/dep divergence), NOT
  independent (LOFT_NO_JOIN_OWN not caught by B/C — Check A's + the runtime leak-check's job). Promote
  to `check` after validating issues/leak/wrap/native + a true-positive test.

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
