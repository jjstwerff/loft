<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster II — slot null-init must dominate its free (the #405 root)

The LIVE cluster (probe `04`). Root cause **VERIFIED from the IR** — this is the
shared root the class-retirement fix targets, not the #405 instance.

## Status

| | |
|---|---|
| Root cause | ✅ VERIFIED (IR evidence below) |
| Fix design | candidate chokepoint named (Stage C); implementation = Stage D |
| Severity | corruption + **interpret SIGSEGV** on `main`; native completes (divergence) |

## The mechanism (VERIFIED)

`main`'s inner loop for the #405 repro lowers to (LOFT_LOG=static, `--interpret`):

```
ki = n_enc(i, __ref_1);
x["__vdb_1"] = null;                  // x's null-init (dep = __vdb_1)
if i == t {                           // CONDITIONAL
  OpDatabase(__vdb_1, 65);            //   __vdb_1's slot WRITTEN only here
  x = OpGetField(__vdb_1, 0, 64);
  OpAppendVector(x, ki, 11);
}
OpFreeRef(ki);
...                                   // (fn scope, unconditional:)
OpFreeRef(__ref_1);
OpFreeRef(__vdb_1);                   // ← reads __vdb_1's slot UNCONDITIONALLY
```

`__vdb_1` (the hidden-buffer dep slot) is **allocated conditionally** (inside
`if i==t`) but **freed unconditionally** at fn scope — and its slot is **never
sentinel-initialised at fn entry**. On the `i != t` path the slot holds stale
per-iteration stack content, and `OpFreeRef(__vdb_1)` treats it as a real
`store_nr` → the #405 "refused free of out-of-range store" + the **#306-class**
"stack-record ref treated as an owned heap store" (the slot's stale bytes decode
to a stack-store ref) → **SIGSEGV** on interpret.

| Claim | Status |
|---|---|
| `__vdb_1` allocated only inside the conditional; freed at fn scope | ✅ VERIFIED (IR) |
| `__vdb_1` slot has no scope-entry null-init/sentinel | ✅ VERIFIED (IR — no `__vdb_1 = null` before the `if`) |
| stale slot → bogus `store_nr` → #405 + #306 + SIGSEGV (interp) | ✅ VERIFIED (probe 04 runtime) |
| native completes (init/free imbalance not fatal there) | ✅ VERIFIED (probe 04) — mechanism may still silently corrupt; unconfirmed |
| this is @PLN51 cluster-II's uncovered (conditional × unused × nested) corner | HYPOTHESIZED (matches its shape; not re-bisected) |

## The invariant (the all-paths fix, not the instance)

> **A heap slot's null-init (sentinel) must DOMINATE its free** — i.e. be emitted
> on every path that reaches the `OpFreeRef`, at (or above) the scope where the
> free is placed.

The bug is a **scope mismatch**: `__vdb_1`'s init is placed at the conditional's
local scope (riding the `OpDatabase`) while its free is hoisted to fn scope. The
fix is not "sentinel-init this `__vdb`" (per-instance) — it is to make the
codegen/scope-analysis guarantee the dominance relation for EVERY heap slot:
whenever an `OpFreeRef(v)` is placed at scope S, a sentinel null-init of `v` is
guaranteed at the entry of S (so a skipped conditional allocation frees the
sentinel — a no-op — never stale bytes). That covers every conditional-alloc /
unconditional-free shape at once, retiring the class rather than #405.

This is the runtime-form of the slot-init-before-lifetime-op invariant from
[recent-bugs.md](recent-bugs.md) Finding 3, localised to the free path.

## Candidate fix site (Stage C — to confirm by reading the placement code)

Free placement lives in scope analysis (`src/scopes.rs`); the null-init/sentinel
ops are emitted in `src/state/codegen.rs` (`gen_set_first_*`, the
`OpInitRef`/`OpInitRefSentinel`/`OpInitCreateStack` block ~1108-1140). The fix is
where the two are reconciled: when scope analysis emits a free for a slot at
scope S, ensure the slot's sentinel-init is emitted at S's entry (dominating the
free). @PLN51 added `OpInitRefSentinel` for several shapes via
`OpVarRef→OpFreeRef→OpInitRefSentinel`; the conditional-alloc-in-nested-loop
shape is the uncovered one — so the fix likely generalises @PLN51's emission
rather than inventing a new mechanism.

## Stage D (implementation) — validation gates

A codegen/scope change on the free path is load-bearing. Gates: probe 04 +
neighbours (vary conditional / unused / nested independently) green on BOTH
backends; the full matrix; `tests/leak.rs` + the wrap leak gate; a debug-mode
full-suite run; the armed double-free build. Graduate probe 04 (once it no longer
SIGSEGVs) to `tests/scripts/`. Re-run @PLN51's cluster-II probes for no
regression. Verify the #306 co-occurrence is also closed (same root) or split it.
