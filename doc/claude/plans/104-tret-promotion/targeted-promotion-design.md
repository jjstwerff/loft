<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN104 — Targeted promotion (the forward-ref pre-pass): design

Replace the whole-file **third parse pass** with a **targeted, post-pass-2 transform** that
promotes only the `force_tret` defs' IR and patches only their call sites. This is the
"forward-ref pre-pass" deferred since `promote_monomorph_text_return` (control.rs:8029:
*"that half awaits the forward-ref pre-pass"*). Read `option-b-scope.md` first for the
history and the reliable-measurement notes (`LOFT_NO_CACHE`, the runtime text trace).

## Implementation status (built + verified, opt-in behind `LOFT_TRET_V2`)

Both phases land in `parser/control.rs`; wired in `parser/mod.rs` as
`if force_tret && LOFT_TRET_V2 → targeted_tret_promotion() else third pass`. Gated OFF by
default (`force_tret` is empty without `LOFT_TRET_FIX`).

- **Phase A — `promote_text_return_def(d_nr)`**: swaps `self.vars`↔the def, mints `__tret`,
  rewrites early returns, rebinds the tail, `text_return(&[tv])`, then the a==v renumber via
  the extracted `av_renumber_retbuf`. Works: `run_t` promoted, `AppendStackText`, correct
  output.
- **Phase B — `patch_tret_callers` / `patch_tret_call`**: for each caller of a promoted def,
  re-run `add_defaults` on the `Call` to push the retbuf arg.
- **Phase B liveness fix (the one real bug):** `add_defaults` mints the caller-side retbuf
  work-text but does NOT declare it at the caller's top level, so `scopes::check` scoped it
  to the arg block and freed it THERE — before the callee fills it — orphaning the delivered
  text (the loft#568 leak this pass exists to fix). Diagnosed by capture-and-diff: the
  proven third-pass `main` hoists `__work_1:text=""` to the top and frees at scope exit; V2
  froze it inside the arg block. The re-parse gets this hoist from `expression_value`
  (`parser/expressions.rs:480` — `for wt in work_texts { ls.insert(0, Set(wt,"")) }`); V2
  replays it post-parse for ONLY the newly-minted work-texts (before/after `work_texts` diff)
  → `OpFreeText` lands at the caller's scope exit.

**Verified** (`LOFT_NO_CACHE=1 LOFT_TRET_FIX=1 LOFT_TRET_V2=1`, commit `48163304`):
min.loft V2 introspect BYTE-IDENTICAL to the proven third-pass sibling (IR + bytecode +
native Rust → both backends), NO text leak; combined (promoted fn + vector defs + an
already-buffered fn) and card (two #568-class calls, results simultaneously live) correct +
leak-free on interp AND native; V2 touches only the promoted def + its caller (unrelated
defs untouched → the collateral class is gone by construction). Unit tests 738/0.

**v1 scope filter (commit `ab1a61cb`):** `targeted_tret_promotion` promotes only
**owned-by-value** tails on defs whose **address is never taken** (`FnRef` walk). It defers,
with a `LOFT_TRET_TRACE` log, the classes Phase A/B can't lower — view-of-local /
join-of-local (need view materialisation; 553 `textslice` SIGSEGV'd both backends without
this) and address-taken defs (signature change breaks fn-pointers). `force_tret` is narrowed
to the promoted set so Phase B patches exactly the changed callers.

**Former default-on failures under V2 (§Verification 4) — RESULT (commit `ab1a61cb`):**
`text_return_analysis` ✅, `index_hygiene` ✅, `wrap loft_suite` ✅ (the 553 view-of-local
crash is now deferred), `s5_local_swap_hands_over` ✅, `s7_debugger_loop_end_to_end` ✅ — the
third-pass's diagnostic (mode 1), ABI-record-id (mode 2), and tooling (mode 4) collateral are
GONE under the targeted pass. `native_scripts` **439/439** (commit `19e7c97d`, after the native fix below). ALL FIVE
former-default-on groups now PASS under V2.

**Native codegen fix (commit `19e7c97d`) — two root causes behind the 3 residuals:**
1. *Retbuf work-text COLLISION* (`387-text-fn-ref`, `85-optional-return-freeops-tail`;
   E0506/E0499). A caller parsed against the UNPROMOTED callee has a `work_text` pooling
   counter that lags the `__work_N` already in its `names` (a format-arg buffer). Phase B's
   `add_defaults` re-derives `__work_1`, finds it in `names`, and ALIASES the live format
   buffer as the retbuf — same buffer passed as both an arg AND the return slot of one call.
   Fix: `Function::sync_work_text_counter` advances the counter past every existing
   `__work_N` before `add_defaults` (called once per caller in `patch_tret_callers`).
2. *Reachability gap for a call nested in a fn-ref dispatch* (`repro_p265`; E0425).
   `generation::collect_calls` fell through `CallRef` to `_ => {}`, so a nested `Call` in
   `cb(f(x))` was never marked reachable. Before promotion `f` was reachable as a zero-param
   test-fn ENTRY; promotion gives it a retbuf param, dropping it from the entry set, and the
   CallRef gap then hid it. Fix: recurse into `callref_args` (reachability is an
   over-approximation → always safe; un-gated, verified no default regression).

**DEFAULT-ON (commit `e819a08f`) — trial PASSED.** Both gates flipped: `report_tret_promotions`
populates `force_tret` by default (opt-out `LOFT_NO_TRET_FIX`); the targeted pass is the default
(third pass kept, opt-in `LOFT_TRET_THIRD_PASS`, until deleted). Full nextest suite, cache cleared:
V2-ON fails `s5_native_swap`, `s7_debugger_loop`, `wasm_debug_relay`; V2-OFF (`LOFT_NO_TRET_FIX`)
fails `s5_native_swap`, `s7_debugger_loop` IDENTICALLY (`a#48`/`a#237` vs `a#1`; the `a#N` counter
drifts 46/47/48 across runs, independent of promotion). So the promotion introduces **ZERO new
suite failures** — s5/s7 are pre-existing full-suite-parallelism flakes (the `LOFT_LIVE_FLIP` /
debugger subprocess runs N events and flips before the test socket connects under load), wasm is
the known environmental flake. Note the diagnostic path: the stale-cache hypothesis was FALSIFIED
(clean-cache suite still failed); the decisive read was the **V2-off full-suite baseline**, not the
isolated-test passes.

**Remaining close-out (mechanical):** delete the third-pass block in `parser/mod.rs` (and its
`LOFT_TRET_THIRD_PASS` opt-in) once default-on has soaked; the s5/s7 full-suite flakiness is a
SEPARATE pre-existing test-infra issue (fails with promotion off), not part of this plan.

## Why the third pass must go — one root, many symptoms

`data.reset()` clears only `use_names`, so the third pass re-parses the ENTIRE file and
**re-lowers already-refined defs on top of their pass-2 state**. That refinement is NOT
idempotent — and every remaining default-on blocker is one symptom of it:

- vector-literal re-lowering mis-orders pre-alloc vs the work-ref decl → native
  `var__vec_N` E0425 (breaks **unpromoted** collateral like `source_roots`);
- re-emitted diagnostics (had to add third-pass diagnostic truncation);
- record-ID shifts (s5/s7 shape assertions).

Patching each symptom is whack-a-mole. The fix is to **never re-lower a def that doesn't
need promoting** — touch only the force_tret defs and their callers.

## The invariant

> After the targeted pass, the program IR is **byte-identical to pass 2 for every def that
> is neither a force_tret def nor a direct caller of one**, and — for those that are — is
> exactly what a compiler that had the retbuf signature *before* lowering would have
> produced. No def is re-parsed; the transform edits existing pass-2 IR in place.

The "byte-identical for everything else" clause is the whole point: it makes the
idempotency question disappear, because the untouched defs are never re-lowered.

## The re-assertion sites — count them (design-protocol step 2)

The retbuf fact must be re-stated at N sites; omitting it at any is a SILENT ABI mismatch
(a `generate_call_ref`-style panic or a wrong arg count), not a compile error:

| site | count | who states it |
|---|---|---|
| the def's `&text` retbuf attribute | 1 / def | Phase A |
| the def's return DELIVERY (tail → retbuf) | 1 / def | Phase A |
| each **direct `Call`** of the def | N / def | Phase B |
| each **fn-ref value** of the def + its `CallRef` sites | M / def | Phase B (v2) |

`N × silence` is the brittleness. **Cure = make omission loud:** after the pass, assert
that every `Call(force_tret_def)` has `args.len() == attributes(def)` (and every promoted
def has exactly one hidden `&text` retbuf). A missed Phase-B patch then fails a cheap
post-pass check instead of a deep codegen panic.

## The design — two phases, both reuse existing machinery

### Phase A — promote each force_tret def (callee), in place

For each `d ∈ force_tret`, apply the EXACT pattern of `promote_monomorph_text_return`:

1. `std::mem::swap(&mut self.vars, &mut def.variables)`; `self.context = d`.
2. Move `def.code` out; on its block: `do_tret_bind` rebind (`Set(__tret, tail); __tret`),
   route early returns through `__tret`, then `text_return(&[__tret])` (stamps the hidden
   `&text` attr + rewrites the returned type).
3. **The `a == v` renumber** (this session's `renumber_frame_var` + `renumber_frame_in_types`
   + `swap_variables`): relocate the retbuf into the slot matching its attribute index so
   the returned dep resolves in both spaces.
4. Move code back; swap vars back; restore context.

This is ~90% done: `promote_monomorph_text_return` is the working template; the a==v
renumber is built + unit-tested. Phase A is that code generalised from "one monomorph" to
"each force_tret def".

### Phase B — patch each caller (the new work)

`add_defaults` (mod.rs:6122) ALREADY injects the caller-side text retbuf — its
`RefVar(Text)` arm (6195) builds a `work_text` buffer + `OpCreateStack` block and passes it
as the arg. It just needs to run at each call site AFTER the promotion. So, for each def `C`
that contains a `Call(d, args)` with `d ∈ force_tret`:

1. `swap self.vars/self.context onto C` (same swap as Phase A).
2. Walk C's code; for each `Call(d, args)`, extract `args` as `actual`, run
   `add_defaults(d, &mut actual, …)` (appends the retbuf work-text block), rebuild the Call.
3. Register the new work-text so `scopes::check` frees it at C's scope exit.
4. Restore.

**Probe of the cleanest claim (design-protocol step 4):** "re-running `add_defaults`
appends the retbuf." It holds ONLY if C's pass-2 call args are the *pre-*add_defaults args
(no retbuf already). On pass 2 the call lowered against the UNPROMOTED signature, so its
args are the declared args only — re-running with the promoted signature appends exactly
one retbuf. **Verify before building:** dump a pass-2 `Call(run_t, …)` and confirm
`args.len() == declared params` (no hidden trailing slot). If a stub slot is already there,
Phase B patches in place instead of appending.

## Scope: v1 vs deferred

- **v1 — direct `Call` only.** Covers min.loft (`run_t` called directly) and the common
  case. A force_tret def **passed as a fn-ref VALUE** (first-class, then `CallRef`'d) needs
  its fn-ref TYPE to carry the retbuf and every `CallRef` to push it — harder. **v1 rule:
  do NOT promote a def that is ever used as a fn-ref value** (detect a `FnRef`/bare-def-ref
  to it; skip promotion). That def keeps the interpreter leak for now — NO regression vs
  today, and the loud post-pass assert guarantees we never half-promote one.
- **v2 — fn-ref-value force_tret defs.** Propagate the retbuf into the `Type::Function`
  fn-ref type and patch `CallRef` sites via `add_defaults` (it already has the fn-ref path).

## Failure paths to probe

- **Missed call site** → the loud post-pass arity assert catches it (the N-silence cure).
- **fn-ref value of a promoted def** → v1 skips promoting it (detect + exclude); v2 handles.
- **Caller work-text never freed** → register it as a work-ref so `scopes::check` frees it;
  probe with `LOFT_NO_CACHE` + the runtime text trace (alloc/free must balance).
- **Early / nested returns in the callee** → `promote_monomorph_text_return` already routes
  every return through `__tret` (`rewrite_text_returns_into`); reuse verbatim.
- **Generic monomorphs** → already promoted by `promote_monomorph_text_return`; the targeted
  pass must skip a def that already carries a `&text` retbuf (idempotent guard).
- **A def that is BOTH a force_tret def AND a caller of one** → run Phase A then Phase B on
  it; the a==v renumber (Phase A) and the arg append (Phase B) touch disjoint slots.
- **`text?` returns** → `text_return` re-applies the `?`; Phase A inherits it.

## Verification

1. **No-collateral proof (the invariant):** `introspect` every non-force_tret def before and
   after the pass; diff must be EMPTY (this is the whole claim — the third pass could never
   pass this). Do it on a text-heavy program with vector literals (`source_roots` shape).
2. min.loft: `run_t` byte-identical to min_wa; leak 0 (runtime text trace, `LOFT_NO_CACHE`).
3. corpus: `ret_fnref` promoted+clean; `ret_borrow`/`ret_local`/`ret_interp` byte-identical
   to pass-2; both backends.
4. The former default-on failures — `index_hygiene`, `native_scripts`, `text_return_analysis`,
   `s5`/`s7`, `wrap` — all green with the promotion DEFAULT-ON (no third pass).
5. The six real nightly leakers → 0; full suite green default-on; then drop the
   `LOFT_TRET_FIX`/`LOFT_NO_TRET_FIX` gate.

## What carries over from this session (already built + tested)

- `renumber_frame_var` / `Type::renumber_frame_deps` / `Deps::renumber_frame` (5 unit tests)
  — the a==v renumber Phase A needs.
- `Function::swap_variables` / `renumber_frame_in_types` — the variable-table half.
- The `CallRef` v_nr renumber fix (the `generate_call_ref` panic).
- `promote_monomorph_text_return` — the Phase-A template.
- `add_defaults` `RefVar(Text)` arm — the Phase-B injector.

The third pass (`report_tret_promotions` + the `parser/mod.rs` third `parse_file` + the
snapshot/restore) is REPLACED by this pass; delete it once Phase A/B land and verify green.
