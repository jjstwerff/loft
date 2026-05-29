<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 52 — Value-block borrow cleanup (text + adjacent types)

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ complete after two sweeps (24 probes total, all run under both backends 2026-05-29).  Cluster II (if/match-as-value) FALSIFIED.  Cluster IV expanded into 4 sub-clusters: IV-Ref FALSIFIED (covered by @PLAN51), but **IV-Vec / IV-Hash / IV-Enum are NEW** — heap-type value-blocks via `??` fail on BOTH backends (interpret: silent corruption; native: E0308 codegen error).  Cluster V verified via probe 24 — currently no live production exposure but real-library shapes exist. |
| B — Mechanism investigation | 🟢 Cluster I mechanism verified via bytecode trace (`LOFT_LOG=fn:main`) — `OpFreeText(_ncc_N)` runs before the outer consumer reads the if-result Str off the eval stack; the Str's `ptr` points into `_ncc_N`'s now-freed `String` heap buffer; rustc 1.96 + macOS libmalloc surface the dangling-read as 7 NUL bytes.  **Shape-only confirmation**: probe 16 shows even static-literal-source calls fail — allocation class doesn't matter, the `_ncc_N` block construction itself is the trigger.  Cluster III's mechanism: `??`-DEPENDENT (probe 18 falsifies the "format buffer alone" hypothesis); the format-buffer's `OpFormatText` op interleaves a write between the if-result push and OpFreeText, producing garbage-byte corruption instead of NUL.  Cluster IV-Vec/Hash/Enum mechanism: native-side has a missing null-check codegen path (`if <DbRef> {...}` instead of `if <DbRef>.is_some() {...}`); interpret-side has analogous heap-value dangling-handle behavior. |
| C — Fix design (OPTIONAL) | ⏸️ pending — cluster I fix shape is now uniquely determined (parent-scope text temp à la B5-L3 for is_return=false).  Cluster III likely closes incidentally with cluster I.  **Cluster IV native predicate fix LANDED 2026-05-29** (`output_test_predicate` helper in `src/generation/emit.rs`) — closes struct-Enum (probe 23) on native; Vec/Hash/Sorted/Index now compile but expose a secondary `var_a` null-sentinel init bug at runtime; Tuple unaffected (needs Tuple-specific sentinel). |
| D — Implementation | 🟡 partial — Step 2 (cluster IV predicate fix) landed 2026-05-29 closing IV-Enum on native; Steps 1, 4-7 still pending. |

**Trigger (2026-05-29):** P383 — `tests/scripts/repro_p323.loft::test_p323_index_coalesce` regressed under rustc 1.96. Bisect pinned the rustc bump (loft commit `42af45d` PR-CI was green on all 3 platforms on 2026-05-27 under rustc 1.95; rustc 1.96.0 dropped 2026-05-28). The defect itself is latent UB in loft's IR (post-consumer `OpFreeText` on a borrowed Str) that rustc 1.94/1.95 happened to mask via codegen / libmalloc behaviour that left freed bytes intact. rustc 1.96 (LLVM 21) changed that, exposing the bug deterministically on macOS. This plan is the 6th investigation cluster of the @PLAN51 hidden-buffer-aliasing family — same structural shape (borrow into a scope-local that gets freed before the consumer reads), text-flavoured, in value-block context (not return context).

**Scope:** every value-block-result emit path where the block's tail expression borrows into a scope-local that scope-exit then frees. The canonical case is `??` on text with a non-trivial LHS; sibling shapes include `if`/`match` as value-expression with branches that read text from block-locals, format-string contexts that bind into block-local work buffers, and the analogous patterns for `Reference` / `Vector` / `Hash` / `Sorted` (verify whether those already escape via @PLAN51's `paired_witness` + S1 NRVO machinery).

## Goal

Ship a scopes-pass + parser-lowering fix that closes the value-block-borrow-cleanup class on the interpreter for at least Text. Subsidiary deliverables: (i) updated `_INVESTIGATION_TEMPLATE.md`-shaped cluster docs for each verified sub-mechanism; (ii) regression-graduated probes in `tests/scripts/`; (iii) a rustc-version-aware CI lever (asan / miri or a per-OS matrix expansion) so future toolchain bumps don't re-expose the family unobserved.

## Cluster catalogue

The failure modes discovered or hypothesised during exploration. Each verified cluster gets its own `cluster-<id>-<slug>.md` once Stage B confirms it.

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I ✅ closed 2026-05-30 | `??`-on-text — non-Var LHS lowers to a value-block whose tail returns `Str` borrowing into block-local `_ncc_N` (now `__ncc_N`); scope-exit `OpFreeText` invalidated the stack Str before consumer read.  **CLOSED on interpret AND native** by iteration 2 (commit pending): parser marks `__ncc_N` skip_free, scope-pass suppresses OpFreeText, native emit forces has_trailing_void so the existing @P323 `_ret.to_string()` materialisation fires inside the block. | n/a (closed) | both backends fixed | 02, 04-06, 11, 13-17, 20, 24-26, 29-34, 37-39, 44 | [`cluster-I-ncc-text.md`](cluster-I-ncc-text.md) (Iteration 2 logged) |
| I-crash ✅ closed 2026-05-30 | Method-chain consumer (`(vec[i] ?? "x").to_uppercase()`) — SIGBUS at opcode 220.  **CLOSED 2026-05-30**: cluster I iteration 2 incidentally fixes I-crash (Set C 3/3 PASS) — the method dispatch now reads from the owned String materialised inside the block tail. | n/a (closed) | both backends fixed | 46, 49 | (bundled with cluster I) |
| II ❌ | ~~`if`/`match`-as-value with text from a block-local~~ — **FALSIFIED**: probes 07/08 PASS.  `unify_if_branches_work_refs` (parser/control.rs:728) materialises branch text into shared work-ref before the consumer reads. | n/a | n/a | 07, 08 | _hypothesis closed_ |
| III ✅ closed 2026-05-30 | Format-string `{x ?? "y"}` interpolation — same root mechanism as cluster I; **CLOSED 2026-05-30** by cluster I iteration 2 (Set D 5/5 PASS).  Format-buffer reads the owned String materialised inside the value-block tail. | n/a (closed) | both backends fixed | 09, 19, 30 | [`cluster-III-format-string.md`](cluster-III-format-string.md) |
| IV-Ref ❌ | ~~`Reference` value-blocks~~ — **FALSIFIED**: probe 10 PASSES.  @PLAN51's `paired_witness` + S1 NRVO covers the Reference value-block case. | n/a | n/a | 10 | _hypothesis closed_ |
| IV-Vec-nested-field-push 🟡 partial | `vector<vector<X>>` STRUCT FIELD `+= inner_vec` misroutes to concatenate path (parser branch order bug at `src/parser/expressions.rs:1370`).  Surfaced via probes 21/36; reclassified in-plan 2026-05-30 (was "out of scope" — overruled by the in-plan-by-default policy below).  Parser fix lands the single-element-push lowering; closes interp fully and native primitives.  Native struct-inner variants surface a SECONDARY `database/allocation.rs:1190` panic — separate fix surface inside this cluster. | BOTH backends — interp silent corruption + native silent corruption + struct-inner alloc panic | BOTH backends | 21, 36, 91, 92, 93, 94, 95, 96, 97 | [`cluster-IV-Vec-nested-field-push.md`](cluster-IV-Vec-nested-field-push.md) |
| IV-Hash ✅ closed (native) | `Hash` value-blocks via `??` — **CLOSED on native 2026-05-29** by dep-strip fix in `src/parser/expressions.rs`.  Interpret still FAILs (separate cluster I-class dangling-handle bug). | Interpret only | Native fixed | 22 | _(bundle with IV-Vec)_ |
| IV-Sorted ✅ closed (native) | `Sorted` value-blocks via `??` — closed on native by same dep-strip fix. | Interpret only | Native fixed | 41 | _(bundle with IV-Vec)_ |
| IV-Index ✅ closed (native) | `Index` value-blocks via `??` — closed on native by same dep-strip fix. | Interpret only | Native fixed | 50 | _(bundle with IV-Vec)_ |
| IV-Spacial ✅ closed | **PARSER hang** — was: `spacial<Point[name]>` caused the PARSER to enter an infinite loop in the spacial arm of `parse_type` (`src/parser/definitions.rs:1548`).  Hand-rolled scanner loop didn't advance when the next token was `[`.  **CLOSED 2026-05-29** by replacing the loop with a conditional `parse_fields` call (same helper sorted/hash/index use).  Probes 51/62/88/89 now emit the existing "spacial<T> is planned for 1.1+" diagnostic and exit cleanly; full PASS requires spacial itself to be implemented (1.1+ work, out of scope). | n/a (closed) | n/a (closed) | 51, 62, 88, 89, 90 | [`cluster-IV-Spacial-parser.md`](cluster-IV-Spacial-parser.md) (fix iteration 1 logged) |
| IV-Enum ✅ closed | struct-`Enum` value-blocks via `??` — **CLOSED on native 2026-05-29** by predicate fix in `output_test_predicate`.  Interpret still returns fallback variant (separate bug, tracked under cluster I). | Interpret only | Native fixed | 23, 38 | _(bundle with IV-Vec)_ |
| IV-Tuple ✅ closed | `Tuple` value-blocks via `??` — **CLOSED 2026-05-30** by a parser-side null-check that projects the FIRST FIELD of the tuple and converts THAT to boolean (the per-field-null-sentinel convention).  Site: `src/parser/operators.rs::build_null_coalesce_default` lines ~1227-1246.  Both backends PASS on probe 40 after the fix. | n/a (closed) | both backends fixed | 40 | (logged in this README) |
| V ✅ | Real-library extraction (probe 24) + practical multi-tier lookup chain (probe 47).  Scan of `lib/*/src/*.loft` shows **NO text-coalesce patterns in production code today** (only `vec[i] ?? 0` integer-typed); but the shape is a near-certain config-lookup pattern for upcoming `lib/server`.  **Probe 47 also surfaced cluster VII** — chained-call native E0308 — promoted to its own cluster row below. | Latent | Interpret only (cluster VII handles the native side separately) | 24, 71 | _(folded with cluster I doc)_ |
| VI ✅ closed 2026-05-30 | Closures with `??` in their body — interpret had cluster I NUL fill AND native E0308.  **CLOSED 2026-05-30** in two steps: (a) cluster I iteration 2 closes interpret (probes 45/65/67/86 interp PASS); (b) extend native's @P321e/P205 scratch-routing in `src/generation/emit.rs` Value::Return path to fire when the returned value is a `Block` containing a `__ncc_*` skip_free temp.  Closures had `__work_ret: &mut String` so the existing `no_work_buffer` gate excluded them; the new `returns_ncc_block` gate matches the cluster-I value-block signature explicitly.  Sets F 4/4 PASS both backends. | n/a (closed) | both backends fixed | 45, 65, 67, 86 | (bundled with cluster I iteration 2 — same commit) |
| VII ✅ closed (native) | Chained `call() ?? call() ?? literal` and recursive `??`-using fns — interpret silent corruption (cluster I), native was failing with `expected &String, found Str` E0308.  **CLOSED on native 2026-05-29** by text-branch unification in `src/generation/emit.rs::output_if_inner` AND `src/generation/pre_eval.rs::output_if_with_subst`: wrap each non-Block branch with `&*(...)` to coerce `&String` / `Str` / `&str` to a common `&str`.  Interpret still fails (cluster-I dangling-buffer issue, separate). | Interpret only | Native fixed | 47, 48, 82 | (logged in this README; cluster-I open for interpret side) |

**Edge-case probes (informative, not new clusters):**
- Probe 28: `null ?? "x"` literal-LHS — PASS both backends.
- Probe 35: `vec_of_bool[i] ?? false` — PASS; primitives escape (type-gate at heap types).
- Probe 42: void-statement `vec[i] ?? "x";` — PASS (no consumer = no exposure).
- Probe 43: empty-string source — PASS (length=0 unaffected).
- Probe 52: char `??` — parse error "Field access not supported on type character"; char isn't a valid `??` operand (separate parser concern).

**Severity ranking** (worst to best, for fix prioritisation):
1. **IV-Spacial hang** (probe 51) — process unresponsive, must kill manually.
2. **I-crash SIGBUS** (probes 46, 49) — process killed by OS signal.
3. **IV-V/H/Sorted/Index/Enum/Tuple native compile error** (probes 21-23, 36, 40, 41, 50) — code doesn't compile; LOUD failure.
4. **VI closure native compile error** + **VII chained-call native E0308** — similar loud-failure class.
5. **I silent corruption** (12 probes) — silent wrong data; the most insidious because it surfaces as wrong-output, not as failure.

✅ = mechanism verified by probe diff.  ❌ = hypothesis falsified.

## Probe suite

Results recorded 2026-05-29 against `main` + `macos-clippy-fixes` branch on rustc 1.96.0 (ac68faa20 2026-05-25), macOS 14 / Apple Silicon.

| File | Shape | Cluster | --interpret | --native |
|---|---|---|---|---|
| `01-simple-var-coalesce.loft` | `v ?? "default"` — simple `Var` LHS, no block | reference | PASS | PASS |
| `02-vec-index-coalesce.loft` | `h.items[0] ?? "fallback"` — P383 canonical | I | **FAIL** `'       '` | PASS |
| `03-vec-index-direct.loft` | `h.items[0]` no `??` — direct read | reference | PASS | PASS |
| `04-field-coalesce.loft` | `obj.maybe_text ?? "x"` — field-access LHS | I | **FAIL** `'       '` | PASS |
| `05-call-coalesce.loft` | `get_text() ?? "x"` — call-result LHS | I | **FAIL** `'       '` | PASS |
| `06-nested-coalesce.loft` | `(vec[i] ?? "a") ?? "b"` — chained `??` | I | **FAIL** `'       '` | PASS |
| `07-if-value-text.loft` | `if cond { vec[i] } else { "x" }` as RHS | II ❌ | PASS | PASS |
| `08-match-value-text.loft` | `match k { 1 => vec[i], _ => "x" }` | II ❌ | PASS | PASS |
| `09-coalesce-in-format.loft` | `"got: {vec[i] ?? \"x\"}"` — format-string consumer | III | **FAIL** `'got: ���-�'` | PASS |
| `10-ref-coalesce.loft` | `vec_of_ref[0] ?? Holder{...}` — Reference shape | IV-Ref ❌ | PASS | PASS |
| `11-loop-coalesce.loft` | `for i in 0..N { x = vec[i] ?? "x"; }` | I | **FAIL** `iter 0: got '  '` | PASS |
| `12-native-passes.loft` | P383 canonical — same as 02 under `--native` | reference | (n/a — native target) | PASS |
| `13-concat-coalesce.loft` | `(a + b) ?? "x"` — text concat LHS | I (border) | **FAIL** `'       '` | PASS |
| `14-method-coalesce.loft` | `s.to_uppercase() ?? "x"` — method-call LHS | I (border) | **FAIL** `'       '` | PASS |
| `15-deep-chain-coalesce.loft` | `vec[i].name ?? "x"` — deep chain | I (border) | **FAIL** `'       '` | PASS |
| `16-static-call-coalesce.loft` | `static_text() ?? "x"` — static-literal source | I (border) | **FAIL** `'       '` | PASS |
| `17-hash-coalesce.loft` | `hash[k].value ?? "x"` — hash deep chain | I (border) | **FAIL** `'       '` | PASS |
| `18-format-no-coalesce.loft` | `"got: {vec[i]}"` — format WITHOUT `??` | (boundary) | PASS | PASS |
| `19-format-multi-coalesce.loft` | `"{a ?? \"x\"} {b ?? \"y\"}"` — two `??`s in format | III (stress) | **FAIL** `garbage` | PASS |
| `20-print-direct-coalesce.loft` | `print(vec[i] ?? "x")` — direct print consumer | I (border) | **FAIL** `'       '` | PASS |
| `21-vector-coalesce.loft` | `vec_of_vecs[i] ?? other_vec` — Vector value-block | **IV-Vec-nested-field-push 🟡** | **FAIL** `a[0].tag = 0` (pre-fix); **FAIL** `a[0].tag = 0` (post-parser-fix — secondary OpCopyRecord deep-copy bug) | **FAIL** alloc:1190 panic both pre- and post-fix |
| `22-hash-value-coalesce.loft` | `wrapper.maps ?? other_hash` — Hash value-block | **IV-Hash ✅ closed (native)** | **FAIL** `e.value = null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `23-enum-value-coalesce.loft` | `vec_of_enums[i] ?? other_enum` — struct-Enum value-block | **IV-Enum ✅ closed (native)** | **FAIL** got fallback | **PASS** (closed by predicate fix 2026-05-29) |
| `24-config-default.loft` | `c.entries[k].value ?? "FINAL"` — real-lib config shape | V (extracted) | **FAIL** `'    '` | PASS |
| `25-return-coalesce.loft` | `fn f() -> text { vec[i] ?? "x" }` — return-position present-path | **I (regression guard)** | **FAIL** `'       '` | PASS |
| `26-compound-assign-coalesce.loft` | `s += vec[i] ?? "x"` — compound-assign consumer | I (border, garbage variant) | **FAIL** `'hello \n``#'` (non-zero garbage) | PASS |
| `28-null-literal-coalesce.loft` | `null ?? "x"` — null-literal LHS edge case | edge case | PASS | PASS |
| `29-print-present.loft` | `print(vec[i] ?? "x")` with present value | I (sub-mode garbage) | **FAIL** `'�6J���'` (garbage) | PASS |
| `30-format-present.loft` | `"got: {vec[i] ?? \"x\"}"` with present value | III (reproduction) | **FAIL** `':؃�h,'` (garbage) | PASS |
| `31-hash-insert-coalesce.loft` | `dst[k] = Entry { value: src[k].value ?? "x" }` | I (garbage variant) | **FAIL** `'zU��c�'` (garbage) | PASS |
| `32-struct-literal-coalesce.loft` | `S { name: vec[i] ?? "x" }` | I (garbage variant) | **FAIL** `'4Ŷ-�I'` (garbage) | PASS |
| `33-vec-append-coalesce.loft` | `vec += [vec_src[i] ?? "x"]` | I (garbage variant) | **FAIL** `'M4��'` (garbage) | PASS |
| `34-comparison-coalesce.loft` | `(vec[i] ?? "x") == "y"` — equality consumer | I (comparison sub-mode) | **FAIL** equality false | PASS |
| `35-bool-coalesce.loft` | `vec_of_bool[i] ?? false` — primitive | type-gate baseline | PASS | PASS |
| `36-iter-over-vec-coalesce.loft` | `for x in (vec_of_vecs[i] ?? other)` — iter consumer | **IV-Vec-nested-field-push 🟡** | **FAIL** count=1 want 3 (parser-fix narrows: count=1, total=3 instead of 6 — secondary deep-copy bug) | **FAIL** total=3 want 6 (post-fix) / alloc:1190 (pre-fix) |
| `37-tuple-destructure-coalesce.loft` | `(a, b) = (vec1[0] ?? "x", vec2[0] ?? "y")` | I (NUL variant) | **FAIL** `'     '` (NUL) | PASS |
| `38-enum-ctor-field-coalesce.loft` | `Named { name: vec[i] ?? "x" }` | I (garbage variant) | **FAIL** `'�3...'` (garbage) | PASS |
| `39-concat-after-coalesce.loft` | `(vec[i] ?? "x") + " suffix"` | I (NUL variant) | **FAIL** `'      world'` (NUL) | PASS |
| `40-tuple-value-coalesce.loft` | `vec_of_tuples[i] ?? other_tuple` | **IV-Tuple ✅ closed** | **PASS** (closed 2026-05-30) | **PASS** (closed 2026-05-30) |
| `41-sorted-coalesce.loft` | `wrapper.sorted_field ?? other_sorted` | **IV-Sorted ✅ closed (native)** | **FAIL** `null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `42-void-coalesce.loft` | `vec[i] ?? "x";` as statement (no consumer) | edge case | PASS | (warns unused) |
| `43-empty-source.loft` | `vec[i]` where source = `""` (length 0) | edge case | PASS | PASS |
| `44-prior-value-set.loft` | `s = "old"; s = vec[i] ?? "x"` — reassign | I (garbage on reassign) | **FAIL** `'K�m��k'` (garbage) | PASS |
| `45-closure-coalesce.loft` | `(fn(idx) { vec[idx] ?? "x" })(0)` — closure body | **I + IV-native** | **FAIL** `'       '` NUL | **FAIL** E0308 |
| `46-method-after-coalesce.loft` | `(vec[i] ?? "x").to_uppercase()` — method chain | **I-crash** | **SIGBUS** at op=220 | PASS |
| `47-fallback-chain.loft` | `lookup1(k) ?? lookup2(k) ?? "default"` | VII ✅ closed (native) | **FAIL** `'    '` (cluster I dangling buffer) | **PASS** (closed by text-branch unification 2026-05-29) |
| `48-recursive-fn.loft` | recursive fn with `??` chain | VII ✅ closed (native) | **FAIL** `'  '` (cluster I) | **PASS** (closed 2026-05-29) |
| `49-sigbus-minrepro.loft` | minimum reproducer of probe 46's SIGBUS | I-crash | **SIGBUS** at op=220, PC 6941 | PASS |
| `50-index-coalesce.loft` | `index<Entry[name]>` value-block | **IV-Index ✅ closed (native)** | **FAIL** `null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `51-spacial-coalesce.loft` | `spacial<Point[name]>` value-block | **IV-Spacial ✅ closed (no longer hangs)** | PARSE-ERR (diagnostic + clean exit; was: parser infinite loop, fixed 2026-05-29) | PARSE-ERR (same) |
| `52-char-coalesce.loft` | `char ?? char` (primitive) | (parse-error finding) | parse: "Field access not supported on type character" | same parse error |
| `91-vov-field-append-no-coalesce.loft` | `o.lists += inner` (struct-Inner, no `??`) | **IV-Vec-nested-field-push** | **FAIL** `len(a)=0` (pre); **PASS** (post-parser-fix) | **FAIL** `len(a)=0` (pre); FAIL alloc:1190 (post — secondary bug) |
| `92-vov-local-append.loft` | `outer += inner` (LOCAL var control) | reference | PASS | PASS |
| `93-vov-field-primitive-inner.loft` | `o.lists += inner` (integer inner) | **IV-Vec-nested-field-push** | **FAIL** flat `len=3` (pre); **PASS** (post-fix) | **FAIL** flat (pre); **PASS** (post-fix) |
| `94-vov-field-text-inner.loft` | `o.lists += inner` (text inner) | **IV-Vec-nested-field-push** | **FAIL** flat `len=2` (pre); **PASS** (post-fix) | **FAIL** flat (pre); **PASS** (post-fix) |
| `95-vov-field-assign-literal.loft` | `o.lists = [inner]` (workaround) | secondary bug | PASS | **FAIL** alloc:1190 panic (both pre + post) |
| `96-vov-struct-ctor-literal.loft` | `Outer{lists:[inner]}` (ctor literal) | secondary bug | PASS | **FAIL** alloc:1190 panic |
| `97-vov-field-deeper-nest.loft` | 3-deep `vector<vector<vector<int>>>` field-push | **IV-Vec-nested-field-push + secondary** | **FAIL** alloc:1190 panic both pre + post | **FAIL** alloc:1190 panic |

**Bytecount-of-failure pattern observation:** Cluster I probes mostly return length-preserved-but-zeroed bytes (the NUL-fill is consistent across all non-Var LHS variants — vec, field, call, concat, method, deep-chain, static-source, hash-deep, direct-print, real-config, return-position).  **Cluster I is shape-only — allocation class doesn't matter** (probe 16: static-literal source still corrupts).

**Probe 26 (`s += vec[i] ?? "x"`) is an OUTLIER**: returns non-zero garbage (`hello \n``#`) rather than NUL.  Consistent with cluster III's "format-buffer reuse rewrites the freed memory" mechanism — compound assign uses `OpAppendText` (the same op cluster III uses to append into format-buffer `__work_N`).  When the target buffer `s` has prior content, `OpAppendText` reads from the dangling stack Str AFTER `s`'s buffer has been written to in a previous step that recycled the freed `_ncc_N` heap region.  **Cluster I has two sub-modes**: pure-Set consumer → NUL fill (libmalloc free-fill); `OpAppendText` consumer → garbage fill (target-buffer reuse).  Both same root cause.

**Probe 25 — load-bearing regression-guard finding**: the B5-L3 return-fix at `src/scopes.rs:998-1015` does NOT actually protect cluster I in return position when the present-path is taken.  `repro_p356.loft::text_vec_coalesce_fallback` happens to pass because it exclusively tests the fallback path (OOB → `"fb"` literal in `.rodata`).  The present-path-via-return is broken identically to value-block context.  This means the cluster I fix MUST cover return-position too — a fix narrowly scoped to `is_return=false` would still leave return-with-present broken.

**Cluster III's probe 09** shows non-zero garbage (`���-�` is UTF-8 lossy decode of format-buffer leftovers); probe 19 (multi-`??` in format) shows compounded garbage (`'      and ��S'`).  Format buffer is the cluster III specific differentiator from cluster I's pure-Set NUL pattern, but it shares the root mechanism with probe 26's `+=` case.

**Cluster IV is NOT closed by @PLAN51** — only Reference (probe 10) passes.  Vector / Hash / struct-Enum value-blocks fail on BOTH backends:
- **Interpret-side**: silent corruption — Vec returns zero-element variant, Hash returns null hash, struct-Enum returns the fallback variant entirely.
- **Native-side**: COMPILE ERROR E0308 — `if var__ncc_N {...}` emits a `DbRef` where `bool` is expected.  The null-check codegen for heap-type `??` is wrong.  Fixed in-plan as step 2 of the roadmap (see `cluster-IV-heap-typed.md` for the predicate-emit fix design); same fix surface as cluster VII.

**Probe naming**: `NN-<descriptive>.loft`.  Numeric ordering for stable references.  Probes promote to `tests/scripts/15N-plan52-<descriptive>.loft` when their cluster's fix lands (matching the @PLAN51 graduation pattern at `141-149`).

**Promotion gate** (from `_INVESTIGATION_TEMPLATE.md`):

1. Assertions pass (`probe NN PASSED` prints).
2. Clean process exit — no SIGSEGV / panic at teardown.
3. No leak warning under the loft_suite leak gate.
4. Bounded runtime.

## Curated probe sets — for fix-attempt validation

**Don't run the full 60-probe sweep against every fix attempt.**  Use curated subsets via the runner script.  Each set targets ONE diagnostic dimension; ~5-15 probes each, runs in <30s.

```bash
# From the repo root:
doc/claude/plans/future/52-value-block-borrow-cleanup/probes/run_set.sh <SET>

# Examples:
probes/run_set.sh A       # cluster I core — non-Var LHS shape coverage
probes/run_set.sh H       # baselines — should ALWAYS pass; regression guard
probes/run_set.sh A -v    # verbose: include probe output on FAIL/CRASH
probes/run_set.sh all     # run every set in A..I order (skip Z)
```

| Set | Purpose | Probes | What it verifies | Current state |
|---|---|---|---|---|
| **A** | Cluster I core (LHS shape coverage) | 02, 13, 14, 15, 16, 17 | All non-Var LHS forms (vec-index / field / call / concat / method / static / hash-deep); fix MUST close all six | 6/6 FAIL interpret, all PASS native |
| **B** | Cluster I consumer-variants (garbage fill) | 26, 29, 31, 32, 33, 38, 39, 44, 81 | `OpAppendText` / format / hash-insert / struct-field / vec-append / enum-ctor / reassign / direct-print | 9/9 FAIL interpret garbage, all PASS native |
| **C** | Cluster I-crash (SIGBUS) | 46, 49, 53 | Method-chain consumers — must not crash.  Run with `LOFT_TIMEOUT=10` | 2/3 CRASH / 1 garbage (46/49 SIGBUS; 53 garbage) |
| **D** | Cluster III (format-string `??`) | 09, 19, 30, 56, 78 | Format-buffer interleave variants | 5/5 FAIL interpret garbage, all PASS native |
| **E** | Cluster IV (heap-type value-block) | 21, 22, 23, 36, 40, 41, 50 | Vec / Hash / Sorted / Index / Enum / Tuple — both backends must compile + pass.  Excludes 51 (parser hang, set Z) | 7/7 FAIL interpret, 7/7 COMPILE-ERR native |
| **F** | Cluster VI (closure body `??`) | 45, 65, 67, 86 | Closure-synthesised fn bodies — both backends | 4/4 FAIL interpret, 4/4 COMPILE-ERR native |
| **G** | Cluster VII (chained-call native E0308) | 47, 48, 82 | `lookup1() ?? lookup2() ?? lit` style — must compile under native | 3/3 FAIL native (if-else incompatible types) |
| **H** | Baselines / regression guards — **MUST always PASS** | 01, 03, 07, 08, 10, 18, 35, 42, 43, 55, 60 | If ANY of these regresses after a fix, the fix introduces a new bug.  Run BEFORE and AFTER every fix attempt | **11/11 PASS** (verified 2026-05-29) |
| **I** | Real-library / practical | 24, 71 | Production-shape patterns; near-future consumer code | Mixed (24 fails, 71 passes — see notes) |
| **Z** | Currently-broken probes — skipped from default runs (each maps to an in-plan cluster) | 51, 52, 80, 85 | spacial parser hang (51, cluster IV-Spacial), char-`??` parse refusal (52, edge-case spinoff candidate per policy), nested closure (80, cluster VI), vec capture in closure (85, cluster VI) — all in-plan except 52 which is borderline | All 4 currently fail; each clears when its cluster closes |
| **J** | Cluster IV-Vec-nested-field-push (added 2026-05-30) | 91, 92, 93, 94, 95, 96, 97 | `field += inner_vec` misroute + secondary `copy_claims` panic.  Probe 92 is the LOCAL-var control (must always PASS).  Probes 91/93/94 close with parser fix.  Probes 95/96/97 still expose the secondary alloc panic — second sub-arc of this cluster | Parser fix in working tree: 91 interpret + 93/94 both PASS; 91 native + 95-97 still FAIL on secondary bug |

**Recommended fix-attempt workflow:**

```bash
# 1. Pre-fix baseline.
probes/run_set.sh H
# Expected: all PASS.

# 2. Confirm current failure shape for the cluster you're fixing.
probes/run_set.sh A   # for cluster I

# 3. Apply fix.

# 4. Verify target cluster closed + baselines still pass.
probes/run_set.sh A
probes/run_set.sh H

# 5. Cross-cluster regression sweep (catch unintended interactions).
for s in B C D E F G I; do probes/run_set.sh $s; done

# 6. If any set unexpectedly regresses, the fix breaks something else —
#    revert and re-design (mirror of the @PLAN51 three-attempt journal).
```

**Promotion to `tests/scripts/`**: Once cluster I's fix lands, graduate one representative probe per set (A's probe 02; B's probe 31; C's probe 49; D's probe 09; E's probe 21; F's probe 45) to `tests/scripts/15X-plan52-<descriptive>.loft`.  Mirrors the @PLAN51 graduation pattern at `tests/scripts/141-149-*.loft`.

## Reference ↔ problem pairings

The diagnostic shortcut: diff a problem probe against its closest passing reference and the mechanism becomes visible at the bytecode level.

| Problem | Reference | What the diff reveals |
|---|---|---|
| 02 (`vec[i] ?? "x"`) | 01 (`v ?? "x"`) | The simple-var LHS at `src/parser/operators.rs:1227` short-circuits the ncc-block construction; non-trivial LHS allocates `_ncc_N` and triggers the dangling-Str pattern. |
| 02 (`vec[i] ?? "x"`) | 03 (`vec[i]` direct) | Direct read pushes the source store's Str onto the stack without an intermediate block-local; no scope-exit free fires before the consumer reads. The `??` lowering's `_ncc_N` is the load-bearing difference. |
| 02 (interpret) | 12 (same loft under `--native test`) | Native's `output_block` at `emit.rs:1283-1297` materialises `_ret.to_string()` inside the block; interpreter has no equivalent. |
| 04 (`obj.f ?? "x"`) | 02 | If 04 fails identically, confirms the cluster is "any non-trivial LHS," not "vector index specifically." |
| 05 (`call() ?? "x"`) | 02, 04 | Three-way confirmation of LHS-shape-independence. |
| 07 (`if cond { vec[i] } else { "x" }`) | 02 | If 07 fails the same way, cluster II is the same root cause expressed through `if` instead of `??`. |
| 09 (in format string) | 02 | If 09 fails too, the format-string buffer machinery has the same exposure. |
| 10 (Reference shape) | 02 | If 10 passes, @PLAN51's `paired_witness` already covers Refs and the cluster is text-specific. If it fails, the fix needs to be type-agnostic. |

## Tool gaps

| Tool | Status | Used for |
|---|---|---|
| `LOFT_LOG=fn:main` | Verified-suitable (existing) | The bytecode trace that pinned cluster I's mechanism — produced the `<raw:0x9dac61a90>` smoking-gun marker showing the dangling-pointer escape via `Str::try_str`. |
| `LOFT_TRACE_DB` | Verified-suitable (PLAN51 inheritance) | Per-`OpDatabase` trace; useful for confirming `_ncc_N`'s store reuse pattern between iterations in probe 11. |
| `LOFT_KEEP_NATIVE_RS` | Verified-suitable (PLAN51 inheritance) | Compare native-passing emit with the equivalent interpreter bytecode to localise where materialisation is missing. |
| `LOFT_TIMEOUT=<s>` + `LOFT_TIMEOUT_CLEAN_EXIT=1` | **Verified-essential 2026-05-29** (existing, PLAN49) | Used to diagnose probe 51's hang as a PARSER infinite loop (`phase=parse` breadcrumb) rather than a runtime issue.  Should be the default invocation for any speculative probe — saves manual `pkill -9` and gives a localised diagnosis.  Should add a probe-runner wrapper script that always sets this. |
| `LOFT_LOG=crash_tail:N` | Verified-suitable (existing) | Capture the last N bytecode operations before SIGBUS — used for probe 46/49 to confirm crash is in opcode dispatch (op=220) at PC ~6941. |
| (Future) sanitizer CI lever | Missing | A `cargo +nightly miri test` or `RUSTFLAGS="-Zsanitizer=address"` job would catch this UB class on Linux too, removing the macOS-only manifestation as the sole signal. Could open as a Phase-F infrastructure deliverable. |

## Status & next-session roadmap

Each fix step has a **binary exit criterion** — a probe-set / CI gate that must PASS before the step is done.  Following this order, after step 8 the plan is provably closed: every cluster is either fixed-on-both-backends or explicitly out-of-scope with a recorded reason.

| # | Step | Exit criteria | Effort | Risk |
|---|---|---|---|---|
| **0** | Pre-flight: probes 87 (closure IR trace) + 88-90 (spacial parser-hang isolation) + source-reading session for the fix sites (`src/scopes.rs::free_vars`, `src/generation/emit.rs` value-block path, `src/parser/`+`src/typedef.rs` spacial registration) | All 6 per-cluster design docs written (✅ done); probes 87-90 recorded (✅ done); fix sites identified per `cluster-*.md` files | 1-2 sessions | none — read-only |
| **1** | **Fix IV-Spacial parser hang** — pure parser-side, no codegen/scope interaction.  Per `cluster-IV-Spacial-parser.md`.  **LANDED 2026-05-29**: hand-rolled scanner loop replaced with `parse_fields` (conditional on `peek_token("[")` so bare `spacial<T>` still works for the existing P22 diagnostic test).  Probes 51/62/88/89 no longer hang; probe 90 still PASSes; Set H green; `cargo test --test issues` 681/681 pass.  Full probe PASS requires spacial implementation (1.1+, out of scope). | Probes 51/62/88/89/90 PASS on both backends; Set H still green; `make ci` green | 1-2 days | LOW |
| **2** | **Fix cluster IV + VII** — same `src/generation/emit.rs` value-block return path; predicate emit + branch-type unification.  Per `cluster-IV-heap-typed.md` + `cluster-VII-chained-call.md`.  **2026-05-29 iteration 1**: predicate fix landed (`output_test_predicate` helper) — closes IV-Enum (probe 23) on native.  **2026-05-29 iteration 2**: dep-strip fix in `parser/expressions.rs` — closes IV-Hash / IV-Sorted / IV-Index on native (probes 22/41/50 PASS).  Still open: IV-Vec / IV-Vec-iter (vector deep-copy path), IV-Tuple (sentinel), Cluster VII (text-branch unification). | Sets E + G PASS on both backends; Set H still green; `make ci` green; moros_* native suite green | 2-3 days | LOW |
| **3** | **Re-run sets A-I + Z** after steps 1+2 land.  Record incidental closures | Updated probe-status matrix in this README; cluster catalogue marks any newly-incidentally-closed sub-clusters | 1 session | none |
| **4** | **Investigate cluster VI** — apply probe 87's `LOFT_LOG=fn:<closure_synth_name>` trace; decide between Hypothesis A (cluster I covers it) vs B/C (separate fix) per `cluster-VI-closure.md` | Hypothesis confirmed; fix surface decision logged in `cluster-VI-closure.md` § "Fix iterations" | 1 session | none |
| **5** | **Fix cluster I** — `src/scopes.rs::free_vars` parent-scope `__ret_text_N` materialisation per `cluster-I-ncc-text.md` Option A.  Expected 2-5 iterations per @PLAN51 history | Sets A + B + H PASS on both backends; moros_* native suite green; `130-gridmesh-crystal-equiv.loft` still passes; `tests/scripts/repro_p323.loft` passes on macOS interpret; `make ci` green | 1-1.5 weeks (multiple attempts likely) | **HIGH** — scope-pass fixes routinely break adjacent things |
| **6** | **Re-run sets C + D + F + I** to verify free-rider closures from cluster I fix | Set C (I-crash) PASS or explicit deferral; Set D (III) PASS; Set F (VI) PASS if step 4 confirmed Hypothesis A; Set I (V) PASS | 1 session | none |
| **7** | (Conditional) **Fix cluster VI standalone** if step 6 shows Set F not closed by cluster I's fix | Set F PASS on both backends; Set H still green | 1-3 days | MEDIUM |
| **8** | **Graduate probes + close the plan** — move one representative probe per set to `tests/scripts/15X-plan52-<descriptive>.loft`; move `cluster-*.md` to `plans/finished/52-…/` | All sets A-I PASS on both backends; Set Z resolved or explicitly recorded as out-of-scope; `make ci` green; plan moved to `finished/` | 1 session | none |

### "We know we're clear" — binary close criteria

The plan is provably closed iff ALL of these hold after step 8:

1. **Probe sets A-I all PASS on both backends** — verified via `probes/run_set.sh all`.  Set Z fixed or explicitly excluded with one-line reason in this README.
2. **`make ci` green** — fmt + clippy (default + `--all-targets --all-features`) + nextest + check-no-default-features.
3. **`make ci-full` green** — package + GL smoke + GL golden suites.
4. **moros_* native library suite green** — the @PLAN51 canary remains a regression guard.
5. **`tests/scripts/repro_p323.loft` passes on macOS interpret** — the original @P383 bug report.

If any of (1)-(5) fail, the plan is NOT closed: the offending step's `cluster-*.md` doc grows a new "Fix iterations" entry recording the attempt and the failure mode.  No informal "we think it's done" closures.

### Aggregate effort

~2-3 weeks total (in line with @PLAN51).  Quickest-user-visible-win: step 1 OR step 2 lands in 1-3 days — both deliver loud, visible improvements (parser hang gone; native compile errors gone) before the higher-risk cluster I work begins.

## Knowledge sufficiency — per cluster

After 60 probes + 5 batches of investigation, knowledge per cluster is summarised below.  **"Ready to fix"** = mechanism is verified AND fix surface identified AND regression-guard probe set covers the cluster.

| Cluster | Mechanism understood? | Fix surface identified? | Probe-set coverage | Ready to fix? |
|---|---|---|---|---|
| **I** (text non-Var LHS) | ✅ Verified via bytecode trace; shape-only diagnosis confirmed across 21 probes; sub-modes (NUL/garbage/false-equality) characterised per consumer | ✅ `src/scopes.rs::free_vars` else-branch — parent-scope `__ret_text_N` temp à la B5-L3 for `is_return=false`.  **Must also cover `is_return=true` present-path** (probe 25 finding) | ✅ Set A (core 6) + Set B (garbage variants 9) + Set H (baselines) | **YES** |
| **I-crash** (SIGBUS) | 🟡 Hypothesised: method-dispatch reads dangling Str's `ptr` from stack, bytes-region unmapped between scope-exit and dispatch.  NOT deeply traced.  Probe 53 garbage vs 46/49 SIGBUS divergence not yet explained | ⏸️ Likely closes with cluster I fix.  Verify after cluster I lands; deeper dive only if 46/49 still crash | ✅ Set C (3 probes) | **Mostly** — verify after cluster I |
| **II** | n/a — falsified | n/a | (skip set) | n/a |
| **III** (format `??`) | ✅ Verified `??`-dependent; format-buffer `OpFormatText` interleaves a write between if-result push and `OpFreeText`; intervening-disturbance changes byte pattern (probe 78) | 🟡 Likely closes incidentally with cluster I (same root: dangling Str on eval stack).  Format-buffer materialisation as fallback if not | ✅ Set D (5 probes) | **Mostly** — verify after cluster I; targeted fix only if needed |
| **IV-Ref** | n/a — falsified (PLAN51 covers) | n/a | (set H baseline) | n/a |
| **IV-Vec/Hash/Sorted/Index/Enum/Tuple** | ✅ Verified — null-check codegen gap: `if <DbRef>` predicate emitted where rustc expects `bool` or a sentinel test | ✅ `src/generation/emit.rs` value-block return path; predicate emit must project `is_some_sentinel()`.  Interpret side: analogous heap-handle dangling check | ✅ Set E (7 probes covering all 6 heap-type families) | **YES** — separate fix from cluster I; can land independently |
| **IV-Vec-nested-field-push** (reclassified IN-plan 2026-05-30) | ✅ Verified via IR trace — parser branch order bug at `src/parser/expressions.rs:1370`: `field += inner_vec` for `vector<vector<X>>` fields misroutes to concatenate when LHS element type EQUALS RHS type | ✅ Parser one-line guard `&& !(**elm_tp).is_equal(&s_type)`; falls through to existing single-element-push lowering at line ~1389.  Secondary `copy_claims` panic in `database/allocation.rs:1190` for `vector<vector<Struct>>` deep-copies — separate fix surface inside the cluster | ✅ Probes 91-97 added 2026-05-30 (primitive/text/struct inner + control + workaround variants + 3-deep) | **Partial — parser fix verified in working tree; secondary `copy_claims` panic still open** |
| **IV-Spacial** | ✅ Verified — **PARSER infinite loop on `spacial<X[k]>` type itself**, NOT a `??` issue (probe 62 confirms no-`??`).  Surfaced during cluster IV-completion probing — surfaced HERE because the `??` lowering exercises type-resolution for `spacial<>` in a way other code doesn't.  Could share root with other type-resolution paths used by cluster I/III/IV codegen | 🟡 Need drill-down within this plan — likely in `src/parser/` type-resolution for `spacial<>`.  In-plan investigation: add a probe varying the key type (`spacial<X[int_k]>` vs `spacial<X[text_k]>`) + run with parser instrumentation to localise the recursion site | 🟡 Currently set Z (skipped from default runs); promote into a working set once fix surface is identified | **In-plan investigation continues** — see "Investigation tasks before fix" below |
| **V** (real-library) | ✅ Scan complete: no live text-coalesce in shipped lib code today; only `?? 0` integer form (value-typed, safe).  Future `lib/server` config / HTTP-middleware patterns will exercise the bug | n/a — closes when cluster I lands | ✅ Set I (2 probes) | **YES** — covered by cluster I closure |
| **VI** (closure body `??`) | 🟡 Partially verified — closures break even with simple-Var LHS (probes 65/86); both backends fail.  **Strong hypothesis worth confirming before fix**: closure synthesis at `src/parser/control.rs` lambda-lowering creates a NEW function-scope, and the `??` ncc-block construction inside the closure body bypasses or duplicates the simple-Var fast-path at `operators.rs:1227`.  May share root mechanism with cluster I — if cluster I's `scopes::free_vars` fix covers the closure-synthesised fn's body identically, cluster VI closes incidentally | 🟡 Need IR-level trace of closure body emit before committing.  Concrete next step: dump `LOFT_LOG=fn:<synth_name>` for probe 65 to see what the closure body's ncc-block IR looks like vs the equivalent non-closure version (probe 02) | ✅ Set F (4 probes) | **In-plan investigation continues** — see "Investigation tasks before fix" below |
| **VII** (chained-call native E0308) | ✅ Verified — chained `lookup1() ?? lookup2() ?? lit` and recursive `??` produce `if-else have incompatible types`.  The chained-coalesce join codegen emits branches with mismatched types | ✅ `src/generation/` — the chained-coalesce IR-to-Rust emit; needs unified branch typing | ✅ Set G (3 probes) | **YES** — can land independently or bundled with IV |

### In-plan vs spinoff policy

Findings stay **in-plan** by default.  A discovered bug only spins off as a separate P-issue / mini-plan when ONE of two criteria is met:

1. **Truly an edge case users will not hit** — e.g., a parser refusal on a syntactically-invalid construct nobody would write deliberately.  Probe 52's `char ?? 'F'` (parser rejects field-access-on-character) is a borderline candidate — users typing this clearly have a different bug, the parser-error message is correct, and "fixing" `??` for char would require designing a char-null-sentinel that has its own semantic questions.  Spinning that off is reasonable.
2. **Needs its own investigation plan** — the fix surface is large enough or touches enough unrelated subsystems that bundling it within PLAN52 would balloon the plan beyond reviewer-friendliness.  E.g., if cluster IV-Spacial's parser-recursion investigation surfaces a SECOND parser-recursion site in `index<X[k]>` and a THIRD in `sorted<X[k]>`, that's plan-shaped work (3+ clusters of type-resolution recursion) and earns its own PLAN53.  But the first finding alone — one site — stays in PLAN52.

**Default**: keep findings in-plan.  Two reasons it's the safer default — (a) cross-cluster overlap: fixing cluster X may close cluster Y too, but only if Y is still tracked under the same probe-set gate; (b) the cumulative probe coverage IS the regression-guard for the whole class.

### Investigation tasks before fix (in-plan)

Two clusters need short investigation passes before their fix shape is ready.  Both block PLAN52 closure:

1. **Cluster VI — closure-body `??`**:
   - **Add probe 87**: same body as probe 65 but compiled with `LOFT_LOG=fn:<synth_name>` to capture the closure's synthesised-fn IR.
   - **Diff against probe 02 / 65**: identify whether closure synthesis (a) bypasses the simple-Var fast-path at `operators.rs:1227`, (b) creates a duplicate `_ncc_N` per closure invocation, or (c) routes through a different scope-handler that doesn't run `get_free_vars` correctly.
   - **Decision point**: if the mechanism is identical to cluster I's `_ncc_N` block + post-consumer OpFreeText, then the cluster I fix MUST cover the closure-synth fn body too (single fix surface).  If the mechanism is different, cluster VI gets a targeted scope-handler fix in the closure-synth path.
   - Effort: 1 session (probe + trace + decision).

2. **Cluster IV-Spacial parser hang**:
   - **Add probes 88-90**: variations of `spacial<X[k]>` with different key types (text, integer, struct) WITHOUT `??`.  Localise the hang to a specific key-type combination.
   - **Add `LOFT_LOG=parse_recursion`** (or eprintln-instrument the parser's type-resolution recursion site if no such lever exists) to localise the infinite-recursion frame.
   - **Hypothesis to test**: the spacial parser's type-merging step recursively expands `Type::Reference(spacial<...>)` without a termination check.  If true, the fix is a depth cap or a memoisation on the same type-node — both small.
   - **Decision point**: in-plan if the fix touches the same scope/type-resolution machinery as cluster I (load-bearing — could share root); standalone follow-up only if the fix surface is completely disjoint AND the probe matrix confirms no cluster-I/III/IV overlap.
   - Effort: 1-2 sessions (probes + instrumentation + fix).

### Aggregate readiness

**5 of 7 confirmed clusters are ready to fix today** (I, I-crash, III, IV-V/H/Sorted/Index/Enum/Tuple, V, VII).  Two need short investigation passes (VI, IV-Spacial).  All seven close within this plan — none are spun off.

**Fix sequencing** (smallest blast radius first):

1. **Cluster IV (heap-type) + VII (chained-call)** — both `src/generation/emit.rs` predicate emit.  Single PR; ~2-3 days.  Visible win: today's compile errors disappear.
2. **Cluster VI investigation pass** (1 session) — either rolls into cluster I's fix or gets its own targeted fix.
3. **Cluster IV-Spacial investigation pass** (1-2 sessions) — depth cap / memoisation fix.
4. **Cluster I + incidental III + V** — `src/scopes.rs::free_vars` parent-scope text temp.  ~1-1.5 weeks.  Closes silent-corruption cases and the largest probe-set.
5. **Cluster I-crash verification** — run set C after cluster I lands; expected to close incidentally.  Deep dive only if 46/49 still crash.
6. **Re-run all sets A-I**, graduate one representative probe per set to `tests/scripts/15X-plan52-…`, close the plan.

Total estimate: ~2-3 weeks (in line with @PLAN51's scope, slightly larger because PLAN52 spans both parser and codegen surfaces).

## See also

- [`plans/finished/51-hidden-buffer-aliasing/`](../../finished/51-hidden-buffer-aliasing/) — sibling investigation plan; PLAN52 is the text-flavoured 6th cluster of that family.
- [`doc/claude/PROBLEMS.md`](../../../PROBLEMS.md) §@P383 — the canonical bug report with the full bytecode-trace diagnosis.
- [`src/parser/operators.rs:1227-1247`](../../../../../src/parser/operators.rs) — the `??` lowering; cluster I's source-side root.
- [`src/scopes.rs:1122-1184`](../../../../../src/scopes.rs) — `free_vars` else-branch + `get_free_vars` text-OpFreeText emission; cluster I's interpreter-side root.
- [`src/generation/emit.rs:1283-1297`](../../../../../src/generation/emit.rs) — the @P321e/@P323 native fix (`_ret.to_string()` materialisation); the working baseline this plan's interpreter fix should match in semantics.
