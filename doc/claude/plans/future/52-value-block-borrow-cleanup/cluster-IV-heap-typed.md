<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster IV — heap-type value-blocks via `??` (Vec / Hash / Sorted / Index / Enum / Tuple)

**Severity:**
- **Native compile error E0308** — `if var__ncc_N {...} else {...}` emits `DbRef` where rustc expects `bool` (or `if`/`else` have incompatible types).  **LOUD failure** — code doesn't compile.
- **Interpret silent corruption** — value-block returns zero-valued elements (Vec: empty / zeroed tags; Hash: null collection; Enum: fallback variant; Tuple: fallback).

**Affected probes:** 21 (Vec), 22 (Hash), 23 (Enum), 36 (Vec via iter consumer), 40 (Tuple), 41 (Sorted), 50 (Index).  All 7 SHARE THE SAME ROOT MECHANISM.  See [Probe set E](README.md#curated-probe-sets--for-fix-attempt-validation).

**Excluded from this doc**: IV-Spacial (probe 51) — separate parser-hang sub-cluster.  See `cluster-IV-Spacial-parser.md`.

**Excluded from this doc**: IV-Ref (probe 10, PASS) — falsified; @PLAN51's `paired_witness` + S1 NRVO machinery covers Reference value-blocks already.

**Backend asymmetry:** BOTH backends fail (interpret silent + native compile error).  This is the only multi-cluster-family case in PLAN52 where native is the LOUDER failure — opposite of cluster I.

## Mechanism (verified)

The `??` lowering for a heap-typed result produces the same value-block shape as cluster I:

```
{ #ncc:heap_type
  _ncc_N = vec_of_vecs[i];           ← null-sentinel DbRef on OOB
  if (_ncc_N != null) _ncc_N else other_vec;
}
```

`_ncc_N` is a `DbRef` (12 bytes: store_nr + rec + pos), not a primitive.  The bug is in how the **null-check predicate** lowers:

### Native side (loft → Rust codegen)

`src/generation/emit.rs` value-block return path emits:

```rust
let _ret = if var__ncc_N {var__ncc_N} else {var_fallback};
//             ^^^^^^^^^^
//             expected `bool`, found `DbRef`
```

rustc rejects with E0308 because `if`'s condition must be `bool`, not `DbRef`.  The codegen forgot to emit the null-check predicate (e.g. `var__ncc_N.is_some_sentinel()` or `var__ncc_N.store_nr != u16::MAX`).

For **chained-call** variants (probes 47/48/82 — cluster VII), the error mode is slightly different: `if`/`else` have incompatible types — both branches return `DbRef` but with different layouts/lifetimes that rustc can't unify.

### Interpret side

Bytecode `OpEqNull` (or its heap-typed equivalent) compares `_ncc_N`'s DbRef against the null sentinel (`store_nr == u16::MAX` or `rec == 0`).  For non-OOB indexes the sentinel comparison correctly returns false, so the true branch runs.  But the returned DbRef's store may have been freed or aliased to another allocation by then — reading the elements produces zeros (Vec/Hash) or the fallback variant (Enum/Tuple).

Confirmed for all 6 sub-types via Set E.  Mechanism is opcode-symmetric.

## Reference probe — 10 (Reference value-block, PASS)

```loft
v.items += Holder { name: "present" };
a = v.items[0] ?? fallback;
```

Lowering: same `_ncc_N` shape, but @PLAN51's `paired_witness` + S1 NRVO machinery (closed 2026-05-29) substitutes hidden buffer args so the inner call writes directly into the caller's buffer — avoiding the post-consumer free that drives the heap-handle-recycle pattern.

## Problem probes

### Probe 21 — Vector value-block

```loft
o.lists += inner;
a = o.lists[0] ?? fallback;   // a is vector<Inner>
```

Native: `if var__ncc_3 {var__ncc_3} else {var_fallback}` → E0308.
Interpret: `a[0].tag == 0` (zeroed elements).

### Probe 22 — Hash value-block

Same shape with `hash<Entry[name]>`.  Same E0308; interpret returns null collection.

### Probe 23 — struct-Enum value-block

```loft
b.items += Named { name: "present" };
a = b.items[0] ?? fallback_anon;
```

Native: E0308 same.
Interpret: `a` is the fallback variant entirely (not Named{name:"present"}).

### Probe 36 — Iteration consumer

```loft
for x in (o.lists[0] ?? fallback) {
```

Native: E0308.
Interpret: loop count == 1 instead of 3 (vector length read from corrupt DbRef).

### Probe 40 — Tuple value-block

```loft
w.tuples += ("present", 42);
a = w.tuples[0] ?? fallback_tuple;
```

Native: E0308.
Interpret: `a.0 == "fallback"` (got fallback tuple entirely).

### Probe 41 — Sorted value-block

Same shape with `sorted<Entry[name]>`.  Same E0308; interpret null.

### Probe 50 — Index value-block

Same shape with `index<Entry[name]>`.  Same E0308; interpret null.

## The divergence

The native emit forgets that `_ncc_N` is a `DbRef`, not a bool.  The fix is to emit a proper null-check predicate (e.g. `var__ncc_N.store_nr != u16::MAX`) — the same null-sentinel comparison that the interpreter's `OpEqNull` already does.

Same single fix surface for all 6 heap-type sub-clusters.

## What we know vs. don't

| | Status |
|---|---|
| All 6 heap-type families (Vec/Hash/Sorted/Index/Enum/Tuple) fail with the same E0308 shape | ✅ Verified — probes 21-23, 36, 40-41, 50 |
| Reference (IV-Ref) escapes via @PLAN51 | ✅ Verified — probe 10 |
| The fix is a single predicate-emit change | ✅ Strong hypothesis — error message is consistent across probes |
| Exact source line of the bad `if` emit | ❌ Not yet pinpointed — needs `grep` of `src/generation/emit.rs` value-block path and `LOFT_KEEP_NATIVE_RS=1` inspection |
| Cluster VII (chained-call `if-else have incompatible types`) shares the same fix surface | 🤔 Strong hypothesis — different error message, same code path; verify when the fix lands |

## Investigation tasks

1. ~~Verify all 6 sub-types fail uniformly~~ — done.
2. ~~Confirm Reference is the only escape~~ — done (probe 10).
3. ~~**Pinpoint the source line(s)** that emit the bad `if var__ncc_N` predicate.~~ — done 2026-05-29.  Site: `src/generation/emit.rs::output_if_inner` lines 842-846 (now refactored to call a new `output_test_predicate` helper).
4. ~~Confirm cluster VII bundles — run probes 47/48/82 with the same investigation steps.~~ — done 2026-05-29.  **Cluster VII does NOT bundle**: probes 47/48/82 fail with `expected &String, found Str` (text-typed branch unification), NOT the DbRef-predicate issue.  Cluster VII has a separate fix surface in the text-branch path.

## Fix surface

**Single point**: `src/generation/emit.rs::output_if_inner` value-block `??` predicate.  Both branches of the `Insert`/non-`Insert` dispatch (was lines 842-846) now route through a new `output_test_predicate` helper that wraps the bare test with `.rec != 0` when `infer_type(test)` returns a heap-DbRef type (`Reference` / `Vector` / `Sorted` / `Hash` / `Index` / struct-`Enum`).

```rust
// FIX (landed 2026-05-29):
fn output_test_predicate(&mut self, w: &mut dyn Write, test: &Value) -> std::io::Result<()> {
    let heap_dbref = matches!(
        self.infer_type(test),
        Some(
            Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Enum(_, true, _),
        ),
    );
    if heap_dbref {
        self.output_code_inner(w, test)?;
        write!(w, ".rec != 0")
    } else {
        self.output_code_inner(w, test)
    }
}
```

**Variant for IV-Tuple**: tuples are `(field_0, field_1, …)` not DbRef; the null-check uses a different sentinel (per-field zero-init).  Probe 40 (tuple) still COMPILE-ERRs after this fix and remains open.

### Fix iterations

**Iteration 1 (2026-05-29) — predicate fix landed**
- Site: `src/generation/emit.rs::output_test_predicate` (new helper).
- Result on Set E after fix (rerun against macos-clippy-fixes branch + this edit):

  | Probe | Type | Before | After |
  |---|---|---|---|
  | 21 | Vector | COMPILE-ERR | runtime panic (separate `var_a` null-sentinel init bug — see "Remaining work" below) |
  | 22 | Hash | COMPILE-ERR | runtime panic (same separate bug) |
  | 23 | struct-Enum | COMPILE-ERR | **PASS** ✅ |
  | 36 | Vector (iter consumer) | COMPILE-ERR | runtime panic |
  | 40 | Tuple | COMPILE-ERR | COMPILE-ERR (predicate not heap-DbRef typed — needs Tuple-specific sentinel) |
  | 41 | Sorted | COMPILE-ERR | runtime panic |
  | 50 | Index | COMPILE-ERR | runtime panic |

- Set H baselines: **all PASS** (no regression).
- Sets A / B (cluster I): unchanged (interpret FAIL, native PASS) — fix doesn't touch text path.
- Interpret-side: unchanged — interpreter wasn't blocked by the predicate-emit bug; its silent-corruption shape is unrelated.

**Remaining work after iteration 1**:

The predicate fix closes struct-Enum but exposes a SECOND latent bug in Vec / Hash / Sorted / Index value-block lowering: `var_a` (the destination of `a = vec[i] ?? other`) is declared as the null-DbRef sentinel BEFORE `OpReplaceKeyed`/equivalent copies the if-result into it.  `OpReplaceKeyed` then crashes on `store_nr=u16::MAX` (out-of-bounds in `stores.allocations`).

This is a separate fix surface: the `??` lowering needs to either (a) allocate a fresh store for `var_a` before emitting the if-result, or (b) emit a direct assignment of the if-result to `var_a` instead of going through `OpReplaceKeyed`.  See `parser/operators.rs` `??` heap-type lowering.

**What iteration 1 fixes:**
- Cluster IV-Enum (probe 23): **closed** — native compile + runtime PASS.
- Cluster IV-Vec/Hash/Sorted/Index: predicate compiles; runtime exposes the secondary bug above.
- Cluster IV-Tuple (probe 40): unaffected — Tuple isn't heap-DbRef-typed.
- Cluster VII (chained-call): unaffected — different error mode (`&String` vs `Str` text branch unification).
- Cluster IV-Spacial: unaffected — parser-hang, separate fix.
- Cluster I (text): unaffected — text uses its own emit branch.

**Effort**: S (~1 day, done).
**Risk**: LOW — verified: no regression on Set H baselines, no test failure beyond pre-existing P383 cluster-I (`repro_p323.loft`).

## Why Reference escapes

@PLAN51's `paired_witness` + S1 NRVO (closed 2026-05-29) — see `plans/finished/51-hidden-buffer-aliasing/cluster-II-latent-leak.md` for the full mechanism.  Reference value-blocks get the inner ref's adoption / hidden-buffer-substitution which prevents the post-consumer free pattern from forming.  Vec/Hash/Sorted/Index/Enum/Tuple don't get this machinery because @PLAN51's scope was buffer-aliasing in ref_return contexts, not all heap-type value-blocks.

This plan's fix completes the family coverage that @PLAN51 left at "Reference only."
