<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# nrvo-inline-leak probe corpus (t9, @P393)

Maps the **real** shape of what the rc-removal corpus filed as a single `t9`
row (`split()` temp leak).  Rigorous re-probing showed `t9` is **not**
split-specific, **not** method-vs-function, and **not** rc-related — it is one
clean mechanism with one discriminator.

> **STATUS: FIXED** in `efdf8a1c` (a `Type::Vector` lift arm in
> `inline_struct_return`, guarded on `dep.is_empty()`).  Every probe below is now
> **clean on both backends**; the `interp = LEAK` column records the **pre-fix**
> behaviour each probe permanently guards against regressing.  The fix is the
> sibling of bug 10's `Type::Function` arm — together they are rc-removal **Phase A
> / Mechanism 1, COMPLETE**.  Regression: `tests/scripts/174-inline-temp-free.loft`.

It WAS a current suite-blocker, not a future-cleanup item: `03-text.loft` leaked
`kt main_vector<text>` at exit and `SCRIPTS_LEAK_ALLOW` is empty, so `wrap text`
+ `wrap loft_suite` were **red on the interpreter** until the fix.  (The rc-removal
README's "t9: rc-on clean, RC_OFF-only" line was recorded against a stale binary
and was **wrong** — corrected here.)

Run each probe on both backends and compare `LOFT_STORES=summary` output:
`./target/release/loft --interpret <f>` vs `./target/release/loft <f>`.

## The mechanism (one sentence)

A fn/method whose returned local is built with **≥2 distinct element-temps** is
**de-NRVO'd** by `01a3f24f` (to fix the `z=[a]; z=[b]; z` returned-`[a]` bug), so
it returns its vector **by value** (empty dep, signature `n_f()` not
`n_f(__vdb_1:…)`); used **inline-unbound** that by-value temp gets **no
`OpFreeRef` on the interpreter** → it leaks.

## The map (2026-06, both backends)

| probe | shape | distinct temps | NRVO'd | interp | native |
|---|---|---|---|---|---|
| 02 one-append-nrvo-clean | `z=[]; z+=[1]; z` | 1 | ✅ `["??"]` | clean | clean |
| 04 pure-loop-nrvo-clean | `for…{ z+=[i]; } z` | 1 | ✅ | clean | clean |
| **01 two-append-inline-leak** | `z=[]; z+=[1]; z+=[2]; z` | 2 | ❌ | **LEAK** | clean |
| **03 loop-trailing-split-shape** | loop `+=` **+ trailing** `+=` (= `split`) | 2 | ❌ | **LEAK** | clean |
| **05 reassign-inline-leak** | `z=[9]; z=[1,2]; z` and conditional reassign | 2 | ❌ | **LEAK** | clean |
| 06 bound-baseline-clean | de-NRVO'd return **bound to a local** | 2 | ❌ | clean | clean |
| 07 borrowed-return-must-not-free | `-> text[self]` view, inline | — | borrow `[self]` | clean | clean |
| 08 irrelevant-axes | int/text/struct/float × arg/receiver/nested × builtin/user-fn | 2 | ❌ | **LEAK** | clean |
| 09 stdlib-split-real-trigger | `len("a-b".split('-'))`, `split().join()` (03-text) | — | ❌ | **LEAK** | clean |

## Findings

1. **The sole discriminator is NRVO de-promotion.**  IR-confirmed: `<2` distinct
   element-temps → `n_f(__vdb_1:…) -> vector<…>["??"]` (hidden buffer param +
   non-empty dep); the caller allocates `__ref_N` and frees it → clean.  `≥2` →
   `n_f() -> vector<…>` (no param, empty dep) → by-value → leaks inline-unbound.
   The count is over distinct **`+=`/`=` SITES** (02 vs 04: a pure loop is 1 site;
   03's trailing `+=` is the 2nd site that tips `split` over).

2. **Three axes are irrelevant** (probe 08).  Once de-NRVO'd, the leak is
   invariant under element type (int/text/struct/float), use-site (arg / method
   receiver / nested call), and consumer (native builtin `len` *or* user fn).
   The only clean consumer is a `for` loop iterating the result directly — and
   that is not a call-argument site, so the call-arg fix neither needs nor
   touches it.

3. **Bound is always clean** (probe 06) — the local's scope-free covers it.  This
   is the user workaround today **and** the correctness oracle for the fix: the
   fix must reproduce exactly the bound behaviour (one free, no double-free).

4. **It is the `01a3f24f` residual, broader than documented.**  `01a3f24f` flagged
   only the *conditional* reassign sub-case and said "no test exercises it."
   Probe 05 shows straight-line reassign and append-build leak identically, and
   probe 03/09 show `split`'s loop+trailing **is** exercised — by `03-text.loft`.

5. **Interpreter-only.**  Native codegen frees the by-value temp; every probe is
   `native = clean`.  So the fix is an interpreter-side concern.

## Fix direction (not yet decided — see `dont_decide_during_exploration`)

The de-NRVO decision in `01a3f24f` is **load-bearing** — re-NRVO'ing a
reassigned/appended return reintroduces the `z=[a]; z=[b]; z` returned-`[a]` bug.
So the fix is **not** "NRVO it anyway"; it is **"give the de-NRVO'd, inline-unbound
vector temp the same statement-end `OpFreeRef` the bound local already gets."**

- **Discriminator for the fix:** lift the inline result of a call whose
  `def.returned` is `Type::Vector` with **`dep.is_empty()`** (owned by value).
  Empty dep ⇒ de-NRVO'd owned return → free.  Non-empty dep excludes both the
  NRVO'd (`["??"]`, caller already frees `__ref`) and the borrowed (`[self]`,
  freeing = UAF) cases — probes 02/04/07 are the regression guards for that.
- **Site:** `inline_struct_return` (`src/scopes.rs`) already lifts the analogous
  Reference / struct-Enum / **Function** (bug-10) cases via `get_free_vars` →
  `OpFreeRef`.  This is the **same lift shape**; the gap is a `Vector` arm that
  reaches loft-source fns **and** `t_` methods (`split` is `t_4text_split`),
  guarded on `dep.is_empty()`.  The existing native-vector branch
  (`def.code == Null`, bare-call) is the precedent for the guard.

Sibling of **bug 10** (closure-temp leak, fixed) — both are "an unbound
heap-returning-call temporary has no statement-end free."  Together they are
**Phase A / Mechanism 1** of the rc-removal plan.
