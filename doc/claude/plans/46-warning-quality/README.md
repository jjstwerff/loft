<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 42 — Warning quality: stop nagging users about safe code

## Status

**In progress — W1 shipped, W2 mechanism shipped.**  Discovered while
auditing `tools/indexer/src/scan.loft` on 2026-05-18: 30+
`s[i] may produce null on out-of-bounds with no defensive check`
warnings, every one a false positive on code that either uses a
correctly-written library function (e.g. `is_digit_leaf(line[j])`
— the helper handles null safely) or wraps the indexing in a
short-circuit guard (`if j < ll && is_digit_leaf(line[j])` —
intentional bounds check).

- **W1 — DONE.** Short-circuit guard recognition (`WarnCtx::guarded_pairs` +
  `len_captures`, skip pattern 5; extended to struct-field indices
  `self.f[i]`).  `scan.loft` is now at **0** null warnings.
- **W2 — mechanism DONE (first increment).** `#null_safe` function annotation
  (after the body, the `parse_rust` slot) → `Parser::null_safe_defs`; the warning
  walk skips a fault op that is a DIRECT argument to a `#null_safe` callee (skip
  pattern 6), reset at nested calls so it never leaks (`outer(raw(s[i]))` with only
  `outer` annotated still warns).  Tests in `tests/runtime_warnings.rs`.
  *Remaining:* persist the flag on `Definition` (survive `LOFT_STDLIB_CACHE`) +
  annotate the stdlib null-tolerant helpers + the per-param `c: #null_safe T` form.
- **W3 / W4 — not started.**

The existing `is_easy_proof` skip patterns in
`src/parser/operators.rs::is_easy_proof` recognize three shapes
(non-zero literal divisor, non-negative literal index, active
for-loop iter var).  Short-circuit guards and null-tolerant
library helpers are NOT recognized.  This plan adds those two
mechanisms — and **keeps the warning fully active on raw user
code that omits the check**.  The point is to remove noise on
correctly-written safe code, not to weaken the warning's signal
on user code that genuinely needs a check.

## Goal

When a user writes `is_digit_leaf(line[j])` or
`if i < len(s) && s[i] == '@'`, no warning.  When a user writes
`s[i] == '@'` directly with no guard and no null-safe
intermediary, the warning still fires.  `text.byte_at()` exists
only for users who genuinely want raw byte semantics, not as a
workaround for analyzer limitations.

**Explicit non-goal:** the analyzer must NOT infer null-safety
from "happens to not fault-prone-op a null value."  A function
that accidentally avoids the unsafe op shape today must still
trip the warning at user call sites — otherwise the user loses
the prompt to think about the edge case.  Inference is allowed
ONLY when the function explicitly asserts safety (annotation OR
explicit `if x == null { return … }` guard).

## Effort + design

- **Effort:** MH overall.  Each sub-arc is S-M.
- **Design:** ~ (sketch — concrete IR patterns below; dataflow
  details to be filled per sub-arc).
- **Last touched:** 2026-05-18

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **W1** — Short-circuit guard recognition at call sites | § W1 design | ✅ Done |
| **W2** — `#null_safe` param annotation (library author opts in) | § W2 design | ◑ Mechanism done; persistence + stdlib annotations remain |
| **W3** — Entry-guard auto-inference (`if x == null { return … }`) — explicit-check shape only | § W3 design | Open |
| **W4** — `s[i] → byte_at` peephole when consumer is ASCII-only | § W4 design | Open |

W1 + W2 + W3 silence false-positive warnings on *intentionally*
safe code.  W4 closes the perf gap so users never have a perf
reason to choose `byte_at` over `s[i]`.

**Dropped (was W4 in draft):** body-classification auto-inference
("function happens to not fault-prone-op a null value").
Rejected per user feedback — it inferentially marks user code as
safe without an explicit safety assertion, removing the
warning's value at user call sites.  W3's `if x == null` guard
is the line: explicit user intent stays the requirement.

## Phase ordering

1. **W1 first** — biggest immediate win, simplest analyzer
   extension.  Eliminates the recurring `if i < len(s) && s[i]`
   class of false positives.  This is users *writing the check
   themselves*; the analyzer just needs to see it.
2. **W2 second** — annotation infrastructure.  Library authors
   (including `default/*.loft`) opt in by adding `#null_safe`
   to params that handle null correctly.  One-time cost per
   helper; permanent benefit at every call site.
3. **W3 third** — entry-guard auto-inference.  Recognises the
   `if x == null { return … }` shape and AUTO-sets
   `null_safe` on that param.  Same storage as W2; user code
   that explicitly checks gets the same benefit as
   `#null_safe`-annotated stdlib code, without forcing every
   library author to type the annotation.
4. **W4 last** — perf peephole.  Independent of W1-W3 but
   benefits from W2's `#null_safe` annotations (the rewrite
   needs to know the consumer can accept the byte form).

W1 + W2 alone cover ~80% of false-positive volume measured on
scan.loft (the short-circuit guards + the per-call `is_digit_leaf`
sites).  W3 cleans up the long tail.  Stop after W3 if W4's
perf-rewrite complexity isn't paying back relative to writing
`byte_at` explicitly in the rare hot loop that needs it.

---

## § W1 design — short-circuit guard recognition

The analyzer's `is_easy_proof` (`src/parser/operators.rs:1568`)
currently sees only the args of a fault-prone call — not the
surrounding boolean context.  Add a fourth skip pattern:

When walking `Value::If(cond, then, else)` or `Value::Loop`
with a guard condition of shape

```
Call(BoolAnd, [bound_check, ...])    →  bound proves index in `then`
Call(BoolOr,  [neg_bound_check, ...]) →  bound proves index in remaining `||` operands
```

extract the bound-check shape:

- `Call(LessInt, [Var(i), <len>])`  →  `i < len(s)` proves `s[i]`
- `Call(LessEqInt, [PlusInt(Var(i), Int(k)), <len>])`  →  `i + k <= len(s)` proves `s[i]`, `s[i+1]`, …, `s[i+k-1]`
- `Call(GreaterEqInt, [Var(i), <len>])` (as left side of `||`) →  same, negated

Add `guarded_indices: HashMap<u16, BoundProof>` to `WarnCtx`.
Push entries when entering a guarded branch; pop on exit
(RAII / scope-guard pattern, same shape as the existing
`iter_vars` tracking).

In `is_easy_proof`, when an index is `Var(v)` and `v` is in
`guarded_indices` with a bound that covers the offset being
indexed, return true.

**Files touched:** `src/parser/operators.rs` (`is_easy_proof` +
`walk_for_warnings` extensions, ~120 lines).

**Test target:** `tools/indexer/src/scan.loft` warning count
drops from 30+ to <5 (the legitimately-unguarded sites, if
any).  Add `tests/warnings_quality.rs` with a reproducer per
short-circuit shape.

## § W2 design — `#null_safe` param annotation

Opt-in annotation on a parameter (or whole function) declaring
"I accept nullable input and produce a defined result."  Same
parse machinery as `#pure` / `#impure(category)`.

Syntax:

```loft
pub fn is_digit_leaf(c: #null_safe character) -> boolean {
  c >= '0' && c <= '9'   // null '\0' fails the comparison
}

pub fn empty_or(x: text = "fallback") #null_safe { ... }   // function-wide
```

Storage: extend `Parameter` struct in `src/data.rs` with
`null_safe: bool` field; extend `Definition` with a per-fn
`null_safe_params: BitSet` (or default to false for all params).

Analyzer change: in `is_easy_proof`, when a fault-prone
expression is the argument to a `Call(def_nr, args)`, look up
`data.def(def_nr).param_null_safe(arg_index)` and skip the
warning if true.

**Files touched:** `src/parser/definitions.rs` (parse the
annotation, ~30 lines), `src/data.rs` (storage, ~20 lines),
`src/parser/operators.rs` (analyzer check, ~30 lines),
`default/*.loft` (annotate ~20-30 stdlib helpers).

**Test target:** annotating `is_digit_leaf` / `is_word_char_leaf`
in scan.loft kills the remaining `is_digit_leaf(line[j])` /
`is_word_char_leaf(line[j])` false positives.  Add an
`#[expect(warning)]` test where a non-`#null_safe` helper still
warns.

## § W3 design — entry-guard auto-inference

Walk each user function's body looking for the canonical
entry-guard pattern:

```loft
fn foo(x: T) -> U {
  if x == null { return <const> }   // ← guard
  ...uses of x...                   // ← inferred null-safe
}
```

Variants to recognize:
- `if x == null { return … }` — most common
- `if !x { return … }` — boolean shorthand (`!null` is true)
- `x = x ?? <default>;` — coalesce-and-rebind
- `if x is None { return … }` — for Option-shaped types (post-coroutines)

When the pattern matches, set `null_safe_params[param_idx] =
true` automatically (same storage as W2).  Subsequent
analyzer queries treat the param as if explicitly annotated.

**Files touched:** new module `src/parser/null_safety.rs`
(detection pass, ~100 lines), `src/parser/mod.rs` (wire into
the second pass), `src/data.rs` (already has the storage from
W2).

**Test target:** functions like

```loft
fn maybe_first(s: text) -> character {
  if s == "" { return '\0' }
  s[0]
}
```

stop warning at call sites without any user annotation.

## § W4 design — body-classification auto-inference

The general case: a parameter is null-safe if EVERY operation
that uses it would produce a defined result on null input.

Walk the body classifying each op for null behavior:
- `==` / `!=` against a non-null operand: defined (null != X is true)
- `&&` / `||`: short-circuit, defined
- arithmetic on integer: propagates null (NOT safe)
- text concatenation: propagates null (NOT safe)
- `text[i]` where `i` is null: produces null (defined but propagates)
- `len(s)` where s is null: …loft semantics here?  Check.

Per parameter, compute the join of all use sites' null
behavior.  If every use is "defined output regardless of
null input," the parameter is null-tolerant.

This is a real dataflow analysis — bounded but non-trivial.
Ship only if W3 leaves meaningful false-positive volume.

**Files touched:** extend `src/parser/null_safety.rs`
(~150-200 lines for the dataflow), `src/data.rs`.

## § W5 design — `s[i] → byte_at` peephole when consumer is ASCII-only

Independent perf optimization: rewrite `text[i]` to
`text.byte_at(i)` (and the comparison/classifier accordingly)
when:

- The result is compared against an ASCII-only `character`
  literal: `s[i] == '\n'` → `s.byte_at(i) == 10`
- The result is in an ASCII-only range: `s[i] >= 'a' && s[i] <= 'z'`
  → `s.byte_at(i) >= 97 && s.byte_at(i) <= 122`
- The result is passed to a `#null_safe` ASCII classifier:
  `is_digit_leaf(s[i])` → `is_digit_b(s.byte_at(i))` (with a
  parallel byte-variant helper auto-generated or hand-written)

Codegen pass at the IR level — pattern match before bytecode
emit / native emit.

**Files touched:** new pass in `src/parser/operators.rs` or
`src/compile.rs` (~100-150 lines), parallel byte-variant
helpers in `default/*.loft` (or auto-generated wrappers).

**Verification:** scan.loft hot loop (`scan_line` /
`scan_link_line`) keeps its current byte-level perf even
after reverting the explicit `byte_at` calls back to `s[i]`
in source.

## Open design questions

1. **Param-level vs function-level `#null_safe`** — W2 sketches
   both syntaxes (`fn foo(x: #null_safe T)` and
   `fn foo(x: T) #null_safe`).  Function-level is shorter for
   the common "every param" case; param-level is more precise.
   Decide which is canonical (probably support both, fn-level
   = "all params").
2. **What does `null` mean for `text`?** — loft's `text` uses
   an internal null pointer for null state.  `len(null) → ?`,
   `(null) == ""  → ?`, `null[0] → ?`.  W4 needs concrete
   answers per type.  Probably documented somewhere; verify
   before implementing W4.
3. **Inference visibility** — should `make doc` / IDE tooltips
   surface "inferred null-safe" for users to see?  Helps debug
   why a warning fires (or doesn't).  Probably yes; cheap to
   add once the data exists.
4. **W5 byte-variant generation** — auto-generate `is_digit_b`
   from `is_digit` based on the body, or require hand-written
   parallel helpers?  Auto-generation is cleaner but adds
   codegen complexity.  Start with hand-written + escape hatch
   for opt-in auto-generation.

## Cross-arc dependencies

- **W2 depends on W1 conceptually** — both extend
  `is_easy_proof` with new skip patterns.  Land W1 first so
  the test infrastructure is in place when W2 lands.
- **W3 + W4 depend on W2's storage** — both populate
  `null_safe_params` automatically.
- **W5 depends on W2 conceptually** — W5's "consumer is
  ASCII-only" check looks up `#null_safe` on the consumer's
  param to know whether the rewrite is sound.
- **Cooperates with @PLN42 (tracker indexer)** — scan.loft is
  the canonical proving ground; its warning count is the
  acceptance metric.

## See also

- [`doc/claude/QUALITY.md`](../../QUALITY.md) — broader
  programmer-quality follow-ups; warning quality is one
  category.
- [`doc/claude/PROBLEMS.md`](../../PROBLEMS.md) — file the
  false-positive shapes that this plan addresses as P-issues
  when encountered in real code.
- `src/parser/operators.rs:1568` `is_easy_proof` — current
  three skip patterns; W1 + W2 extend this function.
- `src/parser/definitions.rs:1020` `#impure` parser — W2's
  `#null_safe` annotation reuses this dispatch table.
- [@PLN42 § scan.loft](../42-tracker-index) — the
  consumer that surfaced the false-positive volume that
  triggered this plan.
