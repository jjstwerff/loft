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

## Why not fixed in the surfacing session

The surfacing session was on macOS-ARM and had done the full diagnosis; the edit
touches the text-return-delivery classifier that drives EVERY text return, so it
needs its own careful both-backends + full-suite pass rather than a tail-end patch
(the loft-codegen stop-conditions). Everything needed to execute it cleanly is
above.
