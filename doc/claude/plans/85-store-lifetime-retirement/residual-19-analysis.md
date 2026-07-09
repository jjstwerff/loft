<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 residual-19 — per-group mechanism analysis (LIVE, probe-backed)

Companion to [`residual-19-fix-plan.md`](residual-19-fix-plan.md) (the fix
sequencing). This is the MECHANISM analysis for the groups OTHER than the
UserCall slice: each group's minimal leaking repro is saved under
[`probes/residual-19/`](probes/residual-19/) and its leak-oracle result recorded
here. **This is deliberately NOT consumed into fixes yet** — the probes are kept as
fixtures to enhance as each slice is taken up. Re-run:

```
ABIN=target/aarch64-apple-darwin/release/loft \
  bash doc/claude/plans/85-store-lifetime-retirement/probes/residual-19/run.sh
```

Oracle = count of `loft::fill::append_text` leak frames (runtime-owner, excl
`ir_read`). All results below are at HEAD (`40095f89`), ASan on macOS-ARM.

## CORRECTION (2026-07-09) — the "generic axis / merge-9" claim was VACUOUS; retracted

The `verify_tuple_NONgeneric_clean` "leak=0" was a **vacuous cell** — that probe does
NOT compile (`Variable 'r' cannot change type from (text, text) to
__tuple<text,text>`), so 0 meant *it never ran*, not clean. The matrix rule (prove the
harness can fail) was violated. Redone with RUNNING probes + INTROSPECT (no ASan), the
real per-case mechanisms are below and the "one generic root of 9" is **retracted**.

### Exact mechanisms (introspect-verified, HEAD)

- **Tuple (`p329` ×3, `p330` ×2) — EXACT, generic-specific.** Non-generic
  `n_pair(__retbuf: __tuple<text,text>) -> __tuple<text,text>` delivers the tuple
  through a caller **`__retbuf` buffer** (builds into the record by reference, frees
  `__work_1`, clean). The generic monomorph `t_1P_pair(...) -> (String, String)`
  delivers **by value** via an unfreed `__ret_text_1` clone
  (`__ret_text_1 = to_text(..).to_string(); return (__ret_text_1.to_string(), "m")`).
  Chokepoint: `try_generic_instantiation` (`parser/mod.rs:3135`,
  `tuple_return_rewrite(substitute_type(tmpl_returned))`) produces a BY-VALUE tuple
  return, bypassing the `ref(__tuple)` buffer-return the non-generic parse builds in
  block_result/scope. Instantiation is a clone+substitute (`mod.rs:3111-3147`) and
  never re-runs the delivery promotion.
- **`plan17_b`, `plan17_printable`, `p243` — NOT generic-specific; they are the
  UserCall gap (category F) wrapped in a generic.** `first`'s tail is `v[0].to_text()`
  — a UserCall HEAD does not promote, so the TEMPLATE isn't promoted and the monomorph
  (`t_7integer_first -> String`) inherits the unpromoted return. Because instantiation
  CLONES the template's return type + hidden attrs (`mod.rs:3112-3147`), a correct F
  fix that promotes the `first<T>` TEMPLATE makes every monomorph inherit the buffer —
  so these fall to F, no separate generic fix.
- **`p241` — block-RHS family, not tuple.** `t_4text_s(...) -> DbRef` — the vector is
  returned by reference; the leak is the generic `[0]` element extraction into the
  `test_value = { … }` block-RHS (its own mechanism, sibling to `n3`).

### Corrected categories (19)

| category | tests | n | exactness |
|---|---|---|---|
| **Tuple by-value return** (generic instantiation) | p329 ×3, p330 ×2 | 5 | **EXACT** (`mod.rs:3135`) |
| **F — UserCall** (incl. generic `.to_text()` templates) | p54_or_pattern, p54_extractors_spec, p54_multi_call_flow, plan17_b, plan17_printable, p243 | 6 | mechanism exact; fix (pre-pass) blocked, see fix-plan |
| **B — fn-ref CallRef ABI** | p227 ×4 | 4 | identified, not exact |
| **block-RHS delivery** | n3, p241 | 2 | mechanism isolated |
| **C — vector `?? ""` copy** | issue_437 | 1 | identified, not exact |
| **E — forward-borrow view** | p54_b6 | 1 | identified, not exact |

The old "generic collapses tuple+vector+1b into one root" table below is SUPERSEDED by
this one. Lesson banked: a leak-oracle "0" is only evidence if the program RAN — every
control probe must be run-checked, not just leak-checked.

### Vacuousness audit (2026-07-09) — after the first vacuous cell was caught

Run-checked EVERY 0-leak control that underpins an isolation. Result: 11/12 ran;
TWO vacuous cells total this session — `verify_tuple_NONgeneric_clean` (caught,
retracted the merge) and `b_native` (block `{…; u.to_json()}` — compile error; it fed
an exploratory "block native-tail is clean" point, NOT load-bearing for any category).
The load-bearing controls (n3's `n_nocopy`/`n_copyonly`, p241's `p_nobrace`/`p_split`/
`p_nongen`, issue_437's minimal `mk` no-append, `g3_tuple_build_discard_clean`) ALL
ran → those isolations stand. Net: category boundaries are audited and holding; two
categories' EXACT chokepoints (B fn-ref, C `?? ""`, E forward-view) remain shape-only.

### Category B (fn-ref, p227 ×4) — VERIFIED: it is the `__ret_N` class, NOT a fn-ref-ABI bug

Discard-vs-return analog (both run-checked, introspect):
```
DISCARD (x = f(42);): x = fn_ref[0](42, cref{__work_1}); OpFreeText(x); OpFreeText(__work_1);  → clean
RETURN  (f(42) tail): __ret_1 = fn_ref[0](42, cref{__work_1}); OpFreeText(__work_1); return __ret_1;  → __ret_1 unfreed = LEAK
```
The fn-ref dispatch + `cref_work_buf` are IDENTICAL and correct in both (the lambda's
buffer is threaded AND freed). The leak is solely `run`'s `CallRef` tail being
unpromoted (`FnRefCall ∉ wants_tret_bind`) → an owned `__ret_1` returned and freed by
neither side. Same root as F/UserCall and the tuple; the tail is just a `CallRef`.

### CONSOLIDATION (verified) — one class now covers 15 of 19

| class | tests | n |
|---|---|---|
| **`__ret_N` owned-text return, unpromoted tail** (callee `skip_free`s / returns it; caller copies; nobody frees) | F: p54_or_pattern, p54_extractors_spec, p54_multi_call_flow, plan17_b, plan17_printable, p243 · tuple: p329 ×3, p330 ×2 · fn-ref: p227 ×4 | **15** |
| block-RHS assignment delivery | p241, n3 | 2 |
| vector `?? ""` element copy | issue_437 | 1 |
| forward-borrow view return | p54_b6 | 1 |

The 15 differ only in TAIL SHAPE (`Call` / `CallRef` / tuple-element / generic
monomorph) and thus in the per-tail PROMOTION delivery — but they share one leak: an
owned `__ret_N` text delivered at a return boundary that neither side frees.

### Caller-side-free hypothesis — PROBED and REFUTED (2026-07-09)

Tested whether the caller freeing the consumed return temp fixes it. Consumption
matrix on `run(){ inner() }` (UserCall), all run-checked:

| caller pattern | leak |
|---|---|
| `r = run(); print(r)` (bound — main emits `OpFreeText(r)`) | **1** |
| `print(run())` (unbound) | 1 |
| `test_value = { run() }` (harness block-RHS) | 1 |

**Bound-and-freed leaks identically to unbound** — so caller-side free does NOT fix
it (consumption-independent, matching Session-2b for the native case). Introspect of
`run` shows why: it allocates `__work_1` (inner's dest buffer) and frees it, but
`__ret_1 = inner(…)` is an owned COPY it returns; `main`'s `r` aliases-and-frees that,
yet the leak persists — the leaked allocation is the interpreter-internal Rust `String`
built inside `append_text` when the copy is made, NOT the store the caller holds
(native drops it → `--native` is clean; the store tracker doesn't surface it). So a
store-level caller free cannot reach it.

**Conclusion: the ONLY fix is PROMOTION** — deliver the callee's result straight into
the caller's `&text` buffer so no owned `__ret_N` copy is ever built. This needs
per-tail delivery + the pass-1 signature pre-pass, which is itself blocked (the pass-1
body erases forward-ref call tails — see `residual-19-fix-plan.md` slice 1). The
caller-side-free shortcut is closed off.

### Tuple mechanism — EXACT site, but NOT a contained fix (corrected again)

The tuple leak is `scopes.rs:3456`: the @P329 hoist mints `__ret_text_N`, marks it
`set_skip_free` (so the callee epilogue leaves it for the caller), and the returned
tuple's Str elements BORROW into it — but the caller COPIES and nobody frees it. This
is verbatim the Session-6 `__ret_N` caller-consumes-but-nobody-frees class, NOT a
contained tuple bug. Interpreter-only (native drops the `String`s). Fix = the campaign's
open problem (caller-side free, or ref-`__retbuf` which `tuple_return_rewrite`'s comment
says breaks p329/p330). Earlier "EXACT/contained" label for the tuple was optimistic.

## (SUPERSEDED) earlier categorization — kept for the record

The interpreter's dispatch is FLAT — every leak funnels through
`append_text ← execute_argv ← Test::drop`, identical for all 19 — so the Rust
stack cannot distinguish categories (confirming the "uniform site" of Session 4).
Verification is therefore BEHAVIOURAL: apply each category's clean-transform to a
probe and see the leak vanish. The decisive experiment (`verify_*` probes):

| probe | leak | reading |
|---|---|---|
| `g3_tuple_return` (GENERIC `pair<T>() -> (text,text)`) | 1 | generic tuple return leaks |
| `verify_tuple_NONgeneric_clean` (same shape, NON-generic) | **0** | drop `<T>` → clean |
| `verify_tuple_generic_LITERAL_elem_clean` (generic, element is a literal) | **0** | text NOT from `T` → clean |
| `verify_vec_NONgeneric_clean` (non-generic vector-elem return) | **0** | drop `<T>` → clean |

**So the axis is the GENERIC MONOMORPH, not the tuple / vector / composite.**
Structural confirmation (introspect, non-ASan): the monomorph returns unpromoted
owned `String`s — `t_1P_pair(...) -> (String, String)` and `t_7integer_first(...)
-> String` — with NO `&text` buffer. Generic instantiation substitutes the template
body without re-running the buffer promotion.

**This MERGES three of the earlier groups into ONE root (9 of the 19):**

| verified category | tests | count |
|---|---|---|
| **A — generic-monomorph text return-delivery** (unpromoted monomorph) | plan17_b, plan17_printable, p243, p329 ×3, p330 ×2, p241 | **9** |
| **B — fn-ref `CallRef` ABI** (non-generic) | p227 ×4 | 4 |
| **C — non-generic `vector<text>` element copy** (`?? ""`) | issue_437 | 1 |
| **D — `OpCopyRecord` embedded text** (non-generic) | n3 | 1 |
| **E — forward-borrow view of a local** (non-generic) | p54_b6 | 1 |
| **F — pure UserCall** (non-generic; the slice-1 targets) | p54_or_pattern, p54_extractors_spec, p54_multi_call_flow | 3 |

Total = 19. **A is the highest-leverage: one fix (promote generic monomorph text
returns) closes 9.** B/C/D/E/F are each verified NON-generic, so they are genuinely
distinct roots — not folded into A. `issue_437` (C) and `p241` (A) are both vectors
but different: `p241` is generic (drops to clean when de-genericised), `issue_437`
is non-generic (its leak is the `?? ""` element copy). The per-group sections below
keep their original probe notes; this table is the verified consolidation.

| probe | leak | group |
|---|---|---|
| `g1b_direct_monomorph_discarded` | 1 | 1b generic |
| `g1b_generic_to_text` | 1 | 1b generic |
| `g1b_nongeneric_STILL_LEAKS_at_head` | 1 | 1b (see note) |
| `g2_fnref_local` | 1 | 2 fn-ref |
| `g2_fnref_struct_field` | 1 | 2 fn-ref |
| `g3_tuple_return` | 1 | 3 tuple |
| `g3_tuple_build_discard_clean` | **0** | 3 tuple (control) |
| `g4_vector_copy_helper` | 1 | 4 vector |
| `g5_n3_copyrecord_block` | 1 | 5 block-RHS |
| `g5_p241_generic_index_block` | 1 | 5 block-RHS |
| `g5b_view_through_forward_borrow` | 1 | p54_b6 |

---

## Group 1b — generic `.to_text()` monomorph (`plan17_b`, `plan17_printable`, `p243`)

**Confirmed mechanism (sharpest repro `g1b_direct_monomorph_discarded`):** a generic
monomorph's OWN body leaks its internal native `x.to_text()` **even when the result
is discarded** (`to_text2<T>(x){x.to_text()}` called and dropped → leak=1). So this
is NOT a return-delivery / caller bug. Generic instantiation is pass-2-only and
mints each monomorph (`t_<LEN><Type>_<fn>`) by SUBSTITUTING the template body — it
does not re-run through `parse_block`, so the **2d NativeCall promotion never fires
for the monomorph** and `x.to_text()`'s `_dest` result is delivered to an owned temp
that no one frees. Earlier introspect corroborates: `t_7integer_first(...) -> String`
(plain owned, no `&text` buffer).

**Chokepoint neighbourhood:** the generic instantiation path (`parse_call` pass-2
monomorphisation + `substitute_type`) — apply the text-return promotion to monomorph
bodies, or promote the TEMPLATE so substitution inherits the `&text` buffer.

**OPEN (enhance later):** `g1b_nongeneric_STILL_LEAKS_at_head` shows the NON-generic
`firsti(v){v[0].to_text()}` chain ALSO leaks at HEAD. Two candidate causes tangled
together: (a) `run(){firsti(nums)}` is a UserCall HEAD doesn't promote; (b) `firsti`'s
`v[0].to_text()` is a native tail 2d *should* promote — does the `v[0]` index arg or
the integer-arg native break the 2d promotion? Untangle by testing `firsti` in
isolation with a rebind after the UserCall slice lands.

## Group 2 — text fn-ref call (`p227` ×4)

**Confirmed:** `g2_fnref_local` (`f(42)` where `f` is a lambda-typed local) and
`g2_fnref_struct_field` (`g.fmt(42)`) both leak 1. The tail is a `CallRef`, whose
adaptive @P387 ABI delivers owned text with no caller buffer.

**Chokepoint neighbourhood:** `text_return`'s lambda work-buffer logic
(`control.rs`, the `is_lambda` / `__work_ret` block) + the `fn_call_ref` hidden-buffer
dispatch. The ABI intends "one hidden `&text` buffer per text-returning fn-ref call";
the delivery does not currently thread it for these shapes.

**OPEN (enhance later):** does the fn-ref var's fn type resolve by end of pass 1 (so a
signature pre-pass could stamp it), or only in pass 2? Probe the capture variants
(`p227_*_with_capture`) separately — the closure record + buffer are distinct hidden
params and the slot order matters.

## Group 3 — tuple-of-text RETURN delivery (`p329` ×3, `p330` ×2)

**Confirmed (control pair is decisive):** `g3_tuple_return` (bind a whole returned
`(text,text)`) leaks 1; `g3_tuple_build_discard_clean` (build a tuple-of-text and
discard, no return) leaks **0**. So the leak is the tuple RETURN-delivery, NOT the
tuple construction and NOT element extraction. A returned tuple carrying text
elements copies those elements on delivery and the source is unfreed.

**Chokepoint neighbourhood:** the tuple return-delivery + the `__ret_text_N` hoist
(`scopes.rs`, @P329/@P330). Promote per text-element into caller buffers, or free the
element sources after the delivery copy.

**OPEN (enhance later):** is the leak per text-element (a 3-text tuple leaks 3?) or
one per return? Add an N-element probe to size the fix.

## Group 4 — vector-of-text element copy (`issue_437`)

**Confirmed:** `g4_vector_copy_helper` — `ct(v){o=[]; for i {o += [v[i] ?? ""]}; return o}`
then `xs = ct(src); xs += ["tags"]` leaks 1. A minimal `mk(){o=[];o+=["A"];return o}`
does NOT leak (separately verified) — so the leak is specifically the `?? ""`-defaulted
element COPY into the result vector, not the vector build or return.

**Chokepoint neighbourhood:** the `??`-defaulted element append into `vector<text>` +
the vector return-delivery (`Delivery`/`materialize_vector_arms_into`, `control.rs`).
Distinct arc from the text-return family — value+LENGTH+leak oracle required (a
doubled vector reads leak-free; only length catches it).

## Group 5 — block-expression-as-assignment-RHS (`n3`, `p241`)

The harness wraps `.expr("X")` as `test_value = {X}` (`tests/testing.rs:209`), so both
leak in a block-RHS delivery — NOT the eval-wrapper return first guessed. Two DISTINCT
bugs:

- **`p241` (`g5_p241_generic_index_block`)** — leak needs generic + index + `{ }`
  block-RHS together. A generic `vector<T>` element (`s<T>("hello")[0]`) delivered into
  a block-value assignment; the copied element is unfreed. (Likely shares 1b's
  monomorph root — the generic vector element delivery.)
- **`n3` (`g5_n3_copyrecord_block`)** — leak needs `OpCopyRecord` (`b = a`) + `{ }`
  block-RHS; WITHOUT the copy it is clean; mutation is NOT required. So `OpCopyRecord`'s
  scope-exit free does not recurse into the copy's embedded text. Matches the test's own
  `src.contains("OpCopyRecord(cell,")` assertion.

## p54_b6 — view-through-forward-borrow (regrouped out of "UserCall")

**Confirmed:** `g5b_view_through_forward_borrow` leaks 1. `extract(p){match p.b { 0 =>
p.a, _ => "other" }}` returns a VIEW of its ARGUMENT `p.a`, so `run(){extract(local)}`
classifies `ForwardArg` and correctly does NOT promote. The leak is the delivery of a
view into a LOCAL passed as the arg — the same class as `n3` (composite embedded text),
NOT a user-call delivery. Belongs with Group 5, not the UserCall slice.

---

## Cross-group observations — now VERIFIED

- **VERIFIED: 1b + tuple(3) + p241 are ONE root (category A, 9 tests)** — the generic
  MONOMORPH text return-delivery, unpromoted. The `verify_*` probes show de-genericising
  each (or making the text a non-`T` literal) turns leak→clean; introspect shows the
  monomorph returns an unbuffered owned `String`. One fix (promote monomorph text
  returns / propagate the template's buffer through instantiation) closes all 9.
- **REFUTED: the "composite-embedded-text" super-merge.** The earlier guess that
  tuple(3) + vector(4) + n3 share a composite-free root does NOT hold. `issue_437` (C)
  and `n3` (D) are NON-generic and stay clean when de-composited by other means; only
  the GENERIC ones merge. Composite-ness was incidental; generic-monomorph is the real
  shared axis.
- **B/C/D/E/F verified distinct and non-generic** — fn-ref ABI (B), the `?? ""` vector
  element copy (C), `OpCopyRecord` embedded text (D), forward-borrow view (E), and pure
  UserCall (F). Each needs its own fix; none folds into A.
- **Nothing here is fixed or consumed.** Each probe is a ready fixture; the next step
  per category is loft-codegen's "prove the working bytecode first" against these repros.
  Start with A (9 tests, one fix, highest leverage).
