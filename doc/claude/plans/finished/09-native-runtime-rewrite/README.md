<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 09 — Per-Op emitter dispatch

## Status — DONE 2026-05-02

Plan-09 + sibling plan-11 closed all 5 native-codegen P-issues
(P200 / P202 / P203 / P204 / P205).  Native suite reached parity
with `main`: **5/5 high-level + 95/95 sub-tests** (pre-plan-09:
2/5 + 87/93).  Reference for the per-Op emitter dispatch
architecture lives in [`../../../NATIVE.md`](../../../NATIVE.md) §
"Per-Op emitter dispatch (plan 09 phase 00)" line ~96.

This file is the closure record; the per-phase plan files in this
directory remain as historical archaeology.

### P-issue closure summary

| P-issue | Closed by | Reference home |
|---|---|---|
| P200 | phase 10 step 10.3 — `IntCompareEmitter` widens both operands of `OpEqInt` / `OpNeInt` / `OpLtInt` / `OpLeInt` to i64 | PROBLEMS.md § 200 |
| P202 | phase 06 — `n_parallel_queue*` runtime fns + emitters in `src/codegen_runtime.rs` + `src/generation/ops/parallel.rs` | PROBLEMS.md § 202 |
| P203 | phase 00 step 0.7b — let-bind-on-repeat in `DefaultTemplateEmitter` (auto-detects repeated placeholders, binds once) | PROBLEMS.md § 203 |
| P204 | plan-11 — unspan walker fix in `detect_ref_tail_capture` | [`../11-p204-ref-propagation/`](../11-p204-ref-propagation/) |
| P205 | phase 07 — `stores.scratch` routing for bounded-generic text returns at TWO `emit.rs` sites (Value::Return wrap + block-tail wrap_result) | PROBLEMS.md § 205 |

### Native suite progression

| Milestone | native_dir | native_scripts | High-level | Gap to main |
|---|---|---|---|---|
| Pre-plan-09 | 29/30 | 87/93 | 2/5 | -3 |
| After phase 06 (P202) | 30/30 | 89/93 | 3/5 | -2 |
| After phase 07 (P205) | 30/30 | 90/93 | 3/5 | -2 |
| After phase 10 step 10.3 (P200) | 30/30 | 91/93 | 4/5 | -1 |
| After plan-11 (P204) | **30/30** | **95/95** | **5/5** | **0 (PR-ready)** |

### Phase outcome

| # | Phase | File | Outcome |
|---|---|---|---|
| 00 | Scaffold | [00-scaffold.md](00-scaffold.md) | DONE — closes P203 (let-bind-on-repeat in `DefaultTemplateEmitter`) |
| 00a | Introspection: after scaffold | [00a-introspect.md](00a-introspect.md) | DONE |
| 01 | ABI consolidation | [01-abi-consolidation.md](01-abi-consolidation.md) | DONE — deletes `LEGACY_STORES_FNS` hardcoded list |
| 02 | Param adapter | [02-param-adapter.md](02-param-adapter.md) | **SUPERSEDED** by plan-12 phase 05 (no longer P200 prereq) |
| 02a | Introspection: after param adapter | [02a-introspect.md](02a-introspect.md) | SUPERSEDED (02 superseded; trigger never fires) |
| 03 | Parallel-for emitter | [03-parallel-emitter.md](03-parallel-emitter.md) | DONE — collapsed 95-line `dispatch.rs:850-944` |
| 04 | Key-keyed Op emitter | [04-key-ops.md](04-key-ops.md) | DONE |
| 05 | File emitters | [05-file.md](05-file.md) | DONE via phase 10 step 10.3 (`IntCompareEmitter` covers both read + write sites) |
| 05a | Introspection: after first bug fix | [05a-introspect.md](05a-introspect.md) | DONE via phase 10 step 10.1 |
| 06 | Threading queue runtime fns | [06-threading.md](06-threading.md) | DONE — P202 closed |
| 07 | Generic text emitter | [07-generics.md](07-generics.md) | DONE — P205 closed |
| 08 | Binary read emitter | [08-binary.md](08-binary.md) | SUPERSEDED by phase 10 step 10.3 (one fix closed read + write) |
| 08a | Retrospective | [08a-introspect.md](08a-introspect.md) | superseded by phase 10 step 10.6 |
| 09 | Parallel runtime consolidation | [09-parallel-runtime-consolidation.md](09-parallel-runtime-consolidation.md) | DONE — collapsed 3 near-duplicate `n_parallel_for_*_native` fns into one generic core |
| 10 | Final closure | [10-final-closure.md](10-final-closure.md) | DONE (P200 + plan-09 close-out) |

9 phases shipped (00, 00a, 01, 03, 04, 06, 07, 09, 10) of the 14
originally listed; 5 phases consolidated/superseded (02 + 02a →
plan-12; 05 + 05a + 08 + 08a folded into phase 10).

## Why each P-issue resolved when prior attempts failed

The four open P-issues had either resisted fix attempts or sat
unfixed because of structural blockers — not because the bugs
were deep.  Plan-09's simplifications dissolved the blockers:

| P-issue | Prior blocker | What dissolved it |
|---|---|---|
| P200 | `narrow_int_cast`'s dual role (block-tail coercion + parameter narrowing); fixing one role broke the other | Phase 00a survey revealed the real bug was block-tail comparison-emission, not param narrowing.  Phase 10 step 10.3's `IntCompareEmitter` widens comparison operands directly — sidesteps the dual-role problem entirely |
| P202 | Adding queue runtime fns would duplicate the 95-line parallel-for special case in `dispatch.rs:837-930` | Phase 03 gave queue fns a slot in the emitter family; phase 09 collapsed for-par runtime fns; phase 06 added 3 flat queue runtime fns (~90 LOC vs originally-projected ~120 LOC trait + wrappers) |
| P203 | Template double-substitution: `OpConvIntFromEnum` substituted `@v1` twice, so a side-effecting comparison evaluated its LHS twice | Phase 00 step 0.7b's `DefaultTemplateEmitter` auto-detects repeated placeholders and emits a `let` binding once — closes the bug class for all 5 affected templates simultaneously |
| P205 | Direct skip-removal might cascade; template lacks the type-binding info needed to emit owned-`String` vs borrowed-`Str` correctly | Phase 07's diagnostic probe (Outcome B) revealed the dangle is at TWO emit.rs sites, not a single Op.  Fix routes the value through `stores.scratch` so the backing `String` lives as long as `stores` |

## Cross-arc impacts

### Sibling plan-11 (P204)

P204 (tail-expression return discarded) is parser/scope-analysis
side, not codegen-template — out of plan-09 scope.  Sibling plan
at [`../11-p204-ref-propagation/`](../11-p204-ref-propagation/)
shipped 2026-05-02 PR #197.

### Follow-up plan-12

Plan-09's audit (2026-05-02) surfaced residual simplifications
worth doing for future development that were out of plan-09's
bug-fix focus.  Captured in
[`../../deferred/12-codegen-simplifications/`](../../deferred/12-codegen-simplifications/):

- **Tier 1** (correctness + cleanup): walker-audit (Span-miss
  bugs in `pre_eval.rs` — same pattern as plan-11) + retire
  `forwarding_smoke.rs`.
- **Tier 2** (structural cleanup): migrate ~22 `dispatch.rs`
  special-case match arms to custom emitters + split
  `narrow_int_cast` dual role.
- **Tier 3** (deep refactor, deferred to plan-13): unify
  `#rust"…"` template path with the registered emitter path —
  ~200 Ops migrate.

Plan-12 is in `deferred/`; trigger conditions in
[`../../DEFERRED.md`](../../DEFERRED.md).

## Memory / process artefacts

7 memory entries codify patterns that worked during plan-09:

- forwarding-first recipe (pre-flight pattern for adding custom
  Op emitters; grep `dispatch.rs` first, register forwarding
  emitter, verify byte-identical, then swap to real logic)
- phase-doc trait sketches as drafts not specs
- actual-error survey before implementing
- branch-after-PR-only
- no-EXPECT_FAIL on PR bugs
- zero-regression tolerance
- extended CI gate guidance

See [`08a-introspect.md`](08a-introspect.md) § Findings for the
full closure narrative.

## See also

- [`../../../NATIVE.md`](../../../NATIVE.md) § "Per-Op emitter
  dispatch (plan 09 phase 00)" line ~96 — architecture reference
  for the registry + DefaultEmitter / custom emitter dispatch
- [`../../PROBLEMS.md`](../../PROBLEMS.md) — per-P-issue closure
  narratives (P200 / P202 / P203 / P205); P204 in plan-11
- [`../../CHANGELOG_TECHNICAL.md`](../../CHANGELOG_TECHNICAL.md) —
  per-phase shipped manifest under "plan-09 phase NN" entries
- [`../11-p204-ref-propagation/`](../11-p204-ref-propagation/) —
  sibling plan that closed the parser-side P204
- [`../../deferred/12-codegen-simplifications/`](../../deferred/12-codegen-simplifications/)
  — follow-up simplifications surfaced by plan-09's audit
- `src/generation/ops/` — per-Op emitter implementations
  (IntCompareEmitter, ParallelQueueEmitter, ParallelBufRenameEmitter, …)
- `src/codegen_runtime.rs` — runtime fn additions (`n_parallel_queue_*`,
  `n_parallel_buf_get_*`, `_drop_*`)
- `src/generation/emit.rs` — the wrap_text + wrap_result scratch-
  routing fix from phase 07
- `src/generation/pre_eval.rs::detect_ref_tail_capture` — sibling
  walker that needed the same Span-handling fix as plan-11
  (captured for plan-12 Tier 1)
- `tests/codegen_emitter.rs` — regression-test net (`p200_*`,
  `p202_*`, `p203_*`, `p205_*` pins)
- `scripts/p09_fast_gate.sh` — fast structural gate used during
  plan-09 development (~4 s)
