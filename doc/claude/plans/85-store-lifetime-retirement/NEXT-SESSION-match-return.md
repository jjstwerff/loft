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
oracle) that every free chokepoint READS. Three over-free shapes; all fixes are gated behind the
`LOFT_JOIN_OWN` env var (off-default → suite byte-identical). **`local_source` and `elem_accumulate`
are DONE on both backends.** `match_return` is the LAST site: the codegen STRUCTURE is correct (it now
produces the proven `deliver3` IR exactly), but the COPY mechanism it emits is broken. This handoff is
how to finish it.

## Branch / gate / commits

- **Branch `tuxedo-pln85-fuzz-proof-gate`** (latest `c748e98b`). It is STACKED on @PLN25's unmerged PR —
  do NOT fork a new branch off `main` (main lacks the foundation). Rebase the stack only after @PLN25 merges.
- **Gate:** `LOFT_JOIN_OWN` env var. OFF → no over-free code runs → suite byte-identical (verify:
  `cargo test --release --test issues` = 746 ok, `--test use_analysis` = 11 ok). ON → the fixes activate.
- DONE both backends (gated): `local_source` (scope-pass dep-strip, `src/scopes.rs`), `elem_accumulate`
  (first-bind `OpBindOrCopy` interp + native inline guard; `state/io.rs`, `state/codegen.rs`,
  `generation/dispatch.rs`). The oracle (`use_analysis.rs`) is built + unit-tested (`tests/use_analysis.rs`).

## The match_return state — what is CORRECT, what is BROKEN

The synthesis lives in **`src/parser/control.rs::jo_copy_borrowed_arm_yield`** (called from `parse_match`
after each arm body, ~line 1947). When an arm yields a `skip_free` vector field binding directly
(`Filled { items } => { items }`), it wraps the yield in an owned copy and lets the existing `ref_return`
promotion build the buffer ABI. **The STRUCTURE it produces is EXACTLY `deliver3`** (verified: sig `["??"]`,
var table `0 e / 1 __retbuf marker / 2 _mv_items_1["e"] source / 3 arg <copy> owned buffer`, caller types
`r:vector["__ref_1"]` and emits `OpFreeRef(cell)`). The ONLY thing wrong: it emits the whole-vector append
`o += items`, which is broken (see P1/P2).

The analysis is SUFFICIENT (do not enhance it): the oracle classifies `deliver` (materialised) and `deliver3`
IDENTICALLY as `return=Join(base=buffer)`. The bug is purely the emitted copy.

## REMAINING PROBLEMS

### P1 — emit the element-loop instead of the whole-vector append *(the path to green)*

`jo_copy_borrowed_arm_yield` must emit, NOT `o += items`, but the proven-clean element-loop:
```
Filled { items } => { o: vector<E> = []; for x in 0..len(items) { o += [items[x] ?? E{<field defaults>}]; } o }
```
- **Proven clean both backends, 4/4 under churn**: `match_return-emit/PROVEN-CLEAN-element-loop-inline-default.loft`.
- Why this exact form (each established by an isolation probe): the OWNED-typed element (the inline default
  `E{<field defaults>}`) forces the deep `OpCopyRecord` path; a BORROWED default (`?? items[0]`) or none
  stays a shallow ref and crashes; an INLINE default (vs a helper fn) dodges P3.
- **Work:** hand-build the `Iter`/`Loop`/`OpNewRecord`/`OpCopyRecord` + default-construction IR. The
  parser's `parse_vector_for`/`build_comprehension_code` (vectors.rs) consume the LEXER, so they cannot be
  called directly — build the IR with `v_block`/`v_if`/`Value::Iter`/`Value::Loop`/`self.cl(...)`, or drive
  a synthetic re-parse. Model it on `deliver3`'s IR (`loft introspect` a `deliver3`-style fn). Substantial
  but well-specified. The inline default is constructible from the element struct's field defaults
  (`Definition` parts carry `default: ...`).

### P2 — PRE-EXISTING: non-deterministic corruption in match-arm whole-append *(RECOMMENDED to fix; unblocks P1)*

`match e { Filled { items } => { o: vector<E> = []; o += items; o }, _ => { [] } }` + churn crashes interp
**NON-deterministically** (CRASH / clean / CRASH across identical runs — the uninitialised-mem / UAF
signature); native clean. **Plain loft, flag-OFF — independent of @PLN85.**
- Repro: `match_return-emit/PREEXISTING-whole-append-in-match-nondeterministic.loft`.
  Control (CLEAN, same append in a PLAIN fn): `CONTROL-whole-append-in-plain-fn-clean.loft`.
- Isolations (don't re-chase): heap-free `struct E { hp: integer }` crashes IDENTICALLY → NOT a deep-copy
  of heap fields; the SAME append in a plain fn is clean → it is the MATCH-ARM context; the element-loop is
  clean → it is the WHOLE-vector append specifically (source is the `vector<ref(E)>` binding).
- **If P2 is fixed, P1 is unneeded** — the existing whole-append synthesis just works (smaller, more general
  fix). Debug: boundary matrix on whole-append-in-match-arm + `LOFT_LOG=minimal`/`crash_tail` to find the
  uninitialised/UAF op. **File as a GitHub issue** (`sev:`/`area:`, both-backend repro).

### P3 — PRE-EXISTING: defining two vector-using functions together crashes interp

Defining `e_default` AND `filler` together (BOTH dead code, never called) crashes interp **flag-OFF**;
either one alone is clean. A def/type-numbering corruption. **This MASKED every matrix file** (all
`/tmp/claude/gen/match_return*` carry both `e_default`+`filler`), so matrix verdicts were unreliable. **File
as a GitHub issue.** Reduce: `deliver` (a borrowed-binding match return) + `e_default` + `filler` + a `main`
that only calls `deliver`.

### Cleanup (not a bug)

The `ref_return` whole-append materialise block (`control.rs`, the `jo_arm_skip` loop ~line 5135 + the
promotion-loop/dep-walk skips) is now **VESTIGIAL** — the `parse_match` synthesis supersedes it. Remove
once the element-loop lands.

## VALIDATION — how to test (avoid the masking traps)

- **Do NOT trust the `/tmp/claude/gen/match_return*` matrix files** — they carry `e_default`+`filler` (P3)
  and crash for that reason. Build CLEAN repros with NO extra functions (just `deliver` + `main`), churn
  pressure inline (`j: vector<E> = []; for q in 0..12 { j += [...] }`).
- **Do NOT use `LOFT_POISON` for this family** — it is an ARTIFACT here (`deliver3` itself SIGSEGVs under
  POISON). Use `LOFT_STORES=warn` (leak) + an `assert(len(r)==N)` (value) + run 3–4× (non-determinism).
- Both backends: `--interpret` and `--native` (`LOFT_NATIVE_LEAK_CHECK=1`). Native is clean throughout;
  the whole story is interp.
- Regression: the non-match shapes (`field_return`, `field_local`, `nested_field`, `elem_accumulate`,
  `local_source`) must stay clean ON.

## Disproven hypotheses — DO NOT re-chase (each killed by an isolation probe)

1. "Two-pass def-numbering corruption in the synthesis" — the synthesis IS pass-consistent (instrumented).
2. "POISON crash" — instrument artifact (`deliver3` SIGSEGVs under POISON too).
3. "Deep-copy of heap/text fields in `vector_add`" — heap-free struct crashes identically.
4. "`OpAppendVector` shallow for a borrowed field source" — `copyf` (`o += b.rows`) is clean.

## Methodology (this stream's spine — recorded in skills/memory)

- **Isolate the ONE variable before naming the fix.** This bug cost FOUR wrong targets (above); each
  disproven by a one-variable probe. The lesson is in `.claude/skills/design-protocol` and
  `doc/claude/CODEGEN_METHOD.md` ("Diff against the proven sibling").
- **Capture-and-diff the answer**: the proven sibling IR (`deliver3`) is the spec; make yours byte-equal.
- The analysis is sufficient; the remaining work is purely codegen structure.

## Recommendation

Fix **P2** (the pre-existing match-arm corruption) first — likely the smaller, more general fix, it
unblocks the simpler whole-append synthesis already in `jo_copy` (no P1 loop-IR needed), and it kills a
real pre-existing bug. File P2 + P3 as issues regardless. P1 is the fallback if P2 proves deep.
