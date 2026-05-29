<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — S4 implementation spec (eval-TOS / frame-base alignment)

Produced by a code-inspection pass (2026-05-29) on top of the landed
S4 scaffold (commit `9bf0894a`).  S4 rounds the eval-TOS push/pop step,
the frame reserve, and the call/args layout up to 8 (max stack
alignment), in lockstep between **codegen** (`Stack::position`) and
**runtime** (`State::stack_pos`).  Gated behind `LOFT_ALIGN=1`
(whole-program); exercised with `LOFT_ALIGN=1 LOFT_SLOT_V2=drive`.
Default (neither) = V1 byte-identical.

**Baseline:** `cargo test --test issues` = 681/0; `LOFT_SLOT_V2=drive`
= 681/0; `LOFT_ALIGN=1 LOFT_SLOT_V2=drive` = **SIGSEGV** (S4 unwired —
runtime steps routed, codegen positions not).

## Scaffold seams already in place
- `variables::aligned_stack_enabled()` (env `LOFT_ALIGN`).
- `variables::aligned_stack_step(size,aligned)` = `if aligned {
  size.next_multiple_of(8) } else { size }`.
- Runtime: `State.aligned_stack` + `State::stack_step`; routed:
  `get_stack`, `put_stack` (+ensure), `reserve_frame`.
- Codegen: `Stack.aligned` + `Stack::step` (UNWIRED — `#[allow(dead_code)]`).

## The invariant to keep verifying every commit
**Codegen `position` and runtime `stack_pos` advance by the identical
stepped amount for every push/pop**, AND **codegen-emitted span
operands (discard / ret / advance / args_size) are consumed RAW at
runtime** (they are already aligned byte counts).  The two classes must
never cross.

## Key de-risking finding
Nearly all `var_pos = position - slot` sites (codegen.rs:357..3162) and
all `get_var/mut_var` reads are **FIXED** — they auto-correct through
the aligned `position`/`stack_pos`.  No edit.  The real work is the
*advance* sites + SPECIAL corrections + frame base.

## Classification — STEP (route through step), FIXED (leave), SPECIAL (scale with step)

### Runtime `src/state/mod.rs`
- 1367 `copy_result`: `stack_pos = fn_stack + size` → **STEP** `+ stack_step(size)` (Risk R1).
- 1587 `put_var`: `stack_pos + size_of::<T>() - pos` → **STEP** `+ stack_step(size_of::<T>())` (critical — put_fn_ref pops step(20) then put_var adds raw 20).
- 1253 yield restore: `stack_pos = base + value_size` → **STEP** `+ step(value_size)`.
- 1979/1990 execute_argv: `stack_pos = 4` / entry `args_base = 4` → **STEP** `stack_step(4)`(=8).
- 1536-1539, 2873 parent_snapshot `4` consts → **STEP** `stack_step(4)`.
- 1086-1098 push_null_value byte-loop `for 0..value_size { put_stack(0u8) }` → **SPECIAL/R4** rounds EACH byte to 8; push one block of `step(value_size)`.
- 628 `get_var::<u32>(0)` return-addr read → **SPECIAL/R3** (see frame-base plan; prefer widening ret-addr to u64).
- 294/860 `args_base = stack_pos - args_size` → **FIXED** (args_size operand already Σ step).
- 615/627 fn_return discard/ret → **FIXED** (codegen aligned spans).
- 597/601 native path `8 +/- stack_pos` → **FIXED** (record header); native push width = Risk R2.
- text.rs 59 `string()` `-= size_ptr()` → **STEP**; 83 debug insert → match.

### Codegen `src/stack.rs`
- `operator()` 133-147: params `-= Σ step(size)` (rework loop to sum step), ret `+= step(ret)`. **E1, highest leverage.**

### Codegen `src/state/codegen.rs`
- 99 args loop `+= size(arg)` → **STEP**; 103 ret slot `+= 4` → **STEP** `step(4)`; 85 dead-body `args_size+4` → `Σ step + step(4)`.
- 117 `frame_hwm` → **STEP** round to 8.
- Slot-bump SPECIAL family (904,929,1143,1224,1447,1958,2000): `advance = step(slot_end) - position` where `slot_end = pos + raw_size`. **THE linchpin — round slot_end to 8.**
- fn-ref `+= 4` (409,1009,2655,2734) / `-= 4` (520,1061,1669,1773,3100): pushes 20B / op returns 16B `text` → **SPECIAL** `step(20)-step(16)` = 8.
- 447 Yield `-= value_size` → **STEP**.
- gen_drop 791-797 `size_code` + `position -= size` + FreeStack discard → **STEP** both.
- emit_push_* 861/874/888 `+= ref_size` + reserve operand → **STEP**; 891 `dep_offset + ref_size` → `dep_offset + step(ref_size)`.
- call paths: 2456/2473/2500 `-= size(arg)` → **STEP-SUM**; 2477/2503/2630 `+= size(ret)` → **STEP**; 2489-2496 OpCall args_size = **Σ step**; 2616-2628 OpCallRef total = Σ step(param)+n·step(size_ref).
- OpCoroutineNext 2333-2349 `-= size_ref(); += byte_size/1` → **STEP**.
- free helpers 2154-2169, 3162-3169 `position -= size(...)` → **STEP**.
- generate_block 2850-2877 `after = to + size(result)` → **STEP** `to + step(size)`.
- 2244 debug assert `expected = size(arg)` → `step(size)` (else fires under alignment).
- 755 / 2236 fn-ref pad threshold `< 16` → SPECIAL/R5 (scale to step).
- 2283/2291 OpStart/OpIterate `+4/+8 - size_ref` → SPECIAL/R6 (key widths FIXED, size_ref term may step).
- all `var_pos`/tuple `elem_abs`/if-match save-restore/`code_add(position)` discard → **FIXED**.

## Frame-base plan (make args_base % 8 == 0 recursively)
1. scopes.rs:189-197 `local_start`: `arg_size = Σ step(size(arg))`, `local_start = arg_size + step(4)`.
2. mod.rs:688-693 `generator_zone2_size`: mirror.
3. codegen.rs:81-85 (dead) + 96-103 (args+ret): Σ step + step(4).
4. codegen.rs:117 `frame_hwm` → step.
5. mod.rs:1979/1990 + 1536-1539/2873: stack_step(4).
6. Return-addr R3: **recommended** widen ret-addr to u64 — `put_stack(self.code_pos as u64)` / `get_var::<u64>(0) as u32` — so offset-0 read is correct and the 8-byte slot is natural.  Probe R3 before committing.

## S5 — text.rs `pos - N`  (N = Σ step(popped arg sizes))
Only **format_single changes numerically: 20 → 24** (f32 pop rounds 4→8); all others already multiples of 8.  Derive each N via the mechanic, not re-hardcoded:
- append_text/append_stack_text: step(16)=16
- append_(stack_)character: step(4)=8
- format_int/stack: step(8)+step(8)=16
- format_float/stack: step(8)×3=24
- **format_single/stack: step(8)+step(8)+step(4)=24** (was 20)
- format_text/stack: step(8)+step(16)=24
- put_text: string() pops step(16); put_var adds step(16) (after put_var STEP fix)

## Ordered, validation-gated edit plan
Gate EVERY commit (flag-OFF identity): `cargo test --test issues`=681/0 AND `LOFT_SLOT_V2=drive cargo test --test issues`=681/0.  Do NOT run --native/full (tmpfs breaks the shell).
- **E1** `operator()` (stack.rs) — params Σ step, ret step. Highest leverage.
- **E2** explicit codegen advances not via operator() (args loop, ret slot, call arg/ret loops + args_size operands, gen_drop, Yield, OpCoroutineNext, free helpers, generate_block, emit_push_*).
- **E3** frame_hwm round (117) + slot-bump SPECIAL family (`step(slot_end)-position`). IR-diff flag-OFF = zero delta.
- **E4** fn-ref SPECIALs (±4→±8) + debug assert expected + pad thresholds (R5).
- **E5** runtime put_var/copy_result/yield-restore/push_null_value(R4)/string().
- **E6** frame base: local_start, generator_zone2_size, execute_argv init, parent_snapshot, return-addr R3.
- **E7** S5 text.rs pos-N derived. Gate on strings+format+wrap.

**Final flag-ON gate:**
- `LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues` → 681/0.
- Differential: per-test stdout default-V1 == `LOFT_ALIGN=1 LOFT_SLOT_V2=drive` (layout-independent).
- Per-test Miri on a pure-compute test (currently aborts at put_stack mod.rs:1663): `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --test issues <test>` → clean.

## Risk register (probe before/while editing)
- **R1** copy_result result width: does caller read ret at step(value) or raw? Probe a 12B-DbRef return.
- **R2** native push width (mod.rs:601): native libs push raw-size; out of S4 interpreter scope, document.
- **R3** return-addr read offset (get_var::<u32>(0)): u64-widen recommended; probe a recursive fn.
- **R4** push_null_value byte-loop: a >8B non-Str struct yield→null; push as one step(value_size) block.
- **R5** fn-ref pad thresholds (`< 16`): probe fn-ref-valued block/arg under aligned.
- **R6** OpStart/OpIterate `was_stack`: probe `for k,v in hash`.
- **R7** bump-family double-rounding when `pos` not 8-aligned (tuple-internal sub-slot): probe tuple-local-with-Str + Miri.
