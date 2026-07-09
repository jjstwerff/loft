<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — steps to close all 19 residual text leaks

Companion to [`signature-pre-pass-spec.md`](signature-pre-pass-spec.md) (the
foundation) and [`text-tail-return-leak.md`](text-tail-return-leak.md) (the arc).
All 19 share ONE owner — `loft::fill::append_text ← State::execute_argv` — an owned
text COPIED at a call/return boundary whose source is never freed. They split into
five slices by the DELIVERY each needs. Ordered by leverage; each slice is a
self-contained loft-codegen pass (probe → working bytecode → types → code →
both-backend gate), landable and committable on its own.

Baseline sweep command (the oracle, re-run after every slice):
```
BIN=target/aarch64-apple-darwin/release/deps/issues-<hash> \
  probes/text-tail-return/sweep_owners.sh
```
(19 leakers today; the target is 0. Rebuild the ASan `issues` test binary per
`mac_sanitizer_toolchain` memory.)

The 19, by slice:

| slice | tests | count |
|---|---|---|
| 0+1 UserCall (pre-pass) | `plan17_b`, `plan17_printable`, `p243`, `p54_or_pattern`, `p54_b6`, `p54_extractors_spec`, `p54_multi_call_flow` | 7 |
| 2 FnRefCall | `p227` ×4 | 4 |
| 3 Tuple return-delivery | `p329` ×3, `p330` ×2 | 5 |
| 4 vector element-copy | `issue_437` | 1 |
| 5 eval-wrapper / copy-record | `n3`, `p241` | 2 |

---

## Slice 0 — the signature pre-pass (foundation; unblocks 1, 2)

**Goal.** Decide every text-returning fn's `&text`-buffer ABI once, before pass-2
codegen, so a forward-referenced callee's buffer is in its signature before any
caller emits the call. Full spec: `signature-pre-pass-spec.md`.

1. **Working bytecode first.** The proven-clean target is the *backward*-ref rebind
   (`fn callee_first()->text{…u.to_json()} ; fn caller_later()->text{callee_first()}`)
   — verified 0-leak + correct on both backends this session. Capture its
   `loft introspect` signature/ABI; the forward-ref version must emit the identical
   signature after the fix. Save the pair under `bytecode-comparisons/`.
2. **Add `Parser::promote_text_return_signatures`** and call it in `Parser::parse`
   (`parser/mod.rs`) AFTER `resolve_deferred_unknowns()` (≈line 1088) and BEFORE the
   `pass1_attr_counts` snapshot (≈line 1096), so the stamped buffers are pass-1
   attributes and H5 (`assert_pass2_def_attr_stable`) stays clean.
3. **Body of the pre-pass.** For every def that is a text-returning fn: classify its
   return tail with the existing `classify_text_return` selector (now all signatures
   are resolved, so a forward-ref call tail resolves to `UserCall`/`FnRefCall`
   instead of pass-1 `Plain`); if the verdict `wants_tret_bind()` and the fn has no
   `RefVar(Text)` hidden attribute yet, stamp one via the same trio the
   `PromoteHidden` arm uses (`control.rs:4830`: `add_attribute` + `hidden=true` +
   push the return-dep).
4. **Types.** No new type fact — the pre-pass reuses `classify_text_return` and the
   existing `RefVar(Text)` hidden-attribute representation. The only shift is WHEN
   the attribute is stamped (end of pass 1, not pass-2 body parse).
5. **Idempotence + monotonicity guards.** `add_attribute` is name-keyed so a re-stamp
   is a no-op — safe against pass-2 body-parse re-adding it. A second pre-pass walk
   must stamp nothing new (debug-assert: promotion never flips owned↔borrow, so one
   ordered walk suffices — no fixpoint).
6. **Gate.** The two forward-ref regression cells (user-call AND native, caller
   BEFORE callee) go `Too few parameters` → clean+correct on both backends; the
   p281 borrow guard (`fwd_borrow`/`second`, stable `ForwardArg`/`Argument`) stays
   unpromoted; H5 assert green in a debug build; `framework/verify.sh` 24/24
   (verdicts unchanged); `issues` 749/0 both backends.

## Slice 1c — BACKWARD-REF UserCall promotion — LANDS (working, validated, uncommitted)

The deep-single-case insight that unblocked it: backward-vs-forward reference is the
pass-stability axis, and it has an exact, pass-stable signal — **definition-number
order** (a callee defined earlier has a smaller def_nr, stable across passes per H5;
verified: backward `callee 631 < caller 632`, forward `page_landing 631 > route 630`).

**Fix** (`control.rs`, 45 lines): add `UserCall` to `wants_tret_bind`, and gate it in
a new `tret_bind_ok` so a `UserCall` tail promotes ONLY when its callee's def_nr <
`self.context` (a backward ref → `UserCall` on both passes → pass-stable). A
forward-ref callee (or a generic monomorph, whose def_nr is minted pass-2 and so reads
as forward) is left unpromoted — leak, but no crash, by construction.

**Validated:** ASan sweep **19 → 16** (`p54_or_pattern`, `p54_struct_enum_extractors_spec`,
`p54_struct_enum_multi_call_flow` cleared); backward-ref promotes clean on BOTH backends;
forward triple-chain + the markdown VIEWER do NOT crash (0 "Too few parameters" — the
attempt-1 regression is avoided by construction); full suite GREEN (no fail/crash/panic);
`issues` 749/0; all `p281` forward-borrow guards hold. Not yet committed.

The remaining 16 are the forward-ref / generic / fn-ref / non-`__ret_N` cases that still
need the signature pre-pass (generic monomorphs, forward user-calls) or their own
delivery (fn-ref, tuple, block-RHS, `?? ""`, view). This slice takes the safe,
pass-stable subset without the pre-pass.

## Slice 1 — enable UserCall promotion (rides on slice 0) — ATTEMPTED, REVERTED

**Attempt 1 (2026-07-09, reverted).** `wants_tret_bind` += `UserCall` + an
end-of-pass-1 pre-pass (`promote_forward_ref_text_returns`) that read each fn's
PASS-1 `def.code` tail and stamped `___tret_1` when it classified `UserCall`.
Closed 3 of the 7 first grouped here (ASan sweep 19 → 16): `p54_or_pattern`,
`p54_struct_enum_extractors_spec`, `p54_struct_enum_multi_call_flow`. `issues`
749/0 both backends, framework 24/24, H5 green, p281 guards held.

**But it REGRESSED the full suite** — `viewer_markdown` crashed with the exact
`Too few parameters on n_page_landing (got 0, need 1)` the doc warned of.
**Root cause of the miss:** the pre-pass keyed on the PASS-1 body tail, but a
COMPLEX body (the viewer's `page_landing` — accumulator + `if/else` + a 2-arg
`page("loft-view", body)` tail) presents that tail DIFFERENTLY on pass 1 than pass
2 (pass 1 can decompose the call to a work `Var`). So the pre-pass's pass-1
decision did NOT equal pass-2's `do_tret_bind` decision → it under-stamped
`page_landing` → the buffer reverted to pass-2-only → the forward-ref caller (line
49, before the def) crashed. A minimal 1-arg repro did NOT reproduce it; only a
complex body does — which is why the early probes missed it.

**Attempt 2 (2026-07-09, reverted) — DEFINITIVE root cause via a minimal repro.**
Reproduced the crash in `bytecode-comparisons` shape `route → page_landing → page`
(each callee defined AFTER its caller — a triple forward chain). Instrumented the
pre-pass to dump each text-fn's `def.code` tail at end of pass 1:

```
PREPASS-scan route         tail=Var(0)    ← decomposed to a work var
PREPASS-scan page_landing  tail=Null      ← synthesized to Null (page unresolved in pass 1)
PREPASS-scan page          tail=Var(2)    ← BuiltLocal accumulator, decomposed
```

**So `def.code` at end of pass 1 is UNUSABLE for tail classification.** It is the
FROZEN pass-1 body: when each fn was parsed *during* pass 1, its forward-ref callee
was not yet resolved, so the tail was lowered to `Null` / a work `Var`. End-of-pass-1
signature knowledge does NOT retroactively re-shape an already-parsed body. So the
classifier stamps NONE of the forward-ref fns (they don't present a `Value::Call`
tail), while pass-2 re-parses them with full signatures and DOES promote → mismatch
→ the `page_landing` crash.

The bitter irony: attempt 1 fixed the 3 *backward*-ref user calls (whose pass-1 tail
happened to stay a classifiable `Call`) and missed EVERY *forward*-ref one — which is
the entire purpose of a pre-pass.

**Verdict: a pre-pass that reads the pass-1 body cannot work.** Promotion depends on
the tail SHAPE, which only exists correctly after re-parsing with full signatures —
i.e. in pass 2. Viable correct designs (each substantial, none a patch):
- **(A) Re-parse only the tail EXPRESSION** of each text-returning fn between passes
  with full signatures, decide promotion, stamp the buffer. Needs the parser to
  re-enter a single expression from saved source position (`code_position`) — lighter
  than a 3rd full pass, but real parser surgery.
- **(B) All-signatures-before-any-body pass 1.** Restructure pass 1 so every fn's
  return type is registered before any body is parsed; then forward-ref call tails
  resolve DURING pass-1 body parse and `do_tret_bind` fires pass-1 (buffer in the
  snapshot). Deepest change; touches the two-pass contract.
- **(C) Call-site buffer synthesis in pass-2 codegen.** At the "Too few parameters"
  site, if the callee's missing trailing params are hidden `&text` buffers, allocate
  and pass them. Localizes the fix to codegen but the caller must then own+free the
  buffer, interacting with its own return delivery — needs care.

Reverted per the loft-codegen stop-condition (regressed the suite). Repro saved:
`bytecode-comparisons/usercall-tail-FORWARD-broken.loft` (+ the `route/page_landing/
page` triple chain). Next session picks one of A/B/C and probes it FIRST.

**Attempt 3 (2026-07-09) — "enhance the detection" ruled out at the op level.** An
op-by-op dump of the pass-1 `def.code` (via a throwaway diag) shows the forward
calls are not merely decomposed — they are ERASED:

```
route:         op[1]=Set(0,Var(0))  page_landing() GONE   op[2]=Var(0)
page_landing:  op[3/5]=Call(228,…) (the body+= ops)        op[7]=Null   page(…) tail GONE
page:          ret=Text(Deps[2])   ← promoted (no forward-ref), buffer present
```

`page` (no forward-ref) is promoted and carries `Deps[2]`; `route`/`page_landing`
return `Text(Deps[])` with NO trace of their forward calls — dropped during pass-1
parse because the callee was not registered yet. **So there is NO pass-1 signal to
detect the promotion from — the detection algorithm cannot be "enhanced" over frozen
pass-1 state, because the input is gone.** The fix must relocate WHERE detection runs
(re-parse the tail with full sigs = A, or register all fn signatures before any body
so forward calls survive pass-1 = B), or synthesize at the call site (C). B is the
truest fix (it un-erases the forward call, and the EXISTING `do_tret_bind` then
promotes correctly on pass 1 with no pre-pass) but is the deepest; A is the most
contained. Recommend probing B's feasibility (a signature-only declaration pre-scan)
first, falling back to A.

**The other 4 first grouped as UserCall are NOT closed by this slice — they are two
distinct sub-arcs discovered while implementing:**

- **Generic `.to_text()` monomorph** (`plan17_b`, `plan17_printable`, `p243`) — the
  caller (`run`) promotes, but its callee is a GENERIC monomorph (`t_7integer_first`
  → returns a plain owned `String`, no `&text` buffer). Generic instantiation is
  **pass-2-only** (H5 note, `parser/mod.rs:1159`) and substitutes the template body
  rather than re-running `parse_block`, so the `__tret` promotion never fires for the
  monomorph. The end-of-pass-1 pre-pass cannot see it. → **Slice 1b** (generic-
  instantiation ABI: apply the text-return promotion to monomorphs, or promote the
  template so the substitution inherits the buffer).
- **View-through-forward-borrow** (`p54_b6`) — `extract(p) -> text { match p.b { 0 =>
  p.a, _ => "other" } }` returns a VIEW of its ARGUMENT (`p.a`), so `run { extract(
  local) }` classifies `ForwardArg` and correctly does NOT promote. The leak is the
  delivery of a view into a LOCAL passed as the arg — the same class as `n3` (slice 5
  / composite-embedded-text), NOT a user-call delivery. Regroup under slice 5.

## Slice 1b — generic `.to_text()` monomorph promotion (`plan17_b`, `plan17_printable`, `p243`)

The monomorph inherits the template's unpromoted signature. Probe first: does the
TEMPLATE (`first<T>`) classify as promotable, and can its promotion be stamped so
`instantiate` carries the `&text` buffer into each monomorph? Neighbourhood: the
generic instantiation path (`parse_call` pass-2 monomorphisation, `t_<LEN><Type>_<fn>`
mint) + `substitute_type`. Distinct ABI arc — its own probe + both-backend gate.

## Slice 2 — retire the latent native forward-ref bug (rides on slice 0)

**Closes:** no current harness test (the suite has no forward-ref-to-native-tail),
but a real correctness bug: a probe crashes it today
(`fn a()->text{b()} ; fn b()->text{u.to_json()}` → `Too few parameters on n_b`).

1. **Requirement:** slice 0's pre-pass must classify the RAW native tail as
   `NativeCall` at end of pass 1 — before `wrap_value_text_dest`'s pass-2 `__work_N`
   wrap makes it read `BuiltLocal`. If the raw form is unavailable post-pass-1,
   record that and scope the pre-pass to `UserCall`/`FnRefCall` only (native stays
   latent — no regression, since it is untriggered by the suite).
2. **Gate.** Add the native forward-ref cell to `bytecode-comparisons/` + the
   matrix so it can never silently return.

## Slice 3 — FnRefCall promotion (`p227` ×4)

**Closes:** `p227_text_fn_ref_local_call`, `_local_with_capture`, `_struct_field`,
`_struct_field_capture`.

1. **Probe first.** Fn-ref tails classify `Plain` on pass 1 (the fn-ref var's fn
   type isn't resolved yet) and `FnRefCall` on pass 2 — confirm whether the fn-ref
   var's type IS resolvable at end of pass 1 (when slice 0 runs). Instrument the
   pre-pass to log the fn-ref-tail verdict at that point.
2. **If resolvable:** slice 0's stamp covers it — add `OwnedVia::FnRefCall` to
   `wants_tret_bind` and verify the adaptive @P387 buffer ABI (`text_return`'s
   lambda work-buffer logic, `control.rs:4861`, and `fn_call_ref` hidden-buffer
   dispatch) delivers the buffer through the indirect call. The ABI is already
   "one hidden `&text` buffer per text-returning fn-ref" (P227 groundwork).
3. **If NOT resolvable at end of pass 1:** the fn-ref typing must be resolved
   earlier, or FnRefCall needs a dedicated stamp keyed on the fn-ref var's declared
   `fn(...)->text` type (available at declaration) rather than the call-site
   verdict. This is a follow-on within the slice.
4. **Gate.** All 4 `p227` drop from the sweep; par (`p235`) + fn-ref value tests
   unregressed; both backends. `p227` is the delicate ABI shape — verify capture
   (closure record + buffer are distinct hidden params, correct attribute slots).

## Slice 4 — vector-of-text element-copy delivery (`issue_437`)

**Closes:** `issue_437_explicit_vector_return_then_append_keeps_elements`.

1. **Isolated repro (verified this session):** the leak is the `ct` copy-helper —
   `fn ct(v)->vector<text>{ o=[]; for i {o += [v[i] ?? ""]} return o }`, then
   `xs = ct(src); xs += ["tags"]` leaks 1/call (the minimal `mk()` form does NOT
   leak — the leak is specifically the element-COPY-into-vector via `?? ""`, not the
   vector build or return). Save this as the probe.
2. **Chokepoint.** The `v[i] ?? ""` element copies a text into the result vector; on
   the vector's return-delivery + subsequent append, that copied element text is not
   freed. Neighbourhood: the vector return-delivery (`Delivery` /
   `materialize_vector_arms_into`, control.rs) + element-append into `vector<text>`
   (the `??`-defaulted element ownership).
3. **Working bytecode + types + gate** per loft-codegen. This is a vector-element
   ownership fix, distinct from the text-return promotion family — do NOT force it
   onto the tail bind. Value+length+leak oracle on both backends (a doubled vector
   reads leak-free — length is load-bearing here).

## Slice 5 — two isolated block-RHS delivery bugs (`n3`, `p241`)

**Closes:** `n3_reference_assignment_emits_copy_record`, `p241_singleton_text`.

**Isolated this session — NOT the eval-wrapper-return first guessed.** The harness
wraps every `.expr("X")` as `pub fn test() { test_value = {X}; assert(…); }`
(`tests/testing.rs:209`, `{{{}}}` → `{<expr>}`), so both leak in a
**block-expression-as-assignment-RHS** context. The named-fn RETURN forms of both
are clean (promotion handles them); the ASSIGNMENT-into-`test_value` path does not.
They are two DISTINCT bugs, each with a minimal repro (`append_text` owner, leak=1):

- **`p241` — generic `vector<T>` element delivered as a block-RHS.** Leaks:
  `fn s<T>(x:T)->vector<T>{o=[]; o+=[x]; return o} ; test_value = { s("hello")[0] }`.
  Clean if the fn is NON-generic, if the `{ }` braces are removed, or if split to
  `v = s(…); test_value = v[0]`. So the required axes are **generic monomorph +
  index + block-RHS** — a generic-instantiation `vector<text>` element-delivery gap
  (the copied element text isn't freed). Chokepoint neighbourhood: the monomorphised
  `vector<T>` element-index delivery into a block value.

- **`n3` — record-copy embedded text unfreed.** Leaks:
  `test_value = { a = Item{name:"hello"}; b = a; a.name }`. Clean WITHOUT the `b = a`
  copy (`{ a=…; a.name }` = 0) and clean if `b = a` is discarded with no view bind.
  Mutation (`b.name += …`) is NOT required. So the required axes are **`OpCopyRecord`
  (`b = a`) + block-RHS**: the copy `b`'s embedded `name` text is not freed at
  block-scope exit. Matches the test's own `src.contains("OpCopyRecord(cell,")`
  assertion. Chokepoint: `OpCopyRecord`'s scope-exit free must recurse into embedded
  texts.

**Steps** (per loft-codegen, each independent — smallest slice, do last):
1. The two minimal repros above ARE the probes — save them to
   `probes/text-tail-return/` (`p241_block_rhs.loft.tpl`, `n3_copy_block_rhs.loft.tpl`).
2. Prove the working bytecode for each (the clean sibling: non-generic for `p241`;
   no-copy for `n3`) and fix at the isolated chokepoint. Value+leak oracle, both
   backends. These do NOT share the tail-return promotion mechanism — do not force
   them onto slice 0.

---

## After all five — flip the @PLN54 S4 gate

With the sweep at 0 non-`ir_read` leakers on both backends:

1. **Graduate the probes** — add the forward-ref cells (slices 0/2), a `p227`
   fn-ref cell, a tuple-return cell, and the `ct` vector cell to @PLN54's
   `native-asan`/`asan` corpus (each leaks/crashes pre-fix → pins the class shut).
2. **Flip `miri.yml`** `asan` → `ASAN_OPTIONS: detect_leaks=1` +
   `LSAN_OPTIONS: suppressions=lsan_suppressions.txt` (one documented line for the
   intentional Class-1 `ir_read` `Box::leak`: `leak:read_block` +
   `leak:read_data_with`).
3. **Keep the runtime-owner-frame detector** as the standing assertion — the gate
   asserts "zero non-`ir_read` store-text leaks," so a NEW `_dest` fn or tail shape
   turns it red even with the `ir_read` line suppressed.
4. **S1 caveat:** a Mac cannot validate the Linux ASan runtime — confirm the frames
   are present on the ubuntu-x86_64 leg before landing the flip.

## Sequencing summary

Slice 0 is the foundation (unblocks 1+2 = 7 tests + the latent native bug). Do 0→1
first — biggest, cleanest win, delivery already built. Then 3 (FnRefCall, delicate
ABI), 4 (vector element, distinct arc), 5 (eval-wrapper residue, smallest). Each
lands independently with the sweep dropping its members; the S4 flip is gated on all
five reaching 0.
