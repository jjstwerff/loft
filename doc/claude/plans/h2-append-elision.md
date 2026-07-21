# H2 — `vec += [f(temp)]` in a loop drops every element but the first

> **Status: root cause NARROWED, not fixed — theory 3 falsified by measurement
> (§ FALSIFIED). The lifetimes-tool attribution upgrade that did it is LANDED.** Interpreter only; `--native` is correct, so it is
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

## Refined — it is the NRVO return buffer, freed once per iteration

The first theory (a double free from the copy's `0x8000` source-free bit plus the
lift's scope free) was **tested and wrong**: clearing the bit leaves the IR with a
single free and the bug unchanged. Reverted.

The IR names the real shape:

```
fn n_mk(d: D, __retbuf: E) -> E          // the result is written into a CALLER buffer
185[136]: Database(var[112], db_tp=65)   // that buffer is allocated ONCE, outside the loop
  __lift_1 = n_mk(d, __ref_3);           // __lift_1 ALIASES the buffer
  OpCopyRecord(__lift_1, _elm_1, 66);
  OpFreeRef(__lift_1);                   // ... and frees it EVERY iteration
```

`scopes.rs`'s `scan_args` lifts the call result into `__lift_N` so scope exit frees
it — correct when the temp owns a fresh store, wrong here, because the store is the
enclosing scope's NRVO return buffer. After iteration 1 the buffer is freed; later
iterations write into freed memory, which by then has been reallocated as a vector
element record — so each iteration corrupts what the previous one appended. That is
exactly why every element **but the last-appended** reads null.

It also explains the four-way boundary: no loop → no reuse (`p3`); a literal temp →
no NRVO call (`p4`); an intermediate local → the result lands in a distinct store
(`p6`); no temp → nothing to lift (`p8`).

**Corrected invariant:** a `__lift_N` temp may free its store only if it OWNS it. A
lift that aliases a caller-provided return buffer must not emit a free — the buffer's
owner is the enclosing scope.

**Fix site:** `scopes.rs` — the lift must recognise an NRVO-buffer-aliasing result and
skip the scope free (or bind without taking ownership). Neither fix option in the
previous section survives: copying more does not help, because the corruption is the
buffer free, not the copy.

## FALSIFIED — the per-iteration free is NOT the defect

The § Refined theory above (theory 3) said the bug is the per-iteration free of the
NRVO buffer. **The lifetimes-tool upgrade below falsifies it.** The check flags exactly
that shape, and it fires on **four** cells — `p1` and `p5` (broken) but also
`p4_literal_tmp` and `p8_no_tmp`, which **produce `1 2 3` on both backends**. Identical
IR shape, opposite outcomes, so the free alone cannot be the cause.

Captured beside its working sibling (`probes/`, loop bodies only), the real
discriminator is visible in one line:

| | free emitted at scope exit |
|---|---|
| `p6_via_local` (works) | `OpFreeRefIfDistinct(e, __ref_3)` — **guarded** |
| `p1`/`p4`/`p5`/`p8` | `OpFreeRef(__lift_N)` — **bare** |

So the guard mechanism **already exists** and the named-local spelling gets it; the
compiler-generated `__lift_N` path does not. `p4`/`p8` share the bare free and are
**latent** — nothing recycles the freed slot there — which is why they pass today.

**Corrected invariant:** a binding that may alias a caller-provided `__retbuf` must
free through the distinct-guard, never unconditionally. `p6` is the proof it is the
intended protocol.

**Not the fix site (measured, both reverted):** the two witness-pairing arms in
`scopes.rs::scan_set` (@P378(a) `witness_buffer`, ~:2846, and the @PLN85
`paired_witness` arm, ~:2918). Routing lifts into either moved **no IR at all** —
both sit inside `if adopts_fresh_store`, which is false for this callee, and the
@PLN85 arm additionally requires `function.is_argument(ov)` while `__lift_N` is a
local. The emission site for a lift's scope-exit free is upstream of both and is where
the next session should start.

## The lifetimes tool should have caught this

`--show-ownership` reported nothing useful for this program, and `LOFT_STORES=warn`
reported only the downstream leak. The shape is exactly what the inspector exists
for — **a store with two owners, freed by the inner one while the outer still uses
it** — so it belongs in the tool:

- flag a `__lift_N` (or any temp) whose store is also reachable from an enclosing
  owner, i.e. an alias of a caller-provided `__retbuf`;
- report frees inside a loop body of a store allocated OUTSIDE it — a per-iteration
  free of a once-allocated store is almost always this bug;
- surface it in `--show-ownership` output rather than only as a downstream leak
  warning, which names the symptom (`Def×4 not freed`) and not the cause.

That is the attribution upgrade the engineering-rigor skill calls for: the diagnostic
reported an effect with no cause, and three theories were needed to get from the
symptom to the buffer. With the above, the tool would have said it directly.

## The invariant (superseded — see § Refined)

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
