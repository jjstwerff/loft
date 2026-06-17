<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Sibling bugs surfaced by the cluster-I probes (edges chased)

Two crashes the store-lifetime probing turned up — *separate* from the
confinement work, characterised here so they're fixable from a clear scope.
**Status:** Bug 1 **fixed** (`19eebc98`).  Bug 2 is a *sibling discovery*, **not**
part of this (store-lifetime / Goal E) investigation — it surfaced here but was
investigated as its own scoped case (see
[plans/README.md § Sibling bugs are discoveries](../../../README.md#sibling-bugs-are-discoveries-to-record-not-cases-to-fix-in-place)).
**Soundness floor LANDED:** every unbuilt/broken capture (writing or mutating an
enclosing local, capturing a parameter) is now a clean **compile error on both
backends**; parent-var *reads* stay legal (the P245 surface test-81 guards).  The
full capture *feature* (copy-out / channels) stays deferred to its driving consumer
(the server/client library).  See the verdict's **Implementation** section for the
rule, the residuals, and the regression test.

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

  **Implementation — LANDED** (`src/parser/control.rs::reject_unsound_parallel_captures`,
  fired from `parse_parallel`; regression `tests/scripts/170-parallel-capture-soundness.loft`;
  both backends).  Mapped against a 67-probe battery, then built precise:
  - **Rule.** An arm may only *read* enclosing state.  Reject (compile `Error`):
    (a) a **write/mutation** of an enclosing user local — detected as a `Set`
    target or the host (descended through `args[0]`) of an in-place mutating op
    (`OpAppendVector`/`OpSetInt`/…); (b) capture of a **parameter** (`is_argument`).
  - **The hard part was telling enclosing from arm-local.**  The two-pass parser
    pre-populates the whole function's var table in pass 1, so `vars.count()` /
    var-nr ordering can't separate them.  The working signal is **`is_defined`
    snapshotted at the block's opening brace** — a var defined before the block is
    enclosing; one first defined inside an arm is arm-local.  Compiler temps
    (`_`/`#` names) and for-loop vars (`was_loop_var`) are excluded so loop/format
    desugar never false-flags.
  - **Residuals (documented, not regressions):** `l01` — passing a captured heap
    value to a function that *mutates* it is transitive and still faults at runtime
    (needs callee analysis).  `i04` (`vv[0] += [2]`) — a `data.rs:3036
    "Unknown definition"` **compile-time codegen assertion that fires outside
    `parallel {}` too**, so it is a *separate* nested-vec element-compound-assign
    bug, not a capture issue (new sibling discovery).
  - **Candidate for a direct machinery fix later:** param-read SIGSEGVs only at
    *teardown* (the read value is correct) — likely a small frame-cleanup bug that
    could promote param-read to *sound* rather than rejected, when a consumer wants
    it.
  - **Known weakness — harden before this code is next touched.** The
    enclosing/arm-local split rides an **undocumented invariant**: "in pass 2
    `is_defined` is set in source order, so a var declared before the block reads
    defined and one first declared inside an arm reads undefined."  Nothing
    *enforces* it — a future parser change to when `is_defined` is set would make
    the diagnostic silently false-positive, a hidden coupling no test guards.  By
    this floor's own governing principle (no hidden machinery; the model must match
    reality) this is its least-principled spot.  Cheap principled fix: document the
    invariant as a contract at the snapshot site **and** add a
    `debug_assert!` that each arm-declared local reads `!is_defined` at block entry
    — turning a relied-upon accident into a stated, enforced fact.  (Secondary: the
    temp exclusion matches *names* `_`/`#` rather than a first-class flag like
    `was_loop_var`; lower risk since those names are un-typeable by users.)

- **The feature (supporting writes) — deferred to its consumer, not built.**
  The real use case for `parallel {}` (vs `for…par`, which owns data-parallelism)
  is **server/client async I/O**: long-lived heterogeneous I/O arms that
  *coordinate* (accept-arm → worker-arms, shared shutdown).  The right primitive is
  almost certainly **message-passing / channels** between isolated arms (keeps
  "no shared mutable state ⇒ no data race", Goal A/E), not parent-writes — so the
  design must be driven by the server/client library consumer, per the dogfood
  cadence.  Until then the surface errors honestly (the floor above).  The unbuilt
  feature errors at compile time *by design* — you opt into a feature by building
  it, not by the grammar accepting the syntax.

**Done:** soundness floor landed (compile errors for the unbuilt surface, both
backends; regression `tests/scripts/170`).  **Deferred:** the channel/coordination
model, consumer-driven by `lib_plans/future/08-server` + `10-game-client`.
