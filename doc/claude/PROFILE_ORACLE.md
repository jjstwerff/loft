<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The profiling oracle — what each instrument must say

What `make profile-corpus` checks, and why each row is the row it is. Written
**before** any instrument (@PLN140 phase 0), because stating the answer first is the
only step that can prove a profiler *wrong* rather than merely exercise it — every
trap this tooling has hit produced a **plausible** profile, not an obviously broken
one, so an instrument that is merely run is not an instrument that is checked.

Each row below names a program whose hot spot is known in advance and states what the
instrument must say about it. The machine-readable form is
[`bench/profile_oracle.tsv`](../../bench/profile_oracle.tsv); `make profile-corpus`
runs it and fails a row that stops holding. The how — which switch, which report —
is [PERFORMANCE.md § Profiling a run](PERFORMANCE.md).

The two roles do not substitute for each other ([@PLN140](plans/140-semi-automatic-profiling/README.md)): **benchmarks say the instrument is correct, consumers say it is usable.**
This file is only the first role.

## Reading the table

`expect` is a regular expression the instrument's **top-ranked row** must match, and
`min_share` the percentage that row must reach. A run that names something else, or
names the right thing on a coin toss, fails.

## CPU — arc B, `--interpret`

The interpreter's hot spot is known because these programs are one hot function each.

| Program | Must name | Share | Why the answer is known |
|---|---|---|---|
| `01_fibonacci` | `fib` | ≥ 85 % | `main` calls `fib(38)` once; every op after that is inside `fib`. |
| `02_sum_loop` | `main` | ≥ 85 % | No helper exists — a profiler that cannot name `main` names nothing. |
| `03_sieve` | `is_prime` | ≥ 70 % | `main`'s loop body is one call; the trial-division loop is the work. |
| `05_mandelbrot` | `mandelbrot` | ≥ 70 % | 40 000 calls × up to 256 iterations, against a 40 000-iteration caller. |
| `10_sort` | `insertion_sort` | ≥ 70 % | `main` builds 3 000 elements once, then sorts them O(n²). |

`02_sum_loop` is the row that matters most and looks least interesting: it is the
**negative control**. Four of the five rows would also pass an instrument that simply
reported the deepest frame, and `02` is the one that would not.

The **hot line** is checked on the same runs (`line` column): `01`'s is the recursive
`if` (line 3), `02`'s is `sum += i` (line 6). A profiler that names the right function
and the wrong line is attributing to the frame, not to the work.

The **path** is checked on `01_fibonacci` alone, where it is known to be
`main → fib → fib → …`: the report must show `fib` reached from `fib`, not only from
`main`. That is the one cell a flat hot-spot table cannot fake.

## Memory — arc A, `--interpret`

| Program | Must name | Share | Why the answer is known |
|---|---|---|---|
| `09_matrix_mul` | `bench.loft:(5\|6)` | ≥ 30 % | Two 5 000 000-element `float` vectors, one per line, and nothing else in the program is within two orders of magnitude. |

**Written prediction, corrected against what ran.** This row said "38.1 MiB each"
(5 000 000 × 8 bytes). The instrument reports **136.7 MiB each**, and it is right: the
ledger counts a store's *capacity*, which is what the process holds — the same figure
the memory ceiling is defined against — and `ps` agrees (292 MB RSS ≈ 273 MiB + the
interpreter). The **share** prediction, which is what the row is checked on, held at
50 %. Recorded rather than quietly edited, because a payload-vs-capacity confusion is
exactly the kind of thing a later reader would otherwise rediscover.

`09` is also the only program in the corpus whose peak is far from its exit: the
vectors are alive across the timing loop and released at scope exit, so an
exit-triggered report sees nothing. **A memory report that says "no sites" on `09`
is measuring the wrong moment**, which is the exact defect arc A exists to fix.

Two blind spots this row cannot see, recorded so a later session does not read a
green corpus as full coverage:

* **`07_string_build` is invisible.** Its 5.9 MB live in a Rust `String`, not a
  store, and the store ledger does not count text buffers. The report says so rather
  than reporting `0 B`.
* **`--native` has no site at all** — `alloc_pc` is zero outside interpretation
  (plan open question 3). The oracle therefore has no native memory row, and the
  driver declines the instrument instead of printing a table of `line 0`.

## Allocation paths — arc C

No benchmark can falsify arc C. Every allocation in `bench/` is reached by exactly
one path, so "captured the path" and "printed the only path there is" produce
identical output. The oracle for C is therefore **purpose-built**:
[`bench/profile_oracle/alloc_paths.loft`](../../bench/profile_oracle/alloc_paths.loft)
allocates from one helper called down two chains at a 9:1 ratio, so the instrument
must report two paths through the same source line at roughly that split. A report
showing one path, or two at 5:5, is wrong in a way no `bench/` program could reveal.

This is the general shape of the gap, not a special case: an instrument that
*attributes* needs an oracle with more than one answer available.

**It paid for itself on the first run.** The sampler took every 16th allocation, the
program allocates twice per iteration and takes the rare path every tenth iteration —
so the sampled events were all on odd iterations and the rare path only ever falls on
even ones. The report showed **one path at 100 %**: not noisy, *confidently wrong*, and
no sample count could have revealed it. The fix is a jittered period (`Jitter` in
`src/profiler.rs`), and it applies to the CPU sampler for the same reason. Two further
defects surfaced in the same hour, both of which read as answers:

* An allocation site in a function *prologue* has no `line_numbers` entry at or below
  it inside its own body, so an unscoped `range(..=pc).next_back()` returned the last
  line of whichever function precedes it in the bytecode — reporting `make` at a line
  inside `hot`. Fixed by scoping the lookup to the containing function
  (`State::line_at_in_fn`). **`LOFT_LEAK_SITES` still has it** — its loop runs from
  `check_store_leaks`, which takes no `Data` and so cannot resolve the owning
  function; a leaked store allocated in a prologue is attributed to the previous
  function's last line there. Left alone deliberately: fixing it means changing a
  signature that a dozen tests call.
* `pc == 0` is the "never stamped" sentinel, not a position, and resolving it landed on
  whichever function happens to start the bytecode — a stdlib name attached to the
  interpreter's own stores.

## What the corpus does NOT prove

Usability. Ranking a thousand sites onto one screen, and whether the overhead is
tolerable on a program big enough to need the instrument, are answered by a grown
consumer (`moros` / `dryopea` / `crawler` / `lib/markdown`), not here. Overhead is
reported by `make profile-corpus` as an on/off ratio per program so the number is at
least measured on something that runs long.
