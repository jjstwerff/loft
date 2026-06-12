<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Session handoff — #354 native-codegen fixes + an exposed pre-existing leak

Branch: `windows-probe`.  Date: 2026-06-12.  Context being cleared; this is the
full state so the next session resumes cleanly.

## The user's queue (in order)

1. **#354** — native codegen errors (E0425 block-scope loss + E0308 bool/discard)
   on crawler's `sim`/`hex` libs.  **Compile fixes DONE + matrix-validated; BLOCKED
   on the exposed leak below.**
2. **#358 is NOT ours** — another agent owns it.  Do not touch.
3. **Then merge** this branch and **rebase on #357** (the `Engine` PR / branch).

Standing rule this session: **stability/bug-fixing lane, fix don't file**
(STABILITY_ROADMAP.md banner; CLAUDE.md § Bug-filing policy).  A parallel agent
owns gaming/feature work.

## Toolchain change made this session (important)

Ran `rustup update stable` → stable is now **rustc 1.96** in BOTH the repo and
`/tmp` (the per-cwd 1.95/1.96 trap in `reference_rustc_cwd_toolchain_trap` is
GONE).  This is what **unmasked** the whole problem: before, the crawler/world
libs failed native compile on the toolchain mismatch and **silently fell back to
the interpreter** — so the native-lib codegen path (and its bugs) never ran.
After the update they compile native and the bugs surface.  A native compile
failure degrading silently to interp is the root reason these went undetected.

## State of commits

- **Already committed + pushed** on `windows-probe` (HEAD before this session's WIP
  = `e700c4d4` "fix #355 + #356"): the one-buffer return ABI for Reference (4cba…
  era), the vector arm (#355) + mid-body nested return (#356), STABILITY_ROADMAP.md,
  the stability-work-stream declaration.  #355/#356 are labelled
  `fixed-pending-merge`.
- **UNCOMMITTED working-tree changes** (this session, for #354) — NOT yet committed:
  - `src/generation/mod.rs` — `collect_scope_hoists()` + the **scalar-only** block-
    scope hoist in `output_function` (after the `__vdb` prologue).
  - `src/generation/dispatch.rs` — `_` discard loop var: a **scalar-only** shadowing
    `let` typed from the loop's own iter value (fixes the `var__: f64` ← i64 E0308).
  - `src/generation/emit.rs` — `bool_unify` widened to cover `Block` arms (fixes the
    `ncc` boolean-Block-then-arm vs `false`-else-arm E0308).
  - `src/parser/control.rs` — removed a leftover `LOFT_TRACE_UNIFY` debug trace
    (cleanup only; no behaviour change).
  - `scripts/find_problems.sh` — **real bug fix**: the summary only grepped cargo-test
    `^test … FAILED`; it MISSED every **nextest** `FAIL [..]` line, so a failing run
    reported "(none)".  Now matches both formats.  (This is why I almost shipped a
    red tree as green — worth keeping.)
  - `tests/scripts/300-native-scope-hoist.loft` (new) — discard `_` + scope-hoist +
    ncc-bool regression, cross-mode.
  - `tests/scripts/301-native-block-scope-hoist.loft` (new) — the exact `intown`/
    `nhouse` block-scope-loss shape + float/enum/loop axes, cross-mode.

  These compile clean, both 300/301 pass interp+native, clippy 0, fmt clean.

## #354 — what was wrong and the validated fix

Three native-codegen bugs, all only triggered by LARGE real functions (in-repo
tests are too small; a 467-line synthetic in the issue did NOT reproduce):

1. **E0425 block-scope loss** (dominant).  `block_NNN` in the generated Rust are
   **inline `{ }` scopes** (comment labels), NOT sub-functions.  A loft local first
   *written* inside one `{ }` block and *read* in a sibling block gets its `let`
   emitted inside the first block → out of scope at the sibling read.
2. **E0308 `var__: f64` ← i64** — the `_` discard loop var shares ONE flat-namespace
   table entry across all loops, so a later int-range loop assigns i64 into the
   f64-typed `var__` from an earlier float loop.
3. **E0308 if/else bool** — an `ncc` (`?? false`) boolean **Block** then-arm vs a
   `false` literal else-arm: the Block arm wasn't u8-cast like the literal.

### THE INVARIANT (matrix-validated — the "scope detection" the user asked for)

> **A local needs the native block-scope hoist iff it has no IR-level fn-scope
> null-init — which is exactly the SCALARS.**

Heap locals (DbRef/Text/Vector/struct/tuple) **already** get an IR-level fn-scope
init: scope analysis emits `s(1):ref(S) = null;` at fn-body level (so they can be
freed at fn exit), so native declares `let mut var_s: DbRef = sentinel` at fn scope
already — they never hit the block-scope loss.  Scalars carry no such init; their
first Set lands wherever written (in-block).  So: **hoist scalars, leave heap
locals alone.**  Hoisting a heap local DOUBLE-declares it / breaks its free → a
store leak (I hit this with an early "hoist everything" version — that's why the
restriction is scalar-only, and it's correct *by construction*, not by trial).

Matrix run (`/tmp/p_followups/scope_matrix/*.loft`, both backends, all OK):
type ∈ {int, float, bool, enum, struct, text, vector} × write-loc ∈ {if, loop,
nested-if} × use-loc ∈ {sibling-if, after-loop, outer} × {single-write, reassign}.
Captured as `tests/scripts/301-native-block-scope-hoist.loft`.

Same scalar-restriction logic for the `_` discard re-let (dispatch.rs): re-`let`
only when BOTH the var's table type AND the loop's iter value are scalar — a
`for _ in <vector-of-structs>` binds a DbRef view whose `OpFreeRef` must keep
seeing a DbRef, so re-typing it scalar there orphaned a store.

**Verify #354 fix:** `leveltest` native compiles with **0 errors**:
```
cd ~/workspace/crawler && LIBS=$(for d in bundles/*/ bundles/*/items/; do [ -d "$d" ] && printf -- "--lib %s " "$d"; done)
PATH="/home/jurjen/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH" \
  ~/workspace/loft/target/release/loft --check --no-warnings --path ~/workspace/loft/ \
  --lib ~/workspace/loft/lib/ --lib ../loft-libs-core-main/ --lib ../loft-libs-world/ \
  $LIBS src/leveltest.loft 2>&1 | grep -cE '^error\['     # → 0
```

## ⚠ THE REAL BLOCKER — #355/#356 regressed the leak gate (CORRECTED)

**`tests/wrap.rs::loft_suite` PASSES on `main` but FAILS on this branch.**  So the
leaks below were **introduced by the already-pushed #355/#356 commit `e700c4d4`**,
not pre-existing.  I committed it believing the suite was green **because the
`find_problems.sh` summarizer bug hid the nextest `FAIL` line** (it only matched
cargo-test's `^test … FAILED`).  That summarizer fix is in this session's WIP and
is load-bearing — keep it.

Definitive evidence:
```
cargo nextest run --release --test wrap loft_suite      # main: PASS; this branch: FAIL
```
- `main` (`.claude/worktrees/prechange`): **PASS**.
- at356 (`e700c4d4`, #355/#356): **FAIL** on `55-stack-trace.loft` (kt=65535).
- HEAD (this WIP): the 55 leak is **FIXED** by my uncommitted
  `callee_forwards_foreign_store` guard (control.rs), so the suite now progresses
  past 55 and **FAILs on `93-vector-advanced.loft`** (kt=19 `main_vector<integer>×15`).

**The suite aborts at the FIRST leaking script, so each fix unmasks the next** —
this is a CLASS, not one bug.  Same family as the tuple leak: a **heap work-ref
result of a call, freed at the wrong scope**.  93's case is `vsort` — a RECURSIVE
`vector<integer>`-returning merge-sort; the ~15 intermediate result vectors are
not freed after `vsort_merge` consumes them.  55's case is the native-wrapper
(already fixed).  These shapes route through the #355/#356 one-buffer vector-arm /
mid-body-return changes — that is where the free placement regressed.

**Consequence: #355/#356 (`fixed-pending-merge`, pushed but NOT merged to main —
so recoverable, branch-internal) must NOT merge until `loft_suite` is green
again.**  93 leaks in isolation = 0 but in the suite = 15 (the wrap harness runs
scripts sequentially; investigate `tests/wrap.rs` if the count attribution
matters, but the root is the #355/#356 free-placement change).

## A SEPARATE pre-existing leak (`sweep`→`hexn`, on main too)

Landing #354 makes the libs compile native, which then **exposes a pre-existing
store-leak** (`cavetest` exhausts the 65535-store table; CAVE OK on `main`/at356
ONLY because the libs fell back to interp there).  So the leak fix is effectively
a **prerequisite for landing #354** — otherwise the consumer goes from "loud
interp fallback" to "silent store exhaustion", which is worse.

### Minimal repro (a few-second rebuild — USE THIS, not the whole crawler)

Library `disclib` with a multi-return-site tuple fn, called native-to-native in a
loop.  `hexn` is `hex_neighbor` from `loft-libs-world/hex_grid`.

`/tmp/disclibs/disclib/loft.toml`:
```toml
[package]
name = "disclib"
version = "0.1.0"
loft = ">=0.8"
[library]
entry = "src/disclib.loft"
```
`/tmp/disclibs/disclib/src/disclib.loft`:
```loft
pub fn hexn(q: integer, r: integer, dir: integer) -> (integer, integer) {
  if (r & 1) == 0 {
    if dir == 0 { return (q + 1, r); }
    if dir == 1 { return (q, r - 1); }
    if dir == 2 { return (q - 1, r - 1); }
    if dir == 3 { return (q - 1, r); }
    if dir == 4 { return (q - 1, r + 1); }
    return (q, r + 1);
  }
  if dir == 0 { return (q + 1, r); }
  if dir == 1 { return (q + 1, r - 1); }
  if dir == 2 { return (q, r - 1); }
  if dir == 3 { return (q - 1, r); }
  if dir == 4 { return (q, r + 1); }
  (q + 1, r + 1)
}
pub fn sweep(reps: integer) -> integer {
  acc = 0;
  for k in 0..reps {
    for d in 0..6 {
      (nq, nr) = hexn(k & 31, (k / 32) & 31, d);
      acc = acc + nq + nr;
    }
  }
  acc
}
```
`/tmp/p_followups/disc_main.loft`:
```loft
use disclib;
fn main() { t = disclib::sweep(80000); println("sweep ok t={t}"); }
```
Run (leaks on BOTH backends, AND on `main`/at356 — it is **pre-existing**, not a
#354 regression):
```
rm -rf /tmp/disclibs/disclib/native-auto ~/.loft/build-cache
LOFT_NO_CACHE=1 ~/workspace/loft/target/release/loft --interpret --no-warnings \
  --lib /tmp/disclibs /tmp/p_followups/disc_main.loft 2>&1 | grep -E 'exhausted|sweep ok'
```

### ROOT CAUSE (precise — read this before touching it)

In the CALLER (`sweep`), the tuple-result work-ref is the leak.  IR
(`LOFT_LOG=static`, fn `n_sweep`):

```
__ref_2(1):ref(__tuple<integer,integer>) = null;    // null-init at FN-TOP
__ref_1(1):ref(__tuple<integer,integer>) = null;    // (the buffer passed to hexn)
...
loop {                                               // inner d-loop
  __ref_2 = n_hexn(..., __ref_1);                    // REASSIGNED every iteration
  nq = OpGetInt(__ref_2, 0); nr = OpGetInt(__ref_2, 8);
}
...
OpFreeRefIfDistinct(__ref_1, __ref_2);               // freed at FN-EXIT only
OpFreeRef(__ref_2);
```

`__ref_1`/`__ref_2` are a **witness pair** (`scopes.rs` `paired_witness`, recorded
in the `!has_ref_params` `v = call(__ref_N)` branch ~line 989).  `hexn` returns
either its passed buffer (adoption) or a fresh store (each `return (a,b)`
allocates a fresh synthetic-tuple store) — statically unresolvable, hence the
`OpFreeRefIfDistinct(__ref_N, v)` at scope exit.  **The bug: the pair is
FN-scoped (null-init at fn-top, free at fn-exit), but the assignment is inside a
loop**, so every iteration's fresh tuple store except the last is orphaned.  The
`owned_ref` pre-Set free (`state/codegen.rs` ~1608) does NOT fire here (the var is
witness-paired, not a plain dep-empty owned ref).

There is already a sibling mechanism for the inverse case — `witness_buffer`
(`@P378(a)`, `scopes.rs` ~line 95) frees a witness that is INNER-scoped relative to
its buffer, each iteration.  The fix likely extends/mirrors that: when a
witness-paired work-ref is **assigned inside a loop** but null-inited at fn scope,
emit a **per-iteration** `OpFreeRefIfDistinct` (free the prior value before the
reassignment, or at loop-body scope exit) instead of only at fn exit.

### Why I did NOT fix it this session

Every codegen/scope change I rushed without a boundary matrix cascaded (the
"hoist everything" leak, the `let _ =` break, the `callee_forwards` forward-ref
fragility — all this session).  A witness-pair loop-scoping change in
`scopes.rs`/`codegen.rs` is exactly that risk class and deserves its OWN matrix
(axes: return-shape {tuple, struct, vector} × {destructured, whole} × loop-depth
{1, 2, nested} × adoption-vs-fresh callee × backend) BEFORE touching the free
placement.  Build that matrix first; then fix at the witness-pair chokepoint.

## Concrete next steps (REVISED PRIORITY)

0. **FIRST: get `loft_suite` green again** — this gates everything (and #355/#356's
   merge).  `cargo nextest run --release --test wrap loft_suite`.  Each fix unmasks
   the next leaking script; keep going until PASS.  Bisect the free-placement
   regression in #355/#356's `ref_return` one-buffer vector arm + the #356
   `chain_site_set_shape` / mid-body changes (control.rs) against `main`'s green
   behaviour — diff `vsort`'s IR (`LOFT_LOG=static`) HEAD vs prechange to see which
   `OpFreeRef`/`OpFreeRefIfDistinct` moved.  The witness-pair-in-loop analysis below
   is the likely shared root for both this and the `sweep` leak.

1. **Fix the witness-pair-in-loop leak** (matrix-first, as above).  Verify with the
   `sweep` repro on both backends, then with `cavetest`:
   ```
   cd ~/workspace/crawler && LIBS=$(for d in bundles/*/ bundles/*/items/; do [ -d "$d" ] && printf -- "--lib %s " "$d"; done)
   rm -rf ~/workspace/loft-libs-world/*/native-auto ~/workspace/crawler/native-auto ~/.loft/build-cache
   LOFT_NO_CACHE=1 ~/workspace/loft/target/release/loft --interpret --no-warnings --path ~/workspace/loft/ \
     --lib ~/workspace/loft/lib/ --lib ../loft-libs-core-main/ --lib ../loft-libs-world/ $LIBS src/cavetest.loft \
     2>&1 | grep -E 'exhausted|CAVE OK'      # want: === CAVE OK ===
   ```
   A/B baseline: at356 worktree is at `.claude/worktrees/at356` (built, CAVE OK);
   prechange (main) at `.claude/worktrees/prechange`.  **Remove these worktrees
   when done** (`git worktree remove --force`).
2. Add a `tests/scripts/` regression for the tuple-in-loop leak (cross-mode; the
   wrap-suite leak gate catches it).
3. Commit #354 (the 5 src files + 300/301 + the find_problems.sh fix) WITH the leak
   fix — they ship together since #354 unmasks the leak.
4. **#358 is another agent's — skip it.**  Then merge this branch + rebase on **#357**.

## Caching gotchas that wasted hours (so you don't repeat them)

- The lib **cdylib** cache (`<lib>/native-auto/*.so` + `~/.loft/build-cache`)
  persists across `--bin loft` rebuilds and across worktrees.  **Always
  `rm -rf <lib>/native-auto ~/.loft/build-cache`** before an A/B, or you test a
  stale `.so` built by a different compiler.  This produced several false
  CAVE-OK/leak flips.
- `cargo build --release --bin loft` does NOT rebuild `libloft.rlib`; the cdylib
  generation links the rlib.  For codegen changes a `--bin` rebuild is enough (the
  codegen lives in the binary), but wipe the cdylib cache so it regenerates.
- A clean `cargo clean` + full `cargo build --release` + `make rebuild-native-cdylibs`
  was needed once after the rustc update to clear `StableCrateId` collisions
  (`log` vs `log`) in registry cdylibs.

## Open worktrees to clean up
`.claude/worktrees/at356`, `.claude/worktrees/prechange`, `.claude/worktrees/refonly`
— all temporary A/B builds; `git worktree remove --force` each when done.
