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
| `c17ba462` | skip_free-orphan class doc | — | **⚠ UNPUSHED** |

**FIRST ACTION next session: `git push origin mac-work`** (c17ba462 + this handoff).

Method notes that paid off: matrix-first (a vacuous compile-error cell nearly misled
the tuple diagnosis — always prove the cell can fail); prove the promoted bytecode
byte-matches the non-generic twin; both-backend + full-suite gate every change.

## The remaining 6 leakers — ONE class: skip_free-orphan (see skip-free-orphan-class.md)

A text temp marked `skip_free` (to outlive its block for a consumer) never freed on
interp. Split by value flow:
- **`issue_437` (1) — case (a) consumed-in-place.** `__ncc_N` (`v[i] ?? ""`) copied
  into the returned vector, temp then dead. Fixable by a per-last-use free — but needs
  ESCAPE ANALYSIS (a `?? ""` that IS the return tail is case b, must stay alive), is
  per-iteration inside a loop, sits in the `??` operator (94 test files), and native
  keys on the `__ncc_* skip_free` pattern in 5 places. NOT a patch.
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
2. **Ratchet now (RECOMMENDED):** `detect_leaks=1` + `ir_read` suppression + a
   documented, SHRINKING allowlist/baseline of the 6 known frames; gate asserts "no
   leak beyond baseline." Catches every NEW leak immediately; allowlist ratchets to 0.
   - Draft: flip `ASAN_OPTIONS` in `miri.yml`'s "ASan sweep" step (+ `ci.yml:933`);
     add `lsan_suppressions.txt` (ir_read) + a baseline check. The existing sweep
     harness (`probes/text-tail-return/sweep_owners.sh`, `probes/residual-19/run.sh`)
     is the detector to wire in.
   - **CAVEAT: cannot be validated on this macOS box** — Linux LSan frames/counts
     differ. Needs a CI run to confirm the exact frames + baseline number. Confirm on
     the ubuntu-x86_64 leg before landing.

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
