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
`match_return` is the LAST over-free site. Validating it surfaced a family of **plain-loft codegen bugs** —
interp-only, flag-OFF, independent of the gate and of the over-free work — that made match-return-into-a-
buffer shapes crash the interpreter, masking the synthesis. **P2 (`2701ad5f`) and P3 (`166cc578`) are
FIXED**, and the **gated synthesis is VALIDATED** (P1, `7ea31d8c`): with the blockers gone the owned-copy
synthesis runs clean + leak-free on both backends. One more bug in the same family remains — **P4**, two
match-return functions over DIFFERENT enum types crash the interpreter together (see REMAINING WORK).

## Branch / gate / commits

- **Branch `tuxedo-pln85-fuzz-proof-gate`** (latest `7ea31d8c`). STACKED on @PLN25's unmerged PR —
  do NOT fork a new branch off `main` (main lacks the foundation). Rebase the stack only after @PLN25 merges.
- **Gate:** `LOFT_JOIN_OWN` controls the OVER-FREE fixes only. The P2/P3 codegen fixes are NOT gated — they
  are plain-loft correctness fixes that run always (suite green: issues 746, use_analysis 13, wrap, native).
- DONE both backends, gated: `local_source` (`src/scopes.rs`), `elem_accumulate` (`state/io.rs`,
  `state/codegen.rs`, `generation/dispatch.rs`). Oracle built + unit-tested (`tests/use_analysis.rs`).
- DONE both backends, ungated codegen: **P2** + **P3** (see "FIXED" below).
- VALIDATED, gated: **P1** — the match_return synthesis (`tests/use_analysis.rs::join_own_match_return_*`).

## The match_return synthesis — what it is

The synthesis lives in **`src/parser/control.rs::jo_copy_borrowed_arm_yield`** (called from `parse_match`
after each arm body, ~line 1947). When an arm yields a `skip_free` vector field binding directly
(`Filled { items } => { items }`), it wraps the yield in an owned copy and lets the existing `ref_return`
promotion build the buffer ABI; the structure matches the proven `deliver3`. The analysis is SUFFICIENT
(do not enhance it): the oracle classifies `deliver` (materialised) and `deliver3` IDENTICALLY as
`return=Join(base=buffer)`.

## REMAINING WORK

### P4 — two match-return functions over DIFFERENT enums crash the interpreter together *(OPEN, plain-loft, gate-independent)*

A THIRD bug in the P2/P3 family. Two match-return-into-a-buffer functions over DIFFERENT enum types,
defined and called together, crash the interpreter (`realloc(): invalid next size` / the `fn_return`
derail). One call each, no churn, no loop. Deterministic on `--interpret` (6/6); `--native` clean.
Gate-independent (fails OFF and ON). Minimal repro:
```
enum Cell { Empty, Filled { items: vector<E> } }
enum Box2 { Nil,   Has    { zs: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }
fn deliverb(e: Box2) -> vector<E> { match e { Has { zs } => { zs }, _ => { [] } } }
fn main() { a = deliver(Filled{items: ci}); b = deliverb(Has{zs: zi}); /* assert len==3 */ }
```
Two match-return functions over the SAME enum are clean (`tests/scripts/441`, `442`) — the trigger is the
DIFFERENT enum types. Likely a def/type-numbering or per-enum-offset interaction, same class as P2/P3.
Isolate-and-fix with the matrix method (the DEBUG build is the deterministic signal; `--native` is the
clean sibling to diff against). File or fix in-plan per the bug-filing policy.

## DONE — P1 (the gated synthesis is validated)

With P2/P3 fixed, the `LOFT_JOIN_OWN` synthesis (`jo_copy_borrowed_arm_yield`) runs clean: the borrowed
arm yield is rewritten to an owned copy, the emitted return drops the subject borrow
(`["__retbuf", "e"]` → `["__retbuf"]`), and the result is value-correct + leak-free on BOTH backends. The
earlier prediction held: **the whole-vector append `o += items` "just works" — no element-loop synthesis
needed.** Pinned by `tests/use_analysis.rs::join_own_match_return_synthesis_both_backends` (runtime) and
`::join_own_match_return_strips_the_borrow` (the gate's structural effect).

Leftovers if the gate is ever flipped on by default:
- The `directyield` borrowed-direct-yield shape is the synthesis INPUT; the gate rewrites it. No separate
  work — it is covered by the synthesis above.
- The `ref_return` whole-append materialise block (`control.rs`, the `jo_arm_skip` loop ~line 5135 + the
  promotion-loop/dep-walk skips) is VESTIGIAL and can be removed.
- The element-loop fallback (`bytecode-comparisons/match_return-emit/PROVEN-CLEAN-element-loop-inline-default.loft`)
  is NOT needed now that the whole-append works; keep it only as a reference.

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
