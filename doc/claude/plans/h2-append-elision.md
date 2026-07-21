# H2 — `vec += [f(temp)]` in a loop drops every element but the first

> **Status: diagnosed, not fixed.** Interpreter only; `--native` is correct, so it is
> also a backend divergence. Silent — exit 0, no diagnostic, wrong data.
> Reported by the crawler consumer (`LOFT-HANDOFF` H2) on toolchain 2026.7.2 as the
> wider form of **loft#496**, which is CLOSED — so either that fix was too narrow or
> this is a regression of it. Probes: `h2-append-elision/probes/`.

## Symptom

```loft
for i in 0..3 { d = pick(t, i); out += [mk(d)]; }
```

Every appended element after the first reads back with **all fields null** — the whole
record, not just its text fields. A store leak accompanies it
(`1 stores not freed at program exit`).

## The boundary — it needs FOUR things at once

Measured on both backends (`probes/`, hand-computed expectations):

| probe | shape | interpret | native |
|---|---|---|---|
| `p1_callres_loop` | the reported shape | **`1 null null`** | `1 2 3` |
| `p5_twoargs` | same, extra scalar arg | **`1 null null`** | `1 2 3` |
| `p2_single` | one iteration, no loop | `1` | `1` |
| `p3_straight` | two appends straight-line, temp reassigned | `1 2` | `1 2` |
| `p4_literal_tmp` | temp from a LITERAL, not a call | `1 2 3` | `1 2 3` |
| `p6_via_local` | `e = mk(d); out += [e]` | `1 2 3` | `1 2 3` |
| `p7_append_tmp` | `out += [d]` — append the temp itself | `1 2 3` | `1 2 3` |
| `p8_no_tmp` | `out += [mk(pick(t,i))]` — no temp at all | `1 2 3` | `1 2 3` |

So it requires **all four** of: a **loop** (`p3` straight-line is fine) · the temp
assigned from a **call** (`p4` literal is fine) · passed **by value into another call**
(`p7` appending the temp is fine) · whose result is appended **directly**
(`p6` via a local is fine).

`p3` passing while `p1` fails is the key cell: the difference is the loop's
per-iteration scope exit, not the reassignment.

## Localised — the append adopts a store the loop then frees

`loft introspect` on the failing shape beside its working sibling (`p1` vs `p6`),
loop body only:

| | `p6` (works) | `p1` (fails) |
|---|---|---|
| after the `mk` call | **`CopyRecord(data, to, tp=66)`** | *absent* |
| `FreeRef` in the loop body | 1 | **2** |

The via-local path **copies** the returned record into the vector element. The direct
append instead **adopts** the callee's returned store — and the loop's scope exit
still frees it, so every element but the last-appended points at freed memory. The
extra `FreeRef` is that free; the leak warning is its mirror image.

## The invariant

> **A value appended to a container must be owned by that container when the
> appending scope exits.** Either the append copies, or it takes ownership AND the
> scope-exit free for that store is suppressed. Adopting without suppressing is the
> defect.

The adopt is a legitimate optimisation — `p8` shows it working when there is no temp,
and the codebase already reasons about it (`returns_borrowed_view()`,
`body_adopts_call` in `parser/operators.rs`). The bug is the missing half of the
transfer, and only in a loop, where the per-iteration free fires.

## Candidate sites

1. The element-append path that chooses adopt-vs-copy for a **call-result** element —
   `p6`/`p7` reach the copy, `p1` does not.
2. Loop scope-exit free emission (`scopes.rs`): the adopted store's free should be
   suppressed once ownership moves into the container.

## Fix options, in preference order

1. **Suppress the scope free when the append adopts.** Correct and allocation-free,
   but it must be exact — suppressing one free too many turns silent corruption into
   a silent leak, which is why this needs the ownership analysis rather than a patch.
2. **Copy on a call-result append** (make `p1` emit what `p6` emits). Obviously
   correct and one decision site, at the cost of a record copy per append — the copy
   the via-local form already pays today.
3. Reject the shape — not acceptable; it is ordinary code.

## Validation

`probes/` on **both** backends, hand-computed. `p1`/`p5` must flip to `1 2 3`; every
other cell must stay green (they are the "already correct" side and one of them,
`p8`, depends on the adopt still happening). Plus: no new store-leak warning under
`LOFT_STORES=warn --interpret`, and `loft#496`'s own reproducer re-verified, since
this is its wider form.
