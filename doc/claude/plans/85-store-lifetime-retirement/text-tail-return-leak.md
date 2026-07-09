<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 residual — text tail-return leak (native text call in tail position)

Surfaced by @PLN54 S4's LeakSanitizer sweep on macOS-ARM (2026-07-09). A REAL,
GROWING production leak in this plan's store-lifetime class. **NOT yet fixed** —
this doc is the proven repro + bytecode pair + the exact chokepoint, set up the
PLN85 way so the fix is a clean both-backends pass.

## Symptom (boundary matrix, both cells output-verified on `--interpret`)

A native text-dest call (`to_json`, `kind`, `as_text`, `to_json_pretty`,
`struct_to_json`, …) used as the **implicit tail-return** of a user function leaks
~**2 allocations per call, and it GROWS** (unbounded in a loop):

| shape | N=10 | N=100 |
|---|---|---|
| **BROKEN** `fn run() -> text { u = U{…}; u.to_json() }` | 20 | 200 |
| **WORKING** `fn run() -> text { u = U{…}; r = u.to_json(); return r; }` | 0 | 0 |

Repro pair: [`bytecode-comparisons/text-tail-return-BROKEN.loft`](bytecode-comparisons/text-tail-return-BROKEN.loft)
· [`…-WORKING.loft`](bytecode-comparisons/text-tail-return-WORKING.loft).
Reproduce: `RUSTFLAGS=-Zsanitizer=address cargo +nightly build --bin loft` then
`ASAN_OPTIONS=detect_leaks=1 loft BROKEN.loft` (leaks) vs `WORKING.loft` (clean).
This is why @PLN54 S4's `asan detect_leaks=1` flip is blocked: the harness's
JSON/text tests (`p54_*`, `q2/q3/q4*`) all use `fn helper() -> text { …native() }`
and leak per test.

## Proven bytecode (the spec is the diff)

`loft introspect` on `n_run`:

**BROKEN** — `fn n_run() -> text` (owned text, no caller buffer, per the named-fn
contract):
```
InitText(__work_1)                       ; work_text for the native's dest
StaticCall(n_struct_to_json_dest)        ; to_json writes INTO __work_1
AppendText(__ret_1, __work_1)            ; B5-L3 hoist: DEEP-COPY __work_1 → __ret_1
Return __ret_1                           ; __work_1 is now dead …
                                         ; … but NO FreeText(__work_1)  ← THE LEAK
```
(`__ret_1` is returned owned and the caller frees it; `__work_1` — the native's
own result — leaks. The copy also makes the delivery needlessly O(2 buffers).)

**WORKING** — `fn n_run(r: &text) -> text["r"]`: the local `r` is promoted to a
hidden `&text` caller buffer, `to_json` writes straight into `r`, the return
borrows it (`GetStackText`), the caller allocates + frees the buffer — no
`__work_1`, no copy, no leak.

## Root cause (the exact chokepoint)

Order of operations pins it:
1. `block_result` → `text_return(ls)` runs on the **bare** `u.to_json()` tail; a
   native's fresh owned text has **empty local deps**, so the per-var promotion
   loop does nothing and the return stays `-> text` (owned, no buffer).
2. `expressions.rs::wrap_value_text_dest` (@PLN10) then wraps the tail native into
   a `work_text` `__work_1` (`Block([Set(__work_1, call), Var(__work_1)])`) so its
   result has a freeable dest instead of the scratch buffer.
3. `scopes.rs::insert_free` (the **B5-L3** hoist, `Set(__ret_N, expr); frees;
   Return(__ret_N)`) hoists the tail into `__ret_1`. For a **text** tail `Set` is
   an `AppendText` — a **deep COPY**. But `__work_1` is **excluded from the free
   set** (the "don't free the value you're returning" rule), even though it was
   COPIED, not MOVED — so its buffer is never freed.

This is the **copy-vs-move / skip-free** class (D-own-2 adopt-vs-copy): a source
that is deep-copied into `__ret_N` must stay in the free set; only a source that is
*moved/renamed* onto the return is legitimately excluded.

## The fix (direction — a focused both-backends pass, per loft-codegen)

The **text** tail-return path is missing the fresh-owned delivery the **vector**
side already has (`fresh_owned_vector_deps` in `control.rs`). Two candidate
chokepoints, prove the working bytecode (WORKING.loft = mc2) on BOTH backends
first, then pick the narrower:

- **(preferred) move, don't copy** — deliver the tail `work_text` by RENAME onto
  the return (as `text_return`'s promotion + the vector `fresh_owned` path do),
  eliminating the `__ret_1` copy entirely → matches WORKING.loft exactly, and is
  also faster (one buffer, no copy).
- **(fallback) free the copy source** — in `scopes.rs::insert_free`, when the
  B5-L3 hoist deep-copies a `work_text` (text `Set` = `AppendText`) into `__ret_N`,
  keep the source `work_text` in the free set (emit `FreeText(__work_1)` after the
  copy). Localized, but leaves the redundant copy.

**Validation gate (both backends, PLN85 standard):** the C1 shape leaks 0 under
`ASAN_OPTIONS=detect_leaks=1` on `--interpret` AND `--native`
(`LOFT_NATIVE_LEAK_CHECK`); a byte-identical `introspect` diff for the untouched
text-return shapes (lambda/optional/tuple/`return <local>`); full `issues` + `wrap`
+ `strings` + `frame_vars` suites green on both backends. Add a regression: the C1
shape to @PLN54's `asan` corpus (leaks pre-fix), which then also unblocks the
`detect_leaks=1` flip.

## Analysis — the boundary map (probe matrix, macOS-ARM ASan, both cells verified)

Harness + probe files: [`probes/text-tail-return/`](probes/text-tail-return/)
(`run_matrix.sh` regenerates this). Verdict = presence of a runtime-owner leak
frame (the oracle below); every cell's output was checked non-vacuous.

The v2 harness asserts THREE things per cell (VALUE == committed `.golden`, LEAK,
UAF) — because the attempt-1 regression (below) failed on VALUE, not leak, so a
leak-only oracle green-lights it. Pre-fix baseline (`VALUE=ok` everywhere):

| shape | memory |
|---|---|
| `fn f() -> text { u.to_json() }` (implicit tail native call) | **LEAK** ~2/call, grows |
| `fn f() -> text { return u.to_json(); }` (explicit `return`) | **LEAK** |
| `fn f() -> text { s.to_uppercase() }` (ANY `_dest` native in tail) | **LEAK** (not `to_json`-only) |
| `fn f() -> text { inner() }` (forward a native-text fn) | **LEAK** |
| `fn f() -> text { x.to_text() + "!" }` (native `+` literal, then transferred) | **LEAK** ← the attempt-1 shape |
| `fn f() -> text { if c { u.to_json() } else { "x" } }` (native in an arm) | **LEAK** |
| `fn f() -> text { j = u.to_json(); "kept" }` (native result in a DROPPED local) | **LEAK** ← not even return position |
| `fn f() -> text? { u.to_json() }` (OPTIONAL tail native call) | **USE-AFTER-FREE** — `_dest` allocs, `append_text` frees, then reads it |
| `fn f() -> text { r = u.to_json(); return r; }` (rebind → promoted/moved) | clean |
| `fn f() -> text { acc = "J="; acc += u.to_json(); acc }` (append INTO an owned accum) | clean |
| `fn f() -> text { "PRE-" + s.to_uppercase() }` (literal `+` native) | clean |
| `fn f() -> text { "…{n}…" }` (interpolation tail) | clean |

**Boundary (broader than first thought):** the trigger is a native text-dest CALL
whose `wrap_value_text_dest` `__work_N` is **orphaned** — return position (implicit/
explicit), forwarded, in an `if` arm, `native + literal`, and even a **dropped
non-returned local**. It is clean only when the result is delivered into an owned
target that is itself freed/transferred: rebind-and-return (promoted buffer), append
INTO an owned accumulator, or `literal + native` (the literal owns the buffer). The
**`text?` variant is a UAF**, higher severity than the leak.

**Two cells are load-bearing guards for the fix:** `concat_suffix`
(`x.to_text() + "!"`) must go **LEAK→clean while staying VALUE=ok** — it is exactly
the shape attempt 1 emptied; and `optional_uaf` must go **UAF→clean+VALUE=ok**.

## Oracle — how to detect this class WITHOUT the ir_read baseline noise

The total `detect_leaks=1` count is **useless** as an oracle: it includes the
intentional `ir_read` `Box::leak` (Class 1, ~311 allocs) which fluctuates per
program and swamps the ~2/call signal. Three layered oracles instead:

1. **Runtime-owner-frame detector (primary, class-isolating).** Count leak/UAF
   stacks whose deepest loft frame is `loft::fill::append_text`,
   `loft::native::*_dest`, or `struct_to_json` — **excluding `loft::ir_read`**.
   A clean shape has **zero** such frames regardless of the Class-1 baseline. This
   is the CI-ready assertion (a grep over the ASan report), and it is what makes a
   `detect_leaks=1` flip meaningful without hand-tuning suppressions per shape.
2. **Growth-differential (confirms per-call vs bounded).** Run N=small vs N=large;
   the leaked **object** count (`in N object(s)` — field 7, NOT the byte field)
   grows ~2/call for a real leak, flat for the bounded Class-1. (LSan dedups
   identical stacks, so the report/frame COUNT is flat even while objects grow —
   use object count for growth, frame presence for classification.)
3. **Both-backend + `LOFT_POISON`.** The UAF variant fires under `LOFT_POISON`
   (freed-store sentinel) and ASan on `--interpret` AND `--native`
   (`LOFT_NATIVE_LEAK_CHECK`); a cross-mode value oracle (@PLN89) guards
   correctness. A fix is closed only when all probe cells read clean on both
   backends under all three.

## Flip — @PLN54 S4 `detect_leaks=1` gate, step by step

1. **Land the fix** (§ above) → every probe cell = 0 runtime-owner frames, UAF
   gone, on both backends; the ~129 harness JSON/text tests stop leaking.
2. **Graduate the probes to regression guards:** add `tail_to_json`, the `text?`
   UAF cell, `tail_upper`, and `forward_to_json` to @PLN54's `native-asan` /
   `asan` corpus — each leaks/UAFs pre-fix, so they pin the class shut.
3. **Flip `miri.yml` `asan`:** `ASAN_OPTIONS: 'detect_leaks=1'` +
   `LSAN_OPTIONS: 'suppressions=lsan_suppressions.txt'`, where the suppression file
   is ONE documented line for the intentional Class-1 `ir_read` `Box::leak` —
   `leak:read_block` (+ `leak:read_data_with` as the direct entry the ~16
   `ir_read`/`ir_schema`/`ir_store` round-trip lib tests hit). These are DIRECT
   calls (not interpreter-inlined), so the frame is present on both ubuntu-x86_64
   and macOS-ARM — but verify on the Linux leg before landing (the S1 caveat: a Mac
   can't validate the Linux ASan runtime).
4. **Keep the runtime-owner-frame detector as the standing assertion** so a NEW
   store-text leak (a fresh `_dest` fn, a new tail shape) turns the gate red even
   though the `ir_read` line is suppressed — the gate asserts "zero non-`ir_read`
   store-text leaks," which is the invariant, not "zero total allocations."

## Attempt 1 (2026-07-09) — REVERTED; refines the fix site

Tried the localized free at the suppression site: in `scopes.rs::get_free_vars`,
lift the `v == ret_var` free-exemption for a `__work_N` text (since a work_text is
copied, not transferred). It emitted the **exact target bytecode** for the isolated
case — mc1's `n_run` gained `FreeText(__work_1)` *after* the `AppendText` copy and
before `Return __ret_1`, output correct (`{"name":"Alice"}`), no UAF — BUT regressed
one test: **`plan17_b_bounded_method_return_type_propagates`** (`fn label<T>(x) ->
text { x.to_text() + "!" }`) returned **empty** (`"" != "42!"`).

**Why:** that shape does NOT copy the work_text — `x.to_text()` fills `__work_1`,
`+ "!"` appends **in place**, and `__work_1` is **transferred directly** to the
caller (caller frees). Freeing it at scope exit emptied the return. So a
work_text-as-`ret_var` is EITHER copied into `__ret_N` (mc1 → must free) OR
transferred in place (plan17_b → must NOT free), and **`get_free_vars` runs before
the copy-vs-transfer decision, so it cannot tell them apart.** Reverted per the
loft-codegen stop-condition (regressed the suite).

**Refined fix site:** emit the free at the `__ret_N` **copy** site itself — i.e., in
the B5-L3 text hoist(s) in `scopes.rs::insert_free` that produce `Set(__ret_N,
expr)` where `expr` reads a work_text (text `Set` = `AppendText` = a copy). Right
after that copy, free the work_text source(s) `expr` read. The direct-transfer path
(fast-path `Return(Var(__work_1))`, no `__ret_N`) emits no such free and correctly
leaves `__work_1` for the caller. This makes the free conditional on a copy actually
happening — the distinction `get_free_vars` lacked. Open sub-question to resolve at
the copy site: exactly which hoist emits mc1's `__ret_1 = AppendText(__work_1)` copy
(trace with `LOFT_LOG` on a clean binary — the synth Block recursion at line ~3349
vs the B5-L3 text branch at ~3380), and free the work_text there.

## Attempt 2 (2026-07-09) — LANDED, partial: callee `__work_N` orphan + UAF fixed

Relocated the free to the copy site per Attempt 1's refinement: a new
`free_copied_work_texts(result, expr, function, data)` called right after the B5-L3
`__ret_N` copy `Set` in `scopes.rs::insert_free` (both the value-hoist and
text-hoist arms). It `collect_return_sources(expr)` and emits `OpFreeText` for each
`__work_N` text source — so the free exists ONLY when a copy actually happened; the
direct-transfer path emits neither copy nor free and correctly leaves the work_text
for the caller (this is why it does NOT regress `plan17_b`/`concat_suffix`, which
attempt 1 emptied). mc1 bytecode now: `AppendText(__ret_1, __work_1)` →
`FreeText(__work_1)` → `Return __ret_1`.

**Validated (both backends):** suite 749/0 (no regression); the oracle matrix VALUE
= ok on every cell on `--interpret` AND `--native`; `optional_uaf` goes
**USE-AFTER-FREE → (no UAF)** — the safety bug is eliminated; `tail_to_json` &c go
from **2 leaked allocs/call → 1**.

**Remaining (a distinct slice — the OTHER half of the ~2/call):** the returned
owned text itself (the `__ret_N` copy, `skip_free`'d in the callee by the
`-> text` "caller consumes it" contract) still leaks **1/call** — the CALLER
consumes it (`r = drive()` → `OpAppendText` into `r`) but never frees the returned
temp. Stack: `append_text` ← `execute_argv` (the copy that built `__ret_N`), freed
by neither side. The `rebind` shape is clean because it returns via a promoted
CALLER buffer (no per-call owned-text temp) — which is also the candidate FULL fix
(promote the tail native-text return to a caller buffer, @P387-adaptive so fn-refs
still work), superseding both halves. Next slice: either free the consumed
return-temp at the caller's `Set(local, <owned-text call>)`, or promote. Guard: the
matrix must reach 0 runtime-owner frames on every cell (not just `optional_uaf`).

### Probing the remaining half (2026-07-09) — it is the native-call return, not consumption

Two matrices on the attempt-2 binary pin the residual 1/call precisely
(`probes/text-tail-return/` companions `cc.*`/`rc.*`; runtime-owner OBJECT count,
N=5 vs N=105):

**Consumption-independent** — every caller pattern leaks the *same* 1/call:
`r = drive()` (reassign) · `print(drive())` · `x = "p" + drive()` · `drive();`
(discard) · `eat(drive())` (arg) · `s = drive(); …` (bind+use). So it is NOT a
caller-consumption bug — a caller-side free would have to fire on all of these.

**Return-shape-specific** — the decisive cut:

| return shape | per-call |
|---|---|
| `"literal"` · `"a" + "b"` · `return s` (built local) · `s` (built-local tail) | **0** |
| `u.to_json()` (native text-dest CALL) | **1** |

So EVERY owned-text return is clean EXCEPT a native text-dest call: the clean ones
deliver through a **promoted caller buffer** (`text_return` promotes the var/
built-text — `fn f(r: &text) -> text["r"]`, the caller allocates + frees), while the
native-call tail instead emits an owned-text `__ret_N` copy that no side frees.

**Conclusion — the clean FULL fix is promotion, not a caller-side free.** Give the
native-call tail the SAME buffer promotion the var tail already gets (the proven
`rebind`/mc2 form): bind the tail native call to a synthetic local so `text_return`
promotes it, so the native writes straight into the caller's buffer and there is no
owned-text `__ret_N` at all → 0 leak, matching every other return shape. This
SUPERSEDES attempt 2's `__work_N` free (no `__work_N`, no `__ret_N`). Risk: it is an
ABI shift (a text fn gains a hidden `&text` buffer), the class @P387 made adaptive
for fn-refs — so the guard must include the `p227_text_fn_ref_*` / par shapes
(#273) alongside the leak matrix. Attempt 2b = this promotion.

## Why not fixed in the surfacing session

The surfacing session was on macOS-ARM and had done the full diagnosis; the edit
touches the text-return-delivery classifier that drives EVERY text return, so it
needs its own careful both-backends + full-suite pass rather than a tail-end patch
(the loft-codegen stop-conditions). Everything needed to execute it cleanly is
above.
