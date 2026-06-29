<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# NEXT SESSION — the join-aware ownership analysis (over-free class fix)

Cold-start handoff. Written so a fresh session can `/clear` and build the @PLN85 over-free fix
**the right way** — without re-deriving anything. This is the **@PLN25 ↔ @PLN85 convergence
keystone** = wide-release **gate 1** ([STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md)).

**Reading order:** this file → [over-free-class-study.md](over-free-class-study.md) (the full class
study + § Root-cause drill-down + § Three chokepoints) → [fuzz-proof-gate.md](fuzz-proof-gate.md)
(the instrument that mapped it) → [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) (`deps` = loft's
borrow checker, the north star) → `src/use_analysis.rs` (the analysis to extend) +
[`../25-nullable-sequences/use-analysis-prework-design.md`](../25-nullable-sequences/use-analysis-prework-design.md)
+ `materialization-algorithm-design.md` (the @PLN25 design this continues).

---

## TL;DR — what to build, and why this way

The over-free class (borrowed view escapes, its store freed while still live → UAF/leak) is **not
fixable by per-site patches**. This session PROVED (not hoped) that it needs **one ownership fact**:
for any value that escapes into an owned position (append / return / reassign), is its store
**Owned**, **Borrowed**, or a runtime **Join** (owned-on-one-branch, borrowed-on-the-other — the
`v[i] ?? default` case)? The free sites must read that fact instead of re-deriving it.

**This fact IS @PLN25's copy-vs-borrow / `materialization_mode` predicate** (`src/use_analysis.rs`).
The over-free fix and the elision are two faces of one analysis. So: **extend `use_analysis`, do NOT
build a parallel analysis in `scopes`.**

**Methodology (the user's explicit direction — "do it the right way"), which `use_analysis` already
embodies:** build the fact → keep it **inert** (compute + dump under `LOFT_MATERIALIZE_DUMP`, wired
into NO codegen) → **test it separately** on the cases → **only then use it optionally** (behind a
flag, like `LOFT_ELIDE_T1`). Its module doc states this verbatim.

---

## What is already established this session (do not re-do)

- **The instrument exists** — a fuzz-proof harness, on branch `tuxedo-pln85-fuzz-proof-gate` (off
  fresh `main`):
  - `fuzz/ownership_fuzz.py` — runner: interp fast-loop + native replay; oracle = CRASH(signal) /
    LEAK(`"stores not freed"`) / DIVERGENCE(interp≠native). `--poison`, `--self-test` (P14).
  - `fuzz/grammar_gen.py` — generator: `shape (source×delivery) × value × churn` = 54 cells.
  - The cross-backend differential ORACLE already existed (`tests/differential_oracle.rs` @PLN89);
    the harness reuses that approach. Do NOT rebuild the oracle.
- **`LOFT_POISON=1` is built** (`keys.rs::poison_enabled` + `allocation.rs::free_named`) — arena
  poison-on-free, BOTH backends. Turns a SILENT UAF loud (it caught `elem_accumulate-none`, which
  the differential alone missed). Under poison the crash class is **churn-independent** → a
  none-churn cell + poison is a deterministic acceptance test, no 200-store stress harness needed.
  (@PLN54 S3, store-record half; the freed-stack-slot half remains.)
- **The over-free class is FULLY PROBED** — boundary map in
  [over-free-class-study.md § Generated boundary map](over-free-class-study.md). Live class =
  exactly **struct-value × {match-arm, element-accumulate, conditional-local-view}**; the entire
  field-view family + index-read (#426B) + nested-field + ALL scalar cells are **clean**.
- **The root-cause was CORRECTED** — it is NOT "borrow-set not propagated" (the dep IS propagated).
  See § Three chokepoints below.

## The three chokepoints (the map the fix must hit)

Repros saved under `bytecode-comparisons/462-*`. Each is a distinct emit site deciding own-vs-borrow
wrong; they SHARE one invariant but need the analysis to read it.

| shape | signature | chokepoint (pinned) | repro |
|---|---|---|---|
| `local_source` | LEAK, both backends | **reassign-free** (`scopes.rs`) — a var first OWNS a fresh store (`chosen = dflt()`), is reassigned to a borrow (`chosen = pool[wj]`), and the displaced owned store is never freed. `LOFT_LEAK_SITES`: leaked at the `dflt()` body. | `462-reassign-displaced-own-{BROKEN,WORKING}.loft` |
| `elem_accumulate` | interp SIGSEGV | **append source-free** — `out += [view]` emits `OpCopyRecord(src, 0x8000)` + `OpFreeRef(src)`; `src` (= `pick`'s return `t[i] ?? m_none()`) is a `??` JOIN. `LOFT_UAF_SRC`: freed-at-`pick` / read-at-`collect`. | `462-elem-accumulate-source-free.loft` (borrow) + `462-elem-accumulate-owned-branch-CLEAN.loft` (owned) |
| `match_return` | interp SIGABRT (downstream `d_nr=u32::MAX` corruption) | **arm-return delivery** (`materialize_vector_arms_into`, `parser/control.rs`) — reassigns the materialize buffer `_mv_items_1` (owned) to `OpGetField(e,4)` (a borrow of the enum field); the borrowed field is freed downstream. | generated `462-match-arm-delivery.loft` |

**The proof that per-site patches fail (do not retry the bit-gate):** `elem_accumulate`'s
`OpCopyRecord 0x8000` source-free is **load-bearing for the owned branch** — `462-elem-accumulate-owned-branch-CLEAN.loft`
(all `m_none()`) is clean BECAUSE the append frees it. Both branches carry the SAME static type
`M["t"]` (the `??` join). So gating the bit on `dep.is_empty()` fixes the borrow case and **leaks the
owned case** — same dep, opposite correct answer. The free decision is **runtime-dependent**. Hence
the analysis must model the **Join**, and the fix must make a Join value Owned where it escapes
(materialize the borrow branch) — not statically suppress the free.

## The one invariant (what the analysis serves)

> At every program point each heap store has exactly ONE owner; all mutation flows through that
> owner; a non-owning alias is read-only and never outlives its owner.

Four violations, all present in the class: leak (free skipped), double-free, use-after-free (alias
outlives owner), silent corruption (two owners). The fix reads ONE carried fact at the free sites
instead of re-deriving it (the OWNERSHIP_MODEL north star; the two already-landed over-free fixes —
instances 1+2 in over-free-class-study.md — used exactly this template and generalised).

---

## The build — staged, each stage its own commit

**`src/use_analysis.rs` is the home** (479 lines). It already tracks every fact needed: `database_vars`
(freshly-allocated = **Owned**), projection ops `OpGetVectorNullable`/`OpGetField` (= **Borrow** of a
base var), `def_count` (reassignment), `append_src`/`append_expr`, the `Uses` pre-order/loop-depth
visitor. `Verdict` is currently the binary `{Borrow, Copy}`; consumed (gated) by `scopes.rs:277`
(`elision_plans`).

### Stage 1 — the inert `Join` fact (behaviour-neutral)
- Extend the analysis with an ownership classification — `Owned | Borrowed | Join` — for the values
  that reach a free site (append element source, return value, reassign RHS). Compute it from the
  facts `Uses` already collects: `database_vars` → Owned; projection-of-a-var → Borrowed of that
  base; a coalesce/`??` (or `if/else`) whose arms split Owned/Borrowed → **Join**.
- **Wire into NOTHING.** Print under `LOFT_MATERIALIZE_DUMP` (the existing flag). Full suite stays
  byte-identical (gate: `make test` green; no IR change).

### Stage 2 — test it separately on the cases
- A unit test (mirror `use_analysis`'s existing tests) asserting the classification on the four
  `462-*` repros: `accum` source → **Join**, `accum_owned` source → **Owned**, `local_source`'s
  `chosen` at the reassign → **Join** (owned-init displaced by borrow), the field-view family →
  **Borrow-of-param** (safe to return, the caller owns it). This tests the VERDICT, not emitted code.
- Gate: the verdicts match the hand-computed expectation for every cell. Iterate the analysis here,
  with NOTHING depending on it yet.

### Stage 3 — use it optionally, one free site at a time
Behind a flag (the `LOFT_ELIDE_T1` pattern — e.g. `LOFT_JOIN_OWN`). Each site reads the verdict:
**Owned → free; Borrowed → don't free; Join → materialize the borrow branch to Owned at the escape,
then free.** Land one site per commit, each gated on its `462-*` repro + the 54-cell matrix +
`LOFT_POISON` + the leak gate, **on BOTH backends** (loft-codegen rule):
1. `elem_accumulate` — the append (`out += [Join]`): materialize the `??`-element view to owned.
2. `local_source` — the reassign-free: free the displaced owned store (the owned-init `dflt()`).
3. `match_return` — `materialize_vector_arms_into`: the buffer-reassign-to-borrow case.
- Flip default-on only when all three repros + the full matrix are green on both backends and
  `LOFT_POISON=1` over the corpus is clean — that green is wide-release gate-1 "stabilized".

### The loft-codegen gate (MANDATORY at every emit change)
Prove the WORKING bytecode beside the broken one, BOTH backends, BEFORE editing (`loft introspect`).
The `462-*` pairs are that artifact for sites 1–2; capture the match pair for site 3. Do not edit a
generator you cannot point the intended ops at. Stop-condition: a change that fixes one backend and
regresses the other is not landable.

---

## Tools + commands (all built this session, on the branch)

```sh
B=target/release/loft
# the fuzz harness (the acceptance instrument):
python3 doc/claude/plans/85-store-lifetime-retirement/fuzz/grammar_gen.py --out /tmp/gen
python3 doc/claude/plans/85-store-lifetime-retirement/fuzz/ownership_fuzz.py --corpus /tmp/gen --poison   # 54-cell map
python3 .../fuzz/ownership_fuzz.py --self-test                                  # P14 positive control
# diagnostics:
LOFT_POISON=1 $B --interpret prog.loft       # arena poison-on-free (silent UAF -> loud), both backends
LOFT_UAF_SRC=1 $B --interpret prog.loft       # OpCopyRecord reads-a-freed-source: where freed / where read
LOFT_LEAK_SITES=1 $B --interpret prog.loft    # leaked stores grouped by allocation site
LOFT_MATERIALIZE_DUMP=1 $B --interpret prog.loft   # the use_analysis verdicts (Stage 1 output)
$B introspect prog.loft                       # IR + bytecode + native Rust (the loft-codegen gate)
```

Acceptance repros: `bytecode-comparisons/462-*.loft`. The 54-cell matrix is `grammar_gen.py`'s output.

## Branch + state

- **Branch `tuxedo-pln85-fuzz-proof-gate`**, off fresh `main` (PR #469 / @PLN25 scalars+DN4 squashed
  in as `749652ea`). Commits: fuzz-proof slot + harness (`grammar_gen.py`, `ownership_fuzz.py`),
  `LOFT_POISON`, the boundary map, the root-cause correction + 3-chokepoint drill-downs. No PR.
- `@PLN25` scalars half: the value model is settling (PR #469 landed scalars Phase 0 + DN4 default-on);
  remaining @PLN25 in [`../25-nullable-sequences/RESUME.md`](../25-nullable-sequences/RESUME.md). The
  Join fact is shared with that work — keep them ONE analysis.

## Cross-refs

- [over-free-class-study.md](over-free-class-study.md) — the class, the boundary map, § Root-cause
  drill-down, § Three chokepoints (the corrected diagnosis + the per-site pins).
- [fuzz-proof-gate.md](fuzz-proof-gate.md) — the instrument + its increments (1–4).
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — `deps` as the borrow checker; the north star.
- `src/use_analysis.rs` + [`../25-nullable-sequences/`](../25-nullable-sequences/) `use-analysis-prework-design.md`,
  `materialization-algorithm-design.md`, `copy-elision-design.md` — the @PLN25 analysis this extends.
- [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md) — gates 1 (this) + 2 (@PLN25).
- @PLN54 S3 (`plans/54-sanitizer-coverage-expansion/`) — `LOFT_POISON` (built); freed-stack-slot half remains.
