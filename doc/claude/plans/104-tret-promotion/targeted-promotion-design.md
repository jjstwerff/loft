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
