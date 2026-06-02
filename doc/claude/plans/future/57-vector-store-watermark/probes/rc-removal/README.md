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
| **02 multi_factory_escape** | two factories, calls interleaved | ok | **CRASH** | **Mechanism B** (cell ownership) |
| **12 two_closures_coexist** | two closures coexist, calls *not* interleaved | ok | **CRASH** | **Mechanism B** |
| **09 factory_loop_churn** | many coexisting factory closures in a loop | ok | **CRASH** | **Mechanism B** |
| **10 closure_passed_as_arg** | closure as an UNBOUND arg temporary | **LEAK** | LEAK | **Mechanism 1** (unbound temp) |
| **t9 split_temp_leak** | `split()` vector temp, unbound | ok | **LEAK** (interp) | **Mechanism 1** |
| t1–t8 | vector<text> build/reassign/append/return/concat/slice/nested/struct | ok | ok | scope covers |

(06 / 08 — closure ↔ collection — moved to [`../closure-collection/`](../closure-collection/):
NOT rc-related, a separate closure-record-layout limitation.)

## Findings — the residual is TWO mechanisms, not five bugs

**Mechanism 1 — an UNBOUND heap-returning-call temporary has no statement-end free.**
A call returns a heap value used inline (not bound to a local); no work-ref buffer + no
`OpFreeRef` is emitted, so cleanup leans on rc.  Two instances, one root:
- **`t9`** — `len("a,b".split(','))`: the native `split()` vector temp leaks on the
  **interpreter** under `RC_OFF` (rc covers it normally; native is clean — it uses an sret
  buffer).  IR-confirmed: no `OpFreeRef` for the split result, vs a *user* fn whose result
  goes through an sret `__ref_N` buffer that IS freed.
- **`10`** — `apply(make())`: the closure temp (`__cell_*` + `__closure_*`) leaks **with rc
  on, always** — rc doesn't even cover closure temps here.
- Both are leak-free when **bound to a local first** (the local's scope-free handles it).
- **One fix:** give an unbound heap-returning-call temporary a statement-end free (an
  sret-style buffer / `OpFreeRef`).  Covers `t9` (Phase A) **and** stray `10`.

**Mechanism B — a captured cell is freed at the DEFINING frame's exit, not the closure's.**
rc is needed ONLY for **≥2 coexisting closures** that own captured cells (02 / 12 / 09);
single (01), read-only (03), in-frame (04), text (05), nested (07), and *sequential* (11)
all survive `RC_OFF`.  The break is **coexistence**, not interleaving (12 crashes without
interleaved calls).
- **Verified by store trace (probe 12 + `RC_OFF`):** `make()` allocates the cell (`#3`);
  on return `RC_OFF` frees `#3` at the frame's exit (`- free #3`); the next `make()`
  **reuses the slot** (`+ alloc #3`) → f1 and f2 alias the same store → `store()` UAF
  (`allocation.rs:472`).  rc suppressed that frame-exit free (`inc_rc` on capture).
- **Fix:** closure-cell ownership — cell lifetime tied to the closure value, freed when the
  closure dies, so the defining-frame free no longer reuses a still-referenced slot.

**Everything else needs no rc** — every t1–t8 (vector<text>) shape and all the
non-coexisting closure shapes survive `RC_OFF`.

## Phase plan (from the map)

- **Phase A** — Mechanism 1: statement-end free for an unbound heap-returning-call
  temporary.  Fixes `t9` (the rc text gap) **and** stray `10` (closure-temp leak) at once.
- **Phase B** — Mechanism 2: closure-cell ownership — the real rc blocker (coexistence).
- **Phase C** — delete `ref_count` / `OpIncRc` / `inc_rc` / `dec_rc`; verify both backends.

Off the rc path: the closure ↔ collection limitation (06/08) in
[`../closure-collection/`](../closure-collection/) — its own home, re-home to a closure /
`P257` plan if one opens.
