<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLAN53 cluster-2 probes

Minimal loft programs that turn the cluster-2 unaligned-eval-stack UB into
deterministic PASS/FAIL/HANG/CRASH signals.  Each runs under the interpreter
(the byte-packed eval stack cluster 2 lives in; `--native` has no such stack)
in two modes:

- **flag-OFF** (production default) — **must PASS** for every probe.  This is
  the invariant: production is clean, the probes only bite under the aligned
  experiment.
- **aligned** (`LOFT_ALIGN=1 LOFT_SLOT_V2=drive`) — currently reproduces the
  bug; **must PASS after the cluster fix lands**.

Run them:

```bash
cargo build --release --bin loft        # once
doc/claude/plans/future/53-sanitizer-ci-lever/probes/run.sh        # all
doc/claude/plans/future/53-sanitizer-ci-lever/probes/run.sh 2a -v  # one sub-cluster, verbose
```

`run.sh` runs each probe as its own subprocess under `LOFT_TIMEOUT`, so a
runaway generator aborts cleanly instead of spinning the machine and a
SIGSEGV stays contained to one probe.  It exits non-zero if any probe breaks
an invariant (fails flag-OFF, or a `*-ref* ` probe fails aligned).

## Why subprocess isolation (and not `cargo test`)

The aligned-mode residuals cannot be inventoried in-process: a single
`cargo test --test issues` run either SIGSEGVs (one heap-corrupting test
kills the whole binary under thread parallelism) or hangs 18 min (a runaway
generator under `--test-threads=1`).  One probe per `loft` subprocess +
`LOFT_TIMEOUT` is the only reliable classifier — and is itself the probe-run
substrate the plan's § Probe suite calls for.

## Sub-cluster 2a — generator argument mis-offset across `yield` (PASS 1)

**Verified mechanism (2026-05-30).** A generator that reads its own
**argument** after a `yield` reads it back **4 bytes too high** in aligned
mode (the `step(4)` return-address slot rounds 4→8, but the args/locals
boundary in the coroutine create / yield / restore path is advanced by the
raw 4 somewhere, shifting the argument region).  An 8-byte `n = 42` reads as
`42 << 32 = 180388626432`.  The symptom escalates with how the corrupted
argument is used:

| Probe | Shape | Aligned now | Role |
|---|---|---|---|
| `2a-01-gen-arg-single-yield` | `yield n` once | **FAIL** (`42<<32`) | minimal reproducer |
| `2a-02-gen-constant-yield-ref` | `yield 7` (no arg) | PASS | reference — isolates *argument* read |
| `2a-03-gen-no-arg-while-ref` | `while i<3` (no arg) | PASS | reference — yield/resume of *locals* is sound |
| `2a-04-gen-arg-two-yields` | `yield n; yield n+1` | FAIL | corruption across a resume cycle |
| `2a-05-gen-arg-while-hang` | `while i<n` (p210 shape) | **HANG** | bound re-read → never terminates |
| `2a-06-gen-arg-for-range-hang` | `for i in 0..n` | HANG | same, range desugar |
| `2a-07-gen-text-arg-format-crash` | `text` arg + format (p218 shape) | **CRASH** (SIGSEGV) | corrupted `Str` ptr deref |
| `2a-08-gen-text-arg-const-yield-while` | `while i<n` yield const text (p211 shape) | HANG | text yield, int-arg bound |
| `2a-09-gen-yield-from-delegation` | `yield from inner(s)` (p225 shape) | CRASH | nested frame + text arg |
| `2a-10-gen-yield-closure-capture` | yield closure capturing `base`, manual `next()` (p328) | FAIL (`100<<32\|5`) | 20-byte fn-ref yield + capture |
| `2a-11-gen-two-int-args-edge` | two int args, `yield a; yield b` | FAIL (`8<<32`) | EDGE — both args shift uniformly (one boundary) |

The reference pair (`2a-02` constant-yield PASS vs `2a-01` arg-yield FAIL;
`2a-03` no-arg-loop PASS vs `2a-05` arg-loop HANG) pins the trigger to the
argument read, not the yield machinery.  `2a-11` pins it to a *single*
shifted args/locals boundary (both args move together), not per-slot padding.

Maps to the failing `tests/issues.rs` cases: `p210`/`p211` (HANG),
`p218`×2/`p225` (CRASH), `p328` (wrong value).

**Incidental finding (not cluster 2):** a float-yielding generator
(`fn f(x: float) -> iterator<float> { yield x; }`) produces NO output even
flag-OFF — coroutines appear to lack a `next_float` path (only `next_i64` /
`next_text` exist, cf. p211's history).  A separate limitation, not an
alignment bug; no probe authored (a clean probe must PASS flag-OFF).

## Later passes (not yet authored)

The other cluster-2 sub-families from the 2026-05-30 aligned-mode sweep
(658 ok / 22 FAIL / 3 CRASH / 2 HANG), to be probed in subsequent passes:

- **2b — sorted-collection iteration**: `inc02`, `inc12_*`, `p190`, `p277`,
  `p295`, `p300`, `p4d_b`, `n2`.
- **2c — hash iteration (`c60`)**: `c60_hash_iter_{single_field_asc,
  multi_field_lex,filter_clause,loop_attributes}`.
- **2d — struct / tuple / vector / misc**: `p145`, `p159`, `p189c`, `p193`,
  `n4`, `n5`, `n8_*`.

## Promotion gate

A probe graduates to `tests/scripts/NN-plan53-…` only once it PASSES under
both flag-OFF and aligned mode (i.e. the cluster fix has landed) AND passes
the standard four gates (assertions, clean exit, no leak, bounded runtime).
Until then it lives here as a fix-validation reproducer.
