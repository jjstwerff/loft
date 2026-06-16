# @PLAN59 — Phase: text fn-values dispatch zero-cost (#387, IMPLEMENTED)

**Status:** implemented. Both backends pass the full text-fn-value matrix.
**Goal:** a text-returning fn used as a first-class `fn` value works on BOTH backends
without changing the ABI of every text fn (the rejected universal-`__retbuf` path).

## The two failures (evidence-based)

All failures were in ONE place: a text fn used as a fn-VALUE. Direct calls always worked.

1. **No-intermediate-local body → SIGSEGV on `--interpret`** (native fine): a bare literal
   `{ "z" }`, a branch of literals `{ if c { "a" } else { "b" } }`, or a forwarded call
   `{ g(n) }`. These build no text local, so `text_return` promotes **0** work-buffers — but
   the fn-ref dispatch injects exactly **one**, and the bufferless callee mis-read it as its
   closure and crashed.
2. **Returning a text PARAMETER directly → type error on BOTH backends**: `{ s }` typed as
   `fn() -> text` — the param was in the return's deps, and the fn-ref signature builder
   excluded by deps, wrongly dropping it.

## The fix (targeted — does NOT touch fns that already work)

The runtime already dispatches struct/vector fn-refs adaptively (`fn_call_ref` reads the
callee's **hidden** return-buffer attrs and pushes exactly those). Text fns didn't ride that
path only because their buffer was non-hidden and statically injected. So:

- **`src/parser/control.rs` `text_return` + lambda block:** mark the promoted text work-buffer
  `hidden = true`. (A fn with no promotable local still has zero — nothing added.)
- **`src/parser/objects.rs` `fn_ref_arg_types`:** exclude buffers by `hidden`, not by
  return-deps. This keeps a genuinely-returned parameter (case 2) while still dropping the
  synthetic buffer.
- **`src/state/mod.rs` `fn_call_ref`:** the parser still injects exactly one text buffer for a
  text-returning fn-ref (it can't know the runtime target). When the *actual* callee has no
  hidden text-buffer attr (literal / forward / param-return), **pop** that one buffer so the
  frame matches — popping one STEPPED `size_ref()` span (16B under 8-byte alignment, not the
  raw 12; this was the bug in the first cut). A callee that does build into a buffer keeps it.
  Interpreter-only; native already delivers text owned.

Return delivery is uniform regardless of buffer: `generate_call_ref` leaves the result as the
return value (a 12B text DbRef) on the stack — the buffer only governs where the text is
*built*, not how it returns. That's why the pop is sufficient and no heap allocation is needed.

## Why NOT universal `__retbuf`

The first design gave every text fn a hidden `__retbuf` (like struct/vector). Rejected: it
changes the internal ABI of every text fn (broad regression surface) to fix a fn-ref-only
problem. The adaptive pop touches only the fn-ref dispatch path; direct calls and fns never
used as values are untouched.

## Verification

`tests/scripts/387-text-fn-ref.loft` (literal/format/local/struct/vector/lambda fn-values, both
backends), `22c-par-sources.loft` (the #273 literal-par guard), the issues.rs text-return family
(#120/#306/#329/#330/#355/#377/P205/P227/P321e/P330), and a hand-computed fn-value matrix
(literal/format/local/branch/forward/param-return/vec-of-fn-refs/capturing-lambda/loop/par) —
all green on interp + native.
