<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Sibling bugs surfaced by the cluster-I probes (edges chased)

Two crashes the store-lifetime probing turned up — *separate* from the
confinement work, characterised here so they're fixable from a clear scope.

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

**Verdict: CLEAR.** It is the tuple-**return** path (copying a tuple with a
vector field across the fn boundary writes to the vector's store while it is
locked).  Local tuples are fine.  Ready to fix — root cause is in the
tuple-return copy + the store lock (`store.rs:1374`, the CONST_STORE lock origin).

## Bug 2 — explicit `parallel {}` parent-stack-var capture is unhandled

| Arm shape | Result |
|---|---|
| function-call arms `parallel { f(); g(); }` (test-80 form) | **OK** |
| single function-call arm | OK |
| assignment arms `parallel { x=1; y=2; }` | **WRONG** — writes silently lost (result 0) |
| arm READS a parent var `parallel { s = x+1; }` | **SIGSEGV** |

**Verdict: NOT a clean targeted fix — a capture-model decision.**  Parallel arms
run in isolated workers with no parent stack; reading a parent var crashes,
writing one is silently dropped.  The right fix is a parallel-subsystem call:
either **reject parent-stack-var capture in a `parallel {}` arm at compile time**
(a clear diagnostic instead of a SIGSEGV / silent loss), or support it with
explicit capture.  That's a THREADING-subsystem design choice, not a one-liner.
