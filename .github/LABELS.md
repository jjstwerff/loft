<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue labels — what they mean

For anyone (human or agent) triaging or fixing a loft issue **without** reading
the loft source.  Each label's GitHub *description* is the one-line gloss; this
file is the detail.  Convention + workflow: [`doc/claude/ISSUE_TRACKING.md`](../doc/claude/ISSUE_TRACKING.md).

Apply **one `sev:`, one `wa:`, and one or more `area:`** to every bug.

## `sev:` — severity (how bad WHEN you hit it)

| Label | Meaning |
|---|---|
| `sev:high` | crash / memory corruption / data loss / a soundness hole (the program does something silently wrong) |
| `sev:medium` | wrong result, hang, or miscompile in a real-world shape — but no corruption |
| `sev:low` | cosmetic, an edge case few will hit, a false-positive warning, a confusing-but-recoverable diagnostic |

## `wa:` — workaround (can you KEEP MOVING, or are you blocked)

The agent's primary "route-around or blocked?" signal.  **Always VERIFIED** — the
workaround was actually run on current HEAD, both backends; see
[ISSUE_TRACKING.md § Workarounds](../doc/claude/ISSUE_TRACKING.md#workarounds--the-agents-can-you-keep-moving-signal).
A wrong workaround is worse than `wa:none`.

| Label | Meaning |
|---|---|
| `wa:clean` | a simple, idiomatic alternative exists (verified) |
| `wa:partial` | a workaround exists but is awkward / loses the intended behaviour (verified) |
| `wa:none` | nothing works — this **blocks** whoever hits it (the most urgent triage axis, often above `sev:`) |

## `area:` — which part of loft (plain-English, with orienting files — NOT required reading)

loft is a tree-walking interpreter **and** a native code generator for a
statically-typed language.  Source flows: **text → parser → IR → codegen →
{ bytecode interpreter | native Rust }**, over a **store-based heap**.

| Label | What it is | Bugs here look like | Orienting files |
|---|---|---|---|
| `area:parser` | turning source text into typed IR: lexer, two-pass parser, type resolution, scope/lifetime analysis | parse errors, a construct mis-resolved, wrong inferred type/scope, a bad diagnostic | `src/parser/`, `src/lexer.rs`, `src/typedef.rs`, `src/scopes.rs` |
| `area:codegen` | turning correct IR into runnable code — the **bytecode** generator (interpreter) and the **native** Rust generator | the program parsed + type-checked fine, but the emitted code is wrong: wrong value, panic, slot/stack drift, a miscompile on one backend | `src/state/codegen.rs`, `src/state/fill.rs`, `src/generation/` |
| `area:store-lifetime` | the heap: store alloc/free, dependency tracking, value-semantic vector/struct lifetime, the watermark | leaks, use-after-free, double-free, a store freed too early/late, high memory watermark | `src/database/`, `src/store.rs`, `src/scopes.rs` (the free analysis) |
| `area:closures` | fn-refs, lambdas, captured cells, the closure-record layout | capture/ownership bugs, closures stored in containers, fn-ref dispatch/iteration | `src/parser/vectors.rs` (capture), closure-record handling in `src/database/` |
| `area:runtime` | executing bytecode — the 233 opcodes + the runtime stack/store ops (interpreter only) | a specific opcode misbehaves, a runtime panic not in codegen | `src/state/`, `src/fill.rs` |
| `area:native` | the `--native` backend specifically: Rust generation, the native runtime shim, `rustc`/`cc` linkage | a native-only divergence from the interpreter, an ABI/marshalling bug, a link/rlib failure | `src/generation/`, `src/codegen_runtime.rs` |
| `area:wasm` | the browser / WASM target: `wasm32`, the virtual FS, host bridges, threading | WASM-only failures, a missing host bridge, a feature gated off in the browser | `src/wasm.rs`, `wasm/` |
| `area:stdlib` | the standard library (`default/*.loft`) + native stdlib functions | a stdlib function returns the wrong thing / is missing an edge | `default/*.loft`, the native-fn registry |
| `area:packages` | the package format, registry, multi-file `use` resolution, library extraction | a cross-package resolution bug, a manifest/registry issue, a build-pipeline gap | `src/package.rs`, `src/manifest.rs`, `doc/claude/lib_plans/` |

## Cross-cutting

| Label | Meaning |
|---|---|
| `both-backends` | reproduces on BOTH `--interpret` and `--native` (vs a single-backend divergence) |
| `needs-design` | the fix needs a design decision, not a mechanical change — don't just patch it |
| `steered` | **owner-applied only.** The owner had to step in to get this fixed, or fixed thoroughly — a shallow first fix, a missed sibling, a matrix that needed asking for. One click, no prose: it is a *counter*, not a complaint, and it pairs with [`scripts/steering_rate.py`](../scripts/steering_rate.py) (see [STABILITY_ROADMAP § how much STEERING the fixing took](../doc/claude/STABILITY_ROADMAP.md)). **An agent must never apply or remove it** — the agent that needed steering is the least likely to notice, so self-reporting would bias exactly where the signal lives. Absence therefore means "not marked", never "no steering needed". |
| `bug` / `enhancement` / `documentation` / … | the GitHub defaults; keep `bug` on every bug |
| `proposal` | a proposed new library or API change/rewrite (the `library_proposal` intake → the @PLN112 provenance view) |
| `showcase` | an accepted COMMUNITY app, listed in the @PLN112 applications tier (first-party apps self-describe in their own repo via a `.loft-showcase.toml` + the `loft-showcase` topic, no label) |
| `showcase:pending` | a submitted `application_showcase` awaiting review — the intake queue; NOT listed until a maintainer relabels it `showcase` |

## Triage-state (where an investigation got stuck)

These describe the **state of the hunt**, not the nature of the bug — they tell
the next agent (or the maintainer) *why this one is still open* and what unblocks
it.  Apply at most one; remove it once the blocker clears.

| Label | When to apply | What unblocks it |
|---|---|---|
| `needs-triage` | A **public report has arrived and not yet been triaged** — the maintainer hasn't reproduced it, minimised it, or applied the `sev:`/`area:`/`wa:` labels. The reporter cannot set labels (the public has no label write), so this marks "a maintainer still needs to look." It is the queryable side of the acknowledgement promise: `gh issue list --label needs-triage` is the un-drained intake — nothing here should sit unacknowledged. See [ISSUE_TRACKING.md § The public intake bridge](../doc/claude/ISSUE_TRACKING.md#the-public-intake-bridge-arc-d). | Triage: reproduce + minimise to a both-backend repro, then apply `sev:`/`area:`/`wa:` (ripe bug) or `status:unclear` with a specific ask back to the reporter. Remove `needs-triage` once triaged. |
| `attention` | The bug has been **tried 2+ times with no clear path to a fix** — repeated attempts stalled, the mechanism is still not understood, or every fix tried regressed something else. A flag for "stop grinding solo; this needs a fresh approach or more eyes." | A new diagnostic angle, a different contributor, or escalation — not another identical attempt. |
| `design` | The bug **cannot be solved at all without a user-validated design decision** — the fix hinges on a language/semantics choice only the maintainer can make. Stronger than `needs-design`: `needs-design` means *a* design call is needed (a contributor may propose one); `design` means it is **blocked on the user** signing off. | The maintainer validates a direction; then it usually downgrades to a normal mechanical fix. |
| `by-design` | The reported "bug" reproduces exactly as described but is **intended behavior, traceable to a decision we already made** — the resolution is to *point at the decision*, not to change code. A **closing** label: apply when closing, and cite the [`DESIGN_DECISIONS.md`](../doc/claude/DESIGN_DECISIONS.md) entry (a `C##`) it rests on (record one in the same close if the decision wasn't written down yet). | Nothing — it's terminal. Re-opens only on **new evidence** that invalidates the decision (per the register's "Revisit when"). |

> Rule of thumb: reach for `attention` when you *don't know how* to fix it after
> genuine attempts; reach for `design` when you *can't decide what correct even
> means* without the user; reach for `by-design` when *correct is already
> decided* and the report just rediscovered it.  The three form a lifecycle:
> `needs-design`/`design` (open, decision pending) → `by-design` (closed,
> decision cited).  A bug can carry both `attention` **and** `design` (stuck
> **and** needs a design call).

## Lifecycle (where the fix is, not what kind of bug)

| Label | Meaning | Apply / clear |
|---|---|---|
| `fixed-pending-merge` | The fix has landed on a **long-lived working branch** but is **not yet in `main`** (the release branch).  The issue stays **open** so the tracker doesn't claim "fixed" while released code still has the bug — but it is **not a pick-up**: no agent work remains, only the merge.  The fixing commit's `Fixes #NNN` line **auto-closes it on merge to `main`**, in one clean transition (no manual close → reopen → close ping-pong).  The full lifecycle is **automated off that one trailer**: [`apply-fixed-pending-merge.yml`](workflows/apply-fixed-pending-merge.yml) adds the label (+ a bookkeeping comment) when a `Fixes #NNN` commit is pushed to any non-`main` branch; the merge to `main` auto-closes the issue; [`strip-fixed-pending-merge.yml`](workflows/strip-fixed-pending-merge.yml) removes the label on close (a closed issue is never "pending merge").  So multiple actors can fix bugs on their own branches concurrently with the tracker staying correct. | **Automatic** — just write `Fixes #NNN` in the commit; the label is applied on push.  The author still adds the **substantive comment** (regression test, verified `wa:*`, or a "still unverified" caveat) — the workflow's auto-comment is mechanical only.  Never close such an issue by hand; let the merge close it.  See [ISSUE_TRACKING.md § Issue lifecycle](../doc/claude/ISSUE_TRACKING.md). |

> **Multi-repo:** `sev:`/`wa:`/cross-cutting are shared across the loft-family
> repos (loft / dryopea / lavition / `loft-lang/*`).  `area:` is loft-specific;
> each game/engine repo defines its own `area:` set in its own `LABELS.md`.
