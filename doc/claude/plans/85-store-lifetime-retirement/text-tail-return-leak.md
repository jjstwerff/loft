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

| tail shape | outcome |
|---|---|
| `fn f() -> text { u.to_json() }` (implicit tail native call) | **LEAK** ~2 allocs/call, grows |
| `fn f() -> text { return u.to_json(); }` (explicit `return` of native call) | **LEAK** (same — not implicit-only) |
| `fn f() -> text { s.to_uppercase() }` (ANY native text-dest in tail) | **LEAK** (not `to_json`-specific) |
| `fn f() -> text { inner() }` (forward a user fn returning native text) | **LEAK** |
| `fn f() -> text? { u.to_json() }` (OPTIONAL tail native call) | **USE-AFTER-FREE** — `_dest` allocs, `append_text` frees, then reads it (`memcpy`) |
| `fn f() -> text { r = u.to_json(); return r; }` (rebind to a local first) | clean |
| `fn f() -> text { "…{n}…" }` (interpolation tail) | clean |
| `t = mk().to_json()` (native result bound in the CALLER) | clean |

**Boundary:** the trigger is a native text-dest **CALL delivered directly as a
user function's return value** (implicit tail OR explicit `return`, any `_dest`
native, and it forwards through wrapper fns). Binding the result to a local first
(rebind), or building the text by interpolation/append, routes through the
promotion/move path and is clean. The **`text?` variant is a UAF** (higher
severity than the leak) — the @PLN25 optional path frees the source before the
copy reads it.

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

## Why not fixed in the surfacing session

The surfacing session was on macOS-ARM and had done the full diagnosis; the edit
touches the text-return-delivery classifier that drives EVERY text return, so it
needs its own careful both-backends + full-suite pass rather than a tail-end patch
(the loft-codegen stop-conditions). Everything needed to execute it cleanly is
above.
