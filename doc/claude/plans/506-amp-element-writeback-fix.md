<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft#506 — `&`-write-back to a computed lvalue: verifiable implementation plan

Tracker: [loft#506](https://github.com/loft-lang/loft/issues/506). Surfaced during @PLN87 P3.2.
This is the **safe-implementation recipe** for the deferred fix — each step has a runnable
check, matrix-validated on BOTH backends, gated. Follow the loft-codegen gate: capture the
WORKING bytecode before touching the generator.

> **✅ LANDED (2026-07-05) — the OWNED-COPY approach, matrix-clean on both backends.** The sound
> fix is exactly the "make `wv` OWN a copy for a write-back callee" branch flagged below, done
> at the right chokepoint (`scan_args`, keyed on the write-back fact so a field-mutation callee is
> untouched). For a computed-lvalue `&`-arg whose callee **whole-reassigns** the param, capture
> the element into a FRESH OWNED temp and store the result back after the call:
> ```
> OpDatabase(tmp, T); OpCopyRecord(items[i], tmp, T)   // preamble: tmp = an owned copy of the element
> setback(OpCreateStack(tmp), 42)                        // the WORKING local-var path (write-back hits tmp)
> OpCopyRecord(tmp, items[i], T)                         // postamble: copy tmp's new record back into the element
> ```
> The callee's write-back frees the displaced **copy** (`tmp`'s first record), never the element —
> the element's record is the stable backing and is never freed, so **repeat / loop write-backs
> stay sound**. `tmp`'s final record is freed at scope exit (`new_lift_var`). This dodges the
> reverted attempt's corruption (which copied INTO the element's own record, which the callee then
> freed). Impl: `scopes.rs::amp_writeback_owned_copy` + the postamble plumbing through `scan_args`
> and its `Value::Call` caller (non-void: scalar → slotted temp, heap → `new_lift_var`). Guard:
> `tests/scripts/87-amp-element-writeback.loft` (element + read-old-field + repeat + non-void +
> field-mutation-P160 + struct-field + loop). Full suite + poison clean, both backends. Issue kept
> OPEN (fix on `tuxedo-formal-compliance`, not yet on `main`).
>
> **Superseded note from the reverted first attempt (kept for the ownership lesson):** a store-back
> that copies the write-back's record INTO `items[i]`'s existing store CORRUPTS on the second
> write-back — the callee's `item = X` FREES the displaced old record via `_old_disp`, and for a
> computed lvalue that IS the element's record, so copy-into writes freed memory (worked ONCE by
> store-reuse luck). The owned-copy above is why the element's record must never be the callee's to
> free.

## The bug (one line)

A whole-binding `&`-write-back reaches the caller only when the argument is a **simple local
variable**. A **computed lvalue** argument (`items[i]`, `s.field`) silently drops the write-back
on both backends (`setback(items[1], 42); items[1].px` → `0`, expected `42`).

## The invariant to enforce

> A whole-binding write-back through a `&`-parameter reaches the caller's argument lvalue —
> **including a vector element or struct field** — never silently dropped, and **leak- and
> double-free-free**. Field mutation (P160) is unchanged.

## Root cause (localized)

`src/parser/mod.rs:2325–2332` — the call-arg `&`-coercion. A `Var` arg becomes
`OpCreateStack(a)` (a stack-ref to `a`'s own slot → write-back updates `a`, which then owns the
new record — the WORKING path). A computed lvalue becomes:
```
wv = orig            // orig = OpGetVector(items, stride, idx)  — a COPY of items[idx]'s DbRef
OpCreateStack(wv)    // stack-ref to wv's slot; wv is skip_free
```
The callee's write-back (`item = X`) reassigns **`wv`** (native: `&mut var___ref_1`); `items[idx]`
is never updated and no store-back is emitted. Reference (both backends):
`doc/claude/plans/90-copy-diagnostics/borrow-return/A1b-*.txt` are the same caller-side-store
class; capture `#506`'s own pair per Step 0.

## Design progress (2026-07-05) — emission + timing RESOLVED; one ownership question OPEN

Investigation resolved the two things that looked hard, and surfaced the real remaining risk:

- **Emission point (resolved).** The store-back goes in `scopes.rs::scan_args`, whose caller
  (`scopes.rs:865`) already assembles `Insert([preamble…, call])`. Add a symmetric **postamble**:
  `Insert([preamble…, (result-capturing) call, postamble…])`. `scan_args` already sees the
  computed-lvalue `&`-arg as `Insert([Set(wv, orig), OpCreateStack(wv)])` with
  `orig = OpGetVector(base, stride, idx)` / `OpGetField(base, fld)` — everything needed to build
  the store-back is in hand there.
- **Write-back fact + timing (resolved).** `scan_args` runs in the **post-parse** `scopes::check`
  pass, so EVERY callee is fully parsed → its `rebind_orig` write-back fact
  (`data.definitions[callee].variables.rebind_orig(param)`) is available. No forward-ref hazard.
  Emit the store-back **only for a write-back callee** (F3: a field-mutation callee mutates R in
  place, `wv == items[idx]`, so no store-back — leave it untouched).
- **OPEN — the vector-element-set + free ownership contract (the load-bearing risk).** `v[i] = x`
  *copies* (#338), not a DbRef transfer, and the callee's `FreeRefIfDistinct(displaced, witness)`
  (n_setback bytecode) already touches the old record. So the store-back must be pinned so that
  across {callee's write-back free} + {store-back} the old R is freed **exactly once**, R′ (which
  the callee leaves live — proven by the leak-clean local case) is **transferred, not leaked**,
  and the field-mutation cell stays a no-op. The choice — a **DbRef-transfer store** into the
  element slot (mirror how the local-var slot receives R′) vs a **copy + free-wv** — must be read
  off the vector-element-set/free contract and **validated by the F1/F2 leak+poison matrix**, not
  assumed. This is the piece that needs the element-set semantics pinned before the codegen edit.

## The fix design

At the **call site** (not the arg-coercion — the store-back must run *after* the call), for a
`&`-argument that is a **computed lvalue** AND whose callee **whole-reassigns** that parameter
(the `rebind_orig` / P2.1 write-back fact, `src/variables/mod.rs:259`), wrap the call:
```
{ Set(wv, orig); r = call(OpCreateStack(wv), …); <store-back(source, wv)>; r }
```
where `<store-back>` is `OpSetVector(base, idx, wv)` (element) or `OpSetField(base, fld, wv)`
(field), with `base`/`idx`/`fld` reconstructed from `orig`. The store-back frees the old record
once and transfers the new one to the source.

**Key ownership facts (CORRECTED 2026-07-05 after the reverted attempt):**
- **The callee FREES the displaced old record** (`_old_disp` free in the `&`-write-back) — for a
  computed-lvalue arg where `wv` aliases the element, that frees the ELEMENT's record `R_5`.  This
  is the corruption source; a copy-into-`R_5` store-back writes to freed memory.
- The callee leaves R′ live in `wv` (the write-back's new record) — verified.
- `wv` is `skip_free` (aliases the element).  So the sound fix must either give `wv` its OWN copy
  before the call (so the callee frees the copy, not the element) — gated on the write-back fact —
  or make the store-back a **DbRef repoint** of the element slot to R′, not a record copy.

## Failure modes to guard (enumerate before coding)

| # | failure | guard |
|---|---|---|
| F1 | **double-free** — old R freed by both the callee and the store-back | callee preserves R (witness == R); store-back is the only free of the old element |
| F2 | **leak** — R′ freed by the callee at exit before the store-back takes it | confirm (Step 0) the WORKING local case leaves R′ for the caller; mirror for the element |
| F3 | **P160 regression** — a field-mutation `&`-computed-arg call breaks | the fix keys on the write-back flag; field-mutation path (no rebind) is left untouched |
| F4 | **nested lvalue** — `s.rows[i]`, `m[k].field` — the store-back reconstruction is shallow | handle the base/index/field reconstruction to full depth, or reject the un-reconstructable case cleanly (loud, not silent) |
| F5 | **wide regression** — an owned shape that should copy now aliases | gate behind a flag; suite byte-identical with the gate OFF |

## Verifiable steps (each: run the check, on BOTH backends)

**Step 0 — the gate (capture working-vs-broken-vs-target).**
- WORKING: `a = Item{px:0}; setback(a, 42)` → `a.px == 42`, **leak-free** (`LOFT_STORES=warn`
  interp + `LOFT_NATIVE_LEAK_CHECK=1` native). `loft introspect` it.
- BROKEN: `items[1]` arg → `items[1].px == 0`. Introspect it.
- TARGET: hand-write the element bytecode = the working shape + the store-back; confirm by hand
  it frees R once and owns R′. Save the trio under a `bytecode-comparisons/` dir.
- **Check:** the diff is exactly the missing store-back + the ownership it implies — no more.

**Step 1 — pin the write-back-fact query.** Confirm `rebind_orig` (`variables/mod.rs`) answers
"does callee def D whole-reassign parameter P?" at the call site (pass 2 has the callee def).
- **Check:** a probe prints the flag TRUE for `setback` (reassigns) and FALSE for `bump`
  (`it.px = v`, field-mutation only). The flag alone must separate the two.

**Step 2 — usage sentinel for the broken shape.** At the call-lowering chokepoint, add a gated
`eprintln` that fires when: arg is a computed lvalue (`OpGetVector`/`OpGetField`, not `Var`) →
`&`-param → callee write-backs.
- **Check (positive control):** it fires on `setback(items[1], 42)`, is SILENT on the local-var
  case and on the field-mutation callee. Only then trust the detection.

**Step 3 — emit the store-back (the fix).** Wrap the call per the design; reconstruct
`base`/`idx`/`fld` from `orig`; emit `OpSetVector`/`OpSetField` after the call.
- **Check:** `setback(items[1], 42); items[1].px == 42` on interp AND native; `LOFT_POISON=1`
  clean; leak-free both backends.

**Step 4 — the boundary matrix.** Cells `{vector element, struct field, nested field} ×
{write-back callee, field-mutation callee} × {interp, native}`. Assert **value AND length AND
leak** per cell (not leak alone — a delivery that doubles reads leak-free).
- **Check:** write-back cells reach the source; field-mutation cells unchanged (P160); every
  cell leak-free + poison-clean on both backends.

**Step 5 — gate + graduate.** Gate the new store-back behind a flag if any matrix cell is
uncertain; prove the suite **byte-identical** with the gate OFF (`loft introspect` before/after).
Flip `tests/scripts/87-amp-element-writeback-limitation.loft` to the POSITIVE case (+ a
field-of-computed case). Run the full suite (`find_problems.sh --bg`) both backends; poison-clean.

**Step 6 — close.** Graduate the matrix to `tests/scripts/` + a `tests/leak_cases/` guard; close
loft#506; if any un-reconstructable nested case remains, keep it a **loud** rejection (F4), never
a silent drop.

## Do-not-ship conditions (revert, don't push through)

- Any matrix cell double-frees or leaks on either backend (F1/F2) → the ownership is wrong; revert.
- A field-mutation cell regresses (F3) → the write-back-flag gating is wrong; revert.
- One backend passes and the other regresses/crashes → not landable (the both-backends rule).
