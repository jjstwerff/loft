<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN104 — text-return promotion corpus (P1, codegen gate step 1)

The working-vs-broken bytecode corpus for the interpreter owned-text leak
(loft-lang/loft#568, bisected to #551). One function per text-return **delivery
path**, so a fix (P3) can prove it changed only the leaking paths and left the
clean ones byte-identical.

## The paths + measured status (main-tip, this branch's base)

Per-path leak = `Direct leak` count from the ASan interpreter build
(`scripts/asan_leak_scan.sh`); return type from `loft introspect corpus.loft`.

| fn | classification | leak? | BROKEN return | notes |
|---|---|---|---|---|
| `ret_fnref` | `Owned:FnRefCall` (pass-2 only) | **LEAK** | `text` (bare) | owned by value → interpreter orphan |
| `ret_index` | `Owned:ViewOfLocal` (pass-2 only) | **LEAK** | `text["v"]` | view of a local freed at scope exit |
| `ret_borrow` | `Borrow(Argument)` | clean | `text["s"]` | borrows the caller's arg — **must NOT promote** |
| `ret_local` | `Owned:BuiltLocal` (pass-stable) | clean | `text["r"]` | already delivered via the local `r` |
| `ret_interp` | `Owned:BuiltLocal` (pass-stable) | clean | `text["__work_1"]` | already retbuf-delivered |

Positive controls = the two LEAK rows; negative controls = the three clean rows
(the harness must keep them clean, and must never promote `ret_borrow`).

## The spec (what P3 must emit)

The two leaking returns must gain a delivery dep like the clean owned rows — a
`__tret`/retbuf so the caller allocates + frees the buffer — WITHOUT touching
`ret_borrow` (arg-borrow, no buffer — the @P273/@P387 reversion) or changing the
already-correct `ret_local`/`ret_interp` emission. The promotion decision keys off
`use_analysis::return_ownership` (the backend-shared verdict `--show-ownership`
renders), not the pass-unstable `classify_text_return` — see the plan.

## Reproduce

```sh
# per-path leak (needs an ASan loft + llvm-symbolizer for the leak:ir_read suppression)
ABIN=target/x86_64-unknown-linux-gnu/release/loft \
  LSAN_OPTIONS=suppressions=.github/lsan_suppressions.txt ASAN_OPTIONS=detect_leaks=1 \
  "$ABIN" --interpret <one-path>.loft

# BROKEN IR (this file's baseline)
loft introspect corpus.loft > broken.ir

# ownership overlay (backend-shared; the fix flips ret_fnref/ret_index to buffer-delivery)
loft introspect --show-ownership corpus.loft
```

`broken.ir` is the captured baseline. `good.ir` (P3) is the post-fix capture; the
diff must be confined to `ret_fnref` + `ret_index` (+ their call sites in `main`).

## P2 result — the oracle pass + the two-class partition

`report_tret_promotions` (env `LOFT_TRET_REPORT`, `parser/control.rs`, run after
`mod.rs:1139`) flags a user text-returning def for promotion iff it has **no** hidden
`&text` retbuf AND its return is backed **frame-locally**:

- `return_ownership == Owned` (a fresh local store — `ret_fnref`), OR
- `Borrowed{base}` / `Join{base}` where `base` is **not an argument** (a view of a
  local — `ret_index`, `base == u16::MAX` = names no visible param → a local view).

A `Borrowed` of an **argument** (`ret_borrow`, `base = 0 = s`) is skipped — the caller
owns it and it outlives the frame. Verified: flags `ret_fnref` + `ret_index`, skips
`ret_borrow`/`ret_local`/`ret_interp`.

**On the real nightly leakers it partitions the class:**

| class | files | P2 |
|---|---|---|
| text-return-tail (this fix) | 387, 85-poison-return-tail-uaf, 85-ncc-container-text-return, 552, 553, 557 | flags |
| **match field-projection (SEPARATE)** | 35n-field-projection, 35p-iterator-match | flags 0 |

`35n`/`35p` leak despite `words` already carrying its `__work_1` retbuf — the orphan is
the match's extracted `w: vector<text>` temporary, not the text return. That is a
distinct bug outside #568's scope (own investigation). The oracle pass making this
boundary visible is P2's main deliverable.
