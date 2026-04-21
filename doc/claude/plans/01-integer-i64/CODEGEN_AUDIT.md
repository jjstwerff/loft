# Post-2c codegen size-audit — proper plan

Status: **partially executed 2026-04-20**.  Pilot and
follow-on fix (`7bf3558` + `edbc9f3`) cleared 7 tests across
categories D.2, D.3, and E — validating the audit's core
hypothesis that post-2c stale 4-byte integer widths in
codegen + worker setup explain most of the mid-tier failure
cluster.  Full Phase A/B/C audit not run; instead a narrower
"follow the panic" approach worked.

**Remaining audit candidates**: D.1 (allocation.rs:265 OOB on
`wrap::dir` / `wrap::last` / `wrap::parser_debug`) shares the
panic shape of the categories just closed.  Apply the same
trace discipline to identify the corrupted DbRef's source
slot and the stale 4-byte write that produced it.

## Context

12 of the 22 remaining failures on `int_migrate` (D.1 × 3 + D.2
× 2 + D.3 × 1 + E × 4 + 2 related native) trace to the same
architectural issue: the compile-time stack-position tracker in
`src/state/codegen.rs` was hand-tuned for a 4-byte
`Type::Integer` world, and Phase 2c widened it to 8 bytes.  A
handful of sites that encode size-specific deltas have not been
updated, causing cumulative drift that only becomes visible
after multi-arg calls or parallel-worker setups.

**Evidence (confirmed 2026-04-20)**:

1. **D.2 `codegen.rs:1780` panic** — `Incorrect var b[520]
   versus 517 on n_main`, 3-byte drift on `tests/scripts/22-
   threading.loft`.

2. **Category E `keys.rs:211` panic** — `par_light_auto_selected`
   fails with `len=4, index=4` in `vector::get_vector` AFTER
   `n_parallel_for_light` returns.  Execution trace shows:
   - `VarInt(var[72]=_par_len_4) -> 42949672962` (should be `10`;
     value is `0x0000_000A_0000_0002` — two concatenated u32s)
   - `VarRef(var[100]=_par_results_3) -> ref(4, 2, 12)` (store_nr
     should be 3 or similar; not 4)
   Both vars read garbage after the parallel call, suggesting
   the stack pointer ended at the wrong position when the call
   returned, shifting every subsequent variable read.

3. **Three documented stale size sites** in `src/state/codegen.rs`:

   | Line | Current | Should be | Reason |
   |------|---------|-----------|--------|
   | 1442-1447 | "Function args are 16B (4B d_nr + 12B closure)" | 20B (8B d_nr + 12B) | fn-ref slot widened to 20B in commit `b3661a2` |
   | 1628 | `stack.position -= 4` per par() extra | `-= 8` | integer is 8B post-2c |
   | 1795-1799 | `+= 4` for OpVarFnRef discrepancy | verify against new sig | OpVarFnRef returns `text` (16B) but slot is 20B |

4. **D.3 `ops.rs:278` long overflow** — `0x8000_0000_FFFF_FFFF`
   bit pattern is what you get when an i32 null sentinel lands
   in the low 32 bits of an i64 slot and the high 32 bits get
   concatenated from the adjacent slot's data (or the reverse).
   Same root as #2: stack slot misalignment.

5. **D.1 `allocation.rs:265` OOB** — `index 8 out of 5` on
   `tests/docs/15-lexer.loft` — fires during `Allocations::claim`
   which accesses `allocations[db.store_nr]`.  A DbRef reading
   `store_nr=8` when only 5 stores exist is the same corruption
   pattern as E's `ref(4, 2, 12)` on a 4-store setup: the DbRef
   was read from a stack slot that now spans the edge of its
   assigned position.

All five symptoms are manifestations of the same bug: stale
size constants in `src/state/codegen.rs` that didn't get
widened during Phase 2c rounds.

## Goal

Audit every site in `src/state/codegen.rs` that hard-codes a
byte count for `stack.position` adjustment.  Fix all sites in
a single coordinated commit with strict pre/post test-snapshot
discipline.

## Approach

### Phase A — exhaustive site inventory

Produce a complete list of every `stack.position +=` and
`stack.position -=` in `src/state/codegen.rs` and adjacent
files.  For each site, classify:

- (a) **Computed from `size()`** — uses `size(&type, ctx)` or
  `size_of::<T>()`.  Trustworthy; no change needed unless the
  type is wrong.
- (b) **Hardcoded literal** — `+= 4`, `-= 4`, `+= 8`, etc.  Each
  needs verification against its semantic meaning and post-2c
  reality.
- (c) **Variable-driven** — uses a size derived from the
  op's declared params / returns.  Trustworthy if the
  declarations are correct.

Candidate search commands:

```bash
grep -n "stack\.position" src/state/codegen.rs src/stack.rs |
    grep -vE "size\(|size_of|size_ref|size_str|super::size"
# Remaining lines are class (b) — hardcoded literals.

grep -n "size(" src/state/codegen.rs |
    grep -E "Context::" | head
# Class (a) sites — confirm each still maps to the right
# Type.
```

Expected output: ~20-30 class (b) sites.  Each needs a
one-line audit.

### Phase B — per-site reasoning

For each class (b) site, document:

- What the delta represents (push of X, pop of Y).
- What type X is in the 2026-04-20 world.
- What the correct post-2c delta is.
- Whether changing it affects only the compile-time tracker
  (safe) or also the runtime-stack layout (requires
  coordinated runtime update).

### Phase C — staged fix

Each site fix is one line.  Commit ONE site at a time with:

```bash
./scripts/find_problems.sh --bg --wait  # baseline
# edit one site
cargo build --release
./scripts/find_problems.sh --bg --wait  # verify
comm -13 /tmp/baseline_fail.txt /tmp/after_fail.txt
# must be empty (no new failures)
# if a test turns green, keep the edit; commit
# if a test turns red, revert that one edit and move on
```

The "revert on any new failure" rule is load-bearing.
Many sites interact — a fix to one that's locally
correct may expose an unfixed bug downstream.  Iterate
until all known-broken tests pass and no new failures
appear.

### Phase D — verify the cluster

After all audit-identified sites are fixed:

```bash
cargo test --release --test wrap            # D.1 + D.2 cluster
cargo test --release --test expressions par_light   # E cluster
cargo test --release --test threading       # D.3 cluster
```

Target state: 22 → ~10 failing tests (12 cleared across D +
E; C and G remain).

## Critical files

### Primary audit target
- `src/state/codegen.rs` — all `stack.position` sites
- `src/stack.rs::add_op` / `operator()` — runtime-stack-size
  reference

### Supporting references
- `src/variables/mod.rs::size` — canonical type size map
- `src/variables/slots.rs::assign_slots` — compile-time slot
  assignment; must agree with codegen
- `src/data.rs::Type::size` — secondary size heuristic

### Test targets (ordered by reproducibility)
- `tests/expressions.rs::par_light_auto_selected` — cleanest
  reproducer; no extras, minimal script, fails in a
  well-defined post-par variable read
- `tests/scripts/22-threading.loft` + its wrappers — 3-byte
  drift on `main()` — good for verifying the fix doesn't
  re-introduce the drift
- `tests/docs/19-threading.loft` — D.3 overflow case; pass
  means null-propagation works through parallel path
- `tests/docs/15-lexer.loft` + `16-parser.loft` — D.1; pass
  means DbRef integrity after complex codegen is OK

## Risk assessment

**Primary risk**: fixing a site that's locally correct but
has a compensating bug elsewhere will expose the compensating
bug (new red test).  Mitigation: the per-site snapshot rule
— any new red = revert that one edit, continue.

**Secondary risk**: a site affects multiple test paths,
fixing it turns one test green but another red.  Mitigation:
compare gross failure counts; if down, keep the edit even
if one test flipped.  If flat or up, revert and analyse.

**Tertiary risk**: the 20-byte fn-ref comment at line 1442
is WRONG (should be 20 per `b3661a2`) but the site's
behavior is correct because some OTHER site compensates.
Fixing the comment without adjusting the behavior site may
break things.  Mitigation: treat comments as documentation
only; don't edit them without testing the corresponding
code.

## Out of scope

- `src/generation/*.rs` — Category C retry, separate plan in
  `CATEGORY_C_FINDINGS.md`.
- `src/codegen_runtime.rs` — also C; sig widening is its
  own work.
- Non-codegen runtime paths (e.g. `src/ops.rs` guards).  If
  D.3 persists after the audit, the guard logic in
  `op_add_long` may need widening its null check to catch
  `i64::MIN | u32::MAX` — but only after the layout is
  stable.

## Estimated effort

- Phase A inventory: 30 min
- Phase B reasoning: 1-2 hr
- Phase C staged fix: 2-3 hr (interactive, depends on how
  many sites flip tests)
- Phase D verify: 30 min

Total: **4-6 hours** in one session.  Breakable at the end of
Phase A / B (pure documentation), but Phase C is best done in
one sitting because the per-site revert discipline benefits
from uninterrupted attention.

## Alternative: single-site fix piloted

If a 4-6 hour session isn't available, a narrower pilot:

1. Fix `codegen.rs:1628` (`-= 4` → `-= 8`).  Snapshot.
2. If any test regresses, revert.
3. Commit if clean.

This wouldn't fix anything by itself (the failing tests have
no par() extras) but it de-risks the bigger audit by
validating the snapshot-and-revert discipline on a known-safe
edit.  Sets a precedent for the audit.

**Recommendation**: pilot first.  Then if the discipline works,
schedule the full audit.
