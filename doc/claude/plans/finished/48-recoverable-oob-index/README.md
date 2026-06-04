<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 48 — Recoverable out-of-bounds index (@P356)

## Status — SHIPPED 2026-05-26

All five sub-arcs landed; both backends now make explicit `v[i]` / `s[i]` /
negative-index out-of-bounds a **recoverable** fault (typed null sentinel +
`Warn` log + continue), with `LOFT_DEV_SOFT_HALT` for opt-in fail-fast.  Full
suite green; the script that crashed (`37-stress`) passes on both backends.
Bug record + the four-part fix summary: [PROBLEMS.md @P356](../../../PROBLEMS.md).

**The gating root-cause finding corrected the original framing:** the crash
that blocked the first (reverted) attempt was NOT the element getters — it was
`gen_if` branch-leveling for the `expr ?? null` ncc (the `null` false branch
skipped to the join without pushing the value-sized slot, mis-offsetting every
later slot → the ncc temp's `OpFreeText` freed uninitialised memory → SIGSEGV).
Pinned via `LOFT_DEV_SOFT_HALT` as a zero-code-change reproduction harness +
`OpInitText`/`OpFreeText` offset instrumentation (`free abs=72` vs inits at
`64`/`88`).

## Goal

Make an explicit out-of-bounds vector/text/negative index a recoverable
fault — null sentinel + `Warn` log + continue, identical on interpreter and
`--native`, with `LOFT_DEV_SOFT_HALT` as the opt-in fail-fast — without
introducing the latent text-free / `??`-aliasing crashes the current halt
masks.

## Effort + design

- **Effort:** MH (4 subsystems: runtime raise path, parser `??` swap, element
  getters / scope-free, plus a test/baseline flip + script re-sweep)
- **Design:** ~ (arc mapped; one gating unknown — see Open design questions)
- **Last touched:** 2026-05-26

## Background — why it's not a one-shot fix

The behaviour change itself is trivial (route the index helpers through a
`raise_recoverable` that logs-and-continues unless `LOFT_DEV_SOFT_HALT`), and
it makes both backends agree (`x=null`, exit 0).  But OOB previously *halted*
the interpreter — the dispatch loop checks `runtime_error.is_some()` after each
op ([`src/state/mod.rs`](../../../../../src/state/mod.rs) ~1966) — so two latent
bugs downstream of an OOB site were never reached.  Making OOB continue
surfaces them:

1. **`??` / ncc keeps the *raising* op for non-int element types.**  The
   swap-to-nullable helper ([`src/parser/operators.rs`](../../../../../src/parser/operators.rs)
   `rewrite_outer_arith_to_nullable`, ~899-906) recurses into an inner
   `OpGetVector` only when the outer wrapper is `GetInt|GetInt4|GetByte|GetShortRaw`.
   It MISSES `GetText`/`GetSingle`/`GetFloat`/`GetShort`/`GetEnum`/`GetCharacter`/
   `GetField`/`GetDbRef`, and the **two-level** `vector<boolean>` shape
   `OpEqInt(OpGetByte(OpGetVector))`.  (`OpGetLong`/`OpGetBool` do **not**
   exist — `long`→`OpGetInt`, `bool`→`OpEqInt(OpGetByte)`.)  So `tv[0] ?? null`
   over a `vector<text>` keeps the raising `OpGetText(OpGetVector(...))`.
2. **Element getters read the `{rec:0}` sentinel unsafely.**  `OpGetText`
   ([`src/fill.rs`](../../../../../src/fill.rs) `get_text`) does
   `get_str(get_u32_raw(0, fld))` — `get_u32_raw(0,…)` reads the store *header*
   (record 0), yields a non-zero garbage word, and `get_str`'s `rec==0` guard
   ([`src/store.rs`](../../../../../src/store.rs)) checks the *argument*, not
   `db.rec` — so it returns a `Str` with a wild ptr/len.  `OpAppendText`
   copying that into the ncc temp corrupts the heap; the crash surfaces later
   in `OpFreeText` → `string_mut(pos).shrink_to(0)`
   ([`src/state/text.rs`](../../../../../src/state/text.rs):307) as
   `Tried to shrink to a larger capacity`.

Text locals are always owned 24-byte `String`s freed unconditionally (deps are
NOT consulted for free — [LIFETIME.md](../../../LIFETIME.md) "always freed"), so
the fix is at the read/assign site, not the free-emission site.

## Sub-arcs

| Item | Source / site | Status |
|---|---|---|
| **0** — Empirically pin the exact corruption op(s) | `LOFT_DEV_SOFT_HALT` harness + init/free offset instrumentation | **DONE** — root cause = `gen_if` ncc leveling (Open question 1) |
| **A** — `gen_if` pushes a typed null on the false path of an expression `if {value} else null` | `src/state/codegen.rs` `gen_if` (the `f_val == Value::Null` branch) | **SHIPPED** |
| **B** — Complete the `??`→nullable swap (all element getters + 2-level bool) | `src/parser/operators.rs` `rewrite_outer_arith_to_nullable` | **SHIPPED** |
| **C** — `raise_recoverable` / `_runtime` + route index helpers | `src/state/mod.rs`, `src/database/mod.rs` (`vec_get_or_raise`, `text_char_or_raise` + `_runtime` peers) | **SHIPPED** |
| **D** — Flip tests + baselines | `runtime_errors.rs`, `runtime_logging.rs` (2), `error_messages` baselines `20`/`21`, `strings.rs` doc | **SHIPPED** |
| **E** — Regression, all element types, both backends | `tests/scripts/repro_p356.loft` | **SHIPPED** |

## Phase ordering

1. **Item 0 (gating).**  The static investigation's obvious fix (guard
   `OpGetText` on `db.rec==0`) was *already tried in the reverted attempt and
   did NOT stop the crash* — so a second corruption site exists, most likely
   the `else null` branch writing a 16-byte null into the 24-byte owned-`String`
   slot of `r:text["dep"]`.  Instrument the minimal repro to identify the exact
   op(s) before writing any fix.  Resolution → a recorded decision.
2. **A + B together.**  A makes the sentinel safe to read/assign/free; B makes
   `??` route text/float/etc. index through the nullable op.  Land together —
   B alone exposes A's crash; A alone leaves the raising op in `??`.
3. **C.**  Only after A+B is the continue-past-OOB path crash-free.  Add
   `raise_recoverable` (log `Warn` + return; `LOFT_DEV_SOFT_HALT`→`raise`) and
   route `vec_get_or_raise` / `vec_ref_or_raise` / `text_char_or_raise` and the
   three `_runtime` peers through it.
4. **D.**  Flip the 4 `runtime_errors.rs` index tests (exit 1 → exit 0 + null +
   warn), regen baselines `20`/`21` (keep `22_negative` — already exit 0), and
   **empirically** re-run each abort-relying script (the grandfather list:
   37-stress / 06-structs / 11-vectors / 93-vector-advanced / 96-slot-assign /
   07-vector / 16-parser / 15-lexer / 23-safety) — a `??`-guard is NOT proof of
   safety (37-stress crashes at line 116 today).
5. **E.**  Lock in with a both-backends regression covering every element type.

## Open design questions

1. **(Gating — RESOLVED 2026-05-26) Where is the corruption?**  Pinned via
   `LOFT_DEV_SOFT_HALT=1` (continues past the OOB raise, reproducing the crash
   on the current build with NO code change — the validated harness for this
   arc).  Findings on the minimal repro:
   - Crash is `OpFreeText` → `string_mut(pos).shrink_to(0)`
     ([`src/state/text.rs`](../../../../../src/state/text.rs):307) on a slot
     `ptr=0x0 cap=1 len=8` — **not a valid owned `String`** (`cap < len`,
     null ptr), freed mid-statement (the ncc temp `_ncc_3`, before the
     following `print` runs).
   - It is **specific to `?? null` over an OOB text index**: `?? "literal"`,
     `int ?? 0`, and an in-range `?? null` all run clean.
   - It is **NOT a raw `OpPutText` write** (instrumented `put_text` — never
     fires) and **NOT fixable by a getter guard** (tried in the reverted
     attempt; the slot is corrupt regardless of what `OpGetText` returns).
   - **Decisive:** instrumenting `OpInitText`/`OpFreeText` absolute slot
     offsets shows `OpFreeText` frees `abs=72` — a slot **no `OpInitText` ever
     initialised** (the inits are at `abs=64` and `abs=88`).  So the free
     resolves its compile-time-constant slot offset against a runtime
     `stack_pos` that is **8 bytes off**, landing on uninitialised stack
     garbage (`ptr=0 cap=1 len=8`).
   - **Conclusion:** the crash is a **stack-offset mismatch**, not a value /
     getter / free-representation bug.  It is specific to the `text-view ?? null`
     shape, which strongly implicates **`gen_if` branch-stack-leveling for the
     ncc** ([`src/state/codegen.rs`](../../../../../src/state/codegen.rs):631,
     the B5 leveling at ~673): the `else null` branch (`OpConvTextFromNull` /
     the `Null→text["dep"]` convert) leaves a different stack delta than the
     `_ncc_3` text-view branch, so the join is mis-levelled by 8 bytes and
     every subsequent slot offset (including `_ncc_3`'s own free) is shifted.
     `?? "literal"` does not crash because both branches push a balanced
     16-byte `Str`.  **Item A retargeted:** fix the `text ?? null` (and
     generally `text-with-dep ?? null`) branch-leveling / `Null→text` convert
     stack delta in `gen_if` / `build_null_coalesce_default`, NOT the getters
     (B is still needed so `??` selects the nullable op, but A — the leveling —
     is the crash fix).  Next: compare the generated stack delta of the
     `null` branch vs the text branch and the `Null→text` convert lowering.
2. **Does the `??` swap need the recursive walker?**  No — the format-string
   recursive `rewrite_subtree_to_nullable` would over-swap nested faults that
   must keep trap semantics (`(a/b)[i] ?? fb` must still trap on `a/b`).  Keep
   the narrow non-recursive helper; just widen its outer-op match + add the
   boolean two-level hop.
3. **Warning visibility with no logger attached.**  Logger logs index OOB at
   `Severity::Warn`; the compile-time `v[i]`-undefended warning already nudges
   `?? <fallback>`.  Decide whether a plain-CLI run (no logger) also emits a
   deduped stderr note, or stays quiet-but-logged.  Lean quiet-but-logged
   (compile-time warning covers developer awareness).

## Cross-arc dependencies

- [plan-21 retire-scratch](../../21-retire-scratch/) — text values are
  `Str` views into `Stores::scratch`; this plan touches the same text-lifetime
  surface (item A) but does not depend on retiring scratch.
- [plan-42 warning-quality](../../future/42-warning-quality/) — the runtime `Warn`
  on OOB + the existing compile-time undefended-index warning are the same
  observability surface; keep the messages consistent.

## See also

- [PROBLEMS.md @P356](../../../PROBLEMS.md) — issue history + the reverted attempt.
- [LIFETIME.md](../../../LIFETIME.md) — text dep / always-free model (item A).
- [feedback: avoid runtime errors] — recoverable-fault policy driving the
  semantics decision.
- Closure record only (SHIPPED) — removed from ROADMAP.md per the maintenance
  rule (was category **S**, silent divergence + hang); closure lives here + in
  CHANGELOG_TECHNICAL.md + git history.
