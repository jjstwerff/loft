<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 94 — CFG + dataflow fixpoint for free/ownership facts

## Status

**Future — design skeleton ready, no implementation.** A considered structural option, not
an imminent build. Today loft's store-lifetime facts are computed by two approximations in
`src/use_analysis.rs`: analysis **A** (copy/borrow/move) walks a **position counter that
proxies execution order and is only valid outside loops** (Tier-1 literally refuses any copy
whose fill sits inside a loop, `use_analysis.rs:201-204`); analysis **B** (the `Owned/Borrowed/Join`
fact) is **flow-insensitive** — a var classifies as the join of *all* its defs, not the
reaching def (`use_analysis.rs:1073-1078`). Both are sound-by-conservatism and validated by
the @PLN89 differential oracle + the `program_ownership` fuzzer. This plan introduces a
**flow-sensitive dataflow fixpoint over a CFG** *beside* the current substrate — first as an
independent completeness **oracle** (shipped codegen untouched), optionally replacing the
approximations later — so loops / branches / early-return are precise *by construction* rather
than approximated. It does **not** change the language, the deps representation, or the "never
reject" contract — only how (and how precisely) the facts those consumers read are computed and
checked.

## Goal

Compute every store-lifetime fact (liveness/last-use, `Owned/Borrowed(base)/Moved`, `Join`)
as the least fixpoint of a monotone transfer function over each function's control-flow graph,
consumed by one oracle every free/own/elision site reads — replacing the position-proxy and
the flow-insensitive join, and making O-Deps + O-NoDiverge structural.

## Effort + design

- **Effort (tiered):** **VH** for the full replacement (P0–P4); **H** for the oracle end-state
  (P0–P3 — build the framework, run it beside forever, no cutover, no fast-path perf constraint);
  **MH** for first value (a liveness-only oracle that already catches A1b-class divergences without
  being complete). The cost driver is the **per-op transfer-function long tail** over loft's
  irregular op surface + modeling loft's (non-Rust) ownership semantics + the P4 cutover — NOT the
  (textbook) CFG/lattice/fixpoint machinery. The oracle-first ordering lets most of the safety value
  land at H (or MH for the first catch) and defers the VH tail indefinitely.
- **Design:** ~ (this README is the skeleton; the lattice + transfer functions get their own
  `DATAFLOW.md` in Phase 0 before any code)
- **Last touched:** 2026-07-07

## The invariant (the design hypothesis)

> Every store-lifetime codegen decision — **free placement**, **own-vs-borrow
> materialisation**, and **move-elision eligibility** — is a pure function
> `f(program-point, dataflow-state)`, where the dataflow-state is the **least fixpoint of a
> monotone transfer function over the function's CFG**. Computed once per function, consumed
> everywhere, backend-agnostic.

This is the **recover-a-known-construction** case, not an open design space: monotone dataflow
(lattice + transfer functions + worklist fixpoint) is textbook, and rustc's MIR borrow-checker
(+ Polonius) is the reference. The design work is *adapting* that construction to loft's
structured IR, not inventing one. The falsifiable core is small and gets probed in Phase 0.

## What actually makes it hard (the crux — it is NOT the dataflow machinery)

rustc's borrow-checker is a **pure analysis over a fixed program with a reject valve**. loft's
ownership analysis is a **self-referential analysis-AND-rewrite whose output is the program it
must be correct about — and it cannot reject.** Three facets, in order of bite:

1. **Analysis ↔ emitted-code mutual recursion.** The ownership fact *decides* the free / copy /
   move; but a move-elision rewrite *changes* the store lifetimes, which changes the fact. Precise
   ⇒ a **joint fixpoint of facts-and-emitted-code**, not a one-way pass. rustc never has this —
   borrowck observes fixed MIR and errors; drop elaboration (the transform) runs after, separately.
2. **No reject valve, self-referential truth.** rustc rejects when unsure (safe fixed default);
   loft must emit *some* code, and "correct" is defined relative to the code it is emitting — a
   wrong fact is a UAF that surfaces only when that exact code runs, not a catchable error.
3. **Shared mutation removes the exclusivity rule.** "Aliasing XOR mutability" is what makes rustc's
   check *local and decidable*. loft allows many live mutable aliases of one owned store, so the
   free-safety proof is over an unbounded aliasing graph, not one borrow at a time.

The current position-proxy + flow-insensitive-join are precisely the **dodge**: one conservative
pass decides-once/rewrites-once, so it never re-analyses after its own rewrite. @PLN94's difficulty
*is* re-introducing precision, which brings the joint fixpoint back. **This is why the oracle form
is materially easier:** an oracle *observes* already-emitted IR (frees/copies present) and checks
consistency — it does not rewrite, so facet 1 vanishes entirely. It stays *rustc-shaped* hard
(facets 2–3: the op-tail transfer functions + the shared-mutation aliasing invariant) but not
*joint-fixpoint* hard. That is the real reason P0–P3 is **H** and the full P0–P4 replacement is **VH**.

## Why at all — and the honest trade

loft's ownership half has taken **rustc's hardest job** (decide every free without UB) while
removing rustc's safety valve — **it never rejects**. So **completeness is load-bearing for
soundness**: an incomplete fact is not a user-fixable compile error, it is a miscompile / leak
(`formal/ownership.md:64-66`). The A1b UAF (@PLN90) — which *shipped latently in the native
backend* — is the existence proof that a heuristic substrate can harbour that class; rustc
structurally cannot emit it because its dataflow is precise at the join. This plan buys that
structural immunity.

The trade is real and this plan is **status:future** for a reason: it re-engineers a subsystem
that already works and is oracle-green. The **trigger** to unpause is one of — (a) the
ownership half's A1b-class incompleteness risk is judged to exceed the migration cost; (b) a
feature needs precision the position-proxy cannot give (e.g. loop-carried borrows, partial
moves); or (c) the perf fork below resolves toward "the fixpoint is the shipped analysis." Until
then it is the *validation oracle's* structural backstop, not a rewrite in flight.

## Beside the current analysis — oracle first (the recommended entry point)

Introduce the fixpoint **beside** the current analyses, not as a replacement — and make its
*first* role a permanent, independent **completeness oracle**, with cutover an optional later
decision. The reason is sharp: the existing differential oracle (@PLN89) compares **interp vs
native**, but both backends read the *same* ownership fact — so a fact that is wrong *but
consistently* wrong passes (this is exactly how A1b shipped latently in native). A flow-sensitive
fixpoint run beside the shipped analysis is a **second, independent source of truth about the
fact itself** — the missing "is the fact right?" check, not "do the two backends agree on it?".

This ordering de-risks the whole plan and delivers value before any cutover:

- **Shipped codegen is untouched** — the oracle only observes and alarms on divergence, so there
  is zero regression risk to the fast path.
- **Value from a PARTIAL fixpoint** — even liveness-only, or just the `Join` arm-meet, run as an
  oracle immediately starts flagging A1b-class divergences; it need not be complete enough to
  *ship*, only precise enough to *disagree correctly*.
- **Cutover / retirement becomes optional and evidence-based** — after the fixpoint has run beside
  for a while (green ⇒ the shipped fact is complete; red ⇒ it caught a latent hole), *then* decide
  whether it earns a place in the fast path (the perf fork, Open question 4). The old analysis is
  retired only if and when that decision lands.

So the coexistence is not just migration scaffolding — permanent coexistence (cheap shipped
analysis + fixpoint oracle) is a legitimate *end state* that fits loft's "validation, not proof"
stance and strengthens the very safety net the rustc evaluation identified as load-bearing.

## Design decisions (recommendations; the forks are Open questions below)

- **Structured dataflow over the Value-IR — NOT a MIR basic-block lowering.** loft's control
  flow is structured (if / match / `Loop` / break / return — no gotos; `for` desugars to
  `Loop`). A syntax-directed dataflow with an explicit **fixpoint at `Loop` nodes** and a
  **meet at if/match arm-joins** is sufficient and far cheaper than lowering to basic blocks.
  break/return propagate an abnormal-exit state to the enclosing loop-exit / function-exit
  (standard structured abstract interpretation). Explicit CFG stays the fallback iff
  irreducible control flow ever appears (it should not).
- **The lattice.** Per-variable state over `⊥(unreachable) ⊑ {Dead, Owned, Borrowed(base-set),
  Moved} ⊑ ⊤`, meet at joins. `Join` (owner depends on which arm ran) stops being the
  all-defs join and becomes a genuine **per-path lattice value** at the arm-meet; its runtime
  witness (`OpBindOrCopy`) is emitted only where the meet is genuinely `Owned ⊔ Borrowed`.
- **Liveness is the core; compute it first.** Backward liveness feeds *both* free-placement
  (dead ⇒ free) and move-elision (last-use). It is the exact-construction kernel — get it
  right and validated before the forward ownership pass rides on it.
- **Interprocedural stays a per-fn summary.** Keep the existing memoised `return_ownership`
  summary as the callee contract; the intra-procedural fixpoint consumes callee summaries at
  call sites (like rustc's per-fn signatures). No whole-program dataflow.
- **Migration is shadow-mode, one consumer per commit — never big-bang.** The old analyses
  keep shipping until each consumer is proven against the new fact via the @PLN89 oracle. This
  is the whole safety story (see Phase ordering).

## Composition matrix — Stage A

The "cells" are the dataflow's **correctness surface**, not a new language feature. Grid,
each a `/tmp` probe first, the differential oracle (old-vs-new fact + runtime value/leak/poison)
is pass/fail, graduate to `tests/scripts/`:

`{liveness, ownership, Join}` × `{straight-line, if/match-join, single loop, nested loop,
break-out-of-loop, early-return-in-loop, loop-carried borrow, the A1b temp-subject-borrow-return
shape, the v[i]??d Join shape, mutate-through-shared-alias (@PLN93)}` × `{interpret, native}`.

Done = every cell green on both backends **and** the new fact strictly subsumes the old
(same where the old was right, correct where the old was incomplete — the A1b class).

## The re-assertion sites it must collapse (design-protocol § count the sites)

The plan's success = all of these read the **one** dataflow solution (O-Deps realised, not
aspired). Phase 0 must produce the *complete* inventory; the ones already known:

- **Free placement** — `scopes::get_free_vars` (`src/scopes.rs:3157-3420`, the
  `owns`/`in_ret`/`skip_free`/`captured_ref` gate).
- **Own-vs-borrow materialisation** — `state/codegen.rs:1753`, `:3700`; `generation/dispatch.rs:53`,
  `:213`, `:466`; `generation/mod.rs:1099` (all via `ownership_of`, gated `join_own_enabled`).
- **Move-elision / borrow-inline** — `scopes::move_elide` (`:454`) reading `MovePlan`;
  `scopes::elide_borrows` (`:307`) reading `ElidePlan`.
- **Displaced-owned strip** — `scopes::run_scan_phase` via `displaced_owned_slots`.

That is N ≫ 1 sites re-reading facts that two producers approximate; the plan is exactly the
"one chokepoint N sites route through" made real.

## Sub-arcs

Concrete, executable per-step gates for the oracle end-state (P0–P3) live in
[`IMPL.md`](IMPL.md) — Step 0 (falsify the oracle's reason to exist) through Step 5 (land the
oracle beside), each green on both backends with shipped output byte-identical.

| Item | Concern | Status |
|---|---|---|
| **P0** — Characterise + equivalence harness + `DATAFLOW.md` (lattice/transfer on paper) + tractability falsification | design gate | Open |
| **P1** — Structured-CFG walker + **backward liveness**, shadow-diffed vs the position-proxy | code | Open (dep: P0) |
| **P2** — **Forward ownership** dataflow (`Owned/Borrowed/Moved` + per-path `Join`), consuming callee summaries | code | Open (dep: P1) |
| **P3** — Run the fixpoint **beside** as an independent completeness oracle (default-on shadow, alarm on divergence); triage every old-vs-new disagreement | validation | Open (dep: P2) |
| **P4** — *Optional, evidence-gated:* cut over consumers one-per-commit (free → own → elision), retire the proxy/insensitive-join, reconcile `formal/ownership.md` | code | Open (dep: P3 + perf fork) |

## Phase ordering

1. **P0 — the falsification gate.** Complete the re-assertion-site inventory; write `DATAFLOW.md`
   (lattice, transfer functions per IR op, join/loop rules); build the differential harness
   (compute old fact + stub new fact, diff at every site on the corpus + `program_ownership`
   fuzzer, both backends); hand-work the fixpoint on the A1b, `v[i]??d`-Join, nested-loop, and
   early-return-in-loop shapes to prove monotonicity + termination. **Gate:** if a consumer needs
   info the lattice can't carry, or the fixpoint isn't monotone on loft's real shapes, STOP and
   redesign before any migration code.
2. **P1 — liveness first.** Cheapest exact-construction kernel; shadow it against the current
   last-use, feed nothing yet.
3. **P2 — ownership forward pass** on the same CFG.
4. **P3 — coexist as the independent oracle (a valuable end state on its own).** Run the fixpoint
   beside the shipped analysis (default-on shadow), diffing at every decision site on the corpus +
   `program_ownership` fuzzer, both backends. Triage every divergence: new-more-precise ⇒ an
   A1b-class latent hole in the *shipped* fact (win — fix + graduate a regression); new-wrong ⇒ fix
   the transfer function. Shipped codegen is untouched, so this phase can be the stopping point.
5. **P4 — optional, evidence-gated cutover + formalise.** ONLY once the oracle has run
   green-or-caught for a while AND the perf fork (Open q4) resolves toward "the fixpoint can ship":
   route each consumer to the new fact one commit at a time (byte-identical-or-strictly-better,
   poison+leak both backends), retire the proxy/insensitive-join, and move `formal/ownership.md`
   from "validated" to "structural". Otherwise the fixpoint stays a permanent oracle and this phase
   is not taken. Keep the `LOFT_POISON`/`LOFT_UAF` runtime backstops either way.

## Open design questions

1. **Structured dataflow vs explicit basic-block CFG.** Recommend structured; P0 must audit
   `break`/labeled-break/`Loop` desugaring to confirm no irreducible flow.
2. **Per-variable vs per-path/per-field granularity.** Recommend per-variable first (sound,
   matches today). Per-path (partial moves — move `x.a` while `x.b` lives) is a *precision*
   follow-on, not a soundness need under loft's copy-default; gate on demonstrated demand.
3. **Unify the elision half onto the same CFG?** Recommend yes — compute liveness once and feed
   both, retiring the position-proxy entirely (removes a whole fragile substrate; the elision
   half gains loop-precision for free).
4. **Perf: is the fixpoint the *shipped* analysis, or the *oracle*?** A worklist fixpoint costs
   more per compile than today's single pass ("pays no `Position` clone, stays byte-identical",
   `use_analysis.rs:206-208`). Fork: (a) ship the fixpoint; or (b) ship a cheaper conservative
   pass and run the fixpoint as the **machine-checkable completeness oracle** in CI/fuzz/verify
   — which fits loft's "validation, not proof" stance and could make the fixpoint valuable even
   if it never enters the fast path. Measure in shadow mode before deciding.

## Cross-arc dependencies

- **@PLN85** (store-lifetime retirement) — defines the `Own` fact this plan makes flow-sensitive.
- **@PLN89** (differential oracle) — the equivalence gate that makes shadow-mode migration safe;
  hard dependency for P0/P3.
- **@PLN90** (copy-diagnostics / move-elision) — a primary *consumer*; its `MovePlan`/`ElidePlan`
  become dataflow queries.
- **@PLN93** (collection capture) — supplies the shared-mutation cells the ownership lattice must
  model (many non-owning aliases of one owned store).

## See also

- Implements the beacon in [`../../OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) ("a sound,
  complete, statically-computed ownership system … from which every codegen decision derives
  mechanically") and would move [`../../formal/ownership.md`](../../formal/ownership.md) from
  *validated-complete* to *structurally-complete*.
- Current substrate: [`../../../src/use_analysis.rs`](../../../src/use_analysis.rs) (analyses A + B),
  [`LIFETIME.md`](../../LIFETIME.md), [`DEPS_INVENTORY.md`](../../DEPS_INVENTORY.md).
- Tracked as [`@PLN94`](https://github.com/loft-lang/plans/issues/94); promoted from the
  "usage-analysis vs rustc" evaluation.
