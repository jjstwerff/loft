<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# H7 — a variable aliases a call's return buffer, and a loop re-enters it

> **Status: LOCALIZED, NOT FIXED (2026-07-26).** Root cause proven with a 17-cell
> boundary matrix and a working-vs-broken IR differential. The fix is a
> store-lifetime substrate change and is deliberately **routed, not patched** — see
> § Why this is not fixed here. Probes: `tests/probes/h7-retbuf-alias/`.

Reported by moros (H7). Both backends agree, so this is the parser, not codegen.

## The symptom

```loft
fn add_i(v: vector<integer>, x: integer) -> vector<integer> { out = v; out += [x]; out }

fn main() {
  a: vector<integer> = [];
  for k in [10, 20, 30] { a = add_i(a, k); }
  println("{len(a)}");        // prints 1 — should be 3
}
```

No diagnostic. `a` ends holding `[30]` — only the last element.

## The mechanism

A function returning a heap type takes a **caller-allocated return buffer** as a
hidden trailing argument, and its first act is to clear it:

```
fn n_add_i(v:vector<integer>, x:integer, out:vector<integer>) {
  OpClearVector(out);            // ← clears the buffer
  OpAppendVector(out, v, 0);     // ← then reads v
  …append x…
  return out;
}
```

The caller allocates **one buffer per CALL SITE** and binds the assignment target
to it as a dependency (`a["__ref_1"] = n_add_i(a, k, __ref_1)`), i.e. `a` becomes a
view of `__ref_1` rather than a copy of it.

For straight-line code that is safe — each statement has its own buffer:

```
a["__ref_3"] = n_add_i(a, 10, __ref_1);     // three DISTINCT buffers
a["__ref_3"] = n_add_i(a, 20, __ref_2);
a["__ref_3"] = n_add_i(a, 30, __ref_3);
```

In a loop the single call site executes N times against **one** buffer. After
iteration 1, `a` *is* `__ref_1`. Iteration 2 calls the helper, which clears `out`
— that same buffer — **before** `OpAppendVector(out, v)` reads `v`, which is also
`a`. The argument is destroyed by the call that is about to read it.

Confirmed directly by making the helper report what it receives, with `a` pre-seeded
to `[7, 8]`:

```
before loop len=2
  helper got len=2 adding 10      ← correct
  after assign len=3
  helper got len=0 adding 20      ← a was emptied between iterations
  helper got len=0 adding 30
after loop len=1
```

## The invariant

**A variable may alias a call's return buffer only while that buffer cannot be
re-entered before the variable is read again.** A loop body violates this
unconditionally; straight-line code satisfies it by construction (one buffer per
call site).

Equivalently, at the failing chokepoint: *the assignment target must not alias the
return buffer of a call that also takes the target as an argument.*

## The boundary matrix (17 probes, `tests/probes/h7-retbuf-alias/`)

Every expectation hand-computed; all appends are 3 elements onto an empty vector.

| # | shape | result | verdict |
|---|---|---|---|
| 01 | loop + helper, `a = f(a,k)` | **1** | ✗ the report's case |
| 02 | same statements, no loop (sequential) | 3 | ✓ distinct buffer per site |
| 03 | loop + inline `a += [k]` (no call) | 3 | ✓ no buffer involved |
| 04 | loop over a **range** instead of a vector | **1** | ✗ not iterable-specific |
| 05 | loop, via temp: `b = f(a,k); a = b` | 3 | ✓ `a` gets a real copy |
| 06 | helper mutates its param: `v += [x]; v` | **0** | ✗ worse — appends into the cleared buffer |
| 07 | helper builds a **fresh** vector, no aliasing of `v` | **1** | ✗ fault is caller-side, not callee-side |
| 08 | `vector<text>` elements | **1** | ✗ not element-type-specific |
| 09 | contents, not length | `[30]` | only the last survives |
| 10 | iteration counter alongside | `iters=3, len=1` | ✓ loop runs; accumulation is lost |
| 11 | 4× sequential self-assign, no loop | 4 | ✓ |
| 12 | loop of ONE iteration | 1 | ✓ (consistent — nothing to lose yet) |
| 13 | `a` pre-seeded `[7,8]` | **1** | ✗ even the initial content is destroyed |
| 14 | helper prints what it received | `len=0` from iter 2 | the direct observation |
| 15 | loop with **two different** call sites | 6 | ✓ each site has its own buffer |
| 16 | `while` loop instead of `for` | **1** | ✗ any re-entry, not `for` |
| 17 | target is a **struct field** (`b.items = f(b.items,k)`) | 3 | ✓ field assignment copies |

15 and 17 are the two cells that pin the mechanism: the bug needs *the same buffer
re-entered* while a live variable still aliases it, and it disappears the moment the
target owns its storage.

## Why the existing ownership tooling is blind to it

`LOFT_POISON=1`, `LOFT_STORES=warn` and `LOFT_NATIVE_LEAK_CHECK=1` all emit **zero**
diagnostics on the failing probe. That is not a gap in their implementation — the
buffer is never *freed*, so there is nothing for a poison-on-free detector or a leak
census to see. It is **cleared while a live alias points at it**, which no current
instrument models.

**This is the tooling gap worth closing first**, because it makes the whole class
visible instead of this one instance: warn (under a flag, ratcheted from a pinned
baseline) when `OpClearVector` / the buffer-reset ops target a store that a live
variable's type currently depends on. That check would have named this bug from the
first run, and would catch every sibling — any op that resets a buffer under a live
alias, not just the return-buffer case.

## Why this is not fixed here

The chokepoint sits in the return-buffer machinery spread across
`parser/control.rs` (12.3k lines), `scopes.rs` (7.9k), `use_analysis.rs` (3.1k) and
`parser/expressions.rs` (4.0k). Two candidate fixes:

1. **Copy instead of alias when the target is also an argument.** Narrow and
   matches the failing family exactly; costs one vector copy in the sequential case
   that currently aliases safely.
2. **Allocate the buffer per ITERATION rather than per call site** when the site is
   inside a loop. Keeps the alias optimisation, and is the more honest reading of
   the invariant — but it touches buffer allocation, which the leak analysis and the
   NRVO collapse both depend on.

Choosing between them is a design act on the ownership substrate
([OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md)), not a patch. Landing either blind
risks re-introducing the leak class the buffer machinery exists to prevent — the
matrix says what is broken, it does not yet say which of these keeps every other
invariant intact.

**Suggested order:** the alias-aware clear warning above (it validates whichever fix
lands), then fix 1 behind a flag with the matrix as the gate, then flip the default
once green on both backends.

## Consumer workaround (works today)

Assign through a temp — probe 05, verified:

```loft
for k in items { b = add_i(a, k); a = b; }
```

A struct field target (probe 17) also works.
