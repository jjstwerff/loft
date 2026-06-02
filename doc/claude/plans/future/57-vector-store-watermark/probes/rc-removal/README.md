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
   - **Mechanism (VERIFIED by store trace, probe 12 + `RC_OFF`):** `make()` allocates
     the cell (`#3`); on return, `RC_OFF` frees `#3` at the frame's scope exit
     (`- free #3`); the next `make()` **reuses the slot** (`+ alloc #3`) — so f1 and f2
     alias the same reused store → `store()` UAF (`allocation.rs:472`).  rc suppressed
     that frame-exit free (the closure `inc_rc`'d the cell on capture).  Sequential
     (probe 11) is fine because f1 is done before the slot is reused.
   - **Implication for the fix (Phase B):** a closure value must **own** its captured
     cells (cell lifetime tied to the closure, freed when the closure dies), so the
     defining-frame free no longer reuses a still-referenced slot.

2. **Three pre-existing, rc-INDEPENDENT closure bugs surfaced** (crash/leak with rc
   *on* too — out of scope for rc removal, filed as their own probes here):
   - **06** capturing a `vector` cell in an escaping closure crashes (both backends).
   - **08** storing closures in a vector crashes (both backends).
   - **10** a closure passed as a fn argument leaks on `--interpret` (ok native).

3. **Text/vector scope-frees do NOT need rc** — every t1–t8 probe survives.  The one
   `RC_OFF` text leak in the suite (`03-text.loft`, `kt=29 main_vector<text>×1`,
   store #16) is **MINIMIZED** (`t9_split_temp_leak`): `split()` (a native stdlib
   builtin) returns a `vector<text>` used as an **unbound temporary** (consumed by
   `len(...)` / `.join(...)`) → the temporary leaks **on the interpreter only** under
   `RC_OFF` (native clean).  Binding to a local first is clean; a *user* fn returning a
   vector temp is clean — so it is `split`-specific (the native builtin's temporary-
   result cleanup leans on rc), a narrow interpreter scoping gap, NOT a general
   fn-returned-temp gap.  Phase-A fix: statement-end free of a native-vector builtin's
   temporary on the interpreter without rc.

## Phase plan (from the map)

- **Phase A** — MINIMIZED (`t9_split_temp_leak`): free a native-`split` builtin's
  unbound `vector<text>` temporary at statement-end on the interpreter without rc.
- **Phase B** — closure-cell ownership: the real blocker (the coexistence cases above).
- **Phase C** — delete `ref_count` / `OpIncRc` / `inc_rc` / `dec_rc`; verify both backends.

The pre-existing closure bugs (06/08/10) are siblings — not on the rc-removal path, but
worth fixing on their own (vector/struct capture is a real gap).
