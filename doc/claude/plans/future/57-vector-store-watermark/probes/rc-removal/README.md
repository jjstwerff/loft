<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# rc-removal probe corpus

Maps **where store ref-counting is load-bearing** before the rc-removal refactor
touches any code (the [tail-end experiment](../../fix-design-store-lifetime.md#tail-end-experiment--disable-store-ref-counting-once-scoping-is-correct)).
Each probe is a minimal closure / text-store shape; run it with `RC_OFF=1` (free at
rc≤0 always) vs rc-on, on both backends, and classify crash / leak / wrong-output / ok.

Run: wrap each `fn test_*` with `fn main() { test_*(); println("PROBE_OK"); }` and
compare `RC_OFF=1` vs default, `--interpret` vs native.

## The map (2026-06)

| probe | shape | rc-on | RC_OFF | verdict |
|---|---|---|---|---|
| 01 single_factory_escape | one escaping closure, mutate cell | ok | ok | scope covers |
| 03 capture_readonly_escape | escaping closure, read-only cell | ok | ok | scope covers |
| 04 no_escape_inframe | closure used in its own frame | ok | ok | scope covers |
| 05 capture_text_escape | escaping closure captures text | ok | ok | scope covers |
| 07 nested_closure_escape | closure returns a closure | ok | ok | scope covers |
| 11 two_factory_**sequential** | f1 done *before* f2 made | ok | ok | scope covers |
| **02 multi_factory_escape** | two factories, calls interleaved | ok | **CRASH** | **rc-dependent** |
| **12 two_closures_coexist** | two closures coexist, calls *not* interleaved | ok | **CRASH** | **rc-dependent** |
| **09 factory_loop_churn** | many coexisting factory closures in a loop | ok | **CRASH** | **rc-dependent** |
| 06 capture_vector_escape | escaping closure captures a vector | **CRASH** | CRASH | **pre-existing bug** |
| 08 closure_in_vector | closures stored in a vector | **CRASH** | CRASH | **pre-existing bug** |
| 10 closure_passed_as_arg | closure passed to a fn (interp) | **LEAK** | LEAK | **pre-existing bug** |
| t1–t8 | vector<text> build/reassign/append/return/concat/slice/nested/struct | ok | ok | scope covers |

## Findings

1. **rc is needed ONLY for ≥2 COEXISTING closures that own captured cells.**  Single
   (01), read-only (03), in-frame (04), text (05), nested (07), and *sequential*
   factory closures (11) all survive `RC_OFF`.  The break is **coexistence** (02 / 12 /
   09), and it is NOT about interleaved calls (12 crashes without interleaving).
   - **Mechanism:** a captured cell lives in the defining frame's store.  Under
     `RC_OFF` that store is freed at the frame's scope exit and the slot is **reused**
     by the next `make()`'s cell — so two coexisting closures end up pointing at the
     same reused slot → corruption / `store()` UAF (`allocation.rs:472`).  rc papers
     over this by keeping each captured cell alive until its closure dies.
   - **Implication for the fix (Phase B):** a closure value must **own** its captured
     cells (cell lifetime tied to the closure, freed when the closure dies), so the
     defining-frame free no longer reuses a still-referenced slot.

2. **Three pre-existing, rc-INDEPENDENT closure bugs surfaced** (crash/leak with rc
   *on* too — out of scope for rc removal, filed as their own probes here):
   - **06** capturing a `vector` cell in an escaping closure crashes (both backends).
   - **08** storing closures in a vector crashes (both backends).
   - **10** a closure passed as a fn argument leaks on `--interpret` (ok native).

3. **Text/vector scope-frees do NOT need rc** — every t-probe survives.  The one
   `RC_OFF` text leak in the suite (`03-text.loft`, `kt=29 main_vector<text>×1`,
   store #16) reproduces only in the *full* file, not in any isolated section — a
   full-file store-reuse interaction to minimise during the fix phase.

## Phase plan (from the map)

- **Phase A** — the `03-text` full-file text leak (minimise from store #16).
- **Phase B** — closure-cell ownership: the real blocker (the coexistence cases above).
- **Phase C** — delete `ref_count` / `OpIncRc` / `inc_rc` / `dec_rc`; verify both backends.

The pre-existing closure bugs (06/08/10) are siblings — not on the rc-removal path, but
worth fixing on their own (vector/struct capture is a real gap).
