<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cold-start handoff — @PLN85 over-free: the `match_return` site

> Read this cold and you can continue. Detail/history: [ownership-analysis-gaps.md](ownership-analysis-gaps.md);
> repros + diffs: [bytecode-comparisons/match_return-emit/](bytecode-comparisons/match_return-emit/).

## Orientation (one paragraph)

@PLN85 retires loft's **over-free** store-lifetime bug class by UNIFYING the per-site ownership
re-derivation onto ONE analysis fact (`src/use_analysis.rs::ownership_of`, the `Owned|Borrowed|Join{base}`
oracle) that every free chokepoint READS. The over-free fixes are gated behind the `LOFT_JOIN_OWN` env var
(off-default → suite byte-identical); `local_source` and `elem_accumulate` are DONE on both backends.
`match_return` is the LAST over-free site. Validating it surfaced **two plain-loft codegen bugs** —
interp-only, flag-OFF, independent of the gate and of the over-free work — that made *every*
match-return-into-a-buffer shape crash the interpreter, masking the synthesis. **Both are now FIXED and
ungated: P2 (`2701ad5f`) and P3 (`166cc578`).** What remains is the gated synthesis itself — P1 below.

## Branch / gate / commits

- **Branch `tuxedo-pln85-fuzz-proof-gate`** (latest `166cc578`). STACKED on @PLN25's unmerged PR —
  do NOT fork a new branch off `main` (main lacks the foundation). Rebase the stack only after @PLN25 merges.
- **Gate:** `LOFT_JOIN_OWN` controls the OVER-FREE fixes only. The P2/P3 codegen fixes are NOT gated — they
  are plain-loft correctness fixes that run always (suite green: issues 746, use_analysis 11, wrap, native).
- DONE both backends, gated: `local_source` (`src/scopes.rs`), `elem_accumulate` (`state/io.rs`,
  `state/codegen.rs`, `generation/dispatch.rs`). Oracle built + unit-tested (`tests/use_analysis.rs`).
- DONE both backends, ungated codegen: **P2** + **P3** (see "FIXED" below).

## The match_return synthesis — what it is

The synthesis lives in **`src/parser/control.rs::jo_copy_borrowed_arm_yield`** (called from `parse_match`
after each arm body, ~line 1947). When an arm yields a `skip_free` vector field binding directly
(`Filled { items } => { items }`), it wraps the yield in an owned copy and lets the existing `ref_return`
promotion build the buffer ABI; the structure matches the proven `deliver3`. The analysis is SUFFICIENT
(do not enhance it): the oracle classifies `deliver` (materialised) and `deliver3` IDENTICALLY as
`return=Join(base=buffer)`.

## REMAINING WORK

### P1 — re-validate the gated synthesis now that P2/P3 are fixed *(the open @PLN85 deliverable)*

P2 and P3 made every match-return-buffer shape crash the interpreter regardless of the synthesis, so the
synthesis could never be validated. With both fixed, re-run the `LOFT_JOIN_OWN=on` path:

- The synthesis emits the whole-vector append `o += items`. The earlier prediction was "if the plain-loft
  bug is fixed, the whole-append synthesis just works." Test it now on both backends under churn. If it runs
  clean, P1 needs **no** element-loop — the simpler whole-append synthesis is enough.
- The borrowed-direct-yield shape still crashes flag-OFF: `directyield`
  (`fn f(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }`) returns a
  *borrowed* enum-field binding directly. That is the synthesis INPUT shape the gate rewrites into an owned
  copy — re-check it under `LOFT_JOIN_OWN=on` once the whole-append path is confirmed.
- Fallback element-loop form (proven clean both backends, 4/4 under churn):
  `bytecode-comparisons/match_return-emit/PROVEN-CLEAN-element-loop-inline-default.loft` —
  `Filled { items } => { o: vector<E> = []; for x in 0..len(items) { o += [items[x] ?? E{<field defaults>}]; } o }`.
  The OWNED inline default forces the deep `OpCopyRecord`; a borrowed default stays shallow and crashes. To
  emit it, hand-build the `Iter`/`Loop`/`OpNewRecord`/`OpCopyRecord`/default IR — the parser's
  `parse_vector_for`/`build_comprehension_code` consume the lexer, so they cannot be called directly.
- Cleanup once the synthesis lands: the `ref_return` whole-append materialise block (`control.rs`, the
  `jo_arm_skip` loop ~line 5135 + the promotion-loop/dep-walk skips) is VESTIGIAL.

## FIXED

### P2 — gen_if arm-join discard *(FIXED `2701ad5f`)*

First filed as "non-deterministic corruption in match-arm whole-append." Both halves were wrong. The real
bug is **deterministic**, in `src/state/codegen.rs::gen_if`'s arm-join (the "B5" branch). When a
value-yielding `match`'s arms exit at different stack levels, gen_if joins at the shorter arm's level but
only ever shrank the FALSE arm — a taller TRUE arm was never shrunk (short a whole result slot), and the
shrink used the raw result size instead of the step-rounded slot (short by the 12→16 padding). Either way
the function-tail `OpReturn` `discard` came out short, so `fn_return` (`state/mod.rs`) read the saved return
address from the wrong slot and the interpreter derailed under churn. `--native` is immune — it returns on
the Rust call stack. Fix: shrink whichever arm is taller, `discard = (arm_stack - target) + step(ret_size)`,
routing an already-emitted taller true arm through a shrink trampoline. The append itself (`vector_add`) is
shared by both backends and was never the bug. Regression: `tests/scripts/441-match-return-buffer-stack.loft`.

### P3 — empty value-block pushes no result *(FIXED `166cc578`)*

First filed as "defining two vector-using functions together crashes." Wrong root. The real bug: a
**multi-line** `_ => { [] }` arm returning into the buffer reduces to a block whose only operator is a bare
`Line` marker, yet it is typed to yield the vector. `generate_block` set `stack.position = after` (claiming
the result slot) without emitting any push, so when that empty arm was TAKEN at runtime (`deliver(Empty)`)
the eval stack was one slot short → the same `fn_return` underflow as P2, from a different site. The
single-line `_ => { [] }` becomes a Null else that gen_if already pads — that is why ONLY the multi-line
spelling crashed (formatting changed the parse). Fix: `generate_block` pushes a typed null of the result
when a value-typed block's operators leave nothing on the eval stack, mirroring gen_if's null-else arm.
Regression: `tests/scripts/442-match-empty-arm-into-buffer.loft`.

## VALIDATION — how to test

- The DEFAULT backend is `--native`; running `loft prog.loft` without `--interpret` tests the wrong side
  (this masked the whole bug across early reads). Always pass `--interpret` for the interp story.
- The interp crash is **deterministic in a DEBUG build** (overflow-checked panic at the `fn_return`
  subtract); a RELEASE build wraps the underflow into nondeterministic heap corruption. Debug is the
  reliable signal.
- Build CLEAN repros (just the match fn + `main`); churn pressure inline
  (`j: vector<E> = []; for q in 0..12 { j += [...] }`). Assert value AND length AND leak (`LOFT_STORES=warn`,
  `LOFT_NATIVE_LEAK_CHECK=1`) on BOTH `--interpret` and `--native`.
- Regression: the non-match over-free shapes (`field_return`, `field_local`, `nested_field`,
  `elem_accumulate`, `local_source`) must stay clean with the gate ON.

## Method (this stream's spine)

- **The default backend is `--native`.** Testing the interp needs `--interpret`; forgetting it shows a clean
  run while the real (interp) bug sits untouched.
- **Isolate the ONE variable before naming the fix.** P2's filed diagnosis ("non-deterministic
  whole-append") and P3's ("two functions") were BOTH wrong — one-variable probes corrected each.
- **Capture-and-diff the answer**: the proven sibling (`deliver3`, the plain `copyf`) is the spec.
- Lessons: `.claude/skills/design-protocol`, `doc/claude/CODEGEN_METHOD.md`.

## Disproven hypotheses — DO NOT re-chase (each killed by an isolation probe)

1. "Non-deterministic UAF" — deterministic; the release-build `u32` underflow wrap made it look random.
2. "Whole-vector append corruption" — `vector_add` is shared by both backends and is clean; the bug was the
   surrounding frame `discard`.
3. "Deep-copy of heap/text fields in `vector_add`" — a heap-free struct crashes identically.
4. "Two functions defined together" (P3) — it is the multi-line empty arm taken at runtime; one function,
   one `deliver(Empty)` call suffices.
5. "POISON crash" — instrument artifact.
