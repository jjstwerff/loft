<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — S4 PROGRESS & SESSION HANDOFF (2026-05-30)

Read this first. It is the resumable state of the S4 (eval-TOS / full
stack alignment) work. Companion docs:
- [`cluster-2-S4-plan.md`](cluster-2-S4-plan.md) — the implementation
  spec (site classification, E1–E7, risk register R1–R7).
- [`cluster-2-fix-design.md`](cluster-2-fix-design.md) — the design +
  the frame-slot (V2 allocator) half.

**Branch:** `plan-53-sanitizer-ci-lever`.  **HEAD:** `8c0a5e72`.
**All work committed AND pushed.**  Working tree clean.  NO open PR.

## TL;DR

S4 makes loft's byte-packed eval stack 8-aligned, gated behind env
`LOFT_ALIGN`.  It went from *instant SIGSEGV* → the aligned suite now
runs from the start through ~`p117`.  Flag-OFF (default, no `LOFT_ALIGN`)
is **byte-for-byte identical** to before at every commit — `main`-quality
is untouched.  Two crashers remain (`non-empty c60_*`, `p117`), both
well-characterized below.

## HOW TO RUN / TEST (read before doing anything)

```bash
# Default (production) — flag OFF, V1 drives, guards absent.  MUST stay 681/0.
cargo test --test issues

# V2 frame-slot allocator drives the whole suite (no eval-TOS alignment yet):
LOFT_SLOT_V2=drive cargo test --test issues          # MUST be 681/0

# FULL aligned mode (eval-TOS + V2 frame slots) — the S4 acceptance path:
LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues

# With the homegrown stack-UB guards ARMED (the workhorse for finding bugs):
LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --features stack_align_guard --test issues
```

### THREE HARD-WON PROCESS RULES (cost many hours to learn — obey them)

1. **`cargo clean -p loft` before testing S4 changes.**
   `[profile.dev.package.loft] debug-assertions = false` + incremental
   compilation routinely leaves a STALE `libloft` linked into the test
   binary, so `cargo test` runs code that does NOT match your source.
   This sent me chasing ghosts for hours (a8, p117).  When a result is
   surprising, `cargo clean -p loft && cargo test …`.  Symptom of a
   stale `cargo test --no-run`: it prints `Executable …` WITHOUT
   `Compiling loft`.

2. **Guards are gated on the `stack_align_guard` cargo feature, NOT
   `debug_assertions`** — because the lib disables `debug_assertions`
   (rule 1), so `cfg(debug_assertions)` is OFF inside this crate even
   under `cargo test`.  An earlier version of the guards used
   `cfg(debug_assertions)` and was silently compiled out (never ran).
   Run the S4 work with `--features stack_align_guard`.

3. **The trace path (`LOFT_LOG=full` → `execute_log_impl`) RESETS
   `stack_pos` to the codegen-tracked position on every op** for display,
   which MASKS runtime drift.  So a `LOFT_LOG=full` dump shows
   *correct* slots even when the real `execute` path is drifting and
   crashing.  For wrong-value bugs, instrument the REAL `execute_argv`
   path (`src/state/mod.rs`), not `execute_log`.  (The dump's *static
   bytecode* section is fine; its *runtime* `[stackpos]` trace lies.)

## THE GUARDS (the single biggest productivity win — use them)

Two debug-only guards, feature-gated (`stack_align_guard`), zero
production cost (the calls + their arg computation don't exist without
the feature).  They are a **homegrown Miri-for-the-stack** that fires on
ANY rustc at the access site.

- **Frame invariant** (`execute_argv` + `execute_log` loops,
  `src/state/mod.rs` ~2112, `debug.rs` ~1066): asserts `stack_pos % 8 ==
  0` after every op.  Catches frame *drifts* — names the op (pc + fn).
- **Access invariant** (`State::check_stack_align<T>`,
  `src/state/mod.rs` ~1384; called from `get_stack`/`put_stack`/
  `get_var`/`mut_var`/`put_var` and `text.rs` `string`/`set_string`/
  `string_mut`/`string_ref_mut`): asserts the SLOT address is a multiple
  of `align_of::<T>()`.  Catches the cluster-2 unaligned-`&T` UB exactly,
  at the access, naming the type + offset.

**They caught `c60` in ONE run** (`i64 at abs offset 116`) — what took
`a8` hours of gdb/valgrind before the guards worked.  When a crasher is
an *alignment* bug the guard fires loudly; when it's a *wrong-value /
slot-collision* bug (a8, p117) the guard stays silent — that distinction
alone tells you which kind of bug you're chasing.

## WHAT IS DONE (committed + pushed, oldest→newest)

| Commit | What |
|---|---|
| `25923f31` | drop dead V2-allocator scaffolding |
| `f933a4b2` | `Stack::var_pos` checked helper + report unfilter (better debug) |
| `36abfe8d` | **fix:** `compute_intervals` recurse into Yield/BreakWith/Parallel (latent liveness bug + latent V1 gap) |
| `9bf0894a` | scaffold the `LOFT_ALIGN` flag + `aligned_stack_step` seam (off by default) |
| `84ae1473` | the S4 implementation spec doc |
| `c87e1fb9` | E1 — `Stack::operator` advances stepped |
| `800c74c9` | E2a — codegen frame-setup advances stepped |
| `e2141bdb` | E2b — codegen call-path advances + `args_size = Σ step` |
| `904a2ca9` | E2c/E3/E4 — remaining codegen advances: slot-bump family, fn-ref ±4→±(step20−step16), gen_drop/Yield/coroutine/free-helpers/generate_block |
| `286046ca` | E5 — runtime `put_var`/`copy_result`/yield-restore/`push_null_value`(R4) stepped |
| `8d8fb510` | E6/E7 — frame base (`local_start`, generator zone) + text `pos−N` (only `format_single` 20→24) |
| `7e34fd4b` | **R2 — alignment-aware native marshalling** (`Stores::get`/`put` step; `Stores.aligned_stack` on all 4 literals). Cleared the b7/JsonValue family |
| `ad34f8c8` | E6 — entry frame base = `step(4)` (8-aligned, `execute_argv`) |
| `caffdca6` | the alignment guards (first cut) |
| `ea459261` | **a8 — `gen_text_dest_call` pops stepped** (the twin of `try_text_dest_pass` I'd missed; raw DbRef pop `12` vs `step(12)=16` → −4 drift → garbage var read) |
| `be516ec9` | **guards re-gated on the `stack_align_guard` feature** (was dead under `cfg(debug_assertions)`) — and proven to fire |
| `8c0a5e72` | **c60/R6 — `OpStart`/`OpIterate` `was_stack` spans stepped** (OpIterate pushes TWO u32s = `step(4)+step(4)`, not `step(8)`) |

**Verification at every commit:** flag-OFF `issues` 681/0, `drive` 681/0,
flag-ON canary passing, clippy clean.

## WHAT REMAINS (the two open crashers)

### 1. Non-empty `c60_*` hash iteration — WRONG VALUE (not a crash)
`c60_hash_iter_single_field_asc`, `_multi_field_lex`, + 2 more.  Under
`LOFT_ALIGN=1 LOFT_SLOT_V2=drive` they produce an **extra leading
element**: `,apple,mango,zebra,` vs `apple,mango,zebra,`.  Guard silent
(it's a wrong-value, not misalignment).  `c60_hash_iter_empty` is FIXED.
Likely the per-element key-gather (`OpNext` / `gather_key`) path under
aligned, or a residual `was_stack`/start-position off-by-one for the
non-empty case.  `OpStart` appears to be a dead codegen arm — confirm
which ops the non-empty for-loop actually emits (`LOFT_IR=test`).

### 2. `p117_gl_multi_text_struct_per_frame` — ACCUMULATING DRIFT (hardest)
`struct FileRef { name: text, found: boolean }`; `lookup` returns a
`FileRef` (a `Reference`/DbRef); in a 1000-iteration loop 3 lookups +
3 `.found` boolean reads + 3 string interpolations.  SIGSEGV in
`ops::format_text` (a work-buffer `String` accessed at a drifted offset
→ garbage `Str` → `push_str` derefs garbage).  Guard SILENT (wrong-value,
the access stays 8-aligned but lands on the wrong/clobbered slot).

**Diagnosis so far:** the real `execute` path drifts a few bytes per
loop iteration; the trace path masks it (process rule 3).  The first
iteration's work-buffer slots are correct; a later iteration's access is
off.  Prime suspect: the struct-`Reference` `lookup` return path under
aligned — `copy_result`/`fn_return`/`PutRef`, or the `.found` **boolean**
(1 byte → `step(1)=8`) read/push.  **Next step:** instrument the REAL
`execute_argv` loop (NOT `execute_log`) — print `stack_pos` at the
loop-back op each iteration and bisect which op leaves the extra bytes.

## ARCHITECTURE QUICK MAP (where the stepping lives)

- **The seam:** `variables::aligned_stack_step(size, aligned)` =
  `if aligned { size.next_multiple_of(8) } else { size }`
  (`src/variables/mod.rs`).  Identity when off ⇒ flag-OFF unchanged.
- **Runtime:** `State.aligned_stack` (env, both ctors incl. worker);
  `State::stack_step`; routed `get_stack`/`put_stack`/`reserve_frame`/
  `put_var`/`copy_result`/text ops (`src/state/mod.rs`, `text.rs`).
- **Native ABI:** `Stores.aligned_stack`; `Stores::get`/`put` step
  (`src/database/mod.rs`).  Set on ALL FOUR `Stores` literals (new +
  3 clone/worker; the worker ones in `src/database/allocation.rs`).
- **Codegen:** `Stack.aligned` (env); `Stack::step` (`src/stack.rs`);
  ~all `position +=/-=` advances in `src/state/codegen.rs` routed
  through it.  Rule: codegen emits the *stepped* advance/operand;
  runtime span-operands (discard/ret/args_size) are consumed RAW.
- **Frame model:** entry base `step(4)`=8 (8-aligned); `local_start =
  Σ step(arg) + step(4)`; V2 places slots at `align`-multiples;
  `frame_hwm` rounded to 8.

## REMAINING SPEC ITEMS / KNOWN-DEFERRED

- **Miri gate** (the original cluster-2 detector): once the suite is
  green under `LOFT_ALIGN=1 LOFT_SLOT_V2=drive`, run
  `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --test
  issues <pure-compute-test>` — currently aborts at `put_stack`
  (mod.rs) on the unaligned i64 push; S4 should clear it.  Miri is
  installed.  (The `stack_align_guard` feature is the cheaper everyday
  detector; Miri is the gold-standard final check.)
- **R2 native push width (the `--native` backend)**: OUT of scope —
  `codegen_runtime.rs` natives use real Rust values, not the byte-packed
  stack.  Confirm unaffected (do NOT run `--test native`: the 7.5G
  `/tmp` tmpfs + concurrent `cc` links exhaust it and BREAK THE SHELL).
- The `o`/`p`-range and later suite past `p117` is unexplored under
  aligned — expect a few more distinct crashers, each now self-locating
  (alignment) or instrument-the-execute-path (wrong-value).

## DEFINITION OF DONE (for S4)

`LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues` → 681/0,
AND `--features stack_align_guard` clean (no guard fires), AND a
per-test Miri run clean, AND flag-OFF + `drive`-only still 681/0.  Then
flip the default (or keep S4 behind the flag and mark cluster 2
"fixed behind `LOFT_ALIGN`" per the plan's known-deferred policy — the
@PLAN53 deliverable is the CI lever, not making V2 the default).
