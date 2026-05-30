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

## Sub-cluster 2b — sorted-collection iteration (PASS 2)

**Mechanism (PINNED, see cluster-2-fix-design.md § Stage C).** `State::step()`
(`src/state/io.rs`) walks the iterator-state block at hard-coded raw byte
deltas (`state_var - 4` = finish, `-8` = next cur, `-12` = done); under
LOFT_ALIGN those state words are `stack_step(4)`-spaced (8 apart, written by
`iterate()`'s `put_stack`), so `finish` reads from the wrong (padding) slot and
`pos >= finish` is immediately true → the `for` body never runs.  The
`iterate()` *setup* is correct (trace: start=MAX, finish=1); the fault is the
`step()` delta arithmetic.  Distinct from 2a (which shifts a *value*); here the
*cursor* dies and NO elements are seen.  Even a single element triggers it
(`2b-04`); empty is fine (`2b-03`, no `step`); plain vector is fine (`2b-02`).

| Probe | Shape | Aligned now | Role |
|---|---|---|---|
| `2b-01-sorted-int-iter` | `for e in sorted<int key>` sum | **FAIL** (`total 0`) | minimal reproducer |
| `2b-02-vector-iter-ref` | same, plain `vector` | PASS | reference — isolates *sorted* gather |
| `2b-03-sorted-empty-ref` | empty sorted | PASS | reference — no gather |
| `2b-04-sorted-single-elem` | one element | FAIL | edge — first gather already breaks |
| `2b-05-sorted-text-key` | text key, order (inc12) | FAIL (empty) | variant |
| `2b-06-sorted-int-ordering` | out-of-order insert, asc check | FAIL (empty) | variant |
| `2b-07-sorted-return-from-fn` | sorted returned from fn (p300) | FAIL (empty) | variant |
| `2b-08-sorted-rebuild-in-loop` | nested gather + reassign (p295) | FAIL (empty) | variant |

Maps to: `inc02`, `inc12`×2, `p190`, `p277`, `p295`, `p300`, `p4d_b`, `n2`.

## Sub-cluster 2c — hash-collection iteration (PASS 3)

**Mechanism (PINNED — shares 2b's root, see cluster-2-fix-design.md § Stage C).**
Hash iteration materialises a scratch rec-nr vector (`{id}#hash_scratch`,
`parser/collections.rs`) and walks it through the SAME `step()` (case 3) — so
the same raw `state_var - N` deltas mis-address the cursor, here producing a
SPURIOUS LEADING element (empty/zero): `,apple,mango,zebra,` instead of
`apple,mango,zebra,`, a count of 4 for 3 entries.  Mirror image of 2b: 2b
(sorted) DROPS all elements; 2c (hash) ADDS a phantom — both from `step()`.
The 2b fix is expected to close 2c; one open check (the scratch-vector build)
noted in the design.  Empty hash is fine (`2c-02`, no `step`); a single
element already triggers it (`2c-04`).

| Probe | Shape | Aligned now | Role |
|---|---|---|---|
| `2c-01-hash-iter-single-field` | string-join iteration | **FAIL** (`,apple,…`) | minimal reproducer |
| `2c-02-hash-empty-ref` | empty hash | PASS | reference — no gather |
| `2c-03-hash-iter-count` | count elements | FAIL (`n=4`) | numeric statement of the phantom |
| `2c-04-hash-single-elem` | one element | FAIL (`n=2`) | edge — first gather breaks |
| `2c-05-hash-multi-field-key` | two-field key (c60 multi) | FAIL | variant — key arity invariant |
| `2c-06-hash-iter-filter-clause` | `for … if` (c60 filter) | FAIL (garbage) | phantom feeds a guard |
| `2c-07-hash-iter-loop-index` | `e#index` (c60 loop-attr) | FAIL (`20` vs `14`) | phantom shifts every index |

Maps to: `c60_hash_iter_{single_field_asc, multi_field_lex, filter_clause,
loop_attributes}`.

## Sub-cluster 2d — composite format / tuple / par / misc (PASS 4)

**Mechanism (dominant shape).** Interpolating a COMPOSITE value (vector /
struct / enum) into a format string — `"{v}"` or `"{v:j}"` — returns an EMPTY
string under LOFT_ALIGN: the format path reads the value's DbRef from a
mis-stepped slot and renders nothing.  Scalars format fine (`2d-02`).  A few
2d members are distinct shapes (tuple+`par` marshalling, hash-loop build,
struct-enum json round-trip) but all read a composite handle from a stepped
slot.

| Probe | Shape | Aligned now | Role |
|---|---|---|---|
| `2d-01-format-vector-int` | `"{sort_it()}"` (n8a) | **FAIL** (empty) | minimal reproducer |
| `2d-02-format-scalar-ref` | `"val={x}"` scalar | PASS | reference — scalar vs composite |
| `2d-03-format-vector-text` | `vector<text>` (n5) | FAIL (empty) | variant |
| `2d-04-format-struct-with-vector` | struct + `&Data=null` (n8b) | FAIL (empty) | variant |
| `2d-05-format-enum-variant` | parse+format variant (n4) | FAIL (empty) | variant |
| `2d-06-struct-multivec-to-json` | 2-vector struct `:j` (p145) | FAIL (empty) | variant — single-file p145 |
| `2d-07-struct-enum-json-roundtrip` | json round-trip + match (p159) | FAIL | variant |
| `2d-08-vector-tuple-par` | `vector<(int,int)>` + `par` (p189c) | FAIL | variant — tuple + par marshalling |
| `2d-09-hash-loop-build-iterate` | loop-built hash, count (p193) | FAIL (count 11) | variant — 2c phantom via loop build |

Maps to: `p145`, `p159`, `p189c`, `p193`, `n4`, `n5`, `n8`×2.

## Sub-cluster 2h — tuple in a parallel `par` worker frame (PASS 5)

**The last open cluster-2 aligned failure** (`p189c`, was mis-filed as 2d-08).
A `vector<tuple>` consumed by `par(...)` reads its tuple argument from a
mis-stepped offset in the marshalled worker frame under LOFT_ALIGN — the
`stack_align_guard` fires `i64 at offset 132` in `get_var` ← `var_int` ←
`execute_at_raw_primitive_input_wide` inside a worker — so every worker sees
garbage and the result collects as 0.  NOT composite format (2d) and NOT par in
general: `2h-02` (scalar par) PASSES, isolating the trigger to the tuple.

| Probe | Shape | Aligned now | Role |
|---|---|---|---|
| `2h-01-tuple-par-min` | `vector<(int,int)>` + `par(...,4)` (p189c) | **FAIL** (sum 0) | minimal reproducer |
| `2h-02-scalar-par-ref` | `vector<int>` + `par` | PASS | reference — par-in-general is sound |
| `2h-03-tuple-par-single-elem` | one tuple | FAIL | edge — per-worker read, not accumulation |
| `2h-04-tuple-par-one-worker` | `par(...,1)` | FAIL | edge — worker-count independent |
| `2h-05-tuple-par-mixed-width` | `(integer, character)` | FAIL | variant — any element widths |
| `2h-06-tuple-par-triple` | `(int,int,int)` | FAIL | variant — arity-independent |

**Incidental finding (separate, NOT alignment — flag-OFF bug):** calling a
`fn f(p: const (integer,integer))` with a tuple loop var in a *sequential*
(non-par) `for p in pairs { f(p) }` fails flag-OFF with
`expected (integer, integer), got __tuple<integer,integer>` — the materialised
loop var's type doesn't unify with the tuple param.  The `par` form works
flag-OFF, so this is a sequential-tuple-arg-passing gap, unrelated to cluster 2.
Noted, not probed (a clean 2h probe must PASS flag-OFF).

## Coverage status (2026-05-30)

Five sub-families authored — **41 probes** covering every one of the 27
aligned-mode failures from the sweep, plus references and edges:

| Sub-cluster | Probes | Failing issues tests covered | Aligned status |
|---|---|---|---|
| 2a generator-arg | 11 | p210, p211, p218×2, p225, p328 | ✅ **FIXED 2026-05-30** |
| 2b sorted-iter | 8 | inc02, inc12×2, p190, p277, p295, p300, p4d_b | ✅ **FIXED 2026-05-30** |
| 2c hash-iter | 7 | c60×4 | ✅ **FIXED 2026-05-30** |
| 2d composite-format/misc | 9 | p145, p159, p193, n2, n4, n5, n8×2 | ✅ **FIXED 2026-05-30** |
| 2h tuple-in-par (byte smear) | 6 | p189c | ✅ **FIXED 2026-05-30** |
| 2i tuple-in-par (non-8-mult total) | 5 | — (probe-discovered) | ✅ **FIXED 2026-05-30** |

**Aligned `issues` suite is now 685 / 0** (zero failures/crashes/hangs, down
from 27 at session start).  2h+2i fix: `execute_at_raw_primitive_input_wide`
block-copies the worker-arg buffer (was a stepped byte-by-byte smear) AND
reserves a `stack_step`-ed worker frame (was a raw total that underflowed for
non-8-multiple tuples).  See cluster-2-fix-design.md § "2h + 2i — LANDED".

**NOT yet switch-ready:** the `stack_align_guard` still fires on the par-worker
path (`2j` — pre-existing entry-base = 4, should be stepped 8; affects scalar
par too), and Miri hasn't been run.  Aligned mode is functionally green but the
guard-clean + Miri validation gates remain.

`run.sh` exits 0 — every probe PASSES flag-OFF and every `*-ref*` PASSES
aligned.  The aligned column is what each cluster fix closes; re-run `run.sh`
after each fix to watch it flip to PASS.

**2b+2c LANDED 2026-05-30** (one fix — pack the iterator-state i64 in
`iterate()`; see cluster-2-fix-design.md § "2b+2c — LANDED").  All 15 `2b-*` /
`2c-*` probes (and `2d-09`, a hash-iter case) now PASS aligned; aligned
`issues` sweep 27→14 failures, zero regressions, flag-OFF 681/0.

**2a LANDED 2026-05-30** (one line — `coroutine_create` reserves the
return-address slot at `stack_step(4)` instead of a raw 4; see
cluster-2-fix-design.md § "2a — LANDED").  All 11 `2a-*` probes PASS aligned;
aligned `issues` 14→8 failures with **0 CRASH / 0 HANG** (the coroutine family
p210/p211/p218×2/p225/p328 closed), zero regressions, flag-OFF 681/0.

**2d LANDED 2026-05-30** (one term, two sites — `format_database`/
`format_stack_database` back up by `stack_step(size_ref())` not raw
`size_ref()`; see cluster-2-fix-design.md § "2d — LANDED").  6 composite-format
probes flip to PASS aligned; aligned `issues` 8→1, closing n4, n5, n8×2, p145,
p159 AND `n2` (which formats a composite — NOT a separate mechanism after all).

After 2a+2b+2c+2d the cluster-2 aligned `issues` surface is a SINGLE remaining
failure: `p189c` — a tuple-in-`par` worker-frame root cause, now probed as
sub-cluster **2h** (`2h-01`…`2h-06`; `run.sh 2h`).  `2d-08-vector-tuple-par`
is the original 2d-filed reproducer, superseded by the focused 2h set.

**Note — `n2` is NOT a 2b case.**  `n2_sorted_field_content_type_registered_
first` was loosely listed under 2b but is a *separate* mechanism (sorted-field
content-type registration order, not iteration); the 2b+2c fix did not close it
and it has no dedicated probe yet.  Tracked as a future sub-cluster.

## Promotion gate

A probe graduates to `tests/scripts/NN-plan53-…` only once it PASSES under
both flag-OFF and aligned mode (i.e. the cluster fix has landed) AND passes
the standard four gates (assertions, clean exit, no leak, bounded runtime).
Until then it lives here as a fix-validation reproducer.
