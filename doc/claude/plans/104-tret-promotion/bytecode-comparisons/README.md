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
