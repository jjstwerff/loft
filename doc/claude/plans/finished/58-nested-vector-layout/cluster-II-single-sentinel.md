<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster II — Single-NaN-sentinel read as wild rec-id

**Severity (split by failure mode):**
- **Corruption / panic / hang:** **SIGSEGV** (`rc=139`) — hard crash, often at an
  unrelated location (memory corruption manifests later, e.g. in
  `default/05_coroutine.loft`).  Highest user harm in this plan.
- **Leak:** not assessed (process dies before teardown).

**Affected probes:** 04 (2-deep literal), 05 (3-deep `+=`, the #262 seed).
**Backend asymmetry:** both (interp SIGSEGV; native produces no correct output).

## Mechanism (verified / hypothesized)

`single` (f32) null is the NaN bit-pattern `0x7FC00000` — **non-zero**.  Where a
nested-vector element handle is laid down, an uninitialised / null-defaulted slot
holding that pattern is later read as a `u32` rec-id.  Non-zero ⇒ a wild rec-id ⇒
out-of-bounds store access ⇒ SIGSEGV.  Contrast `integer` null = `i64::MIN`,
whose low-32 bits are `0` (an empty handle, harmless) — which is why the integer
probes (02, 06) are safe and the single probes (04, 05) crash.

The existing **@P380** fix (`src/parser/vectors.rs`, an `OpSetInt4`-zero of the
handle before `OpCopyRecord` for `Type::Vector` elements) handles *some* single
case but not these.

## What we know vs. don't

| Claim | Status |
|---|---|
| `single` null = `0x7FC00000` (non-zero); `integer` null low-32 = 0 | ✅ Verified — known sentinel model |
| 3-deep `vector<vector<vector<single>>>` `+=` copy SIGSEGVs | ✅ Verified — probe 05 `rc=139` (matches #262) |
| **2-deep `vector<vector<single>>` *literal* SIGSEGVs at construction** | ✅ Verified — probe 04 + `/tmp/p04_construct.loft` (construct-only) `rc=139` |
| #262's claim "2-deep single works fine" | ❌ Refuted for the *literal* form — only the `+=` form may have been tested |
| Flat `vector<single>` is safe | ✅ Verified — `/tmp/p04_inner.loft` PASS |
| The `--vec4` stride lever fixes it | ❌ Refuted — probes 04/05 crash identically ±`--vec4` (orthogonal to stride) |
| Crash site is the single-handle write in `new_record` (construction) and the copy path (3-deep) | 🤔 Hypothesized — needs a trace pinning the OOB store access |

## Investigation tasks

1. Trace probe 04 (construct-only) under `LOFT_LOG=minimal` / `crash_tail:50` to
   pin the exact opcode laying down the non-zeroed single-vector handle.
2. Map the scope: 2-deep literal, 2-deep `+=`, 3-deep literal, 3-deep `+=`,
   write `vv[i][j]=x`, struct-field — which forms cover @P380, which don't.
3. Extend the @P380 `OpSetInt4`-zero (or the construction default) to every
   `Type::Vector`-element form that surfaces the single sentinel.

## Fix surface (preliminary)

The handle slot for a nested-vector element must be **zeroed** before the
element's record is written, regardless of the element's base type — so a
`single`'s NaN never reaches a rec-id read.  @P380 did this for one path; the
fix generalises it.  Likely a `new_record` / construction-default change, not a
copy-path change, since probe 04 crashes at construction with no copy involved.

## Why native also crashes

The strides + handle-zeroing are decided at parse time (IR operands) and the
construction default is shared; native inherits the same un-zeroed single handle,
so the wild rec-id read reproduces under `--native` too.
