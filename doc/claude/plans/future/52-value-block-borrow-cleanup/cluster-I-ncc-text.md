<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster I — `??`-on-text with non-trivial LHS (the @P383 family)

**Severity (split by failure mode):**
- **Corruption / panic / hang:** silent data corruption — caller reads dangling-Str bytes; on macOS+rustc 1.96 these read back as length-preserved-NUL (sub-mode IA, pure-Set consumer) or non-zero garbage (sub-mode IB, buffer-write consumer); on Linux+earlier rustc the bytes happen to survive and the test "passes" via undefined behaviour. *Worst manifestation: probe 46/49 SIGBUS via method-chain consumer (tracked as cluster I-crash sub-doc, same root).*
- **Leak:** none — the inner `_ncc_N` String IS freed (the bug is freeing it too early, not leaking it).

**Affected probes:** 02, 04-06, 11, 13-17, 20, 24-26, 29-34, 37-39, 44, 76, 77, 81 (cluster I core + LHS-shape borders + consumer borders + sub-modes).  See [Probe set A and B](README.md#curated-probe-sets--for-fix-attempt-validation) for the curated subsets.

**Backend asymmetry:** Interpret-side only.  Native escapes via the @P321e/@P323 fix at `src/generation/emit.rs:1283-1297` (`_ret.to_string()` materialisation inside the value-block).  No interpreter equivalent exists.

## Mechanism (verified)

The `??` operator's lowering in `src/parser/operators.rs:1234-1244` constructs a value-block for non-Var LHS:

```rust
let tmp = self.create_unique("ncc", lhs_type);
let set_tmp = v_set(tmp, code.clone());        // _ncc_N = <LHS expr>
let null_check = Value::Var(tmp);
let true_branch = Value::Var(tmp);
let if_expr = v_if(null_check, true_branch, rhs);
*code = v_block(vec![set_tmp, if_expr], result_type.clone(), "ncc");
```

So `h.items[0] ?? "fallback"` becomes:

```
{ #ncc:text
  _ncc_N = h.items[0];          ← OpGetText into block-local
  if (_ncc_N != null) _ncc_N else "fallback";   ← value-block tail
}
```

When `scopes::free_vars` (`src/scopes.rs:1122-1124`) processes the block:

1. The block's tail expression is the `if` (returning `_ncc_N` or the literal `"fallback"`).
2. `get_free_vars` (`src/scopes.rs:1184`) sees `_ncc_N` is `Type::Text(_)` and appends `OpFreeText(_ncc_N)`.
3. Final order: `[Set(_ncc_N, …), if(…), OpFreeText(_ncc_N)]`.

Interpreter bytecode trace (probe 02, via `LOFT_LOG=fn:main`):

```
PC 99:  VarText(var[_ncc_N])  →  push Str(ptr=&_ncc_N.buffer, len=7) on stack
PC 102: ConvBoolFromText(...) →  push true
PC 103: GotoFalseWord(jump=112, if_false=true)   ← takes true branch
PC 106: VarText(var[_ncc_N])  →  push Str(ptr=&_ncc_N.buffer, len=7) again
PC 109: GotoWord(jump=122)    ← skip the else
PC 122: FreeText(var[_ncc_N]) ← drop _ncc_N's String — bytes freed
PC 125: AppendText(var[a], v1=<raw:0x9dac61a90>)   ← reads dangling ptr
```

The `<raw:0x…>` marker at PC 125 IS the smoking gun: `Str::try_str` (`src/keys.rs:78`) escapes when the `Str`'s `ptr` is non-readable, indicating the pointer is dangling.  On macOS+rustc 1.96, the freed bytes get zeroed by libmalloc before the consumer reads → 7 NUL bytes.  On Linux glibc, the freed bytes typically survive → tests pass by accident.

### Why does native escape

`src/generation/emit.rs:1283-1297` (`output_block` text-result branch):

```rust
} else if matches!(bl.result, Type::Text(_)) {
    // @P321e / @P323 — materialise to OWNED String inside the block
    writeln!(w, "_ret.to_string()")?;
}
```

Generated Rust:

```rust
let mut var_a = { //ncc_2: text
  let mut var__ncc_2: String = …;
  let _ret = if (&var__ncc_2) != STRING_NULL { &var__ncc_2 } else { "fallback" };
  _ret.to_string()  // ← owned String, lifetime tied to block expression
}.to_string();
```

The block evaluates to an owned `String`; Rust's drop order leaves it alive past the inner `var__ncc_2`'s drop.  Interpreter has no analogue — bytecode `OpFreeText(_ncc_N)` runs inside the block.

## Reference probe — 01 (simple-var `??`, PASS)

```loft
v = "present";
a = v ?? "fallback";   // LHS is Value::Var(_)
```

**Lowered IR**: `operators.rs:1227-1233` short-circuits the simple-Var path — no block construction, no `_ncc_N`, no scope-exit free.  The `if` directly returns the live local's Str.

## Problem probe — 02 (vec-index `??`, FAIL on interpret)

```loft
h.items += "present";
a = h.items[0] ?? "fallback";   // LHS is non-trivial (OpGetVectorNullable)
```

**Lowered IR**: As described in Mechanism — `_ncc_N` block + post-consumer `OpFreeText`.

## The divergence

Simple-Var LHS hits the fast-path at `operators.rs:1227`; any non-Var LHS allocates `_ncc_N` and constructs the value-block.  The block's tail returns `_ncc_N`'s Str (or the literal); the scope-exit `OpFreeText(_ncc_N)` runs INSIDE the block, before the outer consumer reads the stack value.

## What we know vs. don't

| | Status |
|---|---|
| Cluster fires on every non-Var LHS shape (vec-index, field, call, method, concat, deep chain, static-source, hash-deep, real-library config) | ✅ Verified via 12+ probes |
| Sub-mode IA (NUL fill) on pure-Set consumers (`a = …`, tuple destructure, concat-after) | ✅ Verified — probes 02/13/14/15/16/17/37/39 |
| Sub-mode IB (garbage fill) on buffer-write consumers (`s += …`, format-string, hash-insert, struct-field, vec-append, enum-ctor, reassign, direct-print of present) | ✅ Verified — probes 26/29/31/32/33/38/44/81 |
| Sub-mode IC (false-equality) on `==` consumer | ✅ Verified — probe 34 |
| I-crash sub-mode (SIGBUS) on method-chain consumer | ✅ Verified — probes 46/49.  Mechanism hypothesised (method-dispatch reads dangling Str's ptr).  See `cluster-I-crash` (combined with this doc) |
| Return-position is ALSO broken (probe 25 finding) | ✅ Verified — the B5-L3 fix only covers OOB-fallback path because the literal lives in `.rodata` |
| Cluster I fix incidentally closes cluster III (format-string) | 🤔 Strong hypothesis — same root.  Verify after cluster I lands |
| Cluster I fix incidentally closes cluster VI (closure body) | 🤔 Depends on probe 87 IR trace (TODO) — if closure-synth fn body uses the SAME `_ncc_N` shape, yes; if different, separate fix |

## Investigation tasks

1. ~~Confirm shape independence~~ — done (probes 13-17/76/77).
2. ~~Confirm consumer-mode taxonomy~~ — done (probes 26/29/31/32/33/38/44/81).
3. ~~Confirm return-position exposure~~ — done (probe 25).
4. **Read `src/scopes.rs::free_vars`** thoroughly to understand existing structure for text scope-exit, especially the B5-L3 branch at lines 998-1015.  The cluster I fix is a structural sibling of B5-L3, adapted for value-block context (is_return=false).
5. **Read `src/scopes.rs:1184`** — `get_free_vars` text-Type emission.  Confirm the fix inserts AFTER the existing emission, not by removing it (the block-local _ncc_N still needs to be freed; we just need to insert the materialisation FIRST).
6. Cross-reference with `cluster-VI-closure.md` once probe 87 IR is captured — decide if cluster I's fix surface covers VI or VI needs a separate path.

## Fix surface

Two viable options ranked by effort and risk.  Both touch `src/scopes.rs::free_vars`.

### Option A (recommended): parent-scope `__ret_text_N` temp à la B5-L3, adapted for is_return=false

Mirror the B5-L3 text-return path (`src/scopes.rs:998-1015`) but apply it when the block result is `Type::Text(_)` AND `is_return == false`:

```rust
} else if !is_return && matches!(tp, Type::Text(_)) && !expr_is_terminal && ls_has_text_free() {
    // Materialise the tail's text into a PARENT-scope text temp before
    // the inner OpFreeText fires.  The temp's String gets the AppendText
    // (OWN copy of the bytes), so subsequent OpFreeText on the source
    // doesn't dangle the consumer's Str.
    self.ret_temp_counter += 1;
    let name = format!("__ret_text_{}", self.ret_temp_counter);
    let tmp = function.add_temp_var(&name, tp);
    self.var_scope.insert(tmp, /* outer scope, NOT self.scope */);
    self.var_order.push(tmp);
    let mut result = Vec::with_capacity(ls.len() + 2);
    result.push(v_set(tmp, expr.clone()));   // copy bytes into __ret_text_N
    result.extend(ls);                        // run frees (now safe — bytes copied)
    result.push(Value::Var(tmp));             // block result = __ret_text_N
    return result;
}
```

**What it fixes:**
- Cluster I core (Set A): yes — value-block now yields an OWNED copy.
- Cluster I garbage-consumer (Set B): yes — same root.
- Cluster I-crash (Set C): yes — method dispatch reads from __ret_text_N's live buffer.
- Cluster III (format-string): yes incidentally — format-buffer reads from __ret_text_N.
- Cluster V (real-library): yes — closes via Set I's reliance on cluster I.

**What it doesn't fix:**
- Return-position present-path (probe 25): NEEDS the same fix applied when is_return=true too.  Either drop the `!is_return` guard (apply uniformly) OR add a parallel branch.  Recommended: drop the guard — the materialisation is safe in both contexts.
- Cluster IV (heap-type value-block) — different fix surface (`src/generation/emit.rs`).
- Cluster VI (closure body) — pending probe 87 IR trace; may close incidentally if closure-synth fn body's IR goes through this same code path.
- Cluster VII (chained-call native E0308) — different fix surface.

**Effort**: 1-2 days of careful implementation + ~3-5 days of moros_*-suite gating per the @PLAN51 history (scope-handler fixes routinely break adjacent things).

**Risk**: HIGH — Set H baselines + the @PLAN51 canary (`130-gridmesh-crystal-equiv.loft`) must stay green.  Three @P377 fix attempts were reverted because of this exact failure mode.

### Option B (fallback): emit a stack-discipline-aware materialisation in the bytecode

Rather than introducing a new scope temp, modify the bytecode emit so the value-block's tail expression's RESULT is COPIED INTO A NEW TEXT before the OpFreeText runs.  This is conceptually similar to Option A but lives one layer deeper (codegen, not scope-pass).

```
{ #ncc:text
  _ncc_N = ...;
  __block_result = if (_ncc_N != null) _ncc_N else "fallback";  ← Set into a fresh text
  OpFreeText(_ncc_N);
  __block_result
}
```

**Effort**: comparable to Option A but in `src/state/codegen.rs` instead of `scopes.rs`.

**Risk**: MEDIUM — codegen changes typically have narrower blast radius than scope-pass changes, but the existing @PLAN51 / B5-L3 logic is intertwined enough that side effects are possible.

**Why Option A is recommended over B**: B5-L3 already established the parent-scope-temp pattern for text returns.  Extending it for value-blocks is symmetrical, reuses existing temp-management code, and makes the diff small.  Option B is a structurally different approach that could be revisited if A regresses.

## Fix iterations (to be filled as attempts land)

### Iteration 1 (2026-05-30) — `skip_free` on `_ncc_N` text temp — INTERPRET CLOSED, NATIVE REGRESSED

**Approach:** name the temp `__ncc_N` (double-underscore), mark it `skip_free`, and let
`scopes.rs::get_free_vars` (line 1183-1185) suppress the OpFreeText for skip_free text
vars.  Hypothesis: the existing `has_ret_temp` recognition + `patch_hoisted_returns`
collapse in `src/generation/pre_eval.rs` would handle the native side if the prefix
match was extended to `__ncc_`.

**Result:**
- **Interpret**: Set A all 6 PASS.  Set B all 9 PASS.  Set H baselines unchanged.
- **Native**: COMPILE-ERR E0597 for all of Set A + Set B.  `var___ncc_2: String`
  declared inside the block, block returns `&var___ncc_2`, the outer `.to_string()`
  wrap consumes a borrow that dies at the block's closing brace.

**Why the native fix didn't work:**

`patch_hoisted_returns` at `src/generation/pre_eval.rs:110-240` ONLY collapses the
`[Set(target, Call), ..., Return(Var(target))]` pattern — i.e. it requires `Return` at
the block's tail.  Value-block context (`??` with non-Var LHS) emits an `If` at the
tail, not a `Return`:

```ir
Block {
  operators: [
    Set(__ncc_N, lhs_expr),
    If(null_check, Var(__ncc_N), rhs),     ← block's tail expression
  ],
  result: text,
}
```

When this is wrapped by the outer consumer (e.g. `Set(a, <block>)`), the native emit
declares `var___ncc_N: String` INSIDE the Rust block scope.  The `If` returns
`&var___ncc_N` (a borrow).  The wrap that adds `.to_string()` on the block (outside
the closing `}`) creates a borrow that dies before `.to_string()` runs.

The existing `_ret.to_string()` materialisation in `src/generation/emit.rs:1283-1297`
is gated on `wrap_result && returned == Type::Text` — i.e. it only fires for FUNCTION
RETURN positions, not value-block positions.

**Reverted commit**: yes — all three edits rolled back; tree clean as of 2026-05-30.

**What the next iteration needs:**

A coordinated native-side change is required.  Three possible directions:

1. **Extend `patch_hoisted_returns`** to recognise the value-block-tail-If shape
   (`Set(temp, expr); If(check, Var(temp), rhs)`) and substitute `Var(temp)` with the
   original `expr` (eliminates the temp).  Risk: re-evaluates `expr` (the L6 issue —
   if `expr` is a side-effecting Call, double-evaluation breaks).  Mitigation: only
   collapse when `expr` is side-effect-free (Var, simple field/index reads).

2. **Add a parallel pre-eval pass** for value-block tail-If text temps that materialises
   the `.to_string()` INSIDE the block's tail (mirroring `_ret.to_string()` but for
   non-return contexts).  This produces:
   ```rust
   { let var___ncc_N: String = ...;
     (if (&var___ncc_N) != STRING_NULL {&*(&var___ncc_N)} else {&*("fallback")}).to_string()
   }
   ```
   The block now produces an owned `String`; the outer consumer takes ownership cleanly.

3. **Hoist the temp's declaration out of the block** in the native emit.  Emit
   `let var___ncc_N: String = ...;` BEFORE the block, then the block returns
   `&var___ncc_N` — the borrow now lives at parent scope, surviving the outer
   `.to_string()`.  Risk: parent scope may not support this hoist (e.g. if the
   value-block is inside an expression context that can't accept a preceding `let`).

**Recommended next step:** Implement direction (2) — `.to_string()` inside the
block's tail.  It's localised to native emit (no parser change, no scope-pass change),
mirrors the existing function-return pattern, and has clear precedent.

**Effort**: 2-3 days for direction 2 (emit.rs changes + verify Set A/B + Set H +
moros_* suite + check the wrap test suite).

**Risk**: MEDIUM — same domain as the @P323 fix that already shipped; the new
materialisation path can reuse most of the existing `needs_p205_scratch` logic.

## Why native escapes

`src/generation/emit.rs:1283-1297` materialises `_ret.to_string()` inside the value-block, producing an owned `String` that lives as the block's result.  Rust's drop order keeps it alive past the inner `var__ncc_N`'s scope-end drop.  See [§ "Why does native escape"](#why-does-native-escape) in Mechanism for the generated-code form.

The interpreter has no analogue.  Option A above is the interpreter equivalent.
