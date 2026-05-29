<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster VI — closure body `??` (lambda-bodies break on both backends)

**Severity:**
- **Native compile error E0308** — closure-synthesised fn emits a malformed `if`-predicate (same shape as cluster IV).
- **Interpret silent corruption** — closure body returns NUL-fill for the `??` result.

**Affected probes:** 45 (closure with struct-field LHS), 65 (closure with text-Var LHS — surprise: fast-path lost), 67 (closure passed as fn arg), 86 (pre-bound text + closure), 87 (minimal closure for IR trace).  See [Probe set F](README.md#curated-probe-sets--for-fix-attempt-validation).

**Backend asymmetry:** BOTH backends fail.

## Mechanism (partially verified)

Probe 65's finding is the surprise: a closure body that uses `??` on a captured `text` VARIABLE (simple-Var LHS, which `src/parser/operators.rs:1227` is supposed to fast-path) STILL FAILS.  That means closure synthesis bypasses or duplicates the fast-path.

Three hypotheses, ranked by likelihood:

### Hypothesis A (most likely): closure synthesis re-parses captures as non-Var expressions

When a lambda captures `s` (a text local), the closure-synth path at `src/parser/control.rs` rewrites uses of `s` inside the body as `closure_state.s` (field access on the captured-state record).  After rewriting, `s ?? "fallback"` becomes `closure_state.s ?? "fallback"` — and `closure_state.s` is a FIELD ACCESS, not a Var.  The `operators.rs:1227` fast-path only matches `Value::Var(_)`, so the rewritten body goes through the full `_ncc_N` block construction — and the cluster I bug fires inside the closure.

If A holds: cluster VI is **structurally identical to cluster I but inside a synthesised function body**.  The cluster I fix at `scopes::free_vars` would cover both — IF the closure-synth fn's body goes through the same `free_vars` path as a regular fn body.

### Hypothesis B: closure-synth bypasses scope-pass entirely

The closure body's emit might not go through `scopes::process_block` — or it might go through a SEPARATE scope-pass that doesn't have the same `free_vars` logic.

If B holds: cluster VI needs a separate fix in the closure-synth scope-handler.

### Hypothesis C: native codegen for closure-synth fns has its own bug

Native E0308 might originate from the closure-fn synthesis in `src/generation/` rather than from the body's `??` lowering.

## Reference probe — 01 (simple-var `??`, PASS)

The non-closure equivalent: `v = "present"; a = v ?? "fallback";` works fine because Var LHS triggers the fast-path.

## Problem probe — 65 (closure with text-Var capture, FAIL)

```loft
s = "present";
pick = fn() -> text { s ?? "fallback" };   // s is captured; body looks like Var ??
a = pick();
```

Interpret: `a` is NUL-fill 7 chars.  Native: E0308.

The `s` inside the body LOOKS like Var to the loft programmer, but after closure synthesis it's something else (per Hypothesis A).

## Problem probe — 86 (pre-bind workaround, STILL FAILS)

```loft
items: vector<text> = ["alpha", "beta"];
e0 = items[0];   // pre-bind
e1 = items[1];
pick = fn(which: integer) -> text {
  if which == 0 { return e0 ?? "fb0"; }
  return e1 ?? "fb1";
};
```

Even pre-bound text Vars fail inside the closure body — confirming the closure-synth rewrites these to non-Var accesses.

## The divergence

Outside a closure: `s` is `Value::Var(local_id)` → fast-path.  Inside a closure: `s` may be rewritten to `Value::GetField(closure_state, …)` (or analogous), which is non-Var → block construction → cluster I bug.

## What we know vs. don't

| | Status |
|---|---|
| Closure body fails on both backends | ✅ Verified — probes 45/65/67/86 |
| Failure shape on interpret is cluster I NUL fill | ✅ Verified |
| Failure shape on native is E0308 | ✅ Verified |
| Even simple-Var LHS in closure body fails | ✅ Verified — probe 65 |
| Closure-synth rewrites Var to non-Var field access | 🤔 Hypothesised — needs IR trace via probe 87 |
| Cluster VI shares root mechanism with cluster I (and would close with I's fix) | 🤔 Strong hypothesis pending probe 87 IR diff |

## Investigation tasks

1. ~~Confirm closure body fails uniformly across LHS shapes~~ — done (45/65/67/86).
2. **Probe 87 IR trace**: run probe 65 (or the minimal probe 87) with `LOFT_LOG=fn:main,fn:<closure_synth_name>`.  The IR dump shows both the outer caller and the closure body's IR.  Diff for:
   - Does the closure body show a `_ncc_N` block in places where the outer wouldn't?
   - Does the body's `s` show up as `Var(s)` or as `GetField(closure_state, s_offset)`?
   - Are scope-exit `OpFreeText` ops present, and where in the sequence?
3. **Decision**:
   - If body's IR matches cluster I's shape: cluster I fix covers cluster VI; no separate work needed.  Add a regression-guard probe to Set F that runs WITH the cluster I fix applied.
   - If body's IR is different: closure-synth has its own bug.  Fix in `src/parser/control.rs` (lambda-lowering) or `src/scopes.rs` (closure-synth scope handler).

## Fix surface

**Likely path** (if Hypothesis A confirms): closes incidentally with cluster I's `__ret_text_N` materialisation in `scopes::free_vars`.  The closure-synth fn's body goes through the same scope-pass; the materialisation kicks in symmetrically.

**Fallback path** (if Hypothesis B or C confirms): targeted fix in the closure-synth machinery.  Either:
- Make the closure-synth Var-rewrite preserve the Var fast-path eligibility (i.e., re-run `operators.rs:1227`'s fast-path check on the rewritten body).
- Add a separate ncc-materialisation in the closure-synth's scope handler.

**Effort**: S–M depending on whether cluster I's fix covers it.

**Risk**: LOW–MEDIUM.  Closures are a common user feature; regressions affect every `fn(...) -> text { ... ?? ... }`.  Set F (4 probes) is the validation gate.

## Why this isn't really a separate cluster (probably)

Cluster VI was filed because closure-body `??` fails distinctly, but if Hypothesis A is correct, the underlying mechanism is just cluster I's `_ncc_N` block + post-consumer free, happening inside a closure-synth fn.  The "closure-ness" doesn't add a new root cause — it just exposes the cluster I bug in a context the user might expect to be safe (since the LHS LOOKS like a Var).

Probe 87 is the deciding test.  After cluster I's fix lands, if probe 87 passes without further intervention, cluster VI closes and this doc moves to "fully verified incidentally."
