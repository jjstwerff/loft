<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# NEXT SESSION — start here (@PLN85 text-leak → CI memory-model gate)

Cold-start handoff after the 2026-07-10 session. Branch **`mac-work`**. Read order:
this file → [`skip-free-orphan-class.md`](skip-free-orphan-class.md) →
[`generic-monomorph-text-promotion.md`](generic-monomorph-text-promotion.md) →
[`residual-19-analysis.md`](residual-19-analysis.md).

## THE REFRAME (do not lose this)

The point of the whole text-leak effort is **flipping the CI ASan leak gate on**
(`detect_leaks=1`), so the interpreter memory model is VERIFIED in CI and leak
REGRESSIONS are caught automatically. Today CI runs `detect_leaks=0` (UAF/OOB only —
`.github/workflows/miri.yml:249-259`, `ci.yml:892-934`). The comment at
`miri.yml:233-241` names the blocker: **class (2) — "a native fn's `text` result used
as a user function's implicit tail-return is never registered as owned … rebinding to
a local fixes it."** That IS the class this session closed at four sites. Chasing
individual leakers by hand is NOT the goal — the CI flip is.

## What landed this session (16 → 6 leakers, all one root)

Root class: **unpromoted owned-text return** — a fn builds an owned `String` and
returns it by value (borrowing `Str` + discarded frame → interp orphans the buffer;
native RAII drops it). Fix pattern: **promote the return so it delivers through a
hidden `&text` caller buffer** (= "register it as owned" = the CI comment's "rebind").

| commit | site | leakers | pushed |
|---|---|---|---|
| `9b952402` | `__blk_N` block-value hoist scope (scopes.rs) | n3+p241, 16→14 | ✅ |
| `1df1d2ac` | `ViewOfLocalCall` classify (control.rs) | p54_b6, 14→13 | ✅ |
| `b8719d5d` | `FnRefCall` → `wants_tret_bind` (control.rs) | p227×4, 13→9 | ✅ |
| `20a74329` | `promote_monomorph_text_return` (control.rs + mod.rs) | plan17/p243, 9→6 | ✅ |
| `348622c1` | tuple diagnosis doc | — | ✅ |
| `c17ba462` | skip_free-orphan class doc | — | ✅ |
| _pending_ | `collect_consumed_ncc_text` free-after-consumer (scopes.rs) | issue_437, 6→5 | ⏳ |

**FIRST ACTION next session: `git push origin mac-work`** (c17ba462 + this handoff).

Method notes that paid off: matrix-first (a vacuous compile-error cell nearly misled
the tuple diagnosis — always prove the cell can fail); prove the promoted bytecode
byte-matches the non-generic twin; both-backend + full-suite gate every change.

## UPDATE 2026-07-10 (later): the 5 tuple leakers are now FIXED (5→0)

The p329/p330 generic tuple-of-text returns are closed by the generic-return REORDER
(`generic-tuple-return-fix.md`): narrowing the return-promotion guards from
`is_generic_template` to `return_shape_depends_on_type_var` so a concrete-return
generic template rides the existing non-generic `__tuple` promotion, plus broadening
the caller's `OpFreeRefIfDistinct` pairing from `n_` to `t_` callees. Full suite green;
all `probes/generic-tuple-return/` cells leak=0 on both backends. The tracked
interpreter text-leak classes (issue_437 + p329/p330) are now at **0**. The `g1b` forward-ref
case (`-> text` generic monomorph returned through a non-generic caller) is now ALSO
FIXED via a narrow reorder (`control.rs::backward_ref_defnr` maps a monomorph callee to
its backward template for the promotion gate — see `signature-pre-pass-spec.md`
UPDATE). **The entire residual-19 sweep is now leak=0** on the interpreter; full suite
green. Next: recalibrate the CI ratchet baseline (it should drop toward 0) via a CI
run, then flip `detect_leaks=1` to enforcing.

## The (former) 5 leakers — case (b) of skip_free-orphan (see skip-free-orphan-class.md)

A text temp marked `skip_free` (to outlive its block for a consumer) never freed on
interp. Split by value flow:
- **`issue_437` (1) — case (a) consumed-in-place — ✅ FIXED (6→5).** `__ncc_N`
  (`v[i] ?? ""`) copied into the vector element, temp then dead. Freed after the
  consuming statement in `scopes.rs::convert` (`collect_consumed_ncc_text`). The
  escape split fell out structurally: a tail `?? ""` (case b) is the block tail, not
  a statement, so the walker never touches it. Interp-only — native no-ops
  `OpFreeText` (RAII). Full suite green; leak=0 both backends. See the class doc.
- **p329/p330 (5) — case (b) the temp IS the returned value.** Tuple element is a live
  view into `__ret_text_N` on interp; any in-function free UAFs. Needs the caller to
  own the buffer — the `__tuple`-for-generics route, which has a documented
  "broke p329/p330/p240/plan17" landmine (`tuple_return_rewrite` `from_tv` gate).

## Two paths to the CI flip (the actual deliverable) — DECISION PENDING

The user leans **ratchet-now**; confirm before implementing.

1. **Clean flip (policy-pure, `miri.yml:239` "fix, don't suppress"):** drive the 6 to
   zero first (build the skip_free escape analysis), then `detect_leaks=1` + only the
   `leak:ir_read` suppression (~16 ir_read/ir_schema/ir_store lib tests). Waits on the
   hard arc.
2. **Ratchet now (RECOMMENDED) — DRAFTED, awaiting a calibration CI run.**
   `detect_leaks=1` + `ir_read` suppression + a documented, SHRINKING COUNT baseline;
   the gate asserts "Direct leak roots ≤ baseline." Catches every NEW leak immediately;
   baseline ratchets to 0. **A per-frame allowlist does NOT work** — every residual
   text leak shares the `append_text`←`execute_argv` frames, so a `leak:<frame>`
   suppression can't tell a known leaker from a new one; the gate is a COUNT instead.
   - **Landed (this session):**
     - `.github/lsan_suppressions.txt` — the single `leak:ir_read` suppression
       (verified locally: 42→1 roots on `g3_tuple_return`).
     - `scripts/asan_leak_ratchet.sh` — runs an ASan binary under `detect_leaks=1`,
       counts `^Direct leak` roots, compares to `LEAK_BASELINE`; fails on growth,
       warns (ratchet-down) when below. Locally validated 3 ways (OK / new-leak-fail /
       fixed-below-baseline) + missing-binary guard.
     - New "ASan leak ratchet" step in **both** `miri.yml` (both-OS matrix) and
       `ci.yml` (ubuntu; timeout 30→45), reusing the `issues` ASan binary the sweep
       built. Marked `continue-on-error: true` + `LEAK_BASELINE: '0'` for CALIBRATION.
   - **NEXT: run the nightly `miri.yml` (workflow_dispatch), read "Direct leak roots"
     from each OS leg, then** (a) set `LEAK_BASELINE` to that count (per-OS via matrix
     if ubuntu ≠ macOS), and (b) delete `continue-on-error` in both workflows to make
     the ratchet enforcing.
   - **CAVEAT: the baseline number cannot be pinned on this macOS box** — Linux LSan
     root counts differ from ARM. The calibration run supplies both.

## Tooling / repro (this box)

- ASan interp binary (leak oracle): build ISOLATED so nightly env never leaks into a
  `--native` shell (that pollutes `target/release/deps/libloft_ffi` → false E0514 —
  see [[mac_sanitizer_toolchain]]):
  `export PATH=".../nightly-.../bin:$PATH"; export RUSTUP_TOOLCHAIN=nightly` then ONLY
  `RUSTFLAGS=-Zsanitizer=address cargo build --release --bin loft --target aarch64-apple-darwin --target-dir target/asan`
  ; `ln -sfn "$PWD/default" target/asan/aarch64-apple-darwin/release/default`.
- Leak count for a probe: `ASAN_OPTIONS=detect_leaks=1 <asan-bin> --interpret p.loft 2>&1 | c++filt | grep loft::fill::append_text | grep -v ir_read | wc -l`.
- Native leak / correctness: plain `target/release/loft --native` in a FRESH (stable) shell.
- Stable rustc moved 1.96.1 → 1.97.0 on 2026-07-10 → needed a full `cargo clean`.
