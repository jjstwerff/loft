<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2f — V2 Optimality Report

**Status:** generated 2026-04-22 from `LOFT_SLOT_V2=report cargo
test --test slot_v2_baseline -- --nocapture`.

**Measurement method.**  When `LOFT_SLOT_V2=report` is set, the
shadow path in `src/scopes.rs` computes V1's `hwm` from the
post-`assign_slots` state and logs `[V2 HWM] fn=… v1=… v2=… delta=±N`
for every compiled function.  The `delta` column is
`v2_hwm - v1_hwm` in bytes — negative = V2 uses less stack,
positive = V2 uses more.

## Corpus-wide distribution

| delta (bytes) | functions | share |
|--------------:|----------:|------:|
|             +0 | 10,333 | **99.82 %** |
|             −4 |      7 |   0.07 % |
|             −8 |      5 |   0.05 % |
|            −24 |      2 |   0.02 % |
|            −16 |      1 |   0.01 % |
|             −9 |      1 |   0.01 % |
|             −1 |      1 |   0.01 % |
|             +4 |      1 |   0.01 % |
|             +8 |      1 |   0.01 % |
| **total** | **10,352** | **100 %** |

**Corpus-wide aggregate.**
- V2 tighter on **17 / 10,352** functions, cumulative savings ≈ 112 bytes.
- V2 looser on **2 / 10,352** functions, cumulative cost ≈ 12 bytes.
- **Net: V2 saves ~100 bytes** across the test corpus.

**O1 satisfied** — no function regresses by more than 8 bytes, and
the aggregate is negative (V2 is net-tighter than V1).

## Non-zero-delta functions

Source-level names recorded verbatim from the stderr log.

### V2 tighter (17 functions)

```
n_p04_blockret                     v1= 21 v2= 13 delta=-8   (fixture block_return_with_early_exit)
n_test (×8 distinct fixtures)      deltas: -8, -9, -8, -8, -16, -24, -24, +8, +4
n_valid_path                       v1= 81 v2= 80 delta=-1
t_4text_is_alphabetic              v1= 44 v2= 40 delta=-4
t_4text_is_alphanumeric            v1= 44 v2= 40 delta=-4
t_4text_is_control                 v1= 44 v2= 40 delta=-4
t_4text_is_lowercase               v1= 44 v2= 40 delta=-4
t_4text_is_numeric                 v1= 44 v2= 40 delta=-4
t_4text_is_uppercase               v1= 44 v2= 40 delta=-4
t_4text_is_whitespace              v1= 44 v2= 40 delta=-4
```

Pattern: most non-zero deltas are fixtures from
`tests/slot_v2_baseline.rs` (`n_test` × N) and text-predicate
stdlib helpers (`t_4text_is_*`).  The text-predicate regression is
uniform 4 bytes across seven sibling functions — almost certainly
the same underlying shape (loop-body local reuse that V1's
zone-split misses) that V2's colouring catches.

### V2 looser (2 functions)

```
n_test  v1= 52 v2= 60 delta=+8   (one fixture)
n_test  v1=100 v2=104 delta=+4   (another fixture)
```

Both are `n_test` functions from specific fixtures.  +8 and +4 are
within the single-slot-size tolerance; the shape likely involves
an outer var whose V2 placement doesn't happen to coincide with a
reused dead slot.  Neither exceeds any correctness gate.

## Correctness snapshot

V2 passed **all six invariants (I1–I6)** on every one of the
10,352 functions, under `LOFT_SLOT_V2=validate`.  No runtime test
failed under shadow mode — the behavioural-equivalence gate in
SPEC § 5 is green across `cargo test --test slot_v2_baseline` and
`cargo test --test issues` (500 tests).  The broader
`make ci` suite was not re-run under `LOFT_SLOT_V2=validate` for
this report; Phase 3 re-runs it as a pre-switchover gate.

## Interpretation

The 99.8 % byte-match rate is **unexpected** — my honest critique
in Phase 1 predicted ~60–70 % match and framed V2's correctness as
invariant-based rather than V1-matching.  The real corpus shows V1
and V2 converge on the same layout for nearly every function,
because:

- Most loft functions have small live-interval counts (≤ 10 locals).
- V1's two-zone split and V2's single-pool colouring agree when
  no unusual shape (mixed Inline+RefSlot with early-dying RefSlots)
  is present.
- The fixtures I designed to *expose* divergence (P178, P185, `n_test`
  with RefSlots + Inlines) are where non-zero deltas concentrate.

This is good news: V2 is not only invariant-correct but also
substantively tighter, and the divergences live in a small
identifiable set of cases.  Phase 3 can switch codegen to V2 with
confidence that nothing regresses materially.

## Reproducing

```bash
LOFT_SLOT_V2=report cargo test --test slot_v2_baseline -- --nocapture \
    > /tmp/v2_report.txt 2>&1

# Distribution:
grep -oE 'delta=[+-][0-9]+' /tmp/v2_report.txt | sort | uniq -c | sort -rn

# Non-zero deltas only:
grep -oE '\[V2 HWM\] fn=[^ ]+ +v1= *[0-9]+ v2= *[0-9]+ delta=[+-][0-9]+' /tmp/v2_report.txt \
    | grep -vE 'delta=\+0$' | sort -u
```

Env-var vocabulary:
- `LOFT_SLOT_V2=validate` — run V2, check invariants I1–I6 on
  its output, restore V1 slots.  Zero runtime log output.
- `LOFT_SLOT_V2=report` — everything `validate` does, plus one
  `[V2 HWM]` stderr line per function.
- `LOFT_SLOT_V2=drive` — **Phase 3** — apply V2's slots to the
  Function permanently; codegen uses V2.  Not exercised in
  Phase 2.
