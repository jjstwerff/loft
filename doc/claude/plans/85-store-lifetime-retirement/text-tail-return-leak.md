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

## Attempt 2b (2026-07-09) — promotion via bind-rewrite: REVERTED (native-fn-fragile)

Implemented the promotion the probing pointed to: in `block_result`, when the text
tail is a bare CALL with empty deps, bind it to a synthetic local
(`__tret = <call>; __tret` via an `Insert`) and route through `text_return` so it
promotes to a hidden `&text` caller buffer — the proven-clean `rebind` form. The
target is genuinely clean (the manual `{ t = u.to_json(); t }` probe → 0 leak,
`fn n_drive(t:&text)->text["t"]`), and `tail_to_json` DID promote and go leak-free.

But the blanket rewrite is **native-fn-fragile**, caught by the VALUE oracle:
- v1 matched ANY `Call` → **`forward_to_json` returned empty**: a forwarded USER
  fn (`fn f()->text{ g() }`) is already promoted and dest-passes its own buffer, so
  binding it double-buffers and delivers nothing. Fixed by restricting to the
  `is_text_dest_native` predicate.
- v2 (native-only) → **`tail_upper` returned empty**: `s.to_uppercase()` (a native
  text-dest fn taking a TEXT arg) does not deliver correctly when dest-passed into
  the synthesized promoted buffer, whereas `to_json` (struct arg) does. So the
  ~10 native text-dest fns do NOT uniformly support dest-pass-into-a-promoted-buffer
  from this shape.

Reverted per the loft-codegen stop-condition (regressed the VALUE oracle). The
promotion approach is sound in principle (it's how `rebind`/var-tail already work)
but needs **per-native-fn dest-pass correctness** — the synthesized
`__tret = native(); __tret` must lower to each native writing into `__tret`, which
holds for `to_json`/struct-serialise but not `to_uppercase`/text-arg natives. That
is a `wrap_value_text_dest` / native-dest-signature question, a distinct slice.

### Missing probes (found by asking after 2b) — per-native-fn, and it REHABILITATES 2b

2b broke on `to_uppercase` where `to_json` worked, so the matrix (only those two of
~20 `is_text_dest_native` fns) was undersampled. Building the per-fn set
(`probes/text-tail-return/native-fns/`, categories: text-method `to_upper`/
`to_lower`/`replace`; struct-serialise `to_json`/`to_json_pretty`) shows:

- **Tail leak is UNIFORM** — every native text-dest fn leaks the same 1/call
  post-attempt-2 (not fn-specific), so ONE fix closes all.
- **Flat rebind promotes ALL of them CLEANLY** — `s = …; t = s.to_uppercase(); t`
  (and every other fn) → **0/call, correct value, `fn n_drive(t:&text)`**,
  INCLUDING `to_uppercase`. So `to_uppercase` is NOT incompatible with promotion.

**This rehabilitates 2b.** The regression was NOT the promotion approach — it was my
rewrite REPRESENTATION: `block_result` gets `l: &mut [Value]` (a fixed slice), so I
emitted the two ops as one `Insert([Set(__tret, call), Var(__tret)])`, and that
compound shape broke the native dest-pass for `to_uppercase` (empty). The FLAT form
`Set(__tret, call); Var(__tret)` as two separate block operators promotes every fn
cleanly. **Attempt 2c = the same bind-and-promote, emitted FLAT** — do it where the
block is a growable `Vec` (before `block_result`, or grow `l` upstream) so the two
ops are siblings, not an `Insert`. Guard: the per-fn matrix (all 5 → 0/call +
correct) + the p227 fn-ref/par shapes + full suite, both backends.

**Net after this session:** attempt 2 (LANDED) fixes the callee `__work_N` orphan +
the `-> text?` UAF and halves the leak. The residual 1/call is uniform across native
text-dest fns and has a proven-clean target (flat rebind, all fns 0/call); attempt
2c emits it flat. The VALUE oracle caught both 2b regressions instantly, and the
per-fn probes turned "2b is fn-fragile, deferred" into "2b works flat; Insert was
the bug".

### Attempt 2c (2026-07-09) — flat bind-and-promote: REVERTED (injected local ≠ source local)

Emitted the bind FLAT (two sibling ops `Set(__tret, call); Var(__tret)`, in
`parse_block` where `l` is still a growable `Vec`, so no `Insert`), set the tail type
to `Text(frame1(__tret))`, and let `block_result`'s existing var-tail `text_return`
promote it. `tail_to_json` &c went clean; but the VALUE oracle again caught
**`tail_upper` → empty**. The `n_drive` bytecode diff vs the proven-clean MANUAL
`{ s=…; t = s.to_uppercase(); t }` pins it — the SIGNATURE promotes identically
(`fn n_drive(__tret:&text)`), but the BODY ops do not:

| | manual `t` (clean) | injected `__tret` (empty) |
|---|---|---|
| clear   | `ClearStackText` (deref the `&text` buffer) | `ClearText` (plain text) |
| call dest | `VarRef(t)` → to_uppercase writes INTO the buffer | `InitCreateStack` → a FRESH dest |
| return  | `VarRef; GetStackText` (read the buffer) | `ArgText(__tret)` (the empty local) |

So codegen lowers the injected `__tret`'s body references as a **plain text**, not
the promoted **`&text` buffer** — `to_uppercase` writes to a throwaway dest and the
returned `__tret` stays empty. A local injected mid-`parse_block` via `create_unique`
does NOT acquire a source-level local's full promotion lifecycle (usage tracking /
`RefVar` re-lowering of its body ops), even though the IR and the promoted signature
look identical. Reverted per the stop-condition.

**What the next attempt needs (2d):** create the bind local where the parser tracks
it exactly like a source local — i.e. synthesize the `t = <call>; t` rewrite EARLIER
(during expression/return parsing, before types + usage are finalised), so
`text_return`'s promotion re-lowers its body ops to the `&text` (`ClearStackText`/
`VarRef`/`GetStackText`) form — OR fix the promotion to re-lower an
already-emitted `Set/Var` on a var whose type flips to `RefVar`. The proven-clean
target (manual flat rebind, all native text-dest fns 0-leak) and the VALUE-oracle
guard remain; three representations tried (2b `Insert`, 2c flat-in-`block_result`),
each caught by the guard — the remaining unknown is purely the injected-local
promotion lifecycle, not the approach.

### Attempt 2d (2026-07-09) — LANDED: pass-2-only flat bind-and-promote

The 2c trace (`LOFT_TRACE_2D`) pinned the 2c failure: injecting on BOTH passes made
pass 1 add the hidden `__tret` ATTRIBUTE, which persists on the `Data` def across
passes; pass 2's `classify_text_dep` then saw `__tret` as an existing attr →
returned `Attr` (already-promoted) → never re-set the VAR type to `RefVar`, so the
body lowered as plain text (empty). `to_json` only worked in 2c because its tail
type resolved late, so it accidentally injected pass-2-only.

**Fix:** gate the flat bind-and-promote to `!first_pass` (the codegen pass), so it
promotes once, cleanly, to `RefVar` — exactly what `wrap_value_text_dest` does and
what the working `to_json` case did. Landed in `parse_block`.

**Validated (both backends):** all 13 matrix cells VALUE=ok; suite 749/0; `--native`
correct. Oracle memory after 2d: **`tail_to_json`, `tail_upper` → clean** (native
bare-tail returns now promote to a caller buffer, no owned-text copy) and
**`optional_uaf` → clean** (the `-> text?` UAF is now FULLY resolved — no UAF AND no
leak).

**2d extension — explicit `return <call>` too.** `native_text_call_tail` peels
`Span`/`Return`, and the gate keys on the DECLARED `result` type (not the tail `t`):
an explicit `return <call>` tail is typed `Never` (it diverges), so a `t`-based gate
missed it; `result.base() == Text` catches both the bare tail and `return <call>`.
The rewrite lifts the call out of the `Return` (`Set(__tret, call); return __tret`).
**`return_to_json` → clean** (both backends, suite 749/0). Now BOTH direct
native-tail forms + the UAF are fixed — **8/13 matrix cells clean**.

**Still LEAK (non-direct shapes — the native call is NOT the tail itself):**
`concat_suffix` (`native + "!"` — native is a concat operand), `if_arm_native`
(native in an `if` arm), `forward_to_json` (user-fn forward — 2d excludes user
fns), `local_dropped` (native in a dropped non-returned local), `nested_consume`
(`wrap(native())` — native as a nested arg). Each needs the native SUB-expression
promoted/freed at its own site, not the tail; attempt 2's `__work_N` free covers
them partially (they leak 1/call, not 2). Follow-on.

### Harness impact of 2d (measured, `--test issues` under `detect_leaks=1`)

The real goal is the @PLN54 S4 gate, so measure the ACTUAL harness leakers, not just
the synthetic matrix. Distinct leaking `issues` tests: **~113 pre-fix → 42 after 2d**
(≈71 fixed, a majority) — every DIRECT native-tail return (the bulk of the p54/q3/q4
JSON/text tests) is now clean, plus the UAF. The 42 that remain are the compound /
indirect shapes where the native call is NOT the whole tail, grouped:

- `p54_struct_parse_*` / `p54_struct_enum_*` / `p54_b*` / `p54_match_*` — JSON
  struct-parse + enum-extractor shapes (the native text lands in a struct field or
  match arm, not the return tail).
- `p197_*` (text from a tuple field), `p329_*`/`p330_*`/`p243_*` (generic-tuple text
  elements), `p227_*` (text FN-REF), `p235_*`/`p4d_*` (par text), `plan17_*` (bounded
  generic method), `q4_json_string_round_trips`, `b7_*`, `issue_437`, `n3_*`, `p189c`,
  `p213`, `p241_singleton_text`.

Each needs the native SUB-expression (or the tuple-element / fn-ref / par delivery)
promoted or freed at its own site — a family of follow-on slices, not the tail-return
fix. (16 `ir_read`/`ir_schema`/`ir_store` lib tests also "fail" — the intentional
Class-1 `Box::leak`, handled by the flip's narrow suppression.)

**So the flip is not yet ready** (42 real Class-2 leakers remain); but 2d closed the
largest, most-common slice + the safety bug, validated on both backends, no
regression. Corrects an earlier mis-stated "113 → 0" (a grep bug — the `( N/M)`
nextest progress prefix; the real count is 42).

### The remaining 42 — a DISTINCT family (composite / view-return text), next arc

Probing a representative (`p197`: `struct A { v: (text,text) }; fn first() -> text {
a = A{...}; a.v.0 }`) shows these are NOT native-text-call tails — they are
**view-returns of text embedded in a local composite** (`a.v.0` tuple-field,
`d.ts[0]` vector-of-text field, `vec[0]` generic element). Leak: 1/call, owner
`append_text` — the return delivery copies the viewed text and leaks (the source
composite's embedded text, or the copy). 2d's native-call-tail promotion does not
apply (the tail is a field/index access, not a `Call`).

Grouped, the 42: `p54_struct_parse_*`/`struct_enum_*`/`match_*`/`b*` (JSON text into
a struct field / match arm), `p197`/`p329`/`p330`/`p243` (text in a tuple / generic
tuple element), `p227` (text FN-REF), `p235`/`p4d` (par text delivery), `plan17`
(bounded generic method), `p241`/`q4_json_string`/`b7`/`issue_437`/`n3`/`p189c`/
`p213`.

This is the `materialize_view_return` neighbourhood — that path exists for
`Reference` views (control.rs `materialize_view_return`, the #306 fix) and for
tuples-of-text (the `__ret_text_N` hoist, scopes.rs @P329) but leaves the
composite's SOURCE embedded texts unfreed. It is a **distinct, delicate arc** in the
free-emission code PLN85's campaign hardened — several sub-slices (tuple / struct
field / vector element / fn-ref / par), each needing its own probe + oracle + both-
backend pass. It should be taken fresh, not rushed onto the tail-return fix: the
tail-return class (the largest, most-common harness slice) + the UAF are done and
pushed; this composite/view-return family is the well-scoped follow-on arc, tracked
here with the probe (`probes/text-tail-return/`) and the VALUE+LEAK+UAF oracle ready
to guard it.

## Session 2 (2026-07-09, mac-work rebased on origin/main) — 42 → 22 via 3 promotion slices

The composite/view arc turned out to be **two** things: a genuine promotable-tail
family (fixed here) and a residue of FIVE unrelated subsystems (below). Harness
oracle (per-test, runtime-owner frame, excl `ir_read`): **42 → 22 leakers.**

Landed (each: proven working bytecode = the `r = <expr>; r` rebind, VALUE+LEAK+UAF
matrix clean on both backends, `issues` 749/0, full suite green):

- **3a** (`09354f3a`) — text field/index VIEW of a LOCAL composite (`a.v.0`,
  `s.name`, `d.ts[0]`, generic `v[0]`). The whole **p197** group + the
  `p54_struct_enum`/`p54_b2` match shapes. Gate: `text_view_root` +
  `var_built_in_block` (promote only when the view's root composite is CONSTRUCTED
  in this body — an `OpDatabase`/`Set` here — so a genuine caller-owned ARGUMENT
  view stays a clean borrow; `is_argument` can't tell them apart because an
  NRVO-promoted local also reads as an argument).
- **3b** (`b1e4b175` + refinement `4bd1ae97`) — USER-fn text-call tails
  (`{ inner() }`, `{ wrap(x) }`): **forward_to_json**, **nested_consume**. Gate:
  `user_text_call_tail`, fired ONLY for an OWNED delivery (return borrows nothing
  or only HIDDEN buffer attrs). The refinement fixed **p281** — a forward-BORROW
  of an argument (`second(s) -> text["s"]`) must NOT promote (codegen "Too few
  parameters"). Also re-goldened `introspect_show_types_renders_deps` (`n_first`
  now `text["__ref_1", "a"]` — buffer + host).
- **3c** (`55352db5`) — `if`/`match`-arm OWNED-text tails (**if_arm_native**; match
  lowers to nested `If`). Gate: `if_tail_delivers_owned_text` (promote only when an
  arm delivers a native/user text call; pure literal/borrow `if`s stay borrows).

All three reuse the 2d flat pass-2-only `__tret` bind (`parse_block`): bind the tail
to a synthetic local so `text_return` promotes it to a hidden `&text` caller buffer.

### The remaining leakers — SUPERSEDED by the verified analysis below

The five-subsystem split first recorded here was a source-reading GUESS. The
per-test ASan owner sweep (session 4) corrected it — see "Verified analysis".

## Session 4 (2026-07-09) — VERIFIED per-test analysis of the remaining leakers

Method: rebuilt the ASan `issues` test binary (current wiring), swept **every**
issues test isolated under `detect_leaks=1`, and captured each leaker's deepest
non-`ir_read` loft frame (`probes/.../leakowner.sh`).  Then reproduced each shape
as a standalone `.loft` and measured leak + TRA verdict + does-a-promoted/discarded-
form-still-leak.  This replaces inference with evidence.

**Headline (verified): the leak SITE is UNIFORM.** All 28 remaining leakers have
the identical owner — `loft::fill::append_text ← State::execute_argv`.  So the
mechanism is ONE thing: **a text COPIED via `append_text` at a call/return
boundary whose source is never freed.**  The variation is only the SHAPE that
produces the un-freed copy — and those fall into exactly TWO root classes:

**Class A — un-promoted owned-text DELIVERY at a boundary** (return/arg copy; the
promotion gap).  Fixed by delivering through a caller `&text` buffer (no copy) —
i.e. by the framework's promotion, once the pass-1/pass-2 signature pre-pass lands
(§ Session 3).  Verified members carry an `Owned:*` verdict and are the EXCLUDED
set:
  - user-fn call tails, if/match-arm tails (excluded `UserCall`/`IfMatchArm`);
  - **generic `x.to_text()` — verdict `Owned:UserCall`** (the monomorph is a
    user fn, NOT a native): `plan17_b`, `plan17_printable_integer`, `p243`.  This
    corrects the old "generic-dispatch work-buffer" guess — it is the UserCall
    gap, and `x.to_text() + "!"` leaks via the OPERAND copy even though the concat
    RETURN is `BuiltLocal`-promoted;
  - fn-ref calls — verdict `Owned:FnRefCall` (+ @P387 adaptive ABI): `p227_*` ×4;
  - tuple-element construction — verdict `Owned:TupleElement`: `p329_*` ×3,
    `p330_*` ×2 (with correct `to_text`, the tuple DOES leak — an earlier "clean"
    reading was a mis-typed probe where `T` failed `Printable`);
  - vector-of-text RETURN then append / generic singleton: `issue_437`, `p241`
    (verified: building+discarding a `vector<text>` does NOT leak → the leak is
    the RETURN delivery, not the build);
  - `n3` — return of a field view after a record copy (view-return delivery; the
    `b = a` copy discarded alone does NOT leak).

**Class B — CONSUMED composite's embedded text not freed** (NOT return-delivery; a
scope/lifetime free bug).  Verified decisively: a struct-enum with an embedded
text, and a `json_parse` result, **LEAK even when constructed/matched and
DISCARDED with no return** (`leak=1`).  So the free of a consumed/dropped
struct-enum (or the `json_parse` jsonvalue) does not recurse into its embedded
text.  Members: `p54_struct_enum_as_struct_field`, `_extractors_spec`,
`_in_hash_value`, `_multi_call_flow`, `p54_b2_*` ×2, `p54_b6_match_arm_text_unify`,
`p54_extractor_as_text_wrong_kind`, `p54_or_pattern_mixed_struct_enum`,
`p54_match_on_jsonvalue`, `p54_parse_primitive_string`, `q4_json_string_round_trips`,
`b7_repeated_method_dispatch_on_jsonvalue`.  The framework correctly reads these as
`Plain`/`BuiltLocal` (the return is not the issue) — pointing AWAY from promotion,
toward the composite-free path.  This is a DISTINCT arc from the whole tail-return
campaign: it lives in the enum/composite scope-free logic, not text return.

**So the ~28 remaining split ≈ Class A 13 (promotion gap — one fix, the pre-pass)
+ Class B 15 (composite embedded-text free — a separate scope-free fix).**  Two
arcs, not five.  Matrix representatives: `concat_suffix` (Class A, generic/operand)
and `local_dropped` (Class B, struct_to_json consume).  The correction that
`local_dropped`'s `append_text←execute_argv` is INSIDE `struct_to_json` still
holds, but it is the same Class B "consumed composite" story, not a separate
"native-stdlib-internal" subsystem.

### Session 5 (2026-07-09) — Class B splits into B1 (FIXED) and B2 (characterised)

Building the fix refined Class B into two sub-classes:

**B1 — bound text payload not freed. FIXED (`c5a6abd2`).**  A match arm that
binds a TEXT payload (`match x { B { v } => …v… }`) lowers to
`_mv_v = OpGetText(subj, off)` — an OWNED copy (plain `text`, no dep) — but the
binding was unconditionally `set_skip_free` (correct for HEAP/DbRef bindings,
wrong for an owned text that is also default-init `""`).  Gated skip_free to
skip HEAP bindings only; text bindings now free through normal scope cleanup.
Cleared `p54_parse_primitive_string` + `q4_json_string_round_trips`; suite
2728/0; both backends.  Guard: `probes/.../match_bind_text.loft.tpl`.

**B2 — a struct-enum RETURNED from a fn, then matched, leaks 1 text/call. OPEN.**
Isolated decisively (leak counts, ASan):

| shape | leak |
|---|---|
| enum-with-text-variant construct + DISCARD (local, or via `mk()`) | 0 |
| `x = mk(); match x { A{v:_} => "a", _ => "o" }` (mk returns the enum) | **1** |
| `match json_parse(raw) { <variant> => lit, _ => lit }` (multi-arm) | **1** |
| single-`_` arm, or void arms, or int-subject match | 0 |

So B2 is NOT "consumed composite embedded text" in general (a locally
constructed-and-dropped enum frees its text fine).  It is specifically the
**enum RETURN-DELIVERY**: when a fn returns a struct-enum whose layout carries a
text field (any variant), the return-buffer free path does not recurse into the
embedded text slot, so a multi-arm match on the returned enum leaks 1 text/call.
This lives in the enum/`ref_return` return-delivery machinery (the buffer the
caller allocates + frees for an enum return), NOT the match and NOT text-return.
The p54 struct-enum group + the json-match group are all this shape (their enum
comes from a constructor/`json_parse` CALL and is matched).  A distinct arc:
needs its own probe + the enum-return-buffer free to free embedded texts.

**Revised remaining tally:** Class A ~13 (promotion gap, the pass-1 pre-pass) +
B2 ~13 (enum return-delivery embedded-text free).  B1 (~2 in the harness, but the
whole bound-payload class) is closed.

## Session 3 (2026-07-09) — the analysis FRAMEWORK, and wiring it in

The 3a/3b/3c stacking (five per-shape predicates) hid a shared latent bug: each
decided the buffer promotion during PASS-2 body parse, so a fn's ABI gained a
hidden `&text` buffer a FORWARD-REFERENCE caller (compiled earlier in pass 2)
never saw — codegen `Too few parameters on n_<fn> (got 0, need 1)` (the markdown
viewer's `page_landing`, a user-call tail).  2d has the identical latent bug.
3a/3b/3c were reverted; the promotable-tail work was rebuilt as ONE analysis.

**The framework (`Parser::classify_text_return`, control.rs).** A pure selector
over the return tail → `TextReturn`:
`Owned(NativeCall|UserCall|ViewOfLocal|IfMatchArm|BuiltLocal|TupleElement|FnRefCall)`
· `Borrow(Argument|ForwardArg)` · `Plain`.  Built + verified as a SHADOW first:
`LOFT_TRA_DUMP=<file>` appends `TRA <fn> => <verdict>` per text-returning fn with
NO codegen change; `framework/corpus.loft` (one fn per shape + `// VERDICT:`),
`framework/verify.sh`, and `tests/text_return_analysis.rs` (CI) check all **24
cases**, including the p281 `ForwardArg` vs arg-view boundary and the open
subsystems (which correctly read `Plain` — leak is elsewhere).  Two harness
gotchas found: loft's `eprintln!` races `process::exit` (use a file), and the
content-keyed program cache skips the parse on a warm hit (set `LOFT_NO_CACHE`).

**Wiring it in (`wants_tret_bind`).** The `parse_block` gate now reads the ONE
verdict and fires the `__tret` bind in BOTH passes (so the buffer lands in the
pass-1 signature); `text_return`'s `Attr` arm re-applies `RefVar` on pass 2 (the
double-classify 2d dodged).  **Scoped to the FORWARD-REF-SAFE verdicts:**

- `ViewOfLocal` — a field/index view resolves to `OpGetText` on PASS 1, so its
  buffer is in the signature before any forward-ref caller (verified `fref_view`).
  This recovers the whole **p197 / composite-view** group SAFELY.
- `NativeCall` — kept (= attempt 2d).  A method call resolves only on pass 2, so
  it binds pass-2-only in practice and carries 2d's latent forward-ref limit
  (unchanged, untriggered by the suite).

- **EXCLUDED: `UserCall` / `IfMatchArm`.**  Pinned with an instrument: on pass 1
  a native/user CALL tail is already lowered to a work `Var` (unpromoted →
  `BuiltLocal`, `bind=false`), but on pass 2 it is a bare `Call` (`bind=true`) —
  so the promotion, hence the ABI, DIFFERS across passes and a forward-ref caller
  crashes.  A view tail is a `Var(view)` consistently → safe.  Re-enabling
  user-call / if-arm needs pass-1 and pass-2 tail classification to AGREE — a
  **signature pre-pass** that fixes the ABI before codegen.  That is the next
  arc; it also lifts 2d's native limit and lets `TupleElement` / `FnRefCall` be
  wired.  Until then user-call (`forward_to_json`, `nested_consume`) + if-arm
  (`if_arm_native`) stay unpromoted (leak, but no crash / no viewer regression).

## Why not fixed in the surfacing session (historical)

The surfacing session was on macOS-ARM and had done the full diagnosis; the edit
touches the text-return-delivery classifier that drives EVERY text return, so it
needs its own careful both-backends + full-suite pass rather than a tail-end patch
(the loft-codegen stop-conditions). Session 2 executed that for the promotable-tail
family (3a/3b/3c above); session 3 rebuilt it as one verified analysis + wired the
forward-ref-safe subset.
