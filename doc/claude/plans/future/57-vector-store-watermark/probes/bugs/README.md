<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Sibling bugs surfaced by the cluster-I probes (edges chased)

Two crashes the store-lifetime probing turned up — *separate* from the
confinement work, characterised here so they're fixable from a clear scope.
**Status:** Bug 1 **fixed** (`19eebc98`).  Bug 2 is a *sibling discovery*, **not**
part of this (store-lifetime / Goal E) investigation — it surfaced here but is
being investigated as its own scoped case (see
[plans/README.md § Sibling bugs are discoveries](../../../../README.md#sibling-bugs-are-discoveries-to-record-not-cases-to-fix-in-place)).
A first fix attempt (a rejecting diagnostic) was **reverted** — it was built on a
coarse characterisation and would have broken `tests/scripts/81` (parent-var
*reads* are legal + P245-guarded; only *writes* corrupt).  Re-investigating now
from a proper probe battery, observation-first, both backends (verdict below).

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

## Bug 2 — explicit `parallel {}` parent-stack-var **write** corrupts/crashes

The first-pass table below was **too coarse** — it lumped all parent-var access
together.  `tests/scripts/81-parallel-outer-vars.loft` (the **P245** regression
guard) proves a parent var **read as a call argument** *works*: P245 made the
worker read the parent frame, and that test locks it in.  So the real split is
read-vs-write, not access-vs-no-access.

| Arm shape | Result | Notes |
|---|---|---|
| call arms, literal args `parallel { f(1,2); g(); }` (test-80) | **OK** | no parent ref |
| call arm, **reads** a parent var as an arg `parallel { forward(outer); }` (test-81) | **OK** | **P245-fixed + guarded** |
| **writes** a parent var `parallel { x=1; y=2; }` | **WRONG** — writes silently lost (result 0) | parent `x`/`y` written |
| **writes** a parent var across scope depth `parallel { s = x+1; }` | **SIGSEGV** | the WRITE to `s` (outer-scope) faults; reading `x` is fine |
| read-only in a non-arg position (`tmp = outer+1`, `print("{outer}")`) | **UNVERIFIED** | probe battery `/tmp/par_probe/` pending |

**Verdict: a BUG tangled with a FEATURE — fix the bug now, defer only the feature.**
The bug-vs-feature framing holds; the *boundary* is narrower than first written.

- **The bug — NOT deferrable.**  A *write* to an enclosing-scope local in a
  parallel arm is silently dropped (wrong answer, no error) or SIGSEGVs.  Both are
  the worst class of fault (Goal A soundness); there is no correct behaviour (the
  worker can read the parent frame post-P245 but cannot write it back).  Caught at
  compile time it becomes a clear diagnostic instead.

  **CORRECTION — the diagnostic must NOT reject every enclosing-scope reference.**
  The earlier "reject any ref, zero blast radius" claim was **wrong**: it would
  reject `forward(outer)` and **break test 81**.  The diagnostic must fire only on
  the **broken set** — writes to parent locals, plus any read position later shown
  to still crash — while leaving parent-var *reads* (which P245 fixed) compiling.

  Implementation state: the first fix attempt was **reverted** (tree clean) — it
  was scope-creep into this investigation and was built on the coarse table, so it
  rejected *all* enclosing-scope references and would have failed test 81.  The
  design notes survive for when this case is investigated properly: a complete
  read+write var-ref walk (`Set`/`TuplePut` *targets* matter — `code_references_var`
  drops them, so it can't be reused) + a `parse_parallel` `base = self.vars.count()`
  snapshot (nr < base ⇒ enclosing local), narrowed to **writes** (plus any read
  position the probes show still crashes).  Sub-decision settled: hard `Error` (a
  `Warning` leaves a construct that still compiles to a segfault).

  **Next step — observation-first, no fix yet:** run the 13-probe battery
  (`/tmp/par_probe/`, both backends) to map the true read/write × position ×
  scope-depth × type boundary.  Hypothesis under test: P245 fixed reads wholesale
  ⇒ rejecting *writes* only is the complete, test-81-safe boundary.  Then decide
  the permanent home for the probes + the narrowed fix (its own scoped case, per
  the sibling-bug rule).

- **The feature (supporting writes) — deferrable, an enhancement not a bug.**
  Making `parallel { x = parent; }` *work* needs real concurrency-semantics design:
  copy-in/copy-out vs shared+locks; two arms writing the same parent var (race /
  last-writer / reduction model); the likely vehicle is the existing closure-capture
  machinery (the `par(...)`/`par_light(...)` builtins already capture via closure
  records + `inc_rc`), but wiring explicit `parallel {}` arms to it changes the
  block's semantics, not just its codegen.  That alone routes to a THREADING slot.

**Next action:** probe the read boundary → narrow the diagnostic to the broken set
→ verify test 80 + test 81 still pass → promote `parallel_read_parentvar_SIGSEGV.loft`
and `parallel_assign_arms_WRONG.loft` to expect-compile-error tests.  The write
*feature* is the only future-routed item.
