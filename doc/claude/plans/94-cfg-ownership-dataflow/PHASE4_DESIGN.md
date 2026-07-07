<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN94 Phase 4 — the consistency oracle over emitted IR (design doc, written BEFORE code)

Design Protocol 1: this doc is the generative act. A candidate invariant was **probed and
falsified** before any code (see §"The probe that reshaped this"), which is why the design below is
NOT the one the plan sketched — it is the one that survives the data.

## Coexistence — the frame (load-bearing; do NOT drift from it)

The two algorithms **run beside each other; neither replaces the other, and BOTH flag at the same
time.** The shipped analysis (`use_analysis` A/B) keeps driving codegen — the oracle never touches
the emitted plan (SI-1: byte-identical when `LOFT_OWN_ORACLE` is unset). The oracle is a pure
observer with **two independent flagging channels, kept live together**:

1. **Shadow-diff** (my fact vs B's `ownership_of`): a disagreement is a finding that can indict
   EITHER side. On a *correct* corpus it indicts the newer impl (this cycle: two oracle bugs found
   inward); a residual disagreement on a *wrong* plan indicts the SHIPPED side (the A1b catch).
2. **Free-legitimacy check** (this phase — my fact vs the emitted free-ops): RED when a free frees a
   store my fact says is borrowed.

**The corrections are the product** — REDs/disagreements from either channel, in either direction.
Phase 4 turns channel 1 from *print* into a *gate* and ADDS channel 2; it removes nothing.

**Overhead (measured):** oracle ON adds **~1.6%** to compile+analysis (712-fn corpus 62.4 → 63.4 ms;
~1.4 µs/fn) and **zero** in the fast path (gated off). Both coexist cheaply — so we run both.

## The probe that reshaped this (why the "obvious" check was wrong)

The plan sketched a self-contained A1b check: *"a returned value must be `Owned` or borrow a
PARAMETER — never a freed frame-local."* Read off the correct-vs-`LOFT_NO_A1B` IR diff of `n_h` it
looked clean. **Probed against the oracle's actual facts, it is FALSE:**

| case | return fact | safe? |
|---|---|---|
| `n_h` under `LOFT_NO_A1B` (UAF) | `Join(v65535)` — base UNRESOLVED | **no** |
| `id(x)` borrow-of-param (probe 05) | `Borrowed(v65535)` — base UNRESOLVED | **yes** |

Both have base `65535` (unresolved) — the interprocedural / field-of-param borrow does not resolve to
a concrete var. So "base must be a parameter" **cannot tell the UAF from the safe borrow.** And
"return must not be `Join`" is also false: `g`/`deliver` (`match {Filled => items, _ => []}`) returns
`Join` **legitimately** — safe precisely because its caller COPIES it. The unsafe-vs-safe line is
"the return borrows a store THIS frame frees", which needs reliable base resolution the oracle does
not yet have. **Conclusion: the self-contained A1b catch is DEFERRED** until base resolution improves
(a later increment); A1b is already caught by channel 1 (below), which runs beside.

## What Phase 4 builds — two gating checks under `LOFT_OWN_ORACLE=check`

### Check A — the shadow-diff GATE (the A1b catch, coexistence made hard)

Run the fixpoint + shadow-diff vs B; **any surviving DISAGREEMENT is RED**, printed with the
function, var, and both facts. This promotes channel 1 from a printed diff to a gate.

- **Invariant:** two independent computations of the same per-var ownership fact MUST agree (or my
  flow-sensitive one refines B's `Join` — a PRECISION win, not a disagreement). A residual DISAGREE
  is a real defect in one of them.
- **Catches A1b:** `LOFT_NO_A1B` → `n_h` DISAGREE=1 (`__ref_1 mine=Join / B=Owned`) → RED. Correct
  default → DISAGREE=0 → green.
- **No-crying-wolf is already established (the 4.2 work is done):** DISAGREE=0 across 505 (712 fns),
  the @PLN85 fuzzer (54 cells, both backends), and all 7 probes. Two oracle unsoundnesses were driven
  out to reach it (3.4a, 3.5).

### Check B — free-legitimacy (independent over-free catch, no base resolution)

> **At each `OpFreeRef(v)` / `OpFreeText(v)` site, `v`'s ownership fact at that point must be
> `Owned`. A free of a `Borrowed(_)` alias is RED — an over-free of a store owned elsewhere.**

- Directly checkable from the per-block fact + a scan for free-ops; needs NO base resolution (the
  part the probe showed is unreliable), so it is sound to build now.
- Independent of B: it checks my fact against the emitted free-ops, not against B.
- Covers a DIFFERENT fault class than Check A (freeing a view, vs the A1b borrowing-return). It does
  NOT catch A1b (the wrong plan frees `__vdb_1`, which is `Owned`) — that is Check A's job. Both run.

## Re-assertion sites — N = 1 (the tell passes)

Each check is ONE pass over the IR reading the once-computed fixpoint fact. No N-site spray; a new
free-op family is absorbed by the same walk. `log()` any free-op the walk does not recognise (no
silent gap).

## Failure paths (enumerated)

- **Check B false RED on a legitimately-freed work-ref.** `__ref_*`/`__rref_*` work-refs are freed
  via `OpFreeRefIfDistinct` even when the fact is not a clean `Owned`. Probe 4.2 over the shipped
  suite; if a work-ref trips it, the fact for work-refs needs the same `is_work_ref` carve-out
  `scopes::get_free_vars` uses — narrow to it, don't loosen the whole check.
- **Conditional free (`OpFreeRefIfDistinct`) semantics.** Check B keys off the fact of the FIRST arg
  (the store that may be freed); the runtime distinctness guard does not change that arg's ownership.
- **Op family not modelled (coroutine/`par` frees).** `log()` and count — no silent miss.
- **A1b deferred (documented above), not silently dropped.** Channel 1 covers it; the self-contained
  version waits on base resolution.

## Gates (each independently committable)

- **4.1 ✅ (2026-07-07)** — `check` mode (`LOFT_OWN_ORACLE=check`) + both checks; 2 unit tests on
  hand-built inputs (`free_of_borrowed` flags only the `Borrowed` free; `collect_free_targets` finds
  only free-op arg0). Catches A1b (see 4.3).
- **4.2 — NEARLY DONE (2026-07-07): 377 `tests/scripts` swept, Check B fully clean, Check A 11 → 1.**
  The build was the last probe (§"The probe that reshaped this" plus these):
  - **Check B false positives on `OpFreeText` / `OpFreeRefIfDistinct`** (text subs classed `Borrowed`
    but copied; runtime-guarded frees). Fixed by narrowing `free_op_nrs` to the UNCONDITIONAL
    `OpFreeRef` only. Now **0** across 377 scripts.
  - **Check A `Join(a)` vs `Join(b)`** (the `esc_*`/join/loop shapes, 6 sites) — a base-only
    mismatch, NOT an ownership-decision disagreement. Fixed: compare by KIND (discriminant), not
    base; a conditional free keys off runtime distinctness, not the static base.
  - **Check A self-borrow `Borrowed(v)` for var `v`** (2 sites, `@P302` keyed-collection self-dep) —
    an ownership marker, not a borrow. Fixed: normalise `Borrowed(self) → Owned` in the transfer
    (mirrors the shipped `get_free_vars` carve-out).
  - **Residual: 1** — `n_choose` (`85-struct-copy-return-owned`): `r = x; if cond { r = Box{…} }; r`.
    `r` is the retbuf param; the then-arm re-mints its store via `OpDatabase(r)` (a `Call`, not a
    `Set`, so the transfer misses it → `r` stays `Borrowed(x)` instead of the conditional `Join(x)`).
    A **leak-direction** (safe: mine more conservative than B, no over-free) imprecision. DEFERRED to
    the increment that records `OpDatabase(v)` re-mints as owns entries (same class as the 3.4a
    structural-op fix) + models the retbuf materialisation B collapses to `Owned`.
- **4.3 ✅ (2026-07-07)** — RED on `LOFT_NO_A1B` (`n_h`: `RED … fact-disagree __ref_1 mine=Join /
  B=Owned`, via Check A). Injected-fault true-positive (delete an `OpFreeRef` / flip a fact) still to
  wire as a test (Phase 5).

## Strictness verification — is it strict ENOUGH, not just not-too-strict? (2026-07-07)

Not-too-strict is the 4.2 sweep (0 false positives). Strict-ENOUGH is the true-positive question,
verified two ways — and the result is the coexistence thesis made concrete.

**(A) Strictly stronger than the OLD gates on its target class.** On the assert-stripped A1b
blindspot under `LOFT_NO_A1B` (a definitively wrong plan: `len=0`, correct is `3`):

| gate | verdict on the wrong plan |
|---|---|
| exit code | 0 — **pass** |
| leak-check (`LOFT_STORES=warn`) | 0 leaks — **pass** |
| interp vs native (the differential oracle) | identical `len=0` — **pass** (both backends agree on the WRONG answer) |
| **new `LOFT_OWN_ORACLE=check`** | **RED `n_h`** |

Every old observable gate passes; only the new check flags it. This is the Step-0 premise, now shown
end-to-end on the built check — the new routine catches an over-free class the old gates structurally
miss.

**(B) It is deliberately NOT a superset of the old detectors — proven the other way too.** Under
`LOFT_NO_JOIN_OWN` (a different injected fault), `local_source__struct` LEAKS (an UNDER-free) — the
OLD leak detector catches it (`leak=1`); the new check does NOT (`disagree=0`), because it targets
over-free / wrong-ownership, not under-free. So neither routine is strict enough alone: the new one
owns the over-free/A1b class the old gates miss, the old detectors own the leak class the new one
misses. **Run beside, the pair is stricter than either — which is exactly why we run both.**

**Per-check true-positive status.** Check A: demonstrated end-to-end (`LOFT_NO_A1B` → RED). Check B:
unit-proven (`free_of_borrowed` fires on a `Borrowed` free) — a REGRESSION GUARD, since no current
fault toggle emits an *unconditional* `OpFreeRef` of a borrowed store (the A1b wrong plan frees via
the guarded `OpFreeRefIfDistinct`, which Check B correctly ignores). Wiring an injected-fault test
(delete an `OpFreeRef` / flip a fact) into `tests/ownership_oracle.rs` is Phase 5.

## Deferred (recorded, not dropped)

- **Self-contained A1b catch** (return-borrows-freed-local) — waits on reliable interprocedural /
  field-of-param base resolution (the `65535`-unresolved case). Until then A1b lives on Check A.
- **Freed-exactly-once / no-free-of-live** sub-invariants — a store-multiset tally per path; additive
  to Check B, sequenced after 4.1–4.3 land.
