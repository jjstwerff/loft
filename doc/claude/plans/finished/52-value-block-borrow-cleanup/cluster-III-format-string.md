<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster III — `??` inside a format-string interpolation

**Severity:** silent data corruption — formatted output contains garbage bytes where the `??` result should be.  Distinct from cluster I's NUL-fill: cluster III produces NON-zero garbage because the format-buffer's `OpFormatText` overwrites the freed region BEFORE the buffer is read back.

**Affected probes:** 09, 19, 30, 56, 78.  See [Probe set D](README.md#curated-probe-sets--for-fix-attempt-validation).

**Backend asymmetry:** Interpret-side only.  Native passes (via the same `_ret.to_string()` materialisation that escapes cluster I).

## Mechanism (verified)

Cluster III is **`??`-DEPENDENT** — probe 18 confirms that plain `"{vec[i]}"` interpolation (no `??`) does NOT corrupt.  Only `"{vec[i] ?? \"y\"}"` does.

Same root mechanism as cluster I: the `??` lowering builds an `_ncc_N` block whose tail Str borrows into the block-local String; scope-exit `OpFreeText(_ncc_N)` invalidates the Str before the consumer reads.

The DIFFERENCE from cluster I-IA (pure-Set NUL) is the CONSUMER — `OpFormatText` (the format-string builder op):

```
{ #format:text
  __work_N = "";            ← function-scope work buffer
  __work_N += "got: ";
  { #ncc:text
    _ncc_M = h.items[0];
    if (_ncc_M != null) _ncc_M else "y";
    OpFreeText(_ncc_M);     ← _ncc_M freed at end of inner block
  }
  OpFormatText(__work_N, <dangling Str from outer block>, ...);
  __work_N
}
```

When `OpFormatText` runs, it WRITES bytes into `__work_N`.  Crucially, this write happens AFTER `_ncc_M`'s String has been deallocated but BEFORE the format-buffer's bytes are read out by the outer consumer.  The deallocator returns `_ncc_M`'s heap region to the allocator, which then RE-USES that region as `__work_N`'s growing buffer.  Reading the dangling Str's `ptr` after `__work_N` has written to that region yields the FORMAT-BUFFER'S CONTENT, not NUL.

Compounded with multi-`??` in one format string (probe 19), the garbage compounds (`'      and ��S'`) — each `_ncc_N`'s free interleaves with the next interpolation's `OpFormatText`.

Probe 78 (intervening allocations between format-build and assert) shows the bytes depend on what runs between, confirming the "freed-then-rewritten" mechanism rather than a fixed sentinel.

## Reference probe — 18 (plain format, no `??`, PASS)

```loft
h.items += "present";
msg = "got: {h.items[0]}";   // no ??
```

Lowering: format-string's `OpFormatText` reads `h.items[0]`'s Str directly from the vector's permanent storage.  No `_ncc_N` block; no scope-exit free of a borrow target.

## Problem probe — 09 (format with `??`, FAIL on interpret)

```loft
h.items += "present";
msg = "got: {h.items[0] ?? \"x\"}";   // ?? inside format
```

Lowering: inner ncc-block (`_ncc_M`) gets freed at the end of the interpolation; the format-buffer's `OpFormatText` reads from the dangling Str's ptr after `__work_N`'s own buffer has reallocated the region.

## The divergence

`??` introduces `_ncc_N` and its scope-exit free.  The format-buffer consumer's write-to-`__work_N` happens between the free and the next read, so the dangling region gets rewritten with format-buffer bytes instead of leftover-NUL.

## What we know vs. don't

| | Status |
|---|---|
| Cluster fires only when `??` is INSIDE the format-string interpolation | ✅ Verified — probe 18 (no ??) PASSES |
| Garbage bytes are non-zero (format-buffer leftovers) | ✅ Verified — probes 09/30 |
| Multi-`??` compounds the garbage | ✅ Verified — probe 19 |
| Intervening allocations change the garbage bytes | ✅ Verified — probe 78 |
| Cluster III closes incidentally with cluster I's `__ret_text_N` materialisation | 🤔 Strong hypothesis — same root.  Verify by running Set D after cluster I lands |
| Format-buffer needs its own targeted materialisation if I's fix doesn't close III | 🤔 Fallback plan; consider only if Set D regresses post-I |

## Investigation tasks

1. ~~Verify `??`-dependence~~ — done (probe 18).
2. ~~Verify garbage-not-NUL byte pattern~~ — done (probes 09/30).
3. ~~Verify intervening-disturbance behaviour~~ — done (probe 78).
4. **After cluster I fix lands**: re-run Set D.  If all 5 probes PASS, cluster III closes for free.  If any fail, the format-buffer machinery needs its own materialisation:
   - In `OpFormatText`, the source Str is captured BEFORE the format-buffer's append op.  If we copy bytes immediately (instead of holding the Str pointer), the freed region's later rewrite doesn't matter.

## Fix surface

**Primary path**: closes incidentally with cluster I's `__ret_text_N` parent-scope text-temp fix (see `cluster-I-ncc-text.md`).  The materialisation happens at the value-block level, BEFORE the format-buffer's `OpFormatText` reads — so by the time format-buffer reads, it's reading from `__ret_text_N`'s live String, not a dangling Str.

**Fallback path** (if I's fix doesn't close III): targeted fix in the format-string consumer (probably `src/state/codegen.rs` or `src/state/io.rs` where `OpFormatText` is emitted/dispatched).  Make the op capture-then-copy rather than capture-then-read-later.

**Effort if needed**: S — small targeted change.  Risk LOW — format-string consumer is well-isolated.

## Why native escapes

Same reason as cluster I: `src/generation/emit.rs:1283-1297` materialises the ncc-block result inside the block via `_ret.to_string()`.  The format-string's interpolation reads from an owned `String` rather than a borrow.
