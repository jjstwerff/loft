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
3. ~~**Pinpoint the emit site**~~ — done 2026-05-29.  Site: `src/generation/emit.rs::output_if_inner` AND `src/generation/pre_eval.rs::output_if_with_subst` (the two if-emit paths).  Mismatch: then-branch emits `&var__ncc_N` (a `&String`); else-branch emits a Call returning `Str`.  Cluster VII does NOT share cluster IV's predicate fix site — IV is about the *test* DbRef predicate; VII is about the *branch* type unification.

## Fix surface

**LANDED 2026-05-29 (cycle 4).**  Both if-emit paths now wrap each non-Block branch with `&*(...)` when the if-result is text-typed.  `&*(&String)` → `&str`; `&*(Str)` → `&str` (via `Deref<Target=str>`); `&*(&'static str)` → `&str` (idempotent).  Both branches unify as `&str`, then the surrounding `.to_string()` wrap produces `String`.

```rust
// emit.rs::output_if_inner and pre_eval.rs::output_if_with_subst:
let text_unify = !b_true
    && !b_false
    && !matches!(false_v, Value::Null)
    && matches!(self.infer_type(true_v), Some(Type::Text(_)));
// then emit `{&*(<branch>)}` wrap for each non-Block branch when text_unify
```

### Fix iterations

**Iteration 1 (2026-05-29) — text-branch unification landed**
- Sites: `src/generation/emit.rs::output_if_inner` + `src/generation/pre_eval.rs::output_if_with_subst`.
- Result on Set G:

  | Probe | Shape | Before | After |
  |---|---|---|---|
  | 47 | `lookup1(k) ?? lookup2(k) ?? "default"` | COMPILE-ERR (`&String` vs `Str`) | **PASS** ✅ on native |
  | 48 | recursive fn `?? pick(...)` (pre-evals present) | COMPILE-ERR | **PASS** ✅ (pre_eval.rs path also fixed) |
  | 82 | `vec[i] ?? get_default()` | COMPILE-ERR | **PASS** ✅ |

- Set H baselines: all PASS.
- `cargo test --test issues`: 681/681 pass.
- Probe 02 (cluster I baseline) still PASS native — the wrap is idempotent on the existing `&String` / `&'static str` cases.

**Remaining**: interpret-side for these probes still FAILs on cluster I's dangling-buffer issue (NUL fill or garbage).  Native is closed.

**Effort**: XS (~30 LOC across two files; done in one cycle).
**Risk**: LOW — verified no regression on Set H, Set A baselines, or `tests/issues.rs` (681/681).
