<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 103 — Lifetime inspector: render the ownership fact the store-lifetime bugs kept hiding

**Status — P1 core LANDED, P1.5 next (2026-07-12).** P0 ✅ (all four steps). **P1 ✅ core:** the static
overlay `loft introspect --show-ownership` ships — per-binding `ownership_of` over the final IR, the
corrected render rule (self-base = caller-arg, synth-buffer = Owned-via-backing, else genuine alias),
scalars elided, deterministic, opt-in, matches the acceptance golden, `--diff` works. **Remaining P1:**
P1.5 (delivery-model lens) + P1.4b (free-site / reassign rows). Then P2 (per-backend), P3 (timeline). Canonical id
[`@PLN103`](https://github.com/loft-lang/plans/issues/103) (`status:future`, `subject:loft`). Serves the
ongoing @PLN85 store-lifetime-retirement arc and **supersedes the "Dep-graph / lifetime visualizer"
scope deferred on 2026-05-13** (DEBUG.md). This file is the design + the coverage-driving corpus + the
catalogue of probes and oracles built in this area; it is written design-before-implementation because
every past attempt to "just add a trace" produced another one-off oracle instead of a durable view.

## The one fact (one sentence)

Nearly every store-lifetime bug we have fixed turned on a single fact that is true in the IR but
invisible at every normal observation point: **is this value OWNED or a BORROW/view, and of WHICH
store — and does that ownership TRANSFER on return** — together with its second face, **emit-order and
cross-backend representation** (a free emitted before its alloc/return; a store freed-then-reused while
a live result still points at it; a value that is a stack-ref on interp but an owned deref on native).
A tool that renders that fact per binding / per return / per store, on **both backends at once**, would
have made essentially the entire corpus below obvious on sight.

## Why this is a design, not a feature list (the tell)

We do not lack traces — we have ~12 store-lifetime env-vars (`LOFT_STORES`, `LOFT_STORE_GUARD`,
`LOFT_TRACE_DB/CR/COPY`, `LOFT_REST_ORACLE`, `LOFT_MATERIALIZE_DUMP`, `LOFT_LOG=scope_debug/poison_free`,
`LOFT_UAF_SRC/REUSE/GEN`, `LOFT_WATCH_STORE`, `LOFT_POISON`, `LOFT_NO_SLOT_REUSE`, `LOFT_PLN85_OWN`,
`LOFT_TRACE_RR`) plus an ASan gate and a differential oracle — and a **deferred visualizer scope**. Each
was hand-built to see ONE facet of ONE bug class, then left behind. The richest fact of all,
`use_analysis::ownership_of(...) -> Own{Owned|Borrowed{base}|Join{base}}`, is **Rust-API-only**; the
only ownership fact that reaches a CLI is the raw `[deps]` suffix of `loft introspect types`. The design
question is therefore NOT "what new trace do we add" — it is **"make the ONE fact first-class and
retire the one-off oracles into it."** The tell that this is right: the same fact recurs across every
group of the corpus, and the dead-ends in the recent empty-arm fix (see the corpus) were all *"I could
not see which model / owner applied at this point."*

## The recurring invisible fact — the coverage taxonomy

The corpus (next section) sorts into **five kinds of invisible fact**. These are the coverage axes: the
inspector is complete only when each kind is rendered.

| # | Fact kind | What is invisible | Representative bugs |
|---|-----------|-------------------|---------------------|
| **K1** | **Ownership** | owned vs borrowed vs **runtime-join**, and of WHICH store; does it transfer on return | #405, probe-05/cbor, #437, #457, #306, #316, #496, D-own-2, over-free, fn-ref L1/L2/L3 |
| **K2** | **View vs copy** | a projection read (`v[j]`, `o.field`) ALIASES; only a whole-value bind COPIES | #338, #415, #261, #390, #260 |
| **K3** | **Free ordering / init-dominance** | free emitted before its alloc / return / write; free of an un-initialised slot; abort-skipped frees | Class A/B (`..rest`), #457, @P377, @P383, @P356, #322 |
| **K4** | **Delivery model** | which of Rename / CopyBorrow / ForwardCopy / Materialize / AsIs; per-arm delivery target + static type; an `[]` arm that is an undelivered `null`; a doubled vector | D-own-1, empty-arm (both contexts) + its 4 dead-ends, #492, #409/#410 |
| **K5** | **Backend divergence** | the SAME value-identity fact realised differently per backend (interp borrows / native copies; `&str` vs `String`; OOB raise vs null) | @P383, #356, #496, #347, p9, Class C |

**Recurring theme:** K1 is by far the largest and most damaging (silent whole-store double-frees + per-call
leaks); K2 is the most frequent *silent-corruption* class; K3 is rarer but highest-severity (SIGSEGV /
dangling reads); K4/K5 are K1/K2 seen through the delivery pipeline and through Rust's borrow checker.

## The coverage-driving corpus (what each bug needed made visible)

Distilled from the @PLN85 arc (`plans/85-store-lifetime-retirement/`), the @PLN35 `..rest`/empty-arm
work (`plans/35-match-peg/`, `rest-store-lifetime/`), and the historical PROBLEMS.md corpus. One row per
representative bug; `→` names the single fact an inspector would render to catch it.

### K1 — ownership (owned vs borrowed vs join, of which store)

| Bug | Symptom | → inspector shows |
|-----|---------|-------------------|
| **#405** cond×unused NRVO | interp SIGSEGV / native clean (divergent) | `ki` is a borrowed stack-ALIAS of `__ref_1` — same store, two independent `OpFreeRef` (double-free) |
| **probe-05 / cbor** enum-arg vec-return | `enc(k0..2)` held live → `5 5 5` / garbage | the return-SOURCE set: `returned_var` yields one var, so every arm buffer looks freeable → freed-on-return then reused while a prior result aliases it |
| **#437 / cluster V** NRVO adopt/append | leak 1/fn; multi-arm drops a buffer | a vector local's **claimed dep ≠ the store it owns** after `+=` (append re-points the dep, no strip) |
| **#457** adopt-free-collapse | wrong `len` + non-det SIGSEGV | per-arm `__ref_N` freed BEFORE `return out`; `out` ALIASES the buffer being cleared |
| **#306 / #316 / #496** returned/reassigned view | double-free / leak / cleared source | the slot BORROWS a param store (not owned); ownership CHANGES across reassignment (owned→borrow) with no free at the transition |
| **D-own-2 (#492 / #495)** JOIN return | doubled vector / native over-free | a **runtime** owned-OR-borrow join — one static type, opposite free answer per path |
| **fn-ref/lambda L1/L2/L3** | leak / latent double-free | callee attr-space buffer indices carried into caller space unchanged — a mis-mapped ownership dep |
| **over-free (3 chokepoints)** | leak / UAF / SIGABRT | `v[i] ?? default` runtime join; a reassignment's displaced-owned store invisible once the dep flattens |

### K2 — view vs copy on read/bind

| Bug | Symptom | → inspector shows |
|-----|---------|-------------------|
| **#338** struct-element swap | loses/dupes a record | `tmp = v[j]` is a VIEW of slot j; a later write to `v[j]` mutates what `tmp` sees |
| **#415** struct field read/return | `af = bx.v; bx.v += [9]` → stale | a projection ALIASES the field store; only a whole-value bind COPIES |
| **#261** field `=` appends | `1 2 3 99 100` | the LHS field slot keeps its OLD store identity across `=` |
| **#390 / A9** self-slice-assign | `v = v[a..b]` → nulls | a slice iterator is a VIEW over the source; clear-before-read destroys what it reads |
| **#260** closure snapshot | mutations don't propagate | a non-scalar capture was a COPY, not a live DbRef borrow |

### K3 — free ordering / init-dominance

| Bug | Symptom | → inspector shows |
|-----|---------|-------------------|
| **Class A/B** (`..rest` `__vdb`) | leak | `OpFreeRef(__vdb)` precedes its `OpDatabase(__vdb)` in source order; the block's return-type hoist decision (`hoists` / `NO-hoist`) |
| **@P377** pre-Set free | corruption | Call writes INTO a slot that is also its argument (NRVO); a free is scheduled before that write |
| **@P383** `??` text temp | dangling read | the stack value is a BORROW (ptr+len) into the temp being freed; the free is ordered before the consuming read |
| **@P356** free of uninit slot | SIGSEGV | a branch left the freed slot holding a view/sentinel — init does not dominate free |
| **#322** abort-skipped frees | false "leak" | frees exist in IR but never ran (mid-`main` `raise` → `code_pos=u32::MAX`) — the scheduled-but-unexecuted free set |

### K4 — delivery model

| Bug | Symptom | → inspector shows |
|-----|---------|-------------------|
| **D-own-1** block_result thicket | orphan / undelivered literal | ONE canonical per-return `Delivery` verdict (Rename/CopyBorrow/ForwardCopy/Materialize/AsIs) with the deps fact that produced it — vs 15 per-shape re-derivations |
| **empty-arm E0308** (return + bind) | native `()` where `DbRef` expected | per arm: delivery target + static type — an `[]` arm is an undelivered `null`/`()` while siblings deliver a `DbRef` |
| — dead-end (a) Rename-only | fixed return, not bind | the TWO consumption contexts (retbuf-rename vs alias-owned) side by side |
| — dead-end (b) blanket materialise | DOUBLES a borrowed return | the invisible PREAMBLE precondition (`OpClearVector` before a join_own append) |
| — dead-end (c) `ownership_of` @block_result | can't tell the models apart | ownership at the ORIGINAL arm vs after synthesis — the observation point was downstream of the rewrite |
| — dead-end (d) `CopyMatch` | leak | the copied value still OWNS an internal store → the copy orphans it |
| **#492** JOIN append | doubled (len 5 not 2) | per delivery, clear+replace vs append, and the buffer's occupancy on entry from each arm |
| **#409/#410** FFI return + `+=` | value drop → SIGSEGV | the local borrows a foreign/null store while its own dep is empty |

### K5 — backend divergence

| Bug | Symptom | → inspector shows |
|-----|---------|-------------------|
| **@P383** block-tail text | native copies, interp borrows | per backend: block-tail heap value materialised (owned) vs forwarded (borrowed) |
| **#347** indexed text compare | native E0308 on `>=` | an indexed `vector<text>` element is a borrowed `&str`; a local is an owned `String` |
| **#496** one borrow, two corruptions | native stale-prune vs interp adopt | the borrow verdict for the SAME expression on BOTH backends |
| **Class C** slice-enum reuse | interp corrupts, native clean | a temp `x = for_var` dropped the `["subj"]` dep → owned-looking view frees the subject (interp) vs redundant copy (native) |
| **p9** binary write+read | interp leak/corruption | the value is a stack-ref (interp) vs deref-to-record (native) |

## The probes we built (catalogue — the empirical basis)

Every fix above was earned with a throwaway probe matrix (the CLAUDE.md matrix-first discipline). These
are the durable ones worth knowing — the inspector's acceptance corpus (P0) is assembled FROM them.

| Probe corpus | Home | What it varies / proves |
|---|---|---|
| **D-own-1-corpus** (one fn per delivery path) | `plans/85-…/bytecode-comparisons/` | byte-identical `loft introspect` before/after (Mode-B refactor gate) |
| **D-own-1-promotion-corpus** | `plans/85-…/bytecode-comparisons/` | fn-ref/lambda return verdict rungs (L1/L2/L3) |
| **05-matrix-A..F** | `plans/85-…/probes/` | cond × unused × nested-loop cells, hand-computed "completes cleanly" |
| **d-own-2/** (adopt-vs-copy) | `plans/85-…/probes/` | struct/vector/JOIN × bindings × control; `join-vector-return-append`, `loop-copy-view-native-divergence`, `owned-reassign-mixed` |
| **block-return-move/** (p1–p8) | `plans/85-…/probes/` | owned-fresh-local vs borrows-outer; **p8 is the negative guard** (must stay a borrow) |
| **leak-462/**, **462-\*** BROKEN/WORKING pairs | `plans/85-…/probes/` | adopt-rereturn-leak; elem-accumulate; reassign-displaced-own |
| **grammar_gen.py** (54-cell) | `plans/85-…/fuzz/` | 9 source × delivery × {struct,scalar} × churn, under differential + `LOFT_POISON` + leak, both backends |
| **rest-store-lifetime/** (29 probes) | `plans/35-match-peg/rest-store-lifetime/` | `..rest` free/alloc-order corpus + `ORACLE.txt` + `MATRIX.txt` + `run_oracle.sh` (29/29, 0 mismatch) |
| **85-store-lifetime-empty-vector-match-arm** | `tests/scripts/` | empty-`[]`-arm: return + bind + block-form + reuse (graduated guard) |
| **35d-slice-enum-reuse**, **85-fnref-lambda-return-ownership**, **462-\*** | `tests/scripts/` | graduated regression guards |

**Method lessons baked into these** (carry into P0): assert **value AND length AND leak** (a doubled
vector is leak-free — length catches it, leak does not); prove the harness can fail (a run-checked
control defeats a VACUOUS `leak=0` cell that never compiled); add the **reuse axis** (Class C corrupts
only on a *later* re-read); `LOFT_POISON` makes a crash churn-independent (no 200-store stress needed);
and interp-vs-native value divergence on identical IR is itself the tell.

## The oracles we built (catalogue — what the inspector consolidates)

| Oracle | Where | What it decides | Surface |
|---|---|---|---|
| **`ownership_of` / `Own`** | `use_analysis.rs:1652` / `:1092` | owned vs borrowed{base} vs join{base}, per value | Rust-API (default-on) |
| `return_ownership` / `free_sites` / `reassign_sites` / `displaced_owned_slots` | `use_analysis.rs:1602/1642/1608/1618` | return transfer; the two Gap-A free sites; owned→borrow transitions; displaced-owned slots | Rust-API |
| **`ownership_cfg`** (@PLN90/@PLN94) | `src/ownership_cfg.rs:511/599/764` | flow-SENSITIVE ownership + shadow-diff vs the flow-insensitive oracle | Rust-API |
| **`rest_store_oracle`** | `scopes.rs:6752` (call `:2051`) | `..rest` free/alloc order + confinement gate + hoist decision | env `LOFT_REST_ORACLE` (+ `LOFT_NO_CACHE`), stderr |
| differential value oracle (@PLN89) | test infra | interp == native VALUE (defeats correct-by-coincidence) | test harness |
| `dump_all` / `LOFT_TRACE_RR` / `classify_text_return` TRA | `use_analysis.rs:1703` / gen / — | per-fn `Own` dump; return-promotion sentinel; text-return verdict shadow | env-gated stderr |
| runtime witnesses | native codegen | `_own_store_<name>: DbRef` witness; `LOFT_PLN85_OWN` alloc trace | native / env |
| leak/UAF detectors | store + scopes | `LOFT_STORES=log/warn`, `STORE_GUARD`, `LEAK_SITES`, `UAF_SRC/REUSE/GEN`, `WATCH_STORE`, `POISON`/`poison_free`/`zero_claim`, `NO_SLOT_REUSE`, `check_store_leaks`, `NATIVE_LEAK_CHECK`, ASan `asan-leak-gate`, `LOG=scope_debug` | env / test |
| classifier verdict enums | parser | `Delivery` (`control.rs:175`), `VecBind` (`expressions.rs:300`), `RefDelivery`/`TextDep`/`RetPromotion` | private |

The inspector does not replace the deep-dive detectors (ASan, poison, UAF-gen) — it **subsumes the
routine ones** (`ownership_of`, `LOFT_REST_ORACLE`, `LOFT_STORES`, `LOFT_MATERIALIZE_DUMP`, `TRACE_RR`,
`scope_debug`) into two coherent views and leaves the detectors as backends for the hard cases.

## Existing inspection surface + the gap

- **`loft introspect {bytecode,rust,slots,types}`** (`introspect.rs`, `Section` `:31`, dispatch
  `repl.rs:589`/`main.rs:3851`). `Section::Types` (`emit_types` `:451`) is the ONLY ownership-adjacent
  CLI output — one row per var with a `[deps]` suffix (`Type::show`), i.e. the empty-vs-nonempty
  owned/borrowed split and base var *numbers*, but NOT the resolved `Join` verdict, NOT the
  interprocedural base remap, NOT the free site, NOT the delivery buffer.
- Runtime store tracing is a **flat interleaved** alloc/free stream (`LOFT_STORES=log`), logged under a
  store's *free*-time name (which may differ from its alloc name) — not a per-`store_nr` lifeline.
- The **deferred visualizer note** (DEBUG.md, 2026-05-13, "~1 week / L") already recorded the hard
  parts a real tool must handle: the `Vec<u16>` deps are OVERLOADED (owned / borrow / auto-Reference
  sentinel / callee-frame), the parallel mechanisms (`work_text` / `inline_ref_vars` / `closure_var_map`)
  are not merged, and **deps are TIME-VARYING** — a tool needs per-point snapshots, not one static table.

**The gap in one line:** no view renders the *resolved* ownership verdict + delivery model + free
sites + per-store timeline, and none renders it **per-backend side by side**.

## Failure paths first (the design's core — how a naive inspector breaks)

### F1 — classifying the arm's *result* value reports Owned (RESOLVED by P0.1 — simpler than feared)
The `ref_return` jo-arm synthesis (`control.rs:9972`, gated `keys::join_own_enabled()`) **wraps a
borrowed arm's RESULT in an owned copy** before `block_result`: the arm `Filled { items } => items`
becomes `Block{name:"jo_arm_copy", ops:[OpCopyRecord(_mvcopy_1, _mv_items_1, …), Var(_mvcopy_1)]}`, so
`ownership_of` on the arm's *delivered result value* (and on the function's `return_ownership`) reads
`Owned`. A naive overlay that classifies the delivered tail value therefore reports `Owned` everywhere
and is useless for exactly the K4 bugs.

**BUT P0.1 proved the borrow is DISPLACED, not destroyed** (probe `f1-join-recovery.loft`, both
join_own on and off): the borrowed SOURCE binding `_mv_items_1 = OpGetField(e,…)` survives verbatim in
the committed `def.code`, and `ownership_of(&data, d_nr, Var(_mv_items_1))` reads `Borrowed(base=e)`
on the FINAL IR. **⇒ The overlay renders PER-BINDING ownership** — classify each `Set(v, rhs)`'s
variable over the committed `def.code` — and links each delivered arm to the source binding its
`jo_arm_copy` consumed. **No pre-synthesis snapshot / parser hook is needed** — this was the design's
feared load-bearing risk, and it is retired: `ownership_of` reads `data.def(d_nr).code`, which is exactly
what `introspect` already holds post-parse. (The one caveat: `ownership_of` cannot be called *during*
the parse of a function — `def.code` is unpopulated until the body is committed — so the overlay runs
post-parse, which is where `introspect` runs anyway.)

### F2 — a static one-row-per-var table cannot state a runtime join or a time-varying dep
`Join{base}` (#495, #496, over-free) is owned on one path and borrowed on another; deps change across
reassignment (#316 owned→borrow). No single static cell is correct. **⇒ Render per-def-site / per-path
(a mini timeline per var), not one row per var; defer the genuine runtime joins to Tool 2's
per-execution witness.** The flow-insensitive `ownership_of` over-reports `Join`/`Borrowed` (never
invents `Owned`) — surface that conservatism explicitly rather than hiding it.

### F3 — one backend hides the bug
For all of K5 the bug IS the divergence (interp borrows / native copies — @P383, #496, #347, p9, Class
C). A single-backend view is blind to the entire class. **⇒ Both instruments render interp AND native,
and a divergence is a first-class highlight, not something a human diffs by eye.**

### F4 — the inspector must survive the programs that trip it
These bugs live in match-heavy, enum/struct-payload code — exactly the shapes most likely to exercise
under-tested corners of the dump path, which already carries hard `panic!` sites (e.g. the leak-check
aborts at `debug.rs:1333`/`:1339`). A dump/introspect crash on the target program makes the inspector
vacuous. **⇒ P0 runs the dump path across the full acceptance corpus and treats any panic / `unwrap` /
unreachable as a blocking fix.** Harden before extending. (A simple enum-`match` introspect is clean
today — do not assume a specific crash; find the real ones by running the corpus.)

## The design, per axis

### A1 — Tool 1: static ownership/delivery overlay — `loft introspect ownership`
A new `Section::Ownership` beside `Types` (`introspect.rs:31`/`emit_types` `:451`), consuming the
existing oracle + classifiers (no new analysis). Per function it renders, **per-binding over the
committed `def.code`** (P0.1 proved the borrowed source survives there — no pre-synthesis snapshot, F1):

- **Per slot / binding:** the resolved `Own` verdict (`Owned` / `Borrowed(base=name)` / `Join(base=name)`
  via `ownership_of` + `fmt_own`), the backing store, and the **VIEW vs COPY** disposition of its
  defining bind (via `VecBind` / whole-value-vs-projection) — covering K1 + K2.
- **Per return / arm:** the `Delivery` verdict (`classify_vector_delivery`) and, for a match/if, each
  arm's delivery target + static type — flagging an arm whose value is `null`/`()` while siblings
  deliver a `DbRef`, and clear+replace vs append occupancy — covering K4.
- **Per store / free:** alloc site + every free site (`free_sites`) with a **double-free /
  free-before-alloc / free-before-use / orphaned / abort-unreachable** flag — covering K3.
- **The reassign timeline:** `reassign_sites` + `displaced_owned_slots` render the owned→borrow
  transitions and any owned store abandoned without a free — covering the #316/over-free class.

Make `fmt_own`/`own_kind` (`use_analysis.rs:1686`/`:1676`) `pub`. Reuse `Type::heap_dep`/`is_heap_owned`/
`depend` (`data.rs:1473`/`:1488`/`:1573`) for the dep read. Add `--diff <baseline>` (mirrors the existing
introspect diff) so a before/after ownership delta is a first-class artifact.

### A2 — Tool 2: runtime store timeline — `LOFT_STORES=timeline`
A per-`store_nr` lifeline that consolidates `LOFT_STORES=log/warn` + `STORE_GUARD` + `TRACE_DB/CR/COPY`
into ONE keyed view: `alloc(op,site) → borrow(by var) → copy(to store) → free(op,site)`, with the
store's identity stable across the run. It answers the questions the flat stream cannot: *did this
actually leak, who freed what when, was it freed-then-reused while a live result still pointed at it,
is this a working-set high-water or a real leak* (the exact ambiguity this session hit — 32 "possible
leak" lines that were a working set). Covers K3 + the K1 runtime-join witness. Rendered per backend.

### A3 — per-backend rendering (F3)
Both tools accept a backend selector and a `both` mode that prints interp and native columns side by
side with divergences highlighted. This is what turns the entire K5 class from "diff two runs by eye"
into a single glance.

### A4 — consolidation
Once A1–A3 land, repoint the routine env-vars/docs at the two views (keeping the deep-dive detectors —
ASan, `POISON`, `UAF_GEN` — as named backends), and mark the deferred dep-graph visualizer scope
delivered-by-@PLN103 in DEBUG.md.

## Coverage matrix (fact-kind → observable → bugs)

| Kind | Rendered by | The observable | Corpus covered |
|------|-------------|----------------|----------------|
| K1 ownership | A1 slot/return + A2 witness | `Own` verdict + backing store + transfer-on-return | #405, probe-05, #437, #457, #306, #316, #496, D-own-2, over-free, fn-ref L\* |
| K2 view/copy | A1 binding | VIEW vs COPY + alias edge | #338, #415, #261, #390, #260 |
| K3 free order | A1 free flags + A2 timeline | alloc↔free order, init-dominates-free, abort-unreachable frees | Class A/B, #457, @P377, @P383, @P356, #322 |
| K4 delivery | A1 return/arm | `Delivery` verdict + per-arm target/type + occupancy | D-own-1, empty-arm + 4 dead-ends, #492, #409/#410 |
| K5 divergence | A3 both-backend | same verdict, interp vs native, highlighted | @P383, #356, #496, #347, p9, Class C |

## Probes to falsify FIRST (P0 — before building)

1. **F1 mitigation is real** — ✅ DONE (2026-07-12, see the P0.1 ladder step). The borrowed source
   binding survives on the FINAL IR (`ownership_of(Var(_mv_items_1)) = Borrowed(base=e)`), so the overlay
   classifies per-binding post-parse — no pre-synthesis snapshot, no downgrade. F1 retired.
2. **F4 harden** — run the dump/introspect path across the full acceptance corpus and fix any panic /
   `unwrap` / unreachable it hits; the path must survive the match-heavy, enum-payload shapes these bugs
   live in.
3. **Acceptance corpus** — one function per fact-kind (assembled from the probe catalogue: #338 view,
   #415 field-copy, #306 borrowed-return, #316 owned→borrow, #492 doubled, empty-arm, Class-C reuse),
   each with its KNOWN-correct verdict hand-written. The overlay is correct only when it prints the
   right verdict for every cell, INCLUDING the historically-broken shapes.
4. **Timeline disambiguation** — confirm `LOFT_STORES=timeline` distinguishes this session's working-set
   case (32 simultaneously-live vectors, no leak) from a genuine per-iteration leak.

## Implementation ladder — every step independently verifiable

Each step names its concrete edit AND a **VERIFY** with a runnable check + expected result; a step is
DONE only when its VERIFY passes. Probe/corpus files live under `plans/103-lifetime-inspector/probes/`
(the sanctioned adjacent probe dir). A phase's **GATE** must hold before the next phase starts. Rungs are
ordered so each depends only on the ones above it. Bound every ad-hoc run (`LOFT_TIMEOUT=60`).

### P0 — falsify + harden (prove the design's load-bearing assumptions BEFORE any overlay code)

- **P0.1 — F1 is real: the join is only visible pre-synthesis.** Write `probes/f1-join-recovery.loft`
  with the borrowed-arm-beside-owned-arm shape (`Filled { items } => items, Empty => [V{n:9}]` — an
  owned *literal* arm; a struct-enum `_ => []` fails to type-infer, a separate quirk). Classify each
  binding of the COMMITTED `def.code` via `ownership_of(&data, d_nr, Var(v))` and check the borrowed
  source binding is visible.
  **✅ RESULT (2026-07-12) — PASS, with a design SIMPLIFICATION.** On the committed IR:
  `Var(_mv_items_1) = Borrowed(base=e)` (the field projection), `Var(_mvcopy_1)` / `Var(__retbuf)` =
  owned buffers — stable with join_own ON and OFF. The arm's *result* reads `Owned` (the `jo_arm_copy`
  wrap), but the borrowed SOURCE binding survives and classifies correctly. **So the overlay classifies
  PER-BINDING over the FINAL IR — the feared pre-synthesis snapshot is NOT needed** (F1 retired). Probe
  `probes/f1-join-recovery.loft` (runs clean both backends). Method note: `ownership_of` reads
  `data.def(d_nr).code`, so it must run POST-parse (mid-parse every var reads `Owned` — `def.code`
  unpopulated); reproduce with a temporary per-binding dump in `use_analysis::dump_all` gated on an env
  var (that is how this was verified). **GATE CLEARED** — P1.3/P1.4 proceed on the per-binding design.

- **P0.2 — acceptance corpus + hand-written verdicts.** ✅ DONE (2026-07-12). `probes/acceptance/`
  = `acceptance.loft` (one fn per fact-kind: `k1_borrowed_arm` K1+K4, `k2_copy_vs_view` K2,
  `k4_emptyarm` K4, `k1_owned_or_borrow` K1 runtime-join) + `acceptance.expected` (per-binding verdicts,
  hand-checked). Runs clean, **warning-free** (`LOFT_DENY_WARNINGS=1`) and leak-clean on BOTH backends.
  **✅ RESULT:** the semantically-important borrows are all CORRECT on the final IR — the borrowed field
  projection (`_mv_items_1 = Borrowed(base=e)`), the projection view (`first = Borrowed(base=b)`), and
  critically the runtime JOIN (`r` and `return` = `Join(base=pool)` in `k1_owned_or_borrow`, the
  #316/#495 shape the oracle nails). **P1 FINDING (folded into P1.4):** an owned copy/materialise buffer
  classifies as `Borrowed(base=X)` where X is the var's OWN backing store (a synthesized `__vdb_*` /
  `__ref_*` / `_mvcopy_*`, or a self-base) — defensible but reads as an alias; the overlay's renderer
  must translate `base ∈ {self, synthesized-buffer}` → "Owned (backing=X)", reserving "Borrowed(base=X)"
  for a base naming a USER var/param. (Two originally-listed cells — the #338 in-place swap and a
  `cap = match` bind — were folded into `k2`/the K1 join to keep the corpus compiling clean; the swap
  is a mutation-order fact better shown by the P3 timeline than the static overlay.)

- **P0.3 — F4: the dump path survives the corpus.** ✅ DONE (2026-07-12). `loft introspect` bare + each
  of `--show-bytecode` / `--show-rust` / `--show-slots` / `--show-types` over both probes: all exit 0,
  no panic/`unwrap`/unreachable. (CLI note for the runner: sections are FLAGS — `introspect --show-bytecode <file>`
  — not positional; bare `introspect <file>` emits all.) No dump-path crash surfaced on this corpus;
  re-run when the corpus grows.

- **P0.4 — timeline seam feasibility.** ✅ DONE (2026-07-12). **Hooks:** alloc = `Stores::allocate`
  (`database/allocation.rs:429`, the `LOFT_STORES=log` branch, `+ alloc #<store_nr> <name>`); free =
  `Stores::free_named` (`allocation.rs:576`, `- free #<store_nr> <name>`). Both backends route allocation
  through `Stores::allocate` (native calls the same runtime), so ONE instrumentation covers both.
  **The id-stability catch (confirmed):** `store_nr` is a SLOT index REUSED on realloc (`allocate` reuses
  a `free` slot, `allocation.rs:406-412`), so one `store_nr` names different logical stores over the run
  — a per-`store_nr` lifeline conflates them (this IS the freed-then-reused ambiguity P3 must resolve).
  Also the free line uses the FREE-CALL's `name`, which differs from the alloc-time name (agent-E's
  flag, confirmed at `allocation.rs:578`). `Store.generation` (`store.rs:170`) is NOT the fix — it counts
  claim/resize MUTATIONS (coroutine staleness), resets per store. **P3 fix:** add a global monotonic
  `alloc_seq: u64` on `Stores`, stamp it into the currently-unused `Store.created_at` (`store.rs:186`,
  set to 0 at `allocation.rs:414`) at `allocate`, key every timeline event by `(store_nr, alloc_seq)`,
  and record the alloc-time `name` on the `Store` so the lifeline is labeled by alloc-name. **Feasible:
  two hook sites + one `u64` + repurpose an existing dead field.**

- **GATE P0 — ✅ CLEARED (2026-07-12).** P0.1 (per-binding on the final IR — no snapshot), P0.2
  (acceptance corpus + expected, green/warning-free/leak-clean both backends; the runtime-JOIN verdict
  confirmed correct), P0.3 (dump path clean over the corpus), P0.4 (timeline seam + `(store_nr,
  alloc_seq)` id fix). This branch is rebased onto `main` (which now carries #562's empty-arm fix).
  **P1 may proceed.**

### P1 — static overlay `loft introspect ownership`, single backend (interp)

- **P1.1 — expose renderers.** ✅ DONE. `fmt_own` (`use_analysis.rs`) made `pub`; added `pub render_own`
  (the P1 rendering rule) + `pub is_synth_buffer`. Build clean.
- **P1.2 — wire the section.** ✅ DONE. `Section::Ownership` (`introspect.rs`), `--show-ownership` CLI flag
  (`main.rs`), `:ownership` REPL verb (`repl.rs`). OPT-IN (like `Roundtrip`) — bare `introspect <file>` is
  unchanged (verified: 0 ownership sections in the default dump). NOTE: sections are FLAGS
  (`--show-ownership`), not the positional `introspect ownership` the draft assumed.
- **P1.3 — per-binding classification over the final IR (the P0.1 design).** ✅ DONE. `emit_ownership`
  walks each user fn's vars and classifies via `ownership_of(&data, d_nr, Var(v))` over the committed
  `def.code`; the function header shows `return_ownership`. No snapshot, no parser hook. Confirmed on the
  corpus: `_mv_items_1 = Borrowed(base=e)` and `_empty_arm_1 = Owned (backing=__vdb_1)` — the borrowed
  source + the fixed empty-arm are both visible, not all-`Owned`.
- **P1.4 — per-slot rows + the render rule.** ✅ DONE (core). Renders `#/arg/name/ownership`; a non-heap
  var → `—  (scalar)`. **RENDER RULE (corrected here from the P0.2 draft — a real fix): self-base
  ≠ owned buffer.** `ownership_of` reports a bare arg as `Borrowed{base=self}` and an owned delivery
  buffer as `Borrowed{base=<buffer>}`; the P0.2 draft wrongly folded BOTH self-base and synth-base into
  "Owned", which mislabels params (`e`, `pool`) as owned. Corrected `render_own`: self-base →
  "Borrowed(caller-arg)"; synth-buffer base (≠ self) → "Owned (backing=X)"; any other base → the genuine
  alias "Borrowed(base=X)"; `Join` untranslated. **VERIFY ✅:** overlay is deterministic and matches
  `acceptance.expected`/`.golden` — the runtime JOIN (`r`/`return = Join(base=pool)`) and every dangerous
  alias render correctly. (Deferred to a P1.4b follow-up: the free-site flags + reassign timeline via
  `free_sites`/`reassign_sites`/`displaced_owned_slots` — additive rows, not yet rendered.)
- **P1.5 — per-return/arm delivery.** ⏳ TODO. Render the `Delivery` verdict (`classify_vector_delivery`)
  + per-arm target/type + append-vs-replace. The empty-arm/#492 delivery facts. (The per-binding overlay
  already exposes the underlying ownership; this adds the delivery-model lens.)
- **P1.6 — `--diff <baseline>`.** ✅ DONE (free via the existing introspect `--diff`). Verified: identical
  → exit 0; a mutated baseline → exit 1, with `--show-ownership`.
- **GATE P1:** ✅ core cleared — `acceptance/` overlay == `.golden`, deterministic, opt-in, clippy-clean.
  Remaining for full P1: P1.5 (delivery lens) + P1.4b (free/reassign rows).

### P2 — per-backend divergence (the K5 class)

- **P2.1 — backend selector.** Add `--backend interp|native|both` to the overlay; render native's
  value-identity facts (stack-ref vs deref, `&str` vs `String`) from the generation-side consumers of
  `ownership_of` (`generation/dispatch.rs`). **VERIFY:** `--backend both` prints two columns per binding.
- **P2.2 — divergence highlight + `--only-divergent`.** **VERIFY:**
  `loft introspect ownership --backend both --only-divergent probes/acceptance/k5-classC-reuse.loft`
  lists exactly the binding where interp frees the subject and native copies; the K5 corpus rows
  (@P383 block-tail text, #347 indexed-text compare) each show a highlighted interp≠native cell.
- **GATE P2:** every K5 `.expected` divergence is rendered and highlighted; no false-positive divergence
  on a K1/K2 row that both backends agree on.

### P3 — runtime store timeline `LOFT_STORES=timeline`

- **P3.1 — stable per-store lifeline.** Add the `timeline` mode in the store subsystem (the P0.4 seam);
  capture `alloc(op, site)` with a STABLE id spanning alloc→free. **VERIFY:** on a 2-alloc program the
  mode prints two lifelines with distinct ids that persist to their frees.
- **P3.2 — borrow/copy/free events.** Key `borrow(by var)` / `copy(to id)` / `free(op, site)` to the id;
  render the ordered lifeline. **VERIFY:** for `k1`-style #457 the timeline shows `free` ordered BEFORE
  the returning read; for the freed-then-reused shape (probe-05) it shows `alloc#8 → free#8 →
  alloc#8-REUSED` while a prior result still references `#8`.
- **P3.3 — working-set vs leak summary.** **VERIFY:** on
  `tests/scripts/85-store-lifetime-empty-vector-match-arm.loft` the summary reports
  "N concurrently-live, all freed — NO leak" (this session's false-alarm case), while a deliberate
  per-iteration leaker (`probes/leaker.loft`) reports "K never-freed, growing".
- **P3.4 — per-backend timeline.** **VERIFY:** `k5-classC-reuse` shows the subject freed on interp but
  retained on native.
- **GATE P3:** the K3 corpus + the working-set disambiguation render correctly on both backends.

### P4 — consolidation

- **P4.1 — docs.** Add the two views to DEBUG.md / LIFETIME.md / TESTING.md `LOFT_LOG` ref; mark the
  deferred dep-graph visualizer (DEBUG.md, 2026-05-13) delivered-by-@PLN103. **VERIFY:** `make doc_hygiene`
  (or the doc drift guard) passes.
- **P4.2 — graduate the corpus.** Add a Rust test asserting the overlay output for `acceptance/` and a
  `.loft` guard for the timeline. **VERIFY:** `make test` green; the test FAILS if a verdict is reverted
  (prove it by temporarily reverting P1.3 → red).
- **P4.3 — retire the routine one-off oracles.** Reproduce `LOFT_REST_ORACLE`'s free/alloc-order
  predictions from the overlay's free-flags on the 29-probe `..rest` corpus. **VERIFY:** the overlay's
  flags agree with `rest-store-lifetime/ORACLE.txt` on all 29 (then alias/retire `LOFT_REST_ORACLE`,
  `LOFT_MATERIALIZE_DUMP`, `LOFT_TRACE_RR`; keep the deep-dive detectors).
- **GATE P4:** `make ci` green; docs consistent; the acceptance corpus + timeline guarded in `tests/`.

### Definition of done (the whole plan)

The five fact-kinds each render correctly on both backends against a hand-verified corpus that INCLUDES
the historically-broken shapes; the working-set-vs-leak ambiguity this session hit is resolved by the
timeline; the routine one-off oracles are subsumed; and a regression test fails if any verdict reverts.

## Out of scope (declared)

- **Changing the ownership analysis itself** — precision (collapsing `Join` back to `Owned`/`Borrowed`)
  is the flow-sensitive @PLN94 work; this plan RENDERS what the oracle already computes, faithfully
  (including its conservatism).
- **A GUI / graph render** — text tables + per-store timelines first; a graph is a later nicety, not the
  fact.
- **Fixing any corpus bug** — those are their own fixes; the corpus here is the coverage driver, not a
  work-list.
