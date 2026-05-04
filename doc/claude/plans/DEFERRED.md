<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Deferred Work — Internal Index

Single source of truth for every parked validation phase, deferred
P-issue, and "noted but not now" item.  Distinct from
`doc/claude/USER_FACING.md` (which filters this list to
user-visible-only) and `ROADMAP.md` (planned-but-not-started
features).

**Convention.** Every row carries a `Trigger to unpause:` value —
the **concrete signal** that should re-activate the work.  No row
without a trigger.  When the signal arrives, the row moves out of
this file (into a current plan, a P-issue fix, or a release note).

**Closed-work hygiene** — see `plans/README.md § Companion indexes`
for the project-wide rule.  Short version: closed items are
removed entirely; their closure is recorded in git history,
regression tests, plan READMEs, PROBLEMS.md, and CHANGELOG.md.

**Discoverability.** Two grep targets:

```bash
# Every parked item with its trigger:
grep -r "Trigger to unpause:" doc/claude/

# Every parked test (the locked-in regression net):
cargo test --release -- --ignored 2>&1 | grep "^test " | head -50
```

The first produces the doc index; the second produces the
test-suite index.  Items in either grep list are auto-discoverable
by a future session.

---

## Deferred plans (full plan parked)

| Plan | Status | Trigger to unpause |
|---|---|---|
| [`14-tuple-validation/`](14-tuple-validation/) phases 02–06 | Phases 00 + 01 shipped in PR #207.  P212 (nested-tuple panic) closed 2026-05-04.  Phase 02 matrix wiring not yet started; phases 03-06 untouched. | Default sequence.  No outstanding S0/S1 bugs in the nested-tuple shape. |
| [`15-closure-validation/`](15-closure-validation/) | Drafted; phase 00 wiring not started.  P213 closed 2026-05-04 (parse-time diagnostic; layout-widening fix deferred).  Open: P214 (vector of non-capturing closures, C0/D4), P215 (nested closure name resolution, C6/D1), P216 (tuple capture in closure, C4/D1). | Any of P214-P216 fixed lifts the next batch of cells. |
| [`16-coroutine-validation/`](16-coroutine-validation/) | Drafted; phase 00 wiring not started.  P210 closed 2026-05-04 (Value::Loop missed in collect_segments).  P211 (yield text) still open; Y3/Y4 cells need re-probe. | Re-probe Y3/Y4 to see if P210's fix lifted them; otherwise P211 (text lifetime) is the next coroutine fix. |
| [`17-template-validation/`](17-template-validation/) phases 02–06 | Phase 01 closed in PR #207.  **Phase 02 pre-flight 2026-05-04 confirmed feature gap, not bugs** — `<T: A+B>`, `<A,B>`, `where` clauses all parse-error.  Single-bound generics work; multi-bound and multi-T are scheduled feature work. | Feature work; not bug-yield gated.  Schedule against language priorities. |
| [`18-match-validation/`](18-match-validation/) phases 02–05 | Phase 01 closed in PR #207 (or-pattern + `@`-binding hang).  P209 (match-guard binding) closed 2026-05-04.  Range patterns + guards now pass; phase 02 wiring (matrix tests for range / guard / null patterns) not yet started. | Default sequence.  No outstanding S0 in match guards. |
| [`19-struct-enum-validation/`](19-struct-enum-validation/) phases 00-02 + 04-06 | Phase 03 closed in PR #207 (method-on-parent-enum dispatch).  Phase 00 wiring + remaining phases not started; not pre-flighted in 2026-05-04 round. | Default sequence.  No outstanding S0/S1 surface. |
| [`20-collection-validation/`](20-collection-validation/) | **Self-deferred** — pre-flight panic at `src/database/structures.rs:609` does not currently reproduce (60 hammer runs, 0 panics on unchanged binary). | Any user-reported `index out of bounds: the len is N but the index is 65535` panic at `src/database/structures.rs:609`, OR a deterministic reproducer that surfaces during plans 15/16/17/18/19 cell runs. |

## Deferred plan-phase items (within a partly-shipped plan)

| Item | Plan / phase | Trigger to unpause |
|---|---|---|
| (A) caveat — implicit type-inference of generic-tuple call results | plan-17 phase 01 (A) follow-up | Likely shares root cause with bug B.  Trigger: same as bug B (one fix may close both). |
| `name @ pattern` inside or-patterns | plan-18 phase 01 feature decision | Default sequence (phase 02+ would address).  External trigger: user request, or 2nd request in any forum showing the workaround is awkward. |
| Plan-06 phase 9b — tuple-element vector input to par | plan-06 (phase 9a closed by T1.8a) | Default sequence.  External trigger: any consumer that wants `par(vector<(A,B)>, …)` shape. |
| Plan-06 phase 9c — tuple returns from par workers | plan-06 | Default sequence.  4 ignored canaries already filed.  External trigger: a parallel benchmark or feature wanting tuple-returning workers. |
| Plan-06 phase 9d — fused for-binding tuple destructure in par | plan-06 | Default sequence.  External trigger: parser usability complaint about pre-bind workaround. |
| Plan-06 phase 9e — D11 doc updates | plan-06 | Triggered automatically when 9b/9c/9d close. |
| Plan-15 closure-DbRef leak (LIFETIME.md "Type::Function — NOT YET HANDLED") | plan-15 phase 03 (active risk) | Phase 03's spike-and-decide pass.  External trigger: long-running program (server, REPL) showing memory growth from closure usage. |

## Deferred bugs / P-issues

| ID | What | Trigger to unpause |
|---|---|---|
| P213 layout fix | **Near-term planned work** — widen fn-ref struct fields from 4B (just d_nr) to 16B (d_nr + closure DbRef) so capturing closures can be stored.  Driving use cases: async/IO callback registries, server main loops tracking many signals, game main loops / event buses, state-machine transition tables.  Programmers building any of these will naturally write `struct Handler { on_X: fn(...) -> ... }` with capturing lambdas; today they hit the diagnostic.  Full design recorded in [PROBLEMS.md § 213](../PROBLEMS.md#213-capturing-closures-cannot-be-stored-in-struct-fields--full-design-for-the-proper-fix).  Touches `element_size(Type::Function)`, `set_field_check`, native codegen for OpSet*/OpGet*, and tuple/vector layouts.  Parse-time diagnostic shipped 2026-05-04 keeps users out of the panic until the layout fix lands. | Land this **before** the `server` library, the `game_client` / OpenGL game-loop work, or any plan-06 par-with-fn-field consumer ships — those are the natural users.  Also fold step 5 (get_free_vars Type::Function arm) into plan-15 phase 03's closure-DbRef leak fix; same shape, do them together. |

## Decision-pending items (not bugs, but choices)

| Question | Surfaced by | Trigger to decide |
|---|---|---|
| Lift T1.11a (tuples in struct fields) — already lifted in 0.8.4 via inline `__tuple<…>` layout per `parse_field` / `set_field_check` | plan-14 phase 05 — closed by lift | (already decided / shipped) |
| Decide closure-leak (plan-15 phase 03): fix vs document | plan-15 phase 03 active risk | Trigger: phase 03 execution (decision section filled in before any code). |
| Decide stdlib `to_text` vs retract Printable claim (plan-17 phase 03) | plan-17 phase 01 (C) — closed by adding stdlib impls | (already decided / shipped) |

---

## How rows leave this file

A row leaves DEFERRED.md when the trigger fires AND the work is
either:
- **Closed in code** — entry moves to `CHANGELOG.md` or
  `CHANGELOG_TECHNICAL.md`; the regression test stays as
  permanent lock-in.
- **Reclassified as non-goal** — entry moves to
  `DESIGN_DECISIONS.md` with rationale.
- **Promoted to active plan phase** — entry no longer "deferred";
  it's now a current phase of an open plan.

---

## Cross-references

- [USER_FACING.md](../USER_FACING.md) — user-visible subset of
  this file; what would go in release notes.
- [PROBLEMS.md](../PROBLEMS.md) — open P-issues with reproducers.
- [DESIGN_DECISIONS.md](../DESIGN_DECISIONS.md) — closed-by-decision
  register.
- [ROADMAP.md](../ROADMAP.md) — planned features (distinct axis from
  deferred-during-validation).
- [README.md](README.md) — plans index; live + deferred + finished.
