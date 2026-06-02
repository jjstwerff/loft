<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Preserved experiment — cluster-I gated two-phase scan

A working-but-incomplete implementation of the cluster-I fix, **preserved as a
diff rather than reverted** so a future session studies the *actual code under the
tracer*, not a degraded summary.  (The prior cluster-I/III reverts came back as
*misdiagnoses* — rc, then "timing" — precisely because the work was thrown away
and only a summary survived.  We learn from failures; we do not hide them.)

## How to re-apply

```bash
git checkout 003ead79d5c030d7240c54195c5473e7570df6f8   # the exact build base
git apply doc/claude/plans/future/57-vector-store-watermark/experiments/cluster-I-two-phase.diff
```

Built on **`003ead79`** (`test(plan-57): lock the store-confinement soundness
boundary into CI`).  The diff is [`cluster-I-two-phase.diff`](cluster-I-two-phase.diff)
(2 files: `src/scopes.rs`, `src/database/allocation.rs`).

## What it does

Implements the gated two-phase scan over the committed `store_confinement`
foundation (`00dc10ae`):

- a `Scopes::confined: HashMap<u16, u16>` field + a `put_scope` helper that
  registers a var at its confined block scope instead of `self.scope`;
- `run_scan_phase` factors the scan→apply→set-scope pass so `check` can run it
  twice;
- `check` runs phase 1, calls `store_confinement`, and (if non-empty) re-scans
  with the `vdb`/local → block-scope map so the block-exit `free_vars` sweep emits
  `OpFreeRef(__vdb)` *inside* the confined block.
- Debug/measurement scaffolding included: `CONF_OFF` env disables phase 2 (A/B),
  `CONF_DBG` prints the confined map, `FREE_DBG` (allocation.rs) logs every
  `free_named` with rc + already-free.

## What we learned (the real lessons — keep these accurate)

**The two dimensions are independent — do not collapse them.**  A wrong read of
this is what made "ineffective" look like "wrong".

1. **Free-timing — WORKS, on the interpreter.**  Proven by A/B on the SAME binary
   (`two_read.loft`, 2 read blocks):
   ```
   fix ON : alloc #2, alloc #3, blk1, FREE #3, blk2, FREE #2, end   ← frees at block exits
   fix OFF: alloc #2, alloc #3, blk1, blk2, end, FREE #3, FREE #2   ← frees at function exit
   ```
   The IR/bytecode places `OpFreeRef(__vdb_N)` inside each block (verified
   `LOFT_LOG=static`), and the interpreter honors it (`FREE_DBG`: `free_named` runs
   at block exit with `rc=1`, i.e. it actually frees).  **So the IR transformation
   is correct.**

2. **Alloc-timing — the OPEN problem (separate mechanism).**  `max active` does NOT
   drop (`wm_multi` 5 blocks: 7 ON == 7 OFF; `11-vectors`: 26 == 26) because both
   `OpDatabase` allocations execute **up front** (`alloc #2, #3` both before `blk1`),
   *identically* with the fix on and off.  The watermark is gated by allocation
   timing, which this fix does not touch.  **Next investigation: why the
   `OpDatabase` is hoisted / not deferred to block entry.**  (Rule out an
   `if true` constant-fold artifact first — re-test with a non-constant guard.)

3. **Native does not honor the in-block free — a SEPARATE native bug** (native
   failing to implement the IR), not evidence this work is wrong.

4. **The `LOFT_STORE_GUARD` guard is fooleable.**  It checks the *scope label*
   (`vars.scope(vdb) != b`), which the fix changes, so it goes **silent even when
   the watermark is unchanged**.  The real oracle is the watermark / free-position
   (`LOFT_STORES=log` + `FREE_DBG`), not the guard.  A future guard should assert
   the store actually frees earlier, not just that its scope label moved.

5. **rc correction to the fix-design "resolved misdiagnosis":** rc *is* 1 and the
   in-block `OpFreeRef` *does* free (lesson 1) — so the misdiagnosis-resolution was
   right that no rc surgery is needed.  What it missed is lesson 2 (alloc timing).

## State

Sound (the `172` soundness boundary stays green on both backends — escapes are not
freed early), free-timing correct on interpret, watermark not yet reduced
(alloc-timing open), native unimplemented.  Not committed to avoid shipping the
fooleable-guard state on `main`; preserved here for the next pass.
