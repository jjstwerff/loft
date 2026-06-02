<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Sibling bugs surfaced by the cluster-I probes (edges chased)

Two crashes the store-lifetime probing turned up — *separate* from the
confinement work, characterised here so they're fixable from a clear scope.
**Status:** Bug 1 **fixed** (`19eebc98`); Bug 2 split into a fix-now bug + a
deferred feature (see its verdict).

## Bug 1 — returning a tuple that contains a vector CRASHES (`store.rs:1374`)

`Write to read-only store at rec=N fld=0 (locked by: compile.rs::compile (CONST_STORE init))`

Edges (all `--interpret`):
| Shape | Result |
|---|---|
| `(a, 5)` literal vector, **returned** | CRASH |
| vector built mutably (`a=[]; a+=[1,2,3]`), returned | CRASH — **not** literal/const-store specific |
| `(5, a)` vector second, returned | CRASH — position-independent |
| return tuple, read only the **int** element | CRASH — not about reading the vector |
| **LOCAL** tuple `t=(a,5); t.0[0]` (not returned) | **OK** |

**Verdict: FIXED** (`19eebc98`, regression `tests/scripts/169-tuple-vector-return.loft`,
both backends).  Root cause was narrow: the tuple-return heap-promotion
(`control.rs::rewrite_tail_tuple_with_work_ref`) routed the whole vector through
`set_field_check`'s collection arm (`parser/mod.rs:3185`), which emitted a bare
`OpSetInt4` — writing the 8-byte vector DbRef as a 4-byte int → stack skew →
garbage write into the locked CONST_STORE (the `store.rs:1374` crash; a
`DbRef as i32` cast error on `--native`).  Fix: a guarded arm that **deep-copies**
the vector field into its own store (`OpGetField` + `OpAppendVector`), the same
pattern `emit_set_one_element` and the struct constructor use; an
`!matches!(val_code, Int(_))` guard preserves the empty-header init path and the
`f_nr == usize::MAX` narrow-vec `insert` raw-header path.  Local tuples were never
affected.

## Bug 2 — explicit `parallel {}` parent-stack-var capture is unhandled

| Arm shape | Result |
|---|---|
| function-call arms `parallel { f(); g(); }` (test-80 form) | **OK** |
| single function-call arm | OK |
| assignment arms `parallel { x=1; y=2; }` | **WRONG** — writes silently lost (result 0) |
| arm READS a parent var `parallel { s = x+1; }` | **SIGSEGV** |

**Verdict: a BUG tangled with a FEATURE — fix the bug now, defer only the feature.**
The earlier "capture-model decision, not a one-liner" read conflated two things:

- **The bug (undefined behaviour) — NOT deferrable.**  An arm that *reads* a parent
  local SIGSEGVs; one that *writes* a parent local silently drops the write (wrong
  answer, no error).  Both are the worst class of fault (Goal A soundness), and the
  construct has **no correct runtime behaviour** — the worker has no parent stack.
  A construct that can't run correctly belongs caught at compile time, not as a
  segfault.  The fix is a **compile-time diagnostic** that rejects an arm
  referencing an enclosing-scope local, and it is **small (~20–30 lines) with zero
  blast radius** — it can only fire on code that today SIGSEGVs or corrupts:
  - extend `code_references_var` (`operators.rs:18`, currently missing the
    `Parallel` arm) to walk each arm;
  - snapshot `self.vars.count()` at `parse_parallel` entry (`control.rs:4339`) as
    `base`; every parent local has var-nr `< base`, every arm-declared local has
    nr `≥ base`;
  - any referenced var with nr `< base` →
    `diagnostic!(self.lexer, Level::Error, "cannot reference enclosing-scope
    variable '{}' inside a parallel arm — pass it as a function argument")`.
  - `parallel { sa(1,2); }` (test-80 call-arm form) references nothing `< base` →
    stays clean.  Function args passed by value are fine; only direct parent-local
    references fault.  **The diagnostic cannot break any currently-working program.**
  Open sub-decision: hard `Error` (reject) vs `Warning`.  Lean `Error` — a Warning
  leaves a construct that still compiles to a segfault.

- **The feature (supporting capture) — deferrable, but it's an enhancement, not a
  bug.**  Making `parallel { x = parent; }` *work* needs real concurrency-semantics
  design: copy-in/copy-out vs shared+locks; two arms writing the same parent var
  (race / last-writer / reduction model); the likely vehicle is the existing
  closure-capture machinery (the `par(...)`/`par_light(...)` builtins already
  capture via closure records + `inc_rc`), but wiring explicit `parallel {}` arms
  to it changes the block's semantics, not just its codegen.  That is the only
  part that routes to a THREADING-subsystem slot — a deferred *feature*, not a
  deferred bug.

**Next action:** land the rejecting diagnostic + promote
`parallel_read_parentvar_SIGSEGV.loft` and `parallel_assign_arms_WRONG.loft` to
expect-compile-error tests.  The capture *feature* is the only future-routed item.
