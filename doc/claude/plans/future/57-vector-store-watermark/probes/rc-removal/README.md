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
| **02 multi_factory_escape** | two factories, calls interleaved | ok | ~~CRASH~~ **FIXED** | **Mechanism B** (cell ownership) |
| **12 two_closures_coexist** | two closures coexist, calls *not* interleaved | ok | ~~CRASH~~ **FIXED** | **Mechanism B** |
| **09 factory_loop_churn** | many coexisting factory closures in a loop | ok | ~~CRASH~~ **FIXED** | **Mechanism B** |
| 13 coexist_alias_detector | deterministic wrong-output detector (vs flaky UAF) | ok | ~~a2==1~~ **FIXED** | **Mechanism B** detector |
| **10 closure_passed_as_arg** | closure as an UNBOUND arg temporary | ~~LEAK~~ **FIXED** | ~~LEAK~~ | **Mechanism 1** (unbound temp) — `Type::Function` lift arm |
| **t9 split_temp_leak** | `split()` vector temp, unbound | ~~LEAK~~ **FIXED** | ~~LEAK~~ | **Mechanism 1** `Type::Vector` arm → [`../nrvo-inline-leak/`](../nrvo-inline-leak/) |
| t1–t8 | vector<text> build/reassign/append/return/concat/slice/nested/struct | ok | ok | scope covers |

(06 / 08 — closure ↔ collection — moved to [`../closure-collection/`](../closure-collection/):
NOT rc-related, a separate closure-record-layout limitation.)

## Findings — the residual is TWO mechanisms, not five bugs

**Mechanism 1 — an UNBOUND heap-returning-call temporary has no statement-end free.**
A call returns a heap value used inline (not bound to a local); no work-ref buffer + no
`OpFreeRef` is emitted for the temp.  Both instances are leak-free when **bound to a local
first** (the local's scope-free handles it).  They are **SEPARATE fix sites** (confirmed by
a code-only investigation agent, 2026-06) — same mechanism, different lift arm:
- **`10`** — `apply(make())`: FIXED.  `make()`'s fn-ref result is the unbound temp; it got
  **no `OpFreeRef`**, so after apply's inc-on-pass / dec-on-return the closure record sat at
  **rc 1 → leaked WITH rc on**.  Fix: a `Type::Function` arm in `inline_struct_return`
  (`src/scopes.rs`) lifts it → `get_free_vars` emits the `OpFreeRef`, codegen frees the
  closure DbRef, `free_named` cascades to the `__cell_*`.  Leak-free both backends.
- **`t9`** — NOT YET FIXED, and **worse than this corpus first recorded.**  The earlier
  "rc-on clean, RC_OFF-only" line was measured against a stale binary and is **wrong**: it
  leaks **rc-on, on the interpreter**, today — a live suite-blocker (`03-text.loft` → `wrap
  text`/`loft_suite` red).  Re-probing (see [`../nrvo-inline-leak/`](../nrvo-inline-leak/))
  showed it is NOT split-specific: any **de-NRVO'd (empty-dep) vector return** used
  inline-unbound leaks.  Its fix is the sibling `Type::Vector` lift arm (guarded on
  `dep.is_empty()`), reaching `t_` methods.  Full map + fix direction in that corpus.

**Mechanism B — a captured cell is freed at the DEFINING frame's exit, not the closure's.  FIXED.**
rc was needed ONLY for **≥2 coexisting closures** that own captured cells (02 / 12 / 09);
single (01), read-only (03), in-frame (04), text (05), nested (07), and *sequential* (11)
all survive `RC_OFF`.  The break is **coexistence + escape + a mutable cell**, not
interleaving (12 crashes without interleaved calls); read-only captures are stored inline
(no freeable cell) so they never tripped it.
- **Verified by store trace (probe 12 + `RC_OFF`):** `make()` allocates the cell (`#3`);
  on return `RC_OFF` frees `#3` at the frame's exit (`- free #3`); the next `make()`
  **reuses the slot** (`+ alloc #3`) → f1 and f2 alias the same store → `store()` UAF
  (`allocation.rs:472`).  rc suppressed that frame-exit free (`inc_rc` on capture).  The UAF
  is flaky (slot-reuse dependent) — probe 13 is the deterministic wrong-output detector.
- **Fix (`efdf8a1c`+):** closure-cell ownership.  `get_free_vars` (scopes.rs) now SUPPRESSES
  the defining-frame `OpFreeRef` of a captured `Reference(__cell_*, _)` local, and the
  `OpIncRc` on capture (parser/vectors.rs) is DROPPED — so `Stores::free_named`'s cascade
  (allocation.rs:301), which frees the cell when the closure record dies, is the **sole
  owner** for escaping AND in-frame captures.  All of 01–13 pass `RC_OFF` on both backends;
  rc-on clean (no leaks); closure_matrix 24 + mut_closure_matrix 44 green.  Regression:
  `tests/closure_cell_ownership.rs` (4 shapes × both backends × `RC_OFF`).  This was the
  last load-bearing `OpIncRc` user — dropping it unblocks Phase C.

**Everything else needs no rc** — every t1–t8 (vector<text>) shape and all the
non-coexisting closure shapes survive `RC_OFF`.

## Phase plan (from the map)

- **Phase A** — Mechanism 1: statement-end free for an unbound heap-returning-call
  temporary, via `inline_struct_return` lift arms.  **COMPLETE** (`efdf8a1c`): `10`
  closure-temp (`Type::Function` arm) + `t9` de-NRVO'd vector return (`Type::Vector`
  arm, `dep.is_empty()` guard, reaches `t_` methods).  Full map + regression in
  [`../nrvo-inline-leak/`](../nrvo-inline-leak/) + `tests/scripts/174`.
- **Phase B** — Mechanism 2: closure-cell ownership — the real rc blocker (coexistence).
  **COMPLETE** (`efdf8a1c`+): suppress the captured-cell defining-frame free + drop
  `OpIncRc`; the closure-record cascade is sole owner.  02/09/12/13 pass `RC_OFF` both
  backends; regression `tests/closure_cell_ownership.rs`.
- **Phase C** — delete `ref_count` / `OpIncRc` / `inc_rc` / `dec_rc`.  **COMPLETE**
  (`5745a2c2` part 1 + `80208ab1` part 2).  The Stores ref-count is GONE.
  - **part 1** — retarget the only behavioral rc reader (the const/global PIN, which a
    code-only agent proved load-bearing via a UAF) to a `Store.pinned` flag; make
    `free_named` unconditional; drop the file-close `ref_count <= 1` conjunct.  Regression
    `tests/scripts/175-const-pin-no-free.loft`.
  - **part 2** — delete the `ref_count` field + `inc_rc`/`dec_rc` + the `OpIncRc` op
    (loft decl + regenerated `fill.rs` + native emitter + runtime helper).  No on-disk /
    ABI change (`ref_count` was in-memory only; one shared `Store` struct).
  - Verified: full interpreter suite green; native 6/6 + cache 5/5 in isolation;
    closure_matrix 24 + closure_cell_ownership + const-pin green both backends.
  - The `RC_OFF` env experiment is retired (free is now always unconditional).

Off the rc path: the closure ↔ collection limitation (06/08) in
[`../closure-collection/`](../closure-collection/) — its own home, re-home to a closure /
`P257` plan if one opens.
