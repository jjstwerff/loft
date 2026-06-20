<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster II — the #405 live crash (root NOT yet verified)

The LIVE cluster (probe `04`). **⚠️ CORRECTION (instrument falsified the earlier
"verified" root).** While starting Stage D, a backtrace instrument on the bogus
free + a fully-clean IR re-dump overturned the slot-init-dominance story below:

- The failing free is **`OpFreeRef` via `free_ref`** (a plain `OpFreeRef`, the
  per-iteration `OpFreeRef(ki)`), of an **out-of-range store `#24`** while
  `allocations.len==5` — i.e. a garbage `store_nr`, not a double-free of a valid
  store.
- **`__vdb_1` AND `__ref_1` ARE null-init'd at fn entry** in the clean IR
  (`__vdb_1 = null` / `__ref_1 = null` as the first two ops of `main`). So the
  "no scope-entry sentinel-init" claim below is **FALSE**, and a half-1
  fn-entry-sentinel attempt (verified to prepend the init) did **not** fix it.

So the verified facts are only: probe 04 SIGSEGVs on interpret / completes on
native; the crash is an `OpFreeRef` of a garbage `store_nr`; the fn-entry
null-inits are present. The analysis below (slot-init-dominance, half-1/half-2)
is a now-FALSIFIED hypothesis kept only as a record of what was ruled out.

### Pinned so far (VERIFIED by instrument + bytecode, Stage B continuing)

- The crashing op is **ONE `OpFreeRef`, at `code_pos 7292`** (`free_ref` →
  `free` backtrace), firing **per iteration** — i.e. `OpFreeRef(ki)` (the only
  per-iteration free; the other two frees are fn-exit `__ref_1` / `__vdb_1`).
- The freed DbRef is **`{store_nr = 8·(rec−1), rec, pos=1}`** with `rec`
  advancing 1/iter and `store_nr` advancing **8/iter** (24, 32, 40, …) while
  `allocations.len == 5`. The `×8` cadence = a **stack offset / structured
  garbage read as a heap `store_nr`** (the #306 "stack-record ref treated as a
  heap store" shape), NOT a stale-but-valid double-free.
- `ki` (slot 64) is **`PutRef`-assigned** from the `n_enc` return (bytecode
  `Call(n_enc)` → `PutRef(var[64])`), i.e. it ALIASES the NRVO return buffer
  `__ref_1` (enc materialises into the caller-passed `__retbuf`). So `ki` and
  `__ref_1` name the same store, yet BOTH get an `OpFreeRef` (per-iter `ki` +
  fn-exit `__ref_1`).
- Boundary (matrix): fires on **conditional × unused** (`x += ki` whose result
  `x` is never read); disappears when `x` is read (C/E) or assigned
  unconditionally (B).

**Working mechanism (HYPOTHESIZED, needs live confirmation):** when `x` is unused
the `x += ki` copy is dead, so `ki`'s only consumer is dead, and the NRVO-alias
`ki` is freed per-iteration via a malformed DbRef (the `store_nr` half not a real
store) — a double-free / stack-ref-as-heap of the return buffer. This is a
RETURN-BUFFER-aliasing bug (H1/NRVO family), NOT the `__vdb_1` dep-slot story.

**Next step (live inspection — static reads have hit their limit):** `loft debug
--rpc` breakpoint at the per-iteration `OpFreeRef(ki)`, inspect `ki`'s slot bytes
+ what last wrote them; and bisect the boundary (does removing `x += ki` while
keeping `x` unused still fire?) to confirm the dead-copy link.

---

## (FALSIFIED hypothesis — kept as ruled-out record)

~~Root cause VERIFIED from the IR — this is the shared root.~~ Overturned above.

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

## Localised chokepoint (Stage C — VERIFIED by reading scopes.rs/codegen.rs)

Two facts pin it:

1. **`__vdb_1`'s slot has no null-init of its own.** `x = null` (x's dep =
   `[__vdb_1]`) lowers via `codegen.rs::gen_set_first_*` (the
   `OpInitRef`/`OpInitRefSentinel`/`OpInitCreateStack` block ~1108-1140) to
   `OpInitCreateStack` — which points *x's* slot at `__vdb_1`'s slot but does
   NOT write `__vdb_1`'s slot. `__vdb_1`'s only writer is the **conditional**
   `OpDatabase`; its `OpFreeRef` is unconditional at fn scope.
2. **`scopes.rs` already owns this exact relation** — the Plan-57 cluster-I pass
   (`check`, ~298-318): `store_confinement()` decides a `__vdb` is block-confined,
   then `relocate_null_init()` moves its null-init into that block "so its
   `first_def` / codegen free live there too." The normal (`#410`) IR shows a
   `__vdb_1 = null` BEFORE the `OpDatabase`; the #405 IR has **none** — consistent
   with the null-init being relocated/dropped into the conditional block while the
   free stayed at fn scope.

**One instrument run disambiguates the fix** (do this first in Stage D — don't
theorise): add an `eprintln` behind an env flag in `store_confinement` /
`relocate_null_init` and run probe 04.
- If `store_confinement` returns `__vdb_1` (confined to the `if` block): the bug
  is its **loop/dominance guard** — the `if` block sits *inside* the nested
  loops, which the "non-loop LCA chain" rule (~3979) should already reject; find
  why it doesn't, and tighten so a `__vdb` whose free is NOT inside the candidate
  block is never relocated. (Confinement must imply the free is in the block.)
- If it does NOT fire: the bug is the **codegen gap** — a conditionally-allocated
  `__vdb` with a fn-scope free needs a dominating `OpInitRefSentinel` at fn entry
  (generalise @PLN51's `OpVarRef→OpFreeRef→OpInitRefSentinel` emission to this
  shape).

Either way the enforced invariant is the same (null-init dominates free); the
instrument picks which of the two homes to fix it in. The #306 co-occurrence
should fall out of the same fix (the stale slot decoded to a stack-store ref).

## Stage D — instrument result + the refined fix (in progress)

**Instrument run done** (temporary `eprintln` in `store_confinement`, reverted):
on probe 04 it printed **nothing** → `store_confinement` does NOT classify
`__vdb_1`. So hypothesis (b) confinement/relocation is **RULED OUT**; (a) the
codegen/scope null-init gap is confirmed.

**The chokepoint is `run_scan_phase`'s `lift_vars` prepend** (`scopes.rs` ~144):
it already does exactly the right thing — `for v in lift_vars { bl.operators.insert(0, v_set(v, Null)) }`
— "assigned inside conditional branches but their `OpFreeRef` lives at function
exit; prepend the null-inits so codegen reserves their slot along every path."
But `lift_vars` is populated ONLY by `scan_args` (the `__lift_N` inline-arg path,
~1951). A conditionally-defined `__vdb` freed at fn scope is the same shape and
is NOT added → no prepended null-init → the bug.

**Refinement (the part that makes this load-bearing, not a one-liner):** `x`'s
store is freed **per-iteration** — the pre-Set free of `x = null` reads `x`'s dep
(`__vdb_1`) every loop pass — while `__vdb_1` is REUSED across iterations. So a
single fn-entry `__vdb_1 = null` (the plain `lift_vars` prepend) null-inits only
the FIRST pass; after the store is freed on a later pass, the slot still names the
freed store → a subsequent pre-Set free is a **double-free**. The full invariant
therefore has two halves:
  1. **entry**: the slot is the null sentinel before its first free (the
     `lift_vars` prepend, extended to this `__vdb` shape); AND
  2. **post-free**: a free that consumes a slot **resets it to the null sentinel**,
     so the next (stale) read is a no-op.
Half (2) is the robust, all-paths form (it covers reuse + any future producer);
it is also the higher double-free risk, so it must be matrix-validated, not
guessed. Candidate sites: the free op (`fill.rs`/`Stores::free` — write the
sentinel back after freeing) and/or the `lift_vars` extension for half (1).

**Next concrete step:** boundary matrix varying {conditional, unused, nested,
reuse-count, single-vs-multi-store} on `--interpret` + `--native`; implement
half (1)+(2) at the chokepoint; verify no double-free via the gates below.

## Stage D — boundary matrix + half-1 attempt (DECISIVE: half-2 is required)

**Boundary matrix** (`probes/05-matrix-*`, hand-computed expected = "completes
cleanly", `--interpret`, pre-fix):

| Cell | conditional | unused | nested | result |
|---|---|---|---|---|
| A | ✓ | ✓ | ✓ | **SIGSEGV** |
| B | — (always assign) | — | ✓ | ✅ ok (control) |
| C | ✓ | — (reads x) | ✓ | ✅ ok (control) |
| D | ✓ | ✓ | — (single loop) | BUG #405 (refused, exit 0) |
| E | ✓ | — (reads x) | ✓ | ✅ ok (control) |
| F | ✓ | ✓ | ✓ (8×8) | **SIGSEGV** |

Boundary: **conditional × unused** triggers (D); **nesting escalates to SIGSEGV**
(A/F). B/C/E are passing controls — the matrix is calibrated (can fail AND pass).

**Half-1 attempt (reverted):** extended the `lift_vars` prepend to add a
fn-entry `__vdb_1 = null` for any heap var freed at the top level but defined
only nested. IR confirmed the prepend fired (`__vdb_1 = null` at fn entry) — but
A/D/F **still crashed**. This is the decisive result: the crashing free is NOT
the fn-exit `OpFreeRef(__vdb_1)` — it is the **per-iteration pre-Set free** of
`x = null` (the keyed reassign frees x's prior store via the dep slot every loop
pass). Half-1 fixes the FIRST free only; on a later pass `x = null` frees the
reused store A, leaving `__vdb_1` naming the freed A, so the NEXT `x = null`
**double-frees A** → SIGSEGV.

**Therefore the invariant's half-2 is REQUIRED, not optional:** *a free that
consumes a slot must reset that slot to the null sentinel*, so the next read of
the reused dep slot is a no-op. Fix site = the **keyed-reassign pre-Set free**
(the `x = null` / `OpReplaceKeyed` + `remove_claims` path, `allocation.rs` /
codegen): after freeing x's store, write `DbRef{u16::MAX}` back into the dep slot
(`__vdb_1`). Half-1 (entry sentinel) is still needed for the very first free.

This modifies the reassign/free path for ALL keyed locals → whole-language
double-free risk → it must run the FULL validation harness below, not just the
matrix. Half-1 was reverted (unvalidated + insufficient alone); the design
(half-1 + half-2) is recorded here.

## Stage D (implementation) — validation gates

A codegen/scope change on the free path is load-bearing. Gates: probe 04 +
neighbours (vary conditional / unused / nested independently) green on BOTH
backends; the full matrix; `tests/leak.rs` + the wrap leak gate; a debug-mode
full-suite run; the armed double-free build. Graduate probe 04 (once it no longer
SIGSEGVs) to `tests/scripts/`. Re-run @PLN51's cluster-II probes for no
regression. Verify the #306 co-occurrence is also closed (same root) or split it.
