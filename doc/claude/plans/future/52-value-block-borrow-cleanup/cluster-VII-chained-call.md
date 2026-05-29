<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster VII — chained `call() ?? call() ?? literal` and recursive `??`-using fns (native E0308 if-else incompatible types)

**Severity:**
- **Native compile error E0308** — `if`/`else` have incompatible types in the chained-coalesce join.  Code doesn't compile.
- **Interpret silent corruption** — cluster I sub-mode (NUL or garbage depending on consumer).

**Affected probes:** 47 (chained `lookup1(k) ?? lookup2(k) ?? "default"`), 48 (recursive fn with `??`), 82 (`vec[i] ?? get_default()`).  See [Probe set G](README.md#curated-probe-sets--for-fix-attempt-validation).

**Backend asymmetry:** BOTH backends fail.  Native is the LOUDER failure (compile error).

## Mechanism (verified)

`call1(args) ?? call2(args) ?? "default"` lowers to:

```
{ #ncc:text
  _ncc_N = call1(args);
  if (_ncc_N != null) _ncc_N else { #ncc:text
    _ncc_M = call2(args);
    if (_ncc_M != null) _ncc_M else "default";
  };
}
```

The outer `if`'s ELSE branch is itself a value-block whose return type the codegen tries to unify with the outer's THEN branch (`_ncc_N`).  Native codegen at `src/generation/emit.rs` emits both branches with their per-block return-type — but the OUTER `_ncc_N` is `String` (from the inner `_ret.to_string()` materialisation) while the INNER's `_ret.to_string()` produces a different `String` lifetime/binding, and rustc sees mismatched if-else types:

```
error[E0308]: `if` and `else` have incompatible types
    --> /tmp/loft_native_NNN.rs:NNNN:NN
     |
  if … { _ret.to_string() }
                          - expected because of this
  else { _ret.to_string() }
         ^^^^^^^^^^^^^^^^ expected `String`, found …
```

The exact "found" type depends on the inner branch's structure.  For recursive `??` (probe 48) the inner branch is a recursive call whose return type is reported back to the outer with different binding semantics.

Cluster VII shares the **same code path** as cluster IV's emit fault — the value-block return path in `src/generation/emit.rs`.  The difference: cluster IV's failure is "predicate type wrong" (`if <DbRef>`), cluster VII's is "branch types mismatched."  Both are symptoms of the same emit site not properly unifying value-block return types.

## Reference probe — 25 (simple return-position `??`, FAIL but for different reason)

```loft
fn extract(h: H, idx: integer) -> text { return h.items[idx] ?? "fallback"; }
```

Native: PASSES (B5-L3 return-path covers this — though probe 25 confirms it doesn't cover the value-block context).

The reference here is the simpler "fn return value is `??` with literal default" — works native.  Cluster VII surfaces when the chain depth is ≥ 2 OR when recursion is involved.

## Problem probe — 47 (chained `??` lookups, FAIL on both)

```loft
fn lookup(c: Cache, k: text) -> text { return c.items[k].value; }
…
a = lookup(primary, "theme") ?? lookup(secondary, "theme") ?? "fallback";
```

Native: E0308 if-else have incompatible types.
Interpret: 4 NUL chars (length right, bytes zeroed).

## Problem probe — 48 (recursive `??` fn, FAIL on both)

```loft
fn pick(h: H, idx: integer, depth: integer) -> text {
  if depth <= 0 { return "deep-default"; }
  candidate = h.items[idx] ?? pick(h, idx + 1, depth - 1);
  return candidate;
}
```

Native: E0308.
Interpret: 2 NUL chars.

## Problem probe — 82 (`vec[i] ?? get_default()` — call as default)

```loft
a = h.items[0] ?? get_default();
```

Native: E0308.  Interpret: cluster I NUL.

## The divergence

For chained `??` where one or both branches are CALL EXPRESSIONS, the native emit unifies the branch return-types incorrectly.  For a literal default (e.g., `?? "fallback"`), the else branch is a `&'static str` that unifies cleanly with the materialised String from the true branch.  For a call default, the else branch is whatever the call returns (potentially with its own `_ret.to_string()` materialisation), and the unification fails.

## What we know vs. don't

| | Status |
|---|---|
| Cluster fires on chain depth ≥ 2 with call branches | ✅ Verified — probes 47/82 |
| Cluster fires on recursive `??`-using fns | ✅ Verified — probe 48 |
| Same emit site as cluster IV-V/H/etc. (likely) | 🤔 Strong hypothesis — both errors originate from `src/generation/emit.rs` value-block return; verify on fix |
| Interpret-side joins cluster I (same NUL/garbage manifestation) | ✅ Verified |

## Investigation tasks

1. ~~Confirm chain depth ≥ 2 triggers the bug~~ — done.
2. ~~Confirm recursive fns trigger~~ — done.
3. **Pinpoint the emit site** — same task as cluster IV (`LOFT_KEEP_NATIVE_RS=1 ./target/release/loft --native probe 47`, read the generated .rs, locate the `if`/`else` with mismatched types).  Likely the same site as cluster IV's bad predicate emit, just a different failure mode.
4. **Verify shared fix surface with cluster IV**: once cluster IV's predicate fix lands, re-run Set G.  If all 3 probes PASS, cluster VII closes as a free rider on cluster IV's fix.

## Fix surface

**Primary**: same emit site as cluster IV (likely `src/generation/emit.rs::output_block` value-block return path).  Need to:
1. Compute the unified return type for the value-block (resolve the if-else branches' types to a common type).
2. Emit branches that produce that common type — possibly wrapping each branch in an explicit conversion if mismatched.

For text specifically, the common type is `String`; both branches should be `_ret.to_string()` regardless of whether they're literal or call.  For chained `??` the inner block already produces `String`; the outer just needs to consume that String correctly.

**Fix shape** (rough):

```rust
// In output_block, for text/heap value-block returns:
//   - Resolve the common return type for if-else branches
//   - For text: emit both branches as `String` (literal → `"x".to_string()`, call → as-is)
//   - For DbRef heap-types: emit both branches as `DbRef` with proper null-sentinel handling (cluster IV fix)
```

**What it fixes:**
- Cluster VII (Set G): yes — chained / recursive call patterns compile cleanly.
- Cluster IV (Set E): yes — same emit path.
- Cluster I-interpret side of cluster VII probes: **NO** — still relies on cluster I's `__ret_text_N` materialisation.  Set G's native passes; interpret still fails on cluster I lines.

**Effort**: S, bundled with cluster IV (one fix surface).

**Risk**: LOW — native compile error is loud; if the fix produces a new miscompile, the error shape changes rather than silent corruption.

## Why this bundles with cluster IV

Both cluster IV (heap-type value-blocks with E0308 predicate) and cluster VII (chained-call value-blocks with E0308 if-else types) originate from the SAME emit site in `src/generation/emit.rs`.  Cluster IV's fix unifies the predicate; cluster VII's fix unifies the branch return types.  Same code path, two adjacent improvements.

Land both in a single PR.  Validation: Set E + Set G together.
