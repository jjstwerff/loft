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
3. **Pinpoint the source line(s)** that emit the bad `if var__ncc_N` predicate.  Steps:
   - `LOFT_KEEP_NATIVE_RS=1 ./target/release/loft --native probe 21` → keep the generated .rs at /tmp/loft_native_*.rs
   - Read the file; locate the `if var__ncc_3` line
   - Trace back to `src/generation/emit.rs` (likely `output_block` or `output_set`'s value-block branch) — search for where the predicate is emitted in the heap-type path
4. Confirm cluster VII bundles — run probes 47/48/82 with the same investigation steps; if the `if-else have incompatible types` originates at the same emit site, the fix covers VII too.

## Fix surface

**Single point**: in `src/generation/emit.rs` value-block return path for heap-typed `??`, replace the bare `if var__ncc_N` predicate with a proper null-check:

```rust
// CURRENT (broken):
write!(w, "if {} {{{}}} else {{{}}}", ncc, ncc, fallback)?;

// FIX:
write!(w, "if {}.store_nr != u16::MAX {{{}}} else {{{}}}", ncc, ncc, fallback)?;
```

(Exact syntax depends on what the `DbRef` null sentinel is — confirm via `src/keys.rs` or `src/state/STRING_NULL`-style constants.)

**Variant for IV-Tuple**: tuples are `(field_0, field_1, …)` not DbRef; the null-check may need a different sentinel (per-field zero-init?).  Probe 40's specific case needs re-verification once the Vec/Hash fix lands.

**What it fixes:**
- Cluster IV-Vec/Hash/Sorted/Index/Enum/Tuple (Set E): yes — all 6 sub-types compile and pass on native.
- Cluster VII (chained-call): yes (hypothesis) — same code path emits the chained-`if-else` mismatched-types error.
- Cluster IV-Spacial: NO — that's a parser-hang, different sub-cluster, different fix (`cluster-IV-Spacial-parser.md`).
- Cluster I (text): NO — text has a separate emit branch (the `_ret.to_string()` path).
- Interpret-side IV — likely closes incidentally if the interpreter's `OpEqNull` already handles heap DbRef nulls correctly, but verify with Set E re-run.

**Effort**: S (2-3 days).  The fix surface is small; the validation surface (Set E + cluster VII probes 47/48/82) is bounded.

**Risk**: LOW — native compile error is loud; if the fix introduces a different miscompile, the compile error switches shape rather than silent corruption.

## Why Reference escapes

@PLAN51's `paired_witness` + S1 NRVO (closed 2026-05-29) — see `plans/finished/51-hidden-buffer-aliasing/cluster-II-latent-leak.md` for the full mechanism.  Reference value-blocks get the inner ref's adoption / hidden-buffer-substitution which prevents the post-consumer free pattern from forming.  Vec/Hash/Sorted/Index/Enum/Tuple don't get this machinery because @PLAN51's scope was buffer-aliasing in ref_return contexts, not all heap-type value-blocks.

This plan's fix completes the family coverage that @PLAN51 left at "Reference only."
