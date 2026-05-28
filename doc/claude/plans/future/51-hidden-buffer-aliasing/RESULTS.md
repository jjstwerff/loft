# Stage A — Probe Results (33 probes, two runs 2026-05-28)

## Full result matrix

| # | Probe | Shape one-liner | `--interpret` | `--native` | Cluster |
|---|---|---|---|---|---|
| 01 | canonical-immediate | `cv = call(); cv` | ✅ | ✅ | OK |
| 02 | double-set | `cv = a(); cv = b(); cv` | ⚠️ LEAK ×6 | ✅ | II |
| 03 | intervening-stmt | `cv = a(); _stmt; cv` | ⚠️ LEAK ×6 | ✅ | II |
| 04 | mixed-lit-call | `cv = {…}; cv = call(); cv` | ❌ CORRUPT iter 2 | ✅ | III |
| 05 | nested-call | `cv = outer(inner()); cv` | ✅ | ✅ | OK |
| 06 | interleaved-two-fns | 140 oracle | ✅ | ✅ | OK |
| 07 | explicit-return | `cv = call(); return cv;` | ⚠️ LEAK ×6 | ✅ | II |
| 08 | if-tail | `if c { a() } else { b() }` (tail) | 💥 PANIC | 💥 PANIC | IV |
| 09 | vector-return | `v = []; for…; v` (vector hidden buf) | ✅ | ✅ | OK |
| 10 | loop-mutate | `cv = {}; for { cv.f += … }; cv` (alloc_canvas shape) | ✅ | ✅ | OK |
| 11 | conditional-reassign | `cv = a(); if c { cv = b() }; cv` | ⚠️ LEAK ×6 | ✅ | II |
| 12 | deep-slice-return | `return m.items[0]` | ✅ | ✅ | OK |
| 13 | recursive-fn | `fn f(n) { if n==0 { call() } else { f(n-1) } }` | 💥 PANIC | 💥 PANIC | IV |
| 14 | field-set | `cv = {}; cv.field = call(); cv` | ✅ | ✅ | OK |
| 15 | mut-ref-arg | `fn f(&out: T) { out.field = …; }` | ✅ | ✅ | OK |
| 16 | direct-return-call | `call(p)` as tail (no Set) | ✅ | ✅ | OK |
| 17 | chained-calls | `a = call1(); b = call2(a); b` (multi-local) | ✅ | ✅ | OK |
| 18 | match-tail | `match x { … => call_a(), … => call_b() }` (tail) | ✅ | ✅ | OK |
| 19 | wrap-in-struct | `Wrapper { canvas: call(), … }` | ✅ | ✅ | OK |
| 20 | method-call | `base.method()` returning hidden buffer | ✅ | ✅ | OK |
| 21 | many-iters | 100-iter version of probe 02 | ⚠️ LEAK ×100 | ✅ | II (scales linear) |
| 22 | if-simple-tail | shape-test of probe 08 | 💥 PANIC | 💥 PANIC | IV |
| 23 | if-one-branch-call | only ONE branch is heap call | ✅ | ✅ | OK |
| 24 | deep-slice-with-call | deep-slice + interleaved heap call | ✅ | ✅ | OK |
| 25 | cond-always | `if true { cv = … }` (cond fires) | ⚠️ LEAK ×6 | ✅ | II |
| 26 | cond-never | `if false { cv = … }` (cond doesn't fire) | ⚠️ LEAK ×6 | ✅ | II |
| 27 | if-as-local | `x = if c { a() } else { b() }; x` | 💥 PANIC | 💥 PANIC | IV |
| 28 | only-conditional-set | initial Set + conditional reassign | ❌ CORRUPT iter 2 | ✅ | III (interp-only) |
| 29 | tuple-return | `fn f() -> (Canvas, Canvas)` | ✅ | 💥 PANIC (rs:775) | V (native-only) |
| 30 | lambda-return | lambda body returning heap | ❌ CORRUPT stack frame | 💥 PANIC (rs:2264) | V (both fail) |
| 31 | operator-return | `OpAdd(a: Canvas, b: Canvas) -> Canvas` | (parse: op not registered) | (same) | excluded |
| 32 | vec-of-canvases | `v = [call1(), call2(), call3()]` | ✅ | ✅ | OK |
| 33 | if-with-explicit-returns | `if c { return a(); } else { return b(); }` | 💥 PANIC | 💥 PANIC | IV |

**32 valid probes (1 excluded due to op syntax issue).**

### Stage A-bis: edge-case probes (34-37) from cluster-doc investigation

The detailed cluster-doc investigation raised follow-up questions; probes 34-37 target them:

| # | Probe | Question | Result | Refines understanding |
|---|---|---|---|---|
| 34 | if-same-call-both | Does panic depend on arg distinctness? | 💥 PANIC | NO — even identical calls in both branches panic. Confirms it's the UNIFICATION machinery, not arg-shape. |
| 35 | match-two-arms | Does 2-arm match unify like 2-branch if? | ✅ PASS | NO — match with 2 arms works like 3 arms (per-arm hidden buffer). Match doesn't engage unification. |
| 36 | three-sets | Does leak scale with Set count? | ⚠️ LEAK ×6 | NO — three Sets leak SAME count as two Sets (6 per 6 iters).  The leak is **per-iter**, not per-Set.  Refines mechanism: only ONE Canvas-record-overwrite per iter orphans, regardless of how many intermediate Sets. |
| 37 | nested-if-tail | Does else-if chain panic like 2-branch if? | 💥 PANIC | YES — else-if chains engage the same unification path and panic identically. |

Insights for Stage B:
- Cluster IV's mechanism is **unification-machinery-related**, not arg/value-related (probe 34).
- Cluster II's leak is **per-iter (per outer call), not per intermediate Set** (probe 36).  The "orphan" is the FIRST intermediate record's child store; subsequent intermediates reuse / overwrite in place.
- Match's lowering is fundamentally different from if's (probes 18, 35 pass; 08, 22, 27, 33, 34, 37 panic).  Match takes a non-unification path that handles N arms with N hidden buffers cleanly.

## Cluster summary

| Cluster | Count | Severity | Affected backends |
|---|---|---|---|
| **OK** | 16 | — | — |
| **II — latent leak** | 7 | Per-iter Canvas leak; linear scaling confirmed at 100 iters | `--interpret` only |
| **III — corruption** | 2 | Silent wrong-value reads (probe 04: iter-1 data; probe 28: default-init bypassing cond Set) | `--interpret` only |
| **IV — codegen panic** | 5 | Hard panic at `src/state/codegen.rs:2529:9`; halts compilation | **BOTH** backends |
| **V — native-only failure** | 2 | Native produces invalid Rust (`/tmp/loft_native_*.rs:775`) or runtime panic in generated Rust | `--native` (interpret may also corrupt) |

## Detailed cluster analysis

### Cluster IV — Codegen panic (BOTH backends, 5 probes)

**Probes:** 08, 13, 22, 27, 33

**Shape signature:** A function body has two heap-returning code paths that converge — either as if-expression tail, recursion base case + recursive call, or explicit `return` from each branch.  More precisely:

- `fn f() -> T { if c { call_a() } else { call_b() } }`  (tail-form)
- `fn f() -> T { x = if c { call_a() } else { call_b() }; x }`  (let-form)
- `fn f() -> T { if c { return call_a(); } else { return call_b(); } }`  (explicit-return form)
- `fn f(n) -> T { if n == 0 { call_a() } else { f(n-1) } }`  (recursion)

**Panic message:** `Incorrect var __ref_N[65535] versus <num> on n_<fn_name>` at `src/state/codegen.rs:2529:9`.  `[65535]` is `u16::MAX` (null var sentinel); something expected a real var-nr but found null.

**Negative case (probe 23):** if ONE branch is a heap call and the OTHER is a non-allocating value (a local), it works.  So the panic specifically requires BOTH branches to be heap-returning.

**Positive case (probe 18):** match arms with the same shape (`match x { … => call_a, … => call_b }`) PASS.  Match lowering apparently engages `unify_if_branches_work_refs` correctly, but raw if-tail / explicit-return-in-if do not.

**Hypothesis:** `unify_if_branches_work_refs` at `src/parser/control.rs:721` runs only in `block_result` for tail-position If, and only when the If is the IMMEDIATE last operator.  When the if-expression is assigned to a local (probe 27) or wrapped in explicit Return statements (probe 33), the unification doesn't run, leaving each branch with its own work-ref that downstream codegen can't reconcile.

**Severity:** Worst class — hard panic halts compilation entirely.  Affects both backends → it's a pre-codegen issue (parser / scope-analysis), not interpret-specific.

### Cluster III — Corruption (interpret-only, 2 probes)

**Probe 04** (mixed-lit-call):

```loft
fn render_lit_then_call(p: P) -> Canvas {
  cv = Canvas { data: [], w: 1 };   // struct literal
  cv = alloc_canvas(4, 5, p.tag);   // S1-substituted call
  cv
}
```

iter 2 reads `cv_c.data[0] = 1` (iter-1 value).  Store trace shows `+alloc #9; -free #9` immediately before crash — a transient slot is opened and closed mid-iter, suggesting the struct-literal codegen path differs from the Set-Reference path that S1+S2 cover.

**Probe 28** (only-conditional-set):

```loft
fn render(p: P) -> Canvas {
  cv: Canvas = Canvas { data: [], w: 0 };  // declared, default-init
  if p.tag > 0 {
    cv = alloc_canvas(4, 5, p.tag);
  }
  cv
}
```

iter 2: tag=2, condition `p.tag > 0` IS true, should run `cv = alloc_canvas(4, 5, 2)` → expect `cv.w == 4`.  Actually got `cv.w == 0` — the default-init value.  **The conditional Set didn't take effect** despite the condition being true.  This is corruption of control flow, not just data — the if-branch executed (it must have, given the alloc trace pattern) but the result didn't propagate to cv's slot.

### Cluster II — Latent leak (interpret-only, 7 probes)

**Probes:** 02, 03, 07, 11, 21, 25, 26

**Common shape:** Function body has at least one `Set(cv, …)` (where cv is the ref_return-promoted hidden buffer) that is NOT the immediate-penultimate-Set-followed-by-Var(cv) pattern S1 matches.

**Subcases:**

- **02 — double Set:** `cv = a(); cv = b(); cv`.  First Set isn't S1-substituted.
- **03 — intervening stmt:** `cv = a(); _stmt; cv`.  S1 doesn't fire because penultimate isn't a Set.
- **07 — explicit return:** `cv = call(); return cv;`.  **NEW INSIGHT:** S1's `tail_var` does unwrap Return, but apparently ref_return doesn't run on the body that ends with a Return statement (parse_return takes its own path).  So `ls` is empty when S1 is called from block_result, and S1 bails on the `ls.is_empty()` precondition.  This is a previously-unknown gap.
- **11 — conditional reassign:** `cv = a(); if c { cv = b() }; cv`.  Penultimate is the if, not the Set.
- **21 — many iters:** confirms LINEAR scaling — 100 iters leak 100 Canvases.
- **25 — cond always-true:** same as 11 with always-true condition.  Confirms leak per iter.
- **26 — cond always-false:** **NEW INSIGHT.**  The conditional Set NEVER FIRES at runtime, yet the leak still happens at Canvas×6.  This means the leak is from the **codegen pattern**, not the runtime control flow.  Even unreachable conditional Sets affect the leak.

**Severity:** Slow leak under repeated calls.  Linear scaling (1 Canvas per iter).  The program-exit gate catches this; dryopea-style render loops would accumulate one full-screen Canvas per frame.  Not silent corruption.

### Cluster V — Native-only failure (2 probes)

**Probe 29** (tuple-return):

`fn split(p: P) -> (Canvas, Canvas) { … (a, b) }`.  Interpret PASSES.  Native panics in the generated Rust at `/tmp/loft_native_*.rs:775:14`.  Native codegen for tuple-of-heap-structs is broken; interpret handles it.

**Probe 30** (lambda-return):

`make_renderer = fn(p: P) -> Canvas { … }`.  Interpret CORRUPTS — the iter loop variable `i` reads as `65535` (u16::MAX) after the lambda call, meaning the lambda's frame destruction clobbered main's stack frame.  Native panics in the generated Rust at `/tmp/loft_native_*.rs:2264:89`.

**Severity:** Mixed.  Probe 29 is "interpret-works native-broken" — opposite asymmetry from the rest of this class.  Probe 30 corrupts the stack frame on interpret (worst kind of corruption) AND breaks native codegen.

### Cluster OK — clean on both backends (16 probes)

Worth noting WHAT works to bound the fix surface:

- **All single-Set + heap-return shapes work** (01, 05, 09, 10, 12, 14, 15, 16, 19, 20).
- **The 140 oracle works** (06) — S1+S2 closes the @P377 canonical shape.
- **Multi-local chained calls work** (17) — intermediate locals don't trip the bug; only re-assigning the hidden buffer does.
- **Match arms work** (18) — even though if-tail (08) panics with identical shape.
- **Deep-slice borrows work in isolation** (12) AND with interleaved heap calls (24).
- **Mutable-reference args work** (15) — the EXPLICIT version of buffer-passing is correct.
- **Vector of heap calls works** (32) — collection-literal field-init handles each element correctly.

## Verified mechanism findings

After two runs and 32 valid probes:

1. **S1 only fires for the IMMEDIATE penultimate `Set(cv, Call(…))` followed by `Var(cv)`.**  Any deviation (multi-Set, intervening stmt, explicit return, conditional) defeats S1 → pre-Set OpFreeRef fires → caller's buffer is freed mid-call.
2. **S1 doesn't fire on explicit-return bodies at all** — probably because parse_return takes a code path that doesn't engage block_result's ref_return → S1 chain.  Probe 07 leaks identically to probe 02.
3. **Slot recycling is the ONLY thing protecting the canonical leak shape from corrupting** — when first-fit recycles immediately, slot_nr is preserved.  When something else allocates between free and recycle (interleaved calls, deep nested allocs), the caller's DbRef becomes stale.
4. **Class II leak is CODEGEN-PATTERN-DEPENDENT, not runtime-control-flow-dependent** (probe 26 evidence).  Even a never-firing conditional Set causes the leak.  The codegen for the conditional Set, even though dead-code-eliminated at runtime, perturbs the buffer-protocol invariant during codegen of surrounding statements.
5. **The codegen panic class (IV) affects BOTH backends** — it's a parser / scope-analysis bug, not interpret-specific.  Probably an unfinished arm of `unify_if_branches_work_refs` (@P236).
6. **Match works, if-tail panics** — same logical shape, different IR-emission path.  Match lowering apparently engages a different code path than parse_if for tail-position heap returns.
7. **The DEEP-SLICE BORROW shape (probe 12) works in isolation** — surprising given it was cited as the underlying mechanism for @P377.  The corruption-prone shape is the WRITE side (hidden-buffer-into-call), not the READ side (slice-borrow).

## Open questions for Stage B (MECHANISM.md)

1. **What exact opcode produces the leak in probe 02?**  Function-level bytecode trace of `render_double` will pin which Set's pre-Set FreeRef orphans which store.
2. **Why does probe 26 leak when the conditional never fires?**  Look at the bytecode emitted around the dead conditional.  Is there a static analyzer step that flags it as a hidden buffer with multiple assignment sites?
3. **What does parse_return do that defeats S1?**  Read `src/parser/control.rs:3108` parse_return.  Probably the body's tail type sees Void (because the body ends with a Return statement, not an expression), so block_result's ref_return arms don't fire.
4. **What is `src/state/codegen.rs:2529` asserting?**  Find the assertion and the value-of-65535 condition.  This is the Cluster IV root.
5. **Why does match (18) succeed where if-tail (08) panics?**  Trace both through `unify_if_branches_work_refs`; identify the arm match takes vs. the arm if doesn't.
6. **What native codegen path handles tuple-of-heap-structs (probe 29)?**  Read `src/generation/` for tuple-return.
7. **What does the lambda codegen do (probe 30)?**  The stack corruption suggests fn-ref dispatch / closure frame setup has a buffer-passing bug.

## What this means for the fix design (Stage C)

The class isn't ONE bug, it's a **family of buffer-protocol weaknesses** that S1+S2 only addressed for the canonical shape.  Two architectural reads:

**(A) Incremental "shape-by-shape" extension.**  For each of Clusters II, III, IV, V, design a targeted fix:

- II: extend S1's substitution to cover multi-Set, intervening-stmt, explicit-return, conditional shapes.  Each is a precondition relaxation.  Effort: ~M per shape, ~M+ total.
- III: struct-literal codegen audit + match Class II's fix surface.  Effort: M.
- IV: finish `unify_if_branches_work_refs` to cover if-as-local and explicit-return-in-if shapes.  Effort: M.
- V: native codegen for tuple-return and lambda-return.  Effort: separate per shape; native is its own pipeline.

Pro: each fix is targeted; cumulative gains.  Con: high cumulative complexity; risk of inter-class regressions (S2 already showed how a single gate can shift the trade-off between shapes); the underlying invariant (slot-recycling-determinism) still loadbearing.

**(B) Path C — store refcount.**  Per `project_drop_store_refcount.md`, replace the manual-free model entirely.  Eliminates Cluster II + III at the source.  Cluster IV remains (parse-level issue, separate fix).  Cluster V remains (native codegen, separate fix).

Pro: principled; one-time cost retires multiple bug classes.  Con: L effort (1-2 weeks); touches the most load-bearing data structure in the runtime.

**(C) Hybrid.**  Land Path C for Clusters II + III + future shapes; land targeted fixes for IV and V.

Recommended in Stage C: **(C) hybrid**.  Path C is the right architectural answer for the runtime-ownership class; Cluster IV is a separate scope-analysis issue that warrants its own targeted fix.

## What we still don't know (gaps to close in Stage B)

- All 7 open questions above.
- Whether Cluster IV is just `unify_if_branches_work_refs` not running, or a deeper assertion violation.  The exact assertion at line 2529 hasn't been read yet.
- Whether Cluster III's probes 04 and 28 share a root cause or are distinct.  04 involves struct-literal-then-call; 28 involves default-init + conditional.  Different IR shapes; could be different mechanisms.
- Whether the deep-slice-borrow class (probe 12 + 24 PASS) is actually OK or just untested in the right shape.  The original @P377 history said `moros_map::map_get_hex(m: Map, ...)` shape caused corruption.  Our probe 12 doesn't take a Map; it takes a Container.  Maybe the Map's specific shape (hash<X>, etc.) matters.

These gaps inform what additional probes to add in a Stage A continuation if needed.  The current 32-probe suite is enough to inform the Stage C decision; Stage B's mechanism work would solidify it.
