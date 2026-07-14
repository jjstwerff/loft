<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN107 — Dead-code lint: never-read local (unused variable + dead store)

**Status — FUTURE (design captured 2026-07-14).** Spec + mechanism + false-positive
guards below; implementation deferred to a dedicated session. **Key finding:** loft
**already ships** `unused_variables` (`Function::test_used`, `src/variables/mod.rs:1666`)
— it correctly flags W-scalar/W-accumulator/N-effectful *today*; the real gap is that its
`uses` counter treats a write-**target** as a read, so it misses **W-copy** (the graphics
footgun) and **W-reassign**. The fix is a **surgical extension** of that shipped lint
(split value-observing reads from write-targets), not a new analysis. The motivating
library bug is **already fixed** (see [The payoff](#the-payoff--already-banked)).
Issue: [loft-lang/plans#107](https://github.com/loft-lang/plans/issues/107).

## Goal — the one sentence

> **Warn when code writes something that *looks* like it does useful work but actually
> does not** — a store whose written value is never observed before the binding drops.
> That is dead code, and it is exactly the class of mistake the programmer cannot see by
> reading the source (it reads as productive).

This is loft's analogue of Rust's `unused_variables` + `unused_assignments`, unified
under one liveness rule. It is **not** a copy/borrow lint (that is @PLN90, shipped and
closed) — it fires on **plain scalars too**. The struct-copy footgun is one instance,
not the definition.

The two shapes that must warn, from the user's own examples:

```loft
{ a = 3; a += 1 }                 // W-scalar: a created + mutated, never read → BOTH statements dead
{ d = self.data; d[i] = x }       // W-copy:   d is a C86 field-read COPY; d[i]=x is a lost write
```

`d[i] = x` *reads* as "paint pixel i" but paints a throwaway copy — precisely
"seems useful, isn't." `&` (borrow the field instead of copying) makes the copy case
*safer to write*, but the lint is the **safety net that catches it whether or not the
author reached for `&`**, and it catches the scalar case that `&` has nothing to do with.

## The rule — fixpoint backward liveness

A local variable `v` is **dead** iff its value never flows to an **observable use**.
Compute the observable-use frontier, then propagate liveness backward to fixpoint:

1. **Seed (observable sinks — a use that escapes analysis or has an effect):**
   - passed as a **reader-position argument** to any call (user or native),
   - **returned** from the function,
   - a **branch/loop condition** (`if v`, `while v`, `match v`),
   - read into a value that is itself stored into an **escaping** structure
     (a returned/observable record, a field of a param, an appended element),
   - the **base of a projection that is itself read** (`v[i]`/`v.f` in a read context).
2. **Propagate:** if `w = <expr>` and `w` is live, every variable read in `<expr>`
   becomes live. Iterate until no change.
3. **Report:** any local never marked live is **dead** → warn at its definition site,
   and (nice-to-have) list its dead mutation sites.

The fixpoint is what makes `{ a = 3; a += 1 }` fire: the only read of `a` is the RHS of
`a += 1`, but that write is itself dead, so its read does **not** rescue `a`. A read that
only feeds another dead store is not an observable use.

**loft already approximates this soundly.** The shipped `test_used` counter (see below)
is a conservative one-step version — `reads == 0 ⇒ definitely dead` — and it *already*
handles the `a += 1` self-read correctly (W-scalar fires today). Full fixpoint would catch
strictly more (transitively-dead chains like `a = 3; b = a` where `b` is also dead); that
is a later refinement, not needed for the motivating cases. The concrete Phase-1 mechanism
is the counter split below, not a from-scratch dataflow engine.

## What warns / what stays silent (the spec matrix)

Corpus: [`spec.loft`](spec.loft) — every row hand-verified on **both** backends.

| # | Shape | Verdict | Why |
|---|---|---|---|
| **W-scalar** | `a = 3; a += 1` (a unread) | **WARN** | created + mutated, never observed |
| **W-copy** | `d = self.data; d[i] = x` (d unread) | **WARN** | mutates a copy — lost write |
| **W-accumulator** | `total = 0; for i in xs { total += i }` (total unread after) | **WARN** | leftover accumulator — classic dead loop work |
| **W-reassign** | `a = 3; print(a); a = 5` (a unread after) | **WARN** *(per-store, Phase 2)* | the `a = 5` store is dead |
| **W-method-mut** | `d = Box{…}; d.push(9)` (d unread, `push` only mutates self) | **WARN** *(interprocedural, Phase 2)* | the mutation is unobservable — needs an effect summary to prove it (see below) |
| **N-read** | `d = s.f; d[i] = x; use(d)` | silent | d is observed → the write is real work |
| **N-write-through** | `s.f[i] = x` (no copy) | silent | mutates the live field directly |
| **N-fresh-used** | `e = [1,2,3]; e[i] = x; use(e)` | silent | e escapes into `use` |
| **N-copy-read** | `d = s.f; use(d)` | silent | a copy that is *read* is not dead (it may be a wasteful copy — that is @PLN90, not this lint) |
| **N-method-effect** | `d = Box{…}; d.log_push(9)` (`log_push` prints) | silent | the call does real external work; d being dead is irrelevant |
| **N-method-return** | `x = d.pop(); use(x)` (d unread after) | silent | the call produced an observed value — real work |
| **N-method-escape** | `sink.take(d)` | silent | d escapes into the callee → live |
| **N-builder** | `b = B{}; b.set_x(1); r = b.build(); use(r)` | silent | b is read by `build()` → the setter chain is live |
| **N-effectful** | `x = launch()` (x unread, launch has effects) | **WARN on the binding, not the call** | the *binding* is dead; the RHS effect stays. Message must not imply deleting the computation. |

The **N-copy-read** row is the load-bearing guard: the whole point is to *not* warn when
the programmer legitimately copied and then used the copy. "Seems useful **and is**" ⇒ silent.

### Method calls on a dead receiver — the interprocedural frontier

A **direct** write (`d.f = x`, `d[i] = x`, `d += 1`) has its *entire* effect on `d`, so if
`d` is dead the write is provably dead by **intraprocedural** liveness alone (Phase 1). A
**method call** on a dead receiver (`d.push(9)`) is dead **only if** the call's *sole*
observable effect is mutating `self` **and** its return value is not observed — otherwise
the statement does real work (I/O, a global mutation, an observed result) even though `d`
itself dies. Deciding that needs a **per-function effect summary**:

> A function is **self-mutation-only** iff its only observable effects are writes through
> its `&`/mut arguments, it performs no I/O or global mutation, and its return is `()` (or
> unobserved at the call site). A call to such a function, on a receiver that is dead after
> the call, is a dead store.

Until that summary exists, method-mediated deadness (**W-method-mut**) stays **silent** —
the zero-false-positive bar wins over catching this shape. It is Phase 2 work, gated on the
effect summary, and it is *why* Phase 1 restricts itself to direct writes: those are the
shapes local liveness can prove dead without lying. **N-method-effect / N-method-return /
N-method-escape** are the false positives that summary must avoid.

## What loft already has (verified 2026-07-14)

Running [`spec.loft`](spec.loft) shows loft **already ships two** dead-code lints:

1. **`unused_variables`** — `Function::test_used` (`src/variables/mod.rs:1666`) warns
   "`Variable X is never read`" when `var.uses == 0`. Fires correctly today on **W-scalar**
   (`a = 3; a += 1` — note `a += 1` does *not* bump `uses`, so the self-read doesn't rescue
   it), **W-accumulator**, and **N-effectful** (warns on the binding — exactly right).
2. **`unused_assignments`** — `Function::track_write` (`src/variables/mod.rs:987`, called from
   the whole-var `var = expr` path at `expressions.rs:1384`) warns "`Dead assignment — X is
   overwritten before being read`" when a **whole-variable** reassignment overwrites a prior
   write with no read between (`uses == uses_at_write`, tracked via `write_source` /
   `uses_at_write`, with branch save/restore).

So this is **not greenfield**. The two gaps that motivate @PLN107 are precise:

- **W-copy** (`d = b.data; d[0] = 9`, d unread) — the motivating footgun. `d.uses > 0`
  because the write-target base `d` in `d[0]=9` is `in_use`'d (element writes read the base
  to locate the slot), so `test_used` thinks d is used; and `d[0]=9` is **not** a whole-var
  reassignment, so `track_write` never sees it. Falls through **both** lints.
- **W-reassign-final** (`a = 3; print(a); a = 5`, a unread after) — `track_write` only checks
  at the *next* write, and there is none after `a = 5`; `test_used` sees `uses > 0` (the
  print). The dead *final* store falls through both. (`a = 3; a = 5` with no read between **is**
  caught by `track_write` today.)

The single root cause of the W-copy gap: **`uses` conflates a value-observing read with a
write-target base reference.** `uses` also drives codegen (last-use elision; `uses == 1`
checks in `state/codegen.rs`, `parser/operators.rs`, `parser/collections.rs`), so it **must
not change** — the fix ADDS a counter and derives a read count for the lint only.

## Mechanism — split `reads` from write-targets in the existing lint

The whole fix, in one sentence: **a variable whose only uses are write-targets counts as
never-read.**

- Add a `reads` sub-count to `Variable` (`src/variables/mod.rs`) alongside `uses` — bumped
  only for **value-observing** references, *not* for a `Var` appearing as the target base of
  a `Set`/`OpSet*`. `test_used` then also flags a non-escaping local with `reads == 0 &&
  writes > 0` (the W-copy dead store), distinct from the existing `uses == 0` (never bound
  usefully) case.
- The read-vs-write classifier **already exists**: `src/variables/intervals.rs`
  `compute_intervals` walks the IR and *already distinguishes* `Value::Var(v)` (a read) from
  `Value::Set(v, …)` (a write) — its own comments (`intervals.rs:74`) note "variables that
  are only ever WRITTEN (never read after…)". The subtlety it must handle: `d[i]=x` lowers to
  `Call(OpSetInt, [Var(d), idx, rhs])`, so the *target* `Var(d)` (arg 0 of an `OpSet*`) must
  be classified as a write-target, not a read — the same `first_arg_write_ops` set
  `use_analysis.rs` already defines identifies exactly those ops.
- Keep the existing exclusions (`_`-prefixed, `#`-synthetic, `captured`, global-shadowing).
  Escape stays live: any read that feeds a call arg / return / escaping store is a real read.

This is a **surgical extension of a shipped lint**, not a new analysis engine — far cheaper
and lower-risk than the from-scratch `use_analysis.rs` pass first sketched. The @PLN90
`use_analysis.rs` classification (`first_arg_write_ops`, `Ctx::ReaderArg`) is the reference
for *which op positions are writes*; the emission stays in `test_used`.

## Implementation — small verifiable steps

Phase 1 closes the **W-copy** gap (the motivating footgun). Each step is independently
landable (compiles + suite green), and each has a concrete pass/fail check against the
**S0 oracle**. The order front-loads the risk: make the read/write classifier *observable*
and prove *zero behaviour change* before any warning depends on it; sweep for false
positives before flipping default-on.

- **S0 — Lock the oracle (test-only, no product change).** Add `tests/dead_code_lint.rs`:
  compile [`spec.loft`](spec.loft) on **both** backends, collect emitted warnings, assert the
  *current* set (W-scalar/W-accumulator/N-effectful warn; W-copy + W-reassign-final silent).
  **Verify:** `cargo test --test dead_code_lint` green; adding a read of `d` to the W-copy row
  flips it → proves the harness can fail. *Risk: none. Payoff: the regression net for every
  later step.*
- **S1 — Observable read count, NO warning (the subtle part, isolated).** Add
  `write_target_uses: u16` to `Variable`; at the element/field-write base site in the
  `d[i]=x`/`d.f=x` assign path (`expressions.rs`, where the base is currently `in_use`'d), also
  bump `write_target_uses`. Derive `reads() = uses − write_target_uses`. Expose via a gated
  dump (per user var: `uses`, `write_target_uses`, `reads`). **Verify (hand-computed):** on
  spec.loft, `d`@W-copy → `uses=1, wtu=1, reads=0`; `d`@N-read → `reads ≥ 1`; `e`@N-fresh →
  `reads ≥ 1` (passed to a call). **S0 oracle unchanged** (`uses` untouched ⇒ codegen
  byte-identical). *This proves the classifier before anything trusts it.*
- **S2 — Emit the warning behind an off-by-default flag.** In a sibling `test_dead_stores`
  (next to `test_used`): `reads() == 0 && write_target_uses > 0 && !argument && !captured &&
  name∉{_*, *#*, global}` → Warning *"'d' is mutated but its value is never read — the mutation
  is lost. A whole-value bind (`d = s.f`) COPIES; write through the field (`s.f[i] = …`) or take
  `&`."* Gate on `LOFT_DEAD_STORES` (default OFF), mirroring `report_copies_enabled()`.
  **Verify:** flag on → S0 oracle expects exactly **+W-copy**, all N-* silent; flag off →
  byte-identical to S1.
- **S3 — Escape + branch hardening.** Add corpus rows: escape-after-mutate (`d[0]=9; sink(d)`,
  `return d`), conditional read (`if c { use(d) }`), loop cross-iteration read. Each must keep
  `reads() > 0` (pass-to-call / return / any-path read bumps `uses` but **not**
  `write_target_uses`). **Verify:** oracle — every N-escape row silent, W-copy still warns, both
  backends.
- **S4 — Suite-wide false-positive sweep (the cry-wolf gate).** Run `LOFT_DEAD_STORES=1 make
  test` + `default/*.loft` + fixtures + consumers. Triage each new warning: real bug → fix;
  false positive → add a guard + a corpus row; iterate to **zero FPs**. **Verify:** the suite
  emits only intended warnings; record the real bugs found (expect lib/graphics-class hits).
  *This is the gate that earns default-on.*
- **S5 — Flip default-on + graduate.** Default `LOFT_DEAD_STORES` on with `LOFT_NO_DEAD_STORES`
  opt-out (mirror the @PLN28 diagnostic toggles); fix real hits; graduate `spec.loft` to
  `tests/scripts/`; document in STDLIB/diagnostics + the `LOFT_LOG` quick-ref. **Verify:**
  `make ci` green, default-on, oracle locks behaviour on both backends.

**Phase 2 (later, separate steps).** *P2a — dead FINAL store* (W-reassign-final): extend
`track_write` with a scope-end flush that re-checks the last write's `uses == uses_at_write`.
*P2b — method-on-dead-receiver* (W-method-mut): build the **self-mutation-only** effect summary
(§ interprocedural frontier) and extend the lint to `d.push(9)` without tripping
N-method-effect/return/escape.

## False-positive guards (the entire risk)

The lint is worthless if it cries wolf on legitimate code. Non-negotiable:

- **Copy-then-read is silent** (N-copy-read). A read anywhere on a live path rescues.
- **Effectful RHS**: an unread binding whose RHS calls an impure fn still warns (the
  binding is dead) but the diagnostic must say "unused binding" not "dead statement" —
  never imply removing a side-effecting call.
- **Escape = live.** Anything passed to a call arg, returned, or stored into an escaping
  structure is live (conservative — an unknown callee may observe it).
- **Loops.** Back-edges break position order; a var written in an iteration and read in a
  *later* iteration is live. Liveness (not position comparison) handles this correctly —
  but the pass must treat a loop body's reads as reachable from its writes.
- **Params are never "unused-variable" dead** here (that is a separate lint); this pass is
  locals only. A param passed straight through is observable by contract.
- **Compiler-generated temporaries** (`_`-prefixed, `is_compiler_generated`) are excluded
  from the user-facing report (same exclusion @PLN90 already applies).

## The payoff — already banked

The motivating case is **fixed** on `tuxedo-work` (commit pending): `lib/graphics`
`set_pixel` / `fill_rect` / `hline` / `vline` used `X_d = self.data; X_d[i] = c` — a C86
copy-then-mutate, so the writes were **silent no-ops on both backends** (verified: the
copy leaves `data[0]=0`, direct write-through gives `data[1]=222`). Rewritten to direct
`self.data[i] = c` write-through (matching `blend_pixel`/`clear`), so the software canvas
now actually paints — which is what made on-device text render (@PLN106 B3). This lint
would have flagged all four at authoring time; it is the general guard against the class.

## See also

- @PLN90 [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) + [90-copy-diagnostics/](../90-copy-diagnostics/) —
  the copy/borrow lint and the `use_analysis.rs` classification this reuses.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) (C86: whole-value heap binds COPY; #415) —
  *why* `d = self.data` copies, which is what makes the dead-store shape possible.
- @PLN106 [106-android-build-target.md](../106-android-build-target.md) — the consumer
  that surfaced the copy-mutate footgun in `lib/graphics`.
