<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# H7 — a variable aliases a call's return buffer, and a loop re-enters it

> **Status: FIXED (2026-07-27).** The oracle is 17/17 on both backends with zero
> regressions, and the full suite is green. The fix is the **buffer rotation**
> described in § The fix that shipped — the recommended swap, realised between two
> BUFFERS rather than between the target and its buffer, which is what keeps the
> ownership plan unchanged. Probes: `tests/probes/h7-retbuf-alias/`; regression
> guard: `tests/scripts/h7-loop-retbuf-alias.loft`.

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

## The TEXT path already solves this — copy its shape

The strongest lead, found by probing an axis the first matrix missed: **text
accumulation is CORRECT.**

```loft
fn add_t(s: text, x: text) -> text { out = s; out = "{out}{x}"; out }
for k in ["a","b","c"] { s = add_t(s, k); }     // "abc" — right
```

Identical shape, identical loop, identical self-assignment — and it works. The
difference is in the signature the compiler generates:

| | hidden buffer param | callee's first act |
|---|---|---|
| **vector** (broken) | `out: vector<integer>` — an OWNED buffer | `OpClearVector(out)` then `OpAppendVector(out, v)` |
| **text** (correct) | `out: &text` — a REFERENCE | `out = s` — rebinds, never clears |

So the text path never destroys its argument because it never clears a buffer the
argument might alias: it *rebinds* the reference. That is the same move as the
recommended swap, one level up — which is real evidence the swap is implementable in
this substrate rather than a novel invention, because the text return path is
already doing it in production.

**Next reader: start here.** Read how the text retbuf is declared `&text` and
rebound (`parser/control.rs` § the `__tret` / hidden-`&text` retbuf machinery,
searchable via `hidden \`&text\` retbuf`), and ask whether a vector retbuf can take
the same reference-and-rebind treatment. That is a far better starting point than
the caller-side divert hunt below.

## Two axes the first matrix missed

| probe | result | what it rules in/out |
|---|---|---|
| `30_text_accum` — same shape, `text` | **correct** | NOT all heap returns — vector-specific (see above) |
| `31_no_self_ref` — `a = mk(k)`, target not an argument | **correct** | the fault REQUIRES the target to be an argument, so the fix predicate is exactly P390's `ir_mentions_var` |
| `32_read_before_call` — `seen += len(a)` before the call | **2, want 3** | an independent observable of the same fault; a fix must flip it |

## Implementation notes — what is built, and the blocker

**The oracle is built** (`tests/probes/h7-retbuf-alias/oracle.sh`). Expectations are
hand-computed, not captured from a reference run, so it cannot inherit the bug. It
separates two failure kinds, which matters for a fix that trades one against the
other:

- a `BROKEN` cell going correct → **FIXED**
- an `OK` cell going wrong → **REGRESSION** (exit 2, louder than an unfixed cell)

Baseline on `main`: **7 correct, 7 wrong, 0 regressions**.

**The precedent to copy is @P390** (`parser/expressions.rs`): `v = v[a..b]` is the
same hazard for slices — the RHS reads the variable the assignment is about to
`OpClearVector`. It is detected with `ir_mentions_var(code, var_nr)` and resolved by
routing through a temp. `ir_mentions_var` already exists and already answers exactly
the question H7 needs.

**The blocker, measured.** Instrumenting the P390 site shows an asymmetry:

| form | reaches `parse_assign_op`? |
|---|---|
| `b = add_i(a, k)` (works today) | **yes** — `target=b … mentions_target=false` |
| `a = add_i(a, k)` (the bug) | **no** — never appears |

So the failing shape is **diverted before the generic assignment path**, onto the
route that makes the call deliver into the target directly. That divert site is the
chokepoint the fix belongs at, and it is not `nrvo_collapse_tail_set` (callee-side,
fn-body tail) — it is a caller-side path I have not yet located in `control.rs`.

Finding it is the next concrete step, and it is a *search*, not a design question:
instrument the caller-side call-emission path the same way (`LOFT_PROBE_H7`-style,
one `eprintln` at each candidate) and find where a `Set(target, Call)` acquires the
retbuf dependency without passing through `parse_assign_op`.

## Efficiency — why the obvious fix is the wrong one, measured

| approach | allocations (N=200) | per-assignment | verdict |
|---|---|---|---|
| per-iteration buffer | **N+3** | O(1) | ✗ turns 4 allocs into 200 |
| copy through a temp (the P390 idiom) | 5 | **O(len)** | ✗ **quadratic** — measured below |
| **swap the two store handles** | **4** | **O(1)** | ✓ recommended |

Today's broken code already uses **4 stores for 200 iterations** (`LOFT_STORES=timeline`),
so allocation is O(1) in the loop — any per-iteration allocation is a large regression.

The copy-through-temp idiom is exactly the consumer workaround, and it is quadratic.
Measured on the interpreter, integer elements:

```
N=4000   0.13s
N=8000   0.41s   (3.2x)
N=16000  1.57s   (3.8x)      ← doubling N ~quadruples the time
```

For `vector<text>` or struct elements the per-copy cost is higher still, so the copy
fix would be worse than these numbers suggest.

**The swap:** at the assignment, exchange the store handles instead of binding the
target as a view. The target takes the buffer's store (the fresh result); the buffer
takes the target's previous store, which is stale and cleared on the next call
anyway. The two ping-pong for the life of the loop — zero new allocations, zero
copies beyond what the callee already performs, O(1) per assignment. It also
*subtracts* machinery: each name owns whatever handle it holds, so both are freed
once at scope end and the `skip_free` special-casing the alias needs disappears.

**The claim to falsify first:** the lifetime analysis models the target as statically
*depending on* the buffer; a swap makes ownership alternate at runtime. Cheapest
probe — hand-write the swapped IR for the failing loop, run under
`LOFT_STORES=timeline` + the leak census, and check frees stay at 2 with no leak. If
that holds, the ownership-pair model is sound and the implementation is mechanical.

## The fix that shipped

`parser/expressions.rs::rotate_loop_retbufs`, a post-pass over each function body
once it is parsed. It finds, inside a `Loop`, an assignment whose value is a user
call that both writes into a hidden `__ref_N` buffer AND reads the variable being
assigned — the two halves the matrix proved necessary (probe 31: without the
self-read, the alias is harmless). It mints a SECOND buffer for that site and
rotates the pair after the call:

```
a = n_add_i(a, k, __ref_1);      // a now holds __ref_1's store
OpPutRef(__ref_1, __ref_2);      // the site's next call writes the OTHER store
OpPutRef(__ref_2, a);            // which parks the live one out of reach
```

Two existing `OpPutRef`s — no new opcode. The stores ping-pong, so a call never
clears the store the live target holds.

**Why between two buffers, not between the target and its buffer.** The swap this
document recommended exchanges `a` with `__ref_1`. That fails on the first
iteration: `a` is a FIELD inside `__vdb_1`, not a whole store, so parking it in a
buffer makes the scope-exit `OpFreeRef(__ref_1)` free `__vdb_1` a second time. Two
buffers are the same handle KIND, so rotating them is ownership-symmetric: each is
an ordinary `__ref_N` work-ref, each gets its usual plain free, and the target —
still a view — is still freed by nobody. Nothing in the free plan changed, which
is what the "claim to falsify first" was worried about.

**Measured.** Allocations stay constant (5 for N=200, as before the fix — the
extra buffer replaces one the old code was reusing); frees balance; no leak on
either backend. The residual quadratic in the timing is the probe program's own
`out = v` copy, not the fix: it adds two DbRef copies per iteration.

**Vector targets only.** A struct (`Reference`) target is excluded, and the
exclusion is load-bearing: native's assignment-from-call FREES the store the
target held (`{ let _old = var_s; var_s = f(…); if _old != var_s { OpFreeRef(_old) } }`),
so a parked struct handle is dangling by the next iteration —
`tests/scripts/303-ref-reassign-free.loft` caught exactly that, going `v=null` on
native while the interpreter stayed green. That same free is also why structs do
not NEED the rotation. H6's shape (a struct transform chained through a loop) is
correct on both backends.

## Why this was not fixed at localization time

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
