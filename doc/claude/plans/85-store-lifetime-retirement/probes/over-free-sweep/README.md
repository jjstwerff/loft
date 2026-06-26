<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Over-free class — probe sweep (2026-06-26)

A boundary-matrix sweep of the **borrowed-source over-free class** (see
[../../over-free-class-study.md](../../over-free-class-study.md)): a value that BORROWS
another's store (vector element, struct/enum field, local container) escapes via
return / bind / reassign, and a site frees the borrowed store while it is still live.

Each probe varies one axis (escape path × source shape × ownership) and runs under
**allocation pressure** (a `filler()` loop that recycles freed slots) with a value +
length assert and a leak check — because this class only corrupts once a freed slot is
REUSED (minimal-scale repros pass; cf. cluster-462). Hand-computed expected values; every
probe prints `PN ok` on success.

## How to run

```sh
for f in P*.loft; do
  loft --interpret "$f"                               # value/assert + interp over-free
  LOFT_NATIVE_LEAK_CHECK=1 loft --native "$f"          # native over-free + leak
done
```
`--baseline` classification used a `git worktree` build of `origin/main`.

## Status matrix (HEAD = fix-crawler after the two A.3 / borrow-tail fixes)

| Probe | Exercises | main interp | HEAD interp | HEAD native | verdict |
|---|---|---|---|---|---|
| P1 | struct-field view return via local + reassign + pressure | PASS | PASS | PASS | ✅ guard |
| P2 | borrowed view passed as ARG, appended into a collection | PASS | PASS | PASS | ✅ guard |
| P5 | owned-fresh return adopted across reassign (no over-copy/leak) | PASS | PASS | PASS | ✅ guard |
| P6 | multi-arm `if` returning borrowed views + reassign | PASS | PASS | PASS | ✅ guard |
| P7 | mid-body `return b.rows` (struct-field vector view) | PASS | PASS | PASS | ✅ guard |
| P8 | 3-level call chain forwarding a borrowed element view | PASS | PASS | PASS | ✅ guard |
| P10 | accumulate borrowed-view call results into a vector | PASS | PASS | PASS | ✅ guard |
| P12 | reassign `best = cand` of a whole-STRUCT borrowed view | PASS | PASS | PASS | ✅ guard |
| P13 | nested struct-field chain `o.inner.rows` borrowed return | PASS | PASS | PASS | ✅ guard |
| P16 | borrowed element view returned + read | PASS | PASS | PASS | ✅ guard |
| P11 | struct-field vector borrow, NO reassign (ternary) | **AFAIL** | PASS | PASS | ✅ **FIXED this session** |
| P15 | vector local reassigned in a loop from borrowed-view calls | **AFAIL** | PASS | PASS | ✅ **FIXED this session** |
| **P9** | struct-field **vector** borrow + reassign + pressure | AFAIL (over-free `b.rows`) | **AFAIL** (`r=0` empty) | **CRASH** | 🔴 **OPEN** — improved (b.rows no longer over-freed) but result still wrong/crashes |
| **P14** | enum-field vector borrow via `match` arm + pressure | **CRASH** | **CRASH** (SIGSEGV op=5) | PASS | 🔴 **OPEN** — interp-only over-free of the enum's `items` |
| **P3** | mon_one-cond shape (conditional `chosen=m` view of local) | PASS | PASS | PASS but **LEAK** (1 store) | 🟠 **LEAK OPEN** — the #462 leak-mirror |
| P4 | `vv[i] ?? []` vector-literal coalesce default | PASS | PASS | **E0308** | ⏸ **PARKED** — Family A nullability, not this class |
| P9min | P9 with NO pressure | PASS | PASS | PASS | control — proves P9 needs slot-reuse |
| P14b | P14 with a typed-empty `_` arm | CRASH | CRASH | PASS | control — proves the bug is the `items` borrow, not the `[]` arm |

## What the sweep established

- **The two fixes this session hold and generalise** — P11/P15 were red on `main`,
  green on HEAD; no probe regressed (every clean probe passes on both backends, leak-free).
- **The OPEN items are all pre-existing and all the SAME class** (P9, P14, #462, P3-leak):
  a borrowed view of a vector/field/element escapes and its store is freed under
  slot-reuse. They are NOT caused by the fixes. Root + collapse: the propagated borrow-set
  chokepoint in [../../over-free-class-study.md](../../over-free-class-study.md).
- **P9 / P14 are compact drivers** for the substrate fix — they fail with ~6–8 pressure
  iterations (vs #462's ~190 live stores), so they are far cheaper repros + future guards.
- **`LOFT_UAF_SRC`** reports a freed-source `OpCopyRecord` (used to pin #462); P9/P14 do
  NOT trip it, so their corruption surfaces at a vector read/append, not a record copy —
  the operand-stack/vector-element half of the detector still to extend (cluster-462 gap #1).

## Crash-fix progress (2026-06-26) — two slices landed, #462 root-caused

| Crash | Sub-shape | Status |
|---|---|---|
| **P9** | vector return-buffer bound from a buffer-returning call (PutRef-alias of `__ref`) | ✅ **FIXED** (slice 1, scopes.rs witness-pair) — both backends; suite green |
| **P14** | borrowed match-arm vector binding returned (`Filled { items } => items`) renamed onto the caller's buffer | ✅ **FIXED** (slice 2, control.rs `tail_borrows_arg` → CopyBorrow) — both backends; suite green; guard `tests/scripts/85-store-lifetime-enum-field-vector-borrow.loft` |
| **#462** | STRUCT sibling: `mon_one`'s `chosen = m` where `chosen: M` (dense) and `m: __nullable<M>` (from `pool[wj] ?? …`) | 🔴 **OPEN, root-caused** — the type mismatch (M vs __nullable<M>) skips the same-struct deep-copy → falls to a `PutRef` ALIAS (`one()` bytecode @301), so the dense-struct retbuf aliases the local `pool` element; `pool` freed at exit → return dangles → mon_choose copies a freed `tp=76` store. Entangled with the nullable-element coalesce-unwrap (`m` should be dense `M`, not `__nullable<M>`). Crawler-scale only — verify via `LOFT_UAF_SRC` on the corpus, not a minimal probe. |

Remaining: the **leak-dual** (P9 native — false `["c"]` return-borrow) and **#462** (struct,
nullable-unwrap-in-reassign). Both converge on the propagated borrow-set + the dense/nullable
conversion in assignment.

## Substrate fix progress (2026-06-26, scopes.rs)

First substrate slice, driven by P9: a **VECTOR return-buffer** (hidden SRet arg, NRVO'd)
bound from a `!return_adopts_fresh_store()` call is PutRef-ALIASED to the call's work-ref
arg (no copy — unlike the Reference path, which `gen_set_first_ref_call_copy` copies), and
a plain `OpFreeRef(work-ref)` at scope exit then frees the RETURNED buffer. Fix
(`scan_set`): witness-pair `work-ref → ov` so the free becomes
`OpFreeRefIfDistinct(work-ref, ov)` (no-op since they alias; the caller frees the buffer).

- **P9 crash → FIXED** both backends; full suite 2542 green; no regression.
- **P9 now LEAKS on native** (the two-severity dual): `pick_bigger`'s return carries a
  FALSE `["c"]` borrow dep (it COPIES from `c`, doesn't borrow it), so the caller's `r` is
  classed as a borrow and never freed. Pre-existing return-*classification* error, exposed
  (not caused) by removing the crash. **Next facet** = the false return-borrow inference.
- **#462 (struct sibling) STILL crashes** — `mon_one` returns a `MonsterDef` (Reference,
  tp=76), not a vector, so this vector-specific slice doesn't reach it. The Reference path
  copies on a CALL bind but `chosen = m` (conditional var-assign of a local view) aliases.
- **P14 (enum sibling)** still interp-crashes (enum-field vector via match arm).

The class needs the same treatment per representation (vector ✅ slice, struct/Reference
#462, enum P14) PLUS the return-borrow-classification fix (the leak-dual) — converging on
the one propagated borrow-set in [../../over-free-class-study.md](../../over-free-class-study.md).

## Graduation

A probe graduates to `tests/scripts/85-<slug>.loft` once GREEN on both backends with no
leak. The ✅-guard rows are graduation candidates now; P9/P14 graduate when the substrate
fix lands (they are the fix's acceptance tests).
