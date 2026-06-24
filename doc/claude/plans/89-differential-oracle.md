<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN89 — the differential oracle (operational.md D-op-1 / D-op-2)

> **Status — seed phase LANDED (this PR).** The runner + positive control + an
> 8-program seed corpus are in and green across both backends. The oracle already
> earned a real catch (#448, NRVO mixed-return leak — primary shape fixed, guarded
> by corpus program 08). The corpus now GROWS per the contract below; the immediate
> next user of the net is the **ownership code-simplification exploration**
> ([OWNERSHIP_MODEL.md § ACTIVE](../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable)).
> D-op-1/2 stay OPEN until the corpus is broad enough to reliably catch the class.

The interpreter (`src/state/`) and the native generator (`src/generation/`) are
two separate implementations of one language, kept in agreement **only by
tests**. A program the interpreter runs fine but `--native` miscompiles, leaks,
or halts differently ships until some test happens to exercise it — a coverage
lottery. The canonical case is #433 (interp evaluated it; `--native` failed to
compile, `E0308`); this cycle's overflow-log attempt was another (it broke
`--native` with `E0499` while the interpreter was perfectly happy).

The oracle turns that class from a lottery into a **caught failure**:

> Every program in `tests/oracle/` runs on **both** backends, and their
> observable outcome must **AGREE** — normalised stdout (value / null), exit
> code (halt), and leak-freedom. A divergence on any program is a caught
> cross-backend bug.

This is the "Removal" for operational.md **D-op-1 / D-op-2**: the rules there
stay the written contract that *guides* what the corpus covers; the oracle is
the executable instrument that *enforces* agreement. (It is deliberately chosen
over a single executable shared semantics for now — switchable later; the rules
are reused either way.)

## What's here (seed, this session)

- **`tests/differential_oracle.rs`** — the runner. `divergences(interp, native)`
  is a pure function (stdout / exit / leak), unit-tested by a **positive
  control** (`positive_control_divergences_are_detected`) that proves the
  detector fires on each divergence kind and stays quiet on agreement — a green
  sweep is only evidence once the detector is known to work. The corpus sweep
  (`oracle_corpus_agrees_across_backends`) is `#[ignore]` (rustc per program);
  run it with `cargo test --release --test differential_oracle -- --ignored`.
- **`tests/oracle/*.loft`** — the seed corpus (7 programs), each targeting an
  axis where the backends have historically diverged: arithmetic nested in other
  `&mut` runtime calls (the E0499 shape), vector-return ownership + leaks, struct
  mutation through a `&`-ref, closure capture + map/filter, the arithmetic fault
  contract, left-to-right evaluation order, enum/match dispatch.

All 7 agree across both backends today; the positive control + normalisation
tests pass in the default (fast) path.

## The growth contract

The corpus is **not** meant to be complete — it is meant to **grow**:

1. Every divergence the oracle (or any debugging session) finds graduates a
   guard program into `tests/oracle/` once fixed.
2. New language features add a corpus program exercising them on both backends.
3. The operational.md rules (E-Left, E-Uncomp, E-NullArg, …) are the checklist
   for what coverage to add next.

When the corpus is broad enough that "interp and native disagree on a program
both accept" is reliably caught before ship, D-op-1 / D-op-2 close.

## First finding — [#448](https://github.com/loft-lang/loft/issues/448) (the oracle's first catch)

While authoring the seed, a vector program leaked one store. The boundary matrix
narrowed "four coexisting vectors" down to the real root: a `vector<T>`-returning
function with **multiple return statements of differing construction** leaks the
store of whichever path does NOT deliver into `__retbuf`.

```loft
fn pick(f: boolean, n: integer) -> vector<integer> {
  if f { return squares(n); }   // NRVO-adopts the call into __retbuf
  return [n, n, n];             // builds a SEPARATE store, returns THAT → leaks
}
```

Uniform-construction returns (all-call / all-literal) are clean; only the
**mixed** multi-return leaks, on both backends. Root cause: the call path
promotes the fn to NRVO (`returned = __retbuf`, so the caller frees `__retbuf`),
but the other path returns a fresh store the caller never frees. This is the
**O-Move / O-Complete** gap (formal/ownership.md D-own-2): the #443 cluster-I
materialization delivers `match`/`if` *tails* into `__retbuf`, not the
early-return + tail-return shape. Workaround: a single tail return via a result
local. Once fixed, the minimal repro graduates into `tests/oracle/`.

## Next steps

- Add a CI job that runs the `--ignored` sweep (nightly or per-PR-on-codegen).
- Grow the corpus along the operational.md axes (heap/store steps, iterators,
  coroutines are unwritten in the rules and uncovered here).
- Investigate the coexistence leak above.
