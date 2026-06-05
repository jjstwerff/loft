<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — `Mutable<T>` stdlib helper (escape hatch for Case D)

**Status: open — DEFER-BY-DEFAULT**

## Goal

Implement the `Mutable<T>` stdlib helper per [README § Lowerings:
D — explicit Mutable&lt;T&gt; for shared ownership](README.md#d--explicit-mutablet-for-shared-ownership).

`Mutable<T>` is the explicit-opt-in escape hatch for genuinely-
aliased state across mismatched lifetimes (the case D shape that
phase 04 rejects).  When a user wraps a value in `Mutable<T>`,
they take on the "I know I'm aliasing" responsibility; the
classifier sees `Mutable<T>` as a heap-allocated cell with
explicit lifetime, bypasses the case-D check, and accepts the
program.

## Defer-by-default rationale

Phase 04's case-D rejection is the right user experience for
99% of cases — the diagnostic guides the user toward a fix
(reorder code, factor differently).  `Mutable<T>` is an escape
hatch for the residual cases where the aliasing is intentional
and unavoidable.

Per [README § Drivers](README.md#drivers): TTT v6 server retrofit
+ @PLN6 audience-demo BOTH ship without phase 05 if it doesn't
land.  Real-world demand for `Mutable<T>` should drive
prioritisation; phase 05 lands when a use case surfaces that the
case-D message can't easily fix.

## What ships

A new `lib/mutable/` package:

```toml
# lib/mutable/loft.toml
[package]
name = "mutable"
version = "0.1.0"
loft = ">=0.8"

[library]
entry = "src/mutable.loft"
```

```loft
# lib/mutable/src/mutable.loft
// Mutable<T> — explicit shared ownership for closure captures
// that escape with aliased state.  Backing storage is a 1-field
// record in the host store; .get / .set / .modify are the
// canonical access methods.
//
// Use only when plan-22 case-D rejection blocks a genuinely
// intentional alias.  Most uses can be rewritten to case B
// (co-scoped) or case C (factory) — try those first.

pub struct Mutable<T> {
    cell: Reference<__Cell<T>>,
}

struct __Cell<T> {
    value: T,
}

pub fn new<T>(initial: T) -> Mutable<T> {
    Mutable { cell: __Cell { value: initial } }
}

pub fn get<T>(self: Mutable<T>) -> T {
    self.cell.value
}

pub fn set<T>(self: Mutable<T>, v: T) {
    self.cell.value = v;
}

pub fn modify<T>(self: Mutable<T>, f: fn(T) -> T) {
    self.cell.value = f(self.cell.value);
}
```

Classifier change: when a captured binding's type is
`Mutable<T>`, treat the capture as already-by-reference (case B
trivially holds because `Mutable<T>` IS the explicit
shared-ownership cell).  No case-D check fires.

## Test surface

`tests/mut_closure_matrix.rs`:

```
m_d4_explicit_mutable_in_factory       // make_counter using Mutable<integer>
m_d4_mutable_passes_case_d_shape       // problematic() rewritten with Mutable<T>; accepts
m_d3_mutable_in_struct_field           // s = S{counter: Mutable::new(0)}; both reads OK
m_d2_mutable_passed_to_handler         // el::on(loop, fn() { state.modify(|s| s+1) })
```

Plus a leak guard:

```
p22_phase05_mutable_no_leak            // 100x Mutable<integer> alloc/drop; clean
```

Plus 2 cells in `tests/parse_errors.rs`:

```
m_case_d_diagnostic_suggests_mutable   // case-D message names Mutable<T> as the fix
m_mutable_misuse_warning               // optional: warn if Mutable<T> used inside a case-A body
                                       //   (the cell is unnecessary indirection)
```

## Critical files

| File | Change |
|---|---|
| `lib/mutable/loft.toml` (new) | Package manifest |
| `lib/mutable/src/mutable.loft` (new) | Loft API |
| `src/parser/closure_analysis.rs` | When capture type is `Reference(Mutable<T>)`, classify as case B unconditionally |
| `src/parser/closure_analysis.rs` | Phase 04's case-D message updates to drop "(see lib/mutable; not yet shipped)" qualifier |

## Verification

- All 4 m_* cells green under interp + native cross-mode.
- `m_case_d_diagnostic_suggests_mutable` cell verifies the
  diagnostic UPDATE happened (case-D message references
  `Mutable<T>` without "not yet shipped").
- `p22_phase05_mutable_no_leak` clean over 100 iterations.
- Existing closure_matrix.rs cells (22 from @PLAN15) + Case A
  cells from phase 00 + Case B cells from phase 02 + Case C
  cells from phase 03 all still green.
- CI gate green.

## Risks

| Risk | Mitigation |
|---|---|
| `Mutable<T>` becomes the "easy escape" — users reach for it instead of restructuring code as Case B/C | Phase 05 ships with a docstring explicitly saying "Use only when @PLAN22 case-D rejection blocks a genuinely intentional alias.  Most uses can be rewritten."  Optional: emit a warning when `Mutable<T>` is used in a body that doesn't actually alias (the case-A misuse cell in parse_errors). |
| `Mutable<T>` API ergonomics force `.get()` / `.set()` ceremony that's clunkier than the original C38 baseline | Specifically scoped — `Mutable<T>` is the case-D ONLY path; case A/B/C still use clean syntax.  The ceremony is the explicit-opt-in cost. |
| Generic implementation pulls in cross-mode / native codegen complications | Phase 05 cells include cross-mode runs across all primitive T (integer/text/float/single/boolean/character) plus Reference T (struct).  Parallels @PLAN15 phase 04 coverage. |
| Specialisation (concrete `Mutable<integer>` etc.) may be needed if generic monomorphisation is incomplete | Phase 05 verification includes a Mutable instance for each primitive T; if generic dispatch fails, file as a P-issue and continue with concrete instances. |

## Out of scope

- `Atomic<T>` / `RefCell<T>` / cross-thread shared ownership.
  Plan-06 phase 4d owns the cross-thread fn-ref surface; phase
  05 is single-thread only.

## Cross-references

- [README § D — explicit Mutable&lt;T&gt;](README.md#d--explicit-mutablet-for-shared-ownership)
- [DISCUSSION § Q6 — Mutable&lt;T&gt; API ships regardless](DISCUSSION.md#q6--mutablet-api-ships-regardless) — the long-form open question this phase resolves.
- `lib/server/`, `lib/web/` — packaging convention reference.
