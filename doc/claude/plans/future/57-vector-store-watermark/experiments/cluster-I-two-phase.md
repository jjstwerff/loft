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

## What we learned (the real lessons — corrected; keep these accurate)

1. **The fix has NO runtime effect — it is sound but INEFFECTIVE** (same outcome
   as the two prior reverts, reached again).  The **reliable** signal is the
   stderr-only store log (suppress stdout: `2>&1 >/dev/null`), which is **identical
   ON vs OFF**:
   ```
   two_read: +#2 +#3 -#3 -#2     (ON == OFF)   ← allocs, then frees, LIFO = function-exit
   wm_multi: +#2 +#3 +#4 +#5 +#6 -#6 -#5 -#4 -#3 -#2   (ON == OFF)
   ```
   One `free_named` per store, `rc=1`, no `dec_rc` — each store frees once, at
   function teardown, *whether the fix is on or off*.  The IR/bytecode does place
   `OpFreeRef` inside the block (`LOFT_LOG=static`), but that does **not** translate
   into the interpreter freeing earlier.  **Why the in-block IR free-position does
   not move the effective runtime free is the real open crux** — deeper than
   scope-registration, and the same wall the prior attempts hit.

2. **MEASUREMENT HAZARD that fooled an earlier read of this doc** (now corrected):
   `print(...)` markers go to **stdout**, the store log to **stderr** — separate
   streams, different buffering.  Under `2>&1` the stdout markers flush *late*, so
   the trace *looked* like `…blk1, FREE #3, blk2, FREE #2…` (interleaved → "works").
   That was an artifact.  **Always use stderr-only ordering (`2>&1 >/dev/null`) or
   same-stream markers for execution-order claims.**  (Being hasty here — claiming
   "free-timing works" from the unreliable trace — is exactly the failure this
   plan keeps warning about.)

3. **The `LOFT_STORE_GUARD` guard is fooleable.**  It checks the *scope label*
   (`vars.scope(vdb) != b`), which the fix changes, so it goes **silent even though
   the watermark is unchanged**.  The guard is NOT a valid oracle for this fix; the
   stderr-only store log / watermark is.  A future guard must assert the store
   actually frees earlier, not just that its scope label moved.

4. **Native does not honor the in-block IR free either** — a SEPARATE native bug,
   to be characterised on its own once the interpreter genuinely frees earlier.

5. **`alloc`-timing is a real but DISTINCT axis** (do not conflate with the free
   problem): the `max active` watermark is also gated by when `OpDatabase` runs.
   Worth its own probe once free-timing actually moves.

## State

Sound (the `172` soundness boundary stays green on both backends — escapes are not
freed early) but **INEFFECTIVE at runtime** (the stderr-only store order is
identical fix-on vs fix-off; the interpreter still frees at function exit).  The IR
correctly moves `OpFreeRef` into the confined block, but that does not translate
into an earlier *runtime* free — the open crux.  Not committed to `main`; preserved
here as the durable record for the next pass.
