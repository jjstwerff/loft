<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — the text-return SIGNATURE PRE-PASS (spec)

The last enabler for the 19 residual text leaks (§ `text-tail-return-leak.md`).
> **CORRECTION (2026-07-09, after an implementation attempt — see
> `residual-19-fix-plan.md` § Slice 1).** The claim below that the instability is
> caused by forward references "**not** by the tail being lowered to a work `Var`
> on pass 1" is WRONG — it overgeneralized from simple probes. BOTH mechanisms are
> real: forward-ref → `Plain` on pass 1 (simple cases), AND pass-1 call
> decomposition to a work `Var` (complex bodies — the viewer's `page_landing`). A
> pre-pass that reads the PASS-1 body tail therefore cannot guarantee agreement
> with pass-2's decision. The correct design keys on a PASS-STABLE source (the
> callee's declared signature over a normalized tail), or makes pass-2's classifier
> the sole decider the pre-pass replicates exactly. The rest of this spec (the
> invariant, the H5 slot, the delivery machinery) stands; only the "forward-ref is
> the SOLE cause" framing is superseded.

This spec is grounded in a probe run 2026-07-09. Forward references are ONE cause
of the pass-instability (see the correction above for the other).

## The invariant (the one thing)

> For every text-returning fn, whether it carries a hidden `&text` return buffer
> (its ABI) is DECIDED once — before pass-2 codegen begins — and is IDENTICAL for
> every observer, regardless of source order.

Equivalently: `wants_tret_bind(classify_text_return(tail))` must be a function of
the fn alone, not of *when* it is asked (which pass, before or after the callee is
seen). Today it is not, and the violation is exactly the leak-vs-crash boundary.

## Evidence — the verdict is pass-UNSTABLE only across a forward reference

Instrumented `classify_text_return` to fire on BOTH passes (a throwaway edit to the
`LOFT_TRA_DUMP` block in `parse_block`; reverted). Verdicts per fn:

| fn (tail shape) | callee defined | pass 1 | pass 2 | stable? |
|---|---|---|---|---|
| `user_tail { inner() }` (user call) | **before** | `UserCall` | `UserCall` | ✅ |
| `caller_first { callee_later() }` (user call) | **after** | `Plain` | `UserCall` | ❌ |
| `run_generic { first_g(nums) }` (generic user call) | after | `Plain` | `UserCall` | ❌ |
| `callee_later { u.to_json() }` (native call) | — | `BuiltLocal` | `NativeCall` | ❌ |
| `n_first_g { v[0].to_text() }` (generic method) | before | `UserCall` | `UserCall` | ✅ |
| `n_show { (x.to_text(), "m") }` (tuple) | before | `TupleElement` | `TupleElement` | ✅ |
| `fref_tail { f(1) }` (fn-ref) | — | `Plain` | `FnRefCall` | ❌ |
| `fwd_borrow { second(s) }` (arg forward) | before | `ForwardArg` | `ForwardArg` | ✅ (p281 guard holds) |

Two distinct pass-1 blindnesses produce the ❌ rows:

1. **Forward reference** (`caller_first`, `run_generic`, `fref_tail`). When the
   caller is parsed mid-pass-1, the callee's signature (return type + return-deps,
   or the fn-ref var's resolved fn type) is **not yet known**, so the call tail
   can't be recognised as a text call → `Plain`. At the **end** of pass 1 all
   signatures are known, so re-classifying resolves it to `UserCall`/`FnRefCall`.
   *This is the dominant cause and it is fully resolvable by a pre-pass.*
2. **Native-wrap** (`callee_later`). A native text-dest tail is wrapped into a
   `__work_N` buffer by `wrap_value_text_dest` (@PLN10) on **pass 2 only** (H5
   note, `parser/mod.rs:1188`: "`__work_N` — a text-return work-buffer promotion
   the pass-1 classify could not yet see"), so pass 1 sees `BuiltLocal`. This is
   NOT a forward-ref gap; it is a separate pass-2-derived wrap. It is **latent**
   for native tails today (2d promotes them pass-2-only; the suite has no
   forward-ref-to-native-tail, so it never crashes) — but a probe triggers it:

```
fn caller_first() -> text { callee_later() }      # caller BEFORE callee
fn callee_later() -> text { u = U{..}; u.to_json() }
# → panic: "Too few parameters on n_callee_later (got 0, need 1)"  (BOTH backends)
```

The same crash fires if `UserCall` is naïvely added to `wants_tret_bind` and a
user-call callee is forward-referenced. A **backward** reference with `UserCall`
enabled runs clean on both backends — proving the crash is purely the ordering,
not the promotion.

## Root cause (the chokepoint)

The **H5 two-pass contract** (`parser/mod.rs:1089`) snapshots every def's
attribute count at the end of pass 1 (`pass1_attr_counts`, line 1096) and asserts
pass 2 reproduces it — but it carves out an exception for a trailing `__work_`
append (line 1196). That tolerated divergence is the bug: a promoted fn gets its
`&text` buffer attribute added during its **pass-2 body parse**, which for a
forward-referenced callee happens AFTER its caller has already emitted a
buffer-less call (`state/codegen.rs:2727`, "Too few parameters").

The fix is to move the buffer-attribute decision to a single point where the whole
signature table is known AND which lands INSIDE the pass-1 snapshot.

## The design — a pre-pass between pass 1 and pass 2

Slot a whole-program walk into `Parser::parse` (`parser/mod.rs`) **after**
`resolve_deferred_unknowns()` (line 1088) and **before** the `pass1_attr_counts`
snapshot (line 1096). At that point pass-1 bodies are parsed and every signature is
final, so:

```rust
self.parse_file();
self.resolve_deferred_unknowns();
self.promote_text_return_signatures();   // ← NEW pre-pass
#[cfg(debug_assertions)]
let pass1_attr_counts = …;               // now SEES the promoted buffers
```

`promote_text_return_signatures` does, for every def that is a text-returning fn:

1. **Classify the raw return tail** with the existing `classify_text_return`
   selector — now with all signatures resolved, so a forward-referenced call tail
   classifies identically to how pass 2 will see it (`Plain → UserCall`
   resolved). This reuses the framework verdict verbatim; no new classifier.
2. If the verdict `wants_tret_bind()` and the fn has **no** `RefVar(Text)` hidden
   attribute yet, **stamp one** (the same `add_attribute` + `hidden=true` +
   return-dep the `PromoteHidden` arm of `text_return` emits, `control.rs:4830`).

Because the stamp lands before the snapshot, the buffer is a **pass-1 attribute**;
pass 2 reproduces it (H5 clean, no `__work_` exception needed for these), and every
caller — forward or backward — reads the final arity. Pass-2 body parse then
proceeds unchanged: `do_tret_bind` still fires per-fn to rewrite the body
(`Set(__tret, call); __tret`), and `text_return`'s `Attr` arm (`control.rs:4798`)
re-applies `RefVar` to the already-present attribute — the exact path 2d/3a already
rely on. The decision (pre-pass, whole-program) is thus decoupled from the
mechanism (per-pass body bind), which is what makes it pass-stable.

### Why no fixpoint is needed

A caller's `UserCall`-vs-`ForwardArg` verdict reads the callee's
`returned().depend()` — whether the return borrows only hidden attrs (owned) or a
visible argument (forward-borrow). Promoting a callee only *adds* a hidden buffer
attr; it never flips an owned return to a borrow or vice-versa. So the classify is
monotone under promotion — one ordered walk suffices; no iterate-to-convergence.
(Confirm with a debug assert: a second pre-pass walk stamps nothing new.)

## Scope — what this unlocks, honestly

| Group | Tests | Pre-pass alone? |
|---|---|---|
| **UserCall** — bare user-fn call tails, generic `x.to_text()` monomorph, p54 call/interp arms | `plan17_b`, `plan17_printable`, `p243`, `p54_or_pattern`, `p54_b6`, `p54_extractors_spec`, `p54_multi_call_flow` (**7**) | **Yes** — delivery already built (3b), only the forward-ref decision was missing |
| **NativeCall latent** — forward-ref-to-native-tail (not in the 19, but a real correctness bug the probe triggers) | — | **Yes** — same pre-pass, if it also reads the raw (pre-wrap) native tail |
| **FnRefCall** | `p227` ×4 | **Partly** — pre-pass fixes the decision, but pass 1 can't type the fn-ref var (`Plain`), and the adaptive @P387 buffer ABI (`text_return` lambda logic, `control.rs:4861`) must be re-verified through the pre-pass |
| **TupleElement** | `p329` ×3, `p330` ×2 | **No** — the fn returns a TUPLE, so `do_tret_bind` (which gates on `result.base() == Text`) never fires; the leak is the caller's `show(...).0` element-extraction copy, a different delivery |
| **vector / view return** | `issue_437`, `n3`, `p241` | **No** — vector-of-text / field-view return copies, unrelated to the tail bind |

So the pre-pass is the **highest-leverage single move**: ~7 leakers closed
outright + the latent native forward-ref bug retired, and it is the prerequisite
that makes `FnRefCall` tractable. `TupleElement` (5) and the vector/view returns
(3) are genuinely separate deliveries and must not be forced onto this fix.

## Validation gate (loft-codegen — prove the working bytecode FIRST)

1. **Working bytecode, both backends, before any edit.** The proven-clean target
   is the backward-ref rebind (`caller_later { callee_first() }`, callee first) —
   already 0-leak + correct on interpret AND native (verified in this session).
   Capture its `loft introspect`; the forward-ref version must emit the SAME
   signature/ABI after the fix. Save the pair under `bytecode-comparisons/`.
2. **Forward-ref regression is the load-bearing cell.** Add `caller_first` /
   `callee_later` (caller BEFORE callee) for user-call AND native tails to the
   matrix — each panics `Too few parameters` pre-fix, must be clean+correct
   post-fix on both backends.
3. **p281 / arg-forward guard must stay a borrow.** `fwd_borrow`/`second` classify
   `ForwardArg`/`Argument` on both passes (stable) — the pre-pass must not promote
   them (codegen "Too few parameters" in the OTHER direction). Keep them in the
   corpus.
4. **H5 clean.** With the stamp landing before the snapshot, the promoted buffers
   are pass-1 attributes — `assert_pass2_def_attr_stable` must pass with NO new
   reliance on the `__work_` exception for the UserCall group. Run debug (the
   assert is `cfg(debug_assertions)`).
5. **Full gate.** `framework/verify.sh` 24/24 (verdicts unchanged — the classifier
   is reused, not altered); `issues` 749/0; fn-ref (`p227`) + par (`p235`)
   unregressed; the ASan owner sweep (`sweep_owners.sh`) drops the UserCall group
   from the 19. Both backends throughout.

## Decision points for the implementer

- **Does pass-1 body parse still promote backward-ref UserCalls, or does ONLY the
  pre-pass stamp?** Cleanest is to let the pre-pass be the single source of the
  buffer attr (pass-1 body `do_tret_bind` then only rewrites the body, never adds
  an attr) — but confirm pass-1 `text_return` doesn't add the attr first for the
  backward case (it does today via the `PromoteHidden` arm). If it does, the
  pre-pass stamp must be idempotent (name-keyed `add_attribute` already is), so
  both-sources is safe but the pre-pass-only path is tidier.
- **Native raw-tail reading.** To also retire the latent native bug (item 2),
  the pre-pass must classify the tail BEFORE `wrap_value_text_dest`'s pass-2 wrap —
  i.e. recognise the bare `u.to_json()` call as `NativeCall` at end of pass 1. If
  that raw form isn't available post-pass-1, scope the pre-pass to
  `UserCall`/`FnRefCall` (the 19) and leave native as-is (still latent, still
  untriggered by the suite).
- **FnRefCall pass-1 typing.** Verify whether the fn-ref var's fn type is resolved
  by end of pass 1 (the `f(1)` → `CallRef` recognition). If not, `p227` needs the
  fn-ref typing resolved earlier or a dedicated FnRefCall stamp — a follow-on
  slice, not a blocker for the UserCall group.
